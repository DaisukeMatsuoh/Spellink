// Command cloute-bridge relays questions from the Cloute Supernote plugin to the locally
// installed Claude Code CLI and returns the answers as handwriting strokes.
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
	"syscall"
	"time"

	"github.com/DaisukeMatsuoh/Cloute/bridge/internal/claude"
	"github.com/DaisukeMatsuoh/Cloute/bridge/internal/config"
	"github.com/DaisukeMatsuoh/Cloute/bridge/internal/glyphbuild"
	"github.com/DaisukeMatsuoh/Cloute/bridge/internal/ink"
	"github.com/DaisukeMatsuoh/Cloute/bridge/internal/protocol"
	"github.com/DaisukeMatsuoh/Cloute/bridge/internal/server"
)

// version is set at release build time with -ldflags "-X main.version=…".
var version = "dev"

const usage = `usage:
  cloute-bridge serve                 run the bridge (builds glyph data on first run)
  cloute-bridge pair [out.json]       print the connection file for the plugin
  cloute-bridge build-glyphs          (re)download KanjiVG/Hershey and build glyph data
  cloute-bridge preview TEXT OUT.png [charSize] [maxPages]
  cloute-bridge version`

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
	if errors.Is(err, fs.ErrNotExist) {
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
	handler := server.Handler(server.Deps{
		Name: cfg.Name, Version: version, Token: cfg.Token, Font: font,
		Ask: runner.Ask,
		Report: func(peer string, data any) {
			if err := config.AppendReport(peer, data); err != nil {
				logger.Printf("report: %v", err)
			}
		},
		Log: logger,
	})
	srv := &http.Server{
		Addr:              fmt.Sprintf(":%d", cfg.Port),
		Handler:           handler,
		ReadHeaderTimeout: 10 * time.Second,
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	go func() {
		<-ctx.Done()
		shutdown, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = srv.Shutdown(shutdown)
	}()

	dir, _ := config.Dir()
	logger.Printf("cloute-bridge %s %q listening on port %d (config: %s)", version, cfg.Name, cfg.Port, dir)
	for _, u := range candidateURLs(cfg.Port) {
		logger.Printf("  %s", u)
	}
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

// pair prints or writes the connection file the plugin reads (MyStyle/Cloute/bridges.json).
func pair(out string) error {
	cfg, err := config.Load()
	if err != nil {
		return err
	}
	b, _ := json.MarshalIndent(map[string]any{
		"bridges": []any{map[string]any{"name": cfg.Name, "token": cfg.Token, "urls": candidateURLs(cfg.Port)}},
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
