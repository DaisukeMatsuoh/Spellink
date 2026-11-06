// Command spellink-bridge relays questions from the Spellink Supernote plugin to the locally
// installed Claude Code CLI and returns the answers as handwriting strokes.
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io/fs"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
	"sync"
	"syscall"
	"time"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/claude"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/config"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/glyphbuild"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/history"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/ink"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/lantls"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/protocol"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/relay"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/server"
	"github.com/coder/websocket"
)

// version is set at release build time with -ldflags "-X main.version=…".
var version = "dev"

const usage = `usage:
  spellink-bridge serve                 run the bridge (builds glyph data on first run)
  spellink-bridge pair [out.json]       print the connection file for the plugin
  spellink-bridge build-glyphs          (re)download KanjiVG/Hershey and build glyph data
  spellink-bridge relay [-listen :8780] run a relay that lets plugins reach bridges behind NAT
  spellink-bridge preview TEXT OUT.png [charSize] [maxPages]
  spellink-bridge version`

func main() {
	args := os.Args[1:]
	if len(args) == 0 {
		fmt.Println(usage)
		return
	}
	var err error
	switch args[0] {
	case "serve":
		err = serve()
	case "pair":
		out := ""
		if len(args) > 1 {
			out = args[1]
		}
		err = pair(out)
	case "build-glyphs":
		_, err = buildGlyphs()
	case "preview":
		err = preview(args[1:])
	case "relay":
		err = runRelay(args[1:])
	case "version":
		fmt.Println(version)
	default:
		fmt.Fprintln(os.Stderr, usage)
		os.Exit(2)
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "error:", err)
		os.Exit(1)
	}
}

func clamp(v, lo, hi int) int { return max(lo, min(hi, v)) }

func dir0() string {
	d, _ := config.Dir()
	return d
}

func buildGlyphs() (*ink.Font, error) {
	file, err := config.GlyphFile()
	if err != nil {
		return nil, err
	}
	fmt.Println("Downloading KanjiVG and Hershey fonts…")
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()
	font, err := glyphbuild.Build(ctx, http.DefaultClient)
	if err != nil {
		return nil, err
	}
	var buf bytes.Buffer
	if err := font.Write(&buf); err != nil {
		return nil, err
	}
	if err := os.MkdirAll(filepath.Dir(file), 0o700); err != nil {
		return nil, err
	}
	if err := os.WriteFile(file, buf.Bytes(), 0o644); err != nil {
		return nil, err
	}
	fmt.Printf("Wrote %s: %d glyphs, %d KB\n", file, len(font.Glyphs), buf.Len()/1024)
	return font, nil
}

// loadFont reads the glyph data, building it the first time.
func loadFont() (*ink.Font, error) {
	file, err := config.GlyphFile()
	if err != nil {
		return nil, err
	}
	font, err := ink.LoadFont(file)
	if errors.Is(err, fs.ErrNotExist) || errors.Is(err, ink.ErrFormat) {
		return buildGlyphs()
	}
	return font, err
}

func serve() error {
	cfg, err := config.Load()
	if err != nil {
		return err
	}
	font, err := loadFont()
	if err != nil {
		return err
	}
	runner, err := claude.NewRunner(cfg.ClaudeExecutable, cfg.Model)
	if err != nil {
		return err
	}
	logger := log.New(os.Stdout, "", log.LstdFlags)
	var notes *history.Store
	if cfg.HistoryChars() > 0 {
		histDir, err := config.HistoryDir()
		if err != nil {
			return err
		}
		notes = &history.Store{Dir: histDir}
	}
	var settingsMu sync.Mutex
	deps := server.Deps{
		Name: cfg.Name, Version: version, Token: cfg.Token, Font: font,
		Ask: runner.Ask, History: notes, HistoryChars: cfg.HistoryChars(), Runs: server.NewRuns(),
		Report: func(peer string, data any) {
			if err := config.AppendReport(peer, data); err != nil {
				logger.Printf("report: %v", err)
			}
		},
		Log: logger,
		Settings: func() protocol.Settings {
			settingsMu.Lock()
			defer settingsMu.Unlock()
			return protocol.Settings{
				ShowProgress: cfg.ShowProgress, LineTilt: cfg.LineTiltOn(), TiltDegrees: *cfg.TiltDegrees,
				ThinkingAnimation: cfg.ThinkingAnimationOn(),
				FullWidthSpacing:  cfg.FullSpacing(), HalfWidthSpacing: cfg.HalfSpacing(),
			}
		},
		SetSettings: func(s protocol.Settings) error {
			settingsMu.Lock()
			defer settingsMu.Unlock()
			cfg.ShowProgress = s.ShowProgress
			cfg.LineTilt = &s.LineTilt
			cfg.ThinkingAnimation = &s.ThinkingAnimation
			full := clamp(s.FullWidthSpacing, protocol.SpacingMin, protocol.FullSpacingMax)
			half := clamp(s.HalfWidthSpacing, protocol.SpacingMin, protocol.HalfSpacingMax)
			cfg.FullWidthSpacing, cfg.HalfWidthSpacing = &full, &half
			logger.Printf("settings changed: showProgress=%v lineTilt=%v animation=%v spacing=%d/%d",
				s.ShowProgress, s.LineTilt, s.ThinkingAnimation, full, half)
			return config.Save(cfg)
		},
	}
	handler := server.Handler(deps)
	srv := &http.Server{
		Addr:              fmt.Sprintf(":%d", cfg.Port),
		Handler:           handler,
		ReadHeaderTimeout: 10 * time.Second,
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	if cfg.RelayURL != "" {
		go relay.Connect(ctx, cfg.RelayURL, cfg.Token, func(ctx context.Context, c *websocket.Conn, peer string) {
			server.ServeConn(ctx, deps, c, peer, true)
		}, logger)
	}

	// Direct wss:// on the LAN (a plugin refuses cleartext and untrusted certificates).
	var lanSrv *http.Server
	if cfg.LAN != nil {
		cfg.LAN.StorageDir = filepath.Join(dir0(), "certs")
		tlsCfg, err := lantls.Setup(ctx, cfg.LAN)
		if err != nil {
			logger.Printf("lan wss disabled: %v", err)
		} else {
			lanSrv = &http.Server{Addr: ":8766", Handler: handler, TLSConfig: tlsCfg, ReadHeaderTimeout: 10 * time.Second}
			go func() {
				if err := lanSrv.ListenAndServeTLS("", ""); !errors.Is(err, http.ErrServerClosed) {
					logger.Printf("lan wss: %v", err)
				}
			}()
			logger.Printf("lan wss: wss://%s:8766%s", cfg.LAN.Host, protocol.WSPath)
		}
	}

	dir, _ := config.Dir()
	logger.Printf("spellink-bridge %s %q listening on port %d (config: %s)", version, cfg.Name, cfg.Port, dir)
	for _, u := range pairURLs(cfg) {
		logger.Printf("  %s", u)
	}
	go func() {
		<-ctx.Done()
		shutdown, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = srv.Shutdown(shutdown)
		if lanSrv != nil {
			_ = lanSrv.Shutdown(shutdown)
		}
	}()
	if err := srv.ListenAndServe(); !errors.Is(err, http.ErrServerClosed) {
		return err
	}
	return nil
}

// candidateURLs lists every LAN IPv4 address plus the mDNS host name; the plugin tries them all.
func candidateURLs(port int) []string {
	var hosts []string
	ifaces, _ := net.Interfaces()
	for _, ifc := range ifaces {
		if ifc.Flags&net.FlagUp == 0 || ifc.Flags&net.FlagLoopback != 0 {
			continue
		}
		addrs, _ := ifc.Addrs()
		for _, a := range addrs {
			if ipn, ok := a.(*net.IPNet); ok && ipn.IP.To4() != nil {
				hosts = append(hosts, ipn.IP.String())
			}
		}
	}
	if h, err := os.Hostname(); err == nil {
		hosts = append(hosts, h+".local")
	}
	urls := make([]string, len(hosts))
	for i, h := range hosts {
		urls[i] = fmt.Sprintf("ws://%s%s", net.JoinHostPort(h, strconv.Itoa(port)), protocol.WSPath)
	}
	return urls
}

// pairURLs is what the plugin tries in parallel: LAN addresses first, then configured extras.
func pairURLs(cfg *config.Config) []string {
	urls := append(candidateURLs(cfg.Port), cfg.ExtraURLs...)
	if cfg.RelayURL != "" {
		urls = append(urls, relay.ClientURL(cfg.RelayURL, cfg.Token))
	}
	if cfg.LAN != nil && cfg.LAN.Host != "" {
		urls = append([]string{fmt.Sprintf("wss://%s:8766%s", cfg.LAN.Host, protocol.WSPath)}, urls...)
	}
	return urls
}

// runRelay serves the relay. Put it behind TLS (e.g. a reverse proxy or Cloudflare Tunnel)
// when it is reachable from the internet.
func runRelay(args []string) error {
	fs := flag.NewFlagSet("relay", flag.ContinueOnError)
	listen := fs.String("listen", ":8780", "address to listen on")
	if err := fs.Parse(args); err != nil {
		return err
	}
	logger := log.New(os.Stdout, "", log.LstdFlags)
	srv := &http.Server{Addr: *listen, Handler: relay.NewServer(logger).Handler(), ReadHeaderTimeout: 10 * time.Second}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	go func() {
		<-ctx.Done()
		shutdown, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = srv.Shutdown(shutdown)
	}()
	logger.Printf("spellink relay %s listening on %s", version, *listen)
	if err := srv.ListenAndServe(); !errors.Is(err, http.ErrServerClosed) {
		return err
	}
	return nil
}

// pair prints or writes the connection file the plugin reads (MyStyle/Spellink/bridges.json).
func pair(out string) error {
	cfg, err := config.Load()
	if err != nil {
		return err
	}
	b, _ := json.MarshalIndent(map[string]any{
		"bridges": []any{map[string]any{"name": cfg.Name, "token": cfg.Token, "urls": pairURLs(cfg)}},
	}, "", "  ")
	if out == "" {
		fmt.Println(string(b))
		return nil
	}
	if err := os.WriteFile(out, append(b, '\n'), 0o600); err != nil {
		return err
	}
	fmt.Println("Wrote", out)
	return nil
}
