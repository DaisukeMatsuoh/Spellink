// Package lantls gives the bridge a publicly trusted TLS certificate for a name that points at
// its LAN address, so the Supernote plugin (which refuses cleartext and untrusted certificates)
// can connect directly over wss:// at home.
//
// How: the user owns a domain on Cloudflare. The bridge publishes an A record
// (e.g. rock.lan.example.com → 192.168.2.180) and obtains a Let's Encrypt certificate with the
// DNS-01 challenge, so no inbound port ever has to be opened. Certificates renew automatically.
package lantls

import (
	"context"
	"crypto/tls"
	"fmt"
	"net"
	"net/netip"
	"time"

	"github.com/caddyserver/certmagic"
	"github.com/libdns/cloudflare"
	"github.com/libdns/libdns"
)

// Config is the user's setting (config.json "lan").
type Config struct {
	// Host is the DNS name to publish, e.g. "rock.lan.example.com". Its zone must be on Cloudflare.
	Host string `json:"host"`
	// CloudflareToken is an API token with Zone → DNS → Edit for that one zone.
	CloudflareToken string `json:"cloudflareToken"`
	// Email is optional (Let's Encrypt expiry notices).
	Email string `json:"email,omitempty"`
	// Zone overrides the zone detected from Host (two-label guess is wrong for e.g. co.jp).
	Zone string `json:"zone,omitempty"`
	// IP overrides the detected LAN address.
	IP string `json:"ip,omitempty"`
	// StorageDir holds certificates; defaults to the bridge's config directory.
	StorageDir string `json:"-"`
}

// Valid reports what is missing, or "" when usable.
func (c *Config) Valid() string {
	switch {
	case c == nil || c.Host == "":
		return "lan.host is empty"
	case c.CloudflareToken == "":
		return "lan.cloudflareToken is empty"
	}
	return ""
}

// PrivateIPv4 returns the first private LAN IPv4 of this machine.
func PrivateIPv4() (netip.Addr, error) {
	ifaces, _ := net.Interfaces()
	for _, ifc := range ifaces {
		if ifc.Flags&net.FlagUp == 0 || ifc.Flags&net.FlagLoopback != 0 {
			continue
		}
		addrs, _ := ifc.Addrs()
		for _, a := range addrs {
			if ipn, ok := a.(*net.IPNet); ok {
				if ip, ok := netip.AddrFromSlice(ipn.IP.To4()); ok && ip.IsPrivate() {
					return ip, nil
				}
			}
		}
	}
	return netip.Addr{}, fmt.Errorf("no private IPv4 address found")
}

// Setup publishes the A record and returns a TLS config whose certificate is obtained (and
// renewed) automatically. It blocks until the first certificate is ready.
func Setup(ctx context.Context, c *Config) (*tls.Config, error) {
	if msg := c.Valid(); msg != "" {
		return nil, fmt.Errorf("%s", msg)
	}
	ip := netip.Addr{}
	var err error
	if c.IP != "" {
		if ip, err = netip.ParseAddr(c.IP); err != nil {
			return nil, fmt.Errorf("lan.ip: %w", err)
		}
	} else if ip, err = PrivateIPv4(); err != nil {
		return nil, err
	}
	provider := &cloudflare.Provider{APIToken: c.CloudflareToken}

	if err := publishA(ctx, provider, c, ip); err != nil {
		return nil, fmt.Errorf("publishing A record: %w", err)
	}

	cm := certmagic.NewDefault()
	cm.Storage = &certmagic.FileStorage{Path: c.StorageDir}
	issuer := certmagic.NewACMEIssuer(cm, certmagic.ACMEIssuer{
		CA:     certmagic.LetsEncryptProductionCA,
		Email:  c.Email,
		Agreed: true,
		DNS01Solver: &certmagic.DNS01Solver{
			DNSManager: certmagic.DNSManager{DNSProvider: provider},
		},
	})
	cm.Issuers = []certmagic.Issuer{issuer}
	if err := cm.ManageSync(ctx, []string{c.Host}); err != nil {
		return nil, fmt.Errorf("certificate for %s: %w", c.Host, err)
	}
	return cm.TLSConfig(), nil
}

func publishA(ctx context.Context, p *cloudflare.Provider, c *Config, ip netip.Addr) error {
	zone := c.Zone
	if zone == "" {
		zone = baseDomain(c.Host)
	}
	name := libdns.RelativeName(c.Host+".", zone+".")
	_, err := p.SetRecords(ctx, zone+".", []libdns.Record{libdns.Address{Name: name, IP: ip, TTL: 2 * time.Minute}})
	return err
}

// baseDomain guesses the registrable zone as the last two labels. Set lan.zone for names under
// multi-label suffixes (co.jp, com.au…).
func baseDomain(host string) string {
	labels := splitLabels(host)
	if len(labels) <= 2 {
		return host
	}
	return labels[len(labels)-2] + "." + labels[len(labels)-1]
}

func splitLabels(s string) []string {
	var out []string
	start := 0
	for i := 0; i <= len(s); i++ {
		if i == len(s) || s[i] == '.' {
			out = append(out, s[start:i])
			start = i + 1
		}
	}
	return out
}
