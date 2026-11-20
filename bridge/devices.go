package main

import (
	"context"
	"errors"
	"fmt"
	"log"
	"net/url"
	"os"
	"strings"
	"time"

	"github.com/coder/websocket"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/config"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/devicecli"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/devices"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/protocol"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/relay"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/server"
)

// deviceURLs are the addresses a device is told to use with `secret`: every wss:// address the bridge
// has (the plugin refuses anything else), each naming the device, and the relay room that the
// secret opens. No address at all is not an error here; the package builder says so.
func deviceURLs(cfg *config.Config, id, secret string) []string {
	var urls []string
	named := func(base string) string {
		u, err := url.Parse(base)
		if err != nil || u.Scheme != "wss" {
			return ""
		}
		q := u.Query()
		q.Set("d", id)
		u.RawQuery = q.Encode()
		return u.String()
	}
	if cfg.LAN != nil && cfg.LAN.Host != "" {
		urls = append(urls, named(fmt.Sprintf("wss://%s:8766%s", cfg.LAN.Host, protocol.WSPath)))
	}
	for _, u := range cfg.ExtraURLs {
		urls = append(urls, named(u))
	}
	if cfg.RelayURL != "" && secret != "" {
		urls = append(urls, relay.ClientURL(cfg.RelayURL, secret))
	}
	out := urls[:0]
	for _, u := range urls {
		if u != "" {
			out = append(out, u)
		}
	}
	return out
}

// roomsFor is the set of relay rooms to be registered in: one for every secret that may connect,
// each served as the device it belongs to.
func roomsFor(secrets []devices.Secret, serve func(id string) relay.ServeFunc) map[string]relay.ServeFunc {
	want := make(map[string]relay.ServeFunc, len(secrets))
	for _, s := range secrets {
		want[relay.RoomID(s.Secret)] = serve(s.ID)
	}
	return want
}

var errNoBasePackage = errors.New("set pairing.pluginPackage in config.json to the path of the plugin package (Spellink.snplg) to build setup files from")

// packageBuilder returns the function that builds a device's setup file from the plugin package.
func packageBuilder(cfg *config.Config) func(devices.PackageData) ([]byte, error) {
	return func(d devices.PackageData) ([]byte, error) {
		path := cfg.PluginPackage()
		if path == "" {
			return nil, errNoBasePackage
		}
		base, err := os.ReadFile(path)
		if err != nil {
			return nil, fmt.Errorf("reading %s: %w", path, err)
		}
		return devices.BuildPackage(base, d, deviceURLs(cfg, d.ID, d.B))
	}
}

// startDevices opens the ledger and starts what keeps it right: the timers, and the relay rooms
// that follow the devices. It returns what the server needs.
func startDevices(ctx context.Context, cfg *config.Config, logger *log.Logger, deps *server.Deps) error {
	file, err := config.DevicesFile()
	if err != nil {
		return err
	}
	adminToken, err := config.AdminToken()
	if err != nil {
		return err
	}
	sessions := server.NewSessions()
	var rooms *relay.RoomSet
	if cfg.RelayURL != "" {
		rooms = relay.NewRoomSet(ctx, cfg.RelayURL, logger)
	}
	var reg *devices.Registry
	syncRooms := func() {
		if rooms == nil || reg == nil {
			return
		}
		rooms.Sync(roomsFor(reg.ActiveSecrets(), func(id string) relay.ServeFunc {
			return func(ctx context.Context, c *websocket.Conn, peer string) {
				server.ServeDevice(ctx, *deps, c, peer, true, id)
			}
		}))
	}
	reg, skipped, err := devices.New(devices.Config{Window: cfg.PairingWindow(), TTL: cfg.PairingTTL(), Log: logger},
		devices.FileStore{Path: file}, devices.Hooks{
			Endpoints: func(id, k string) []string { return deviceURLs(cfg, id, k) },
			Changed:   syncRooms,
			CloseAll:  sessions.CloseDevice,
		})
	if err != nil {
		return fmt.Errorf("%s: %w", file, err)
	}
	for _, s := range skipped {
		logger.Printf("devices: %s: skipped %s", file, s)
	}
	deps.Devices, deps.Sessions = reg, sessions
	deps.Admin = &server.AdminDeps{Devices: reg, Token: adminToken, Package: packageBuilder(cfg)}
	syncRooms()
	go func() {
		t := time.NewTicker(time.Second)
		defer t.Stop()
		for {
			select {
			case <-t.C:
				reg.Tick()
			case <-ctx.Done():
				return
			}
		}
	}()
	return nil
}

// deviceCmd is `spellink-bridge device …`.
func deviceCmd(args []string) error {
	cfg, err := config.Load()
	if err != nil {
		return err
	}
	tok, err := config.AdminToken()
	if err != nil {
		return err
	}
	cwd, err := os.Getwd()
	if err != nil {
		return err
	}
	c := &devicecli.Client{Base: fmt.Sprintf("http://127.0.0.1:%d", cfg.Port), Token: tok}
	return devicecli.Run(c, args, os.Stdin, os.Stdout, cwd)
}

// describeURLs is for the log: the addresses a new device would be told.
func describeURLs(cfg *config.Config) string {
	u := deviceURLs(cfg, "<id>", "")
	if len(u) == 0 {
		return "none (add a wss:// address: extraUrls, lan or relayUrl)"
	}
	return strings.Join(u, ", ")
}
