package lantls

import "testing"

func TestValid(t *testing.T) {
	cases := map[string]*Config{
		"lan.host is empty":            {},
		"lan.cloudflareToken is empty": {Host: "rock.lan.example.com"},
		"":                             {Host: "rock.lan.example.com", CloudflareToken: "t"},
	}
	for want, c := range cases {
		if got := c.Valid(); got != want {
			t.Errorf("%+v: got %q, want %q", c, got, want)
		}
	}
}

func TestBaseDomain(t *testing.T) {
	for in, want := range map[string]string{
		"rock.lan.example.com": "example.com",
		"example.com":          "example.com",
		"a.b.c.example.org":    "example.org",
	} {
		if got := baseDomain(in); got != want {
			t.Errorf("%s: got %s, want %s", in, got, want)
		}
	}
}

func TestValidChecksWhatGoesIntoPublicDNS(t *testing.T) {
	ok := Config{Host: "rock.lan.example.com", CloudflareToken: "t"}
	if m := ok.Valid(); m != "" {
		t.Fatalf("a good setting: %s", m)
	}
	for name, mutate := range map[string]func(*Config){
		"a URL as the host":          func(c *Config) { c.Host = "https://rock.example.com" },
		"a host with a port":         func(c *Config) { c.Host = "rock.example.com:8766" },
		"a wildcard host":            func(c *Config) { c.Host = "*.example.com" },
		"a single label":             func(c *Config) { c.Host = "rock" },
		"a trailing dot":             func(c *Config) { c.Host = "rock.example.com." },
		"a host with a space":        func(c *Config) { c.Host = "rock example.com" },
		"a label that starts with -": func(c *Config) { c.Host = "-rock.example.com" },
		"a zone that is not a name":  func(c *Config) { c.Zone = "exa mple.com" },
		"a host outside its zone":    func(c *Config) { c.Zone = "other.org" },
		"a public IP":                func(c *Config) { c.IP = "8.8.8.8" },
		"an IPv6 address":            func(c *Config) { c.IP = "fe80::1" },
		"a non-address":              func(c *Config) { c.IP = "rock" },
	} {
		c := ok
		mutate(&c)
		if c.Valid() == "" {
			t.Errorf("%s must be refused", name)
		}
	}
	good := ok
	good.Zone, good.IP = "example.com", "192.168.2.180"
	if m := good.Valid(); m != "" {
		t.Errorf("a zone the host is inside, and a private IPv4: %s", m)
	}
}
