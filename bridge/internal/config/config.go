// Package config stores the bridge settings and diagnostic reports in the user's config directory
// (~/.config/spellink-bridge on Linux, ~/Library/Application Support/spellink-bridge on macOS).
package config

import (
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"time"
)

type Config struct {
	// Name shown to the user in the plugin; defaults to the host name.
	Name string `json:"name"`
	Port int    `json:"port"`
	// Token is the shared secret the plugin must present. Unrelated to any Anthropic credential.
	Token            string `json:"token"`
	ClaudeExecutable string `json:"claudeExecutable"`
	// Model is passed to `claude --model` (e.g. "haiku"); empty uses Claude Code's default.
	Model string `json:"model,omitempty"`
	// ExtraURLs are additional addresses the plugin may try, such as a tunnel that reaches
	// this bridge from outside the LAN (e.g. "wss://bridge.example.com/v1").
	ExtraURLs []string `json:"extraUrls,omitempty"`
	// RelayURL is a Spellink relay (e.g. "wss://relay.example.com") the bridge registers with so
	// the plugin can reach it from outside the LAN (ADR 0010). Empty disables the relay.
	RelayURL string `json:"relayUrl,omitempty"`
	// ShowProgress is the plugin's debug view setting (protocol.Settings), changed from the plugin.
	ShowProgress bool `json:"showProgress,omitempty"`
	// LineTilt defaults to on; nil means "not set".
	LineTilt *bool `json:"lineTilt,omitempty"`
}

// LineTiltOn reports the line-tilt setting, on unless turned off.
func (c *Config) LineTiltOn() bool { return c.LineTilt == nil || *c.LineTilt }

// Save writes the config back (used when the plugin changes a setting).
func Save(c *Config) error {
	file, err := ConfigFile()
	if err != nil {
		return err
	}
	out, err := json.MarshalIndent(c, "", "  ")
	if err != nil {
		return err
	}
	tmp := file + ".tmp"
	if err := os.WriteFile(tmp, append(out, '\n'), 0o600); err != nil {
		return err
	}
	return os.Rename(tmp, file)
}

// Dir is where config.json, glyphs.bin and reports.jsonl live.
func Dir() (string, error) {
	base, err := os.UserConfigDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(base, "spellink-bridge"), nil
}

func path(name string) (string, error) {
	dir, err := Dir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, name), nil
}

func GlyphFile() (string, error)  { return path("glyphs.bin") }
func ReportFile() (string, error) { return path("reports.jsonl") }
func ConfigFile() (string, error) { return path("config.json") }

// Load reads the config, creating it with a fresh random token on first run.
func Load() (*Config, error) {
	file, err := ConfigFile()
	if err != nil {
		return nil, err
	}
	b, err := os.ReadFile(file)
	if err == nil {
		c := &Config{}
		if err := json.Unmarshal(b, c); err != nil {
			return nil, fmt.Errorf("%s: %w", file, err)
		}
		if c.Port == 0 {
			c.Port = 8765
		}
		if c.ClaudeExecutable == "" {
			c.ClaudeExecutable = "claude"
		}
		return c, nil
	}
	if !errors.Is(err, fs.ErrNotExist) {
		return nil, err
	}
	host, _ := os.Hostname()
	secret := make([]byte, 24)
	if _, err := rand.Read(secret); err != nil {
		return nil, err
	}
	c := &Config{
		Name: host, Port: 8765,
		Token:            base64.RawURLEncoding.EncodeToString(secret),
		ClaudeExecutable: "claude",
	}
	if err := os.MkdirAll(filepath.Dir(file), 0o700); err != nil {
		return nil, err
	}
	out, _ := json.MarshalIndent(c, "", "  ")
	return c, os.WriteFile(file, append(out, '\n'), 0o600)
}

// AppendReport adds one diagnostics entry (from the plugin) to reports.jsonl.
func AppendReport(peer string, data any) error {
	file, err := ReportFile()
	if err != nil {
		return err
	}
	f, err := os.OpenFile(file, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600)
	if err != nil {
		return err
	}
	defer f.Close()
	return json.NewEncoder(f).Encode(map[string]any{
		"at": time.Now().UTC().Format(time.RFC3339), "peer": peer, "data": data,
	})
}
