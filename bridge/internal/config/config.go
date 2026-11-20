// Package config stores the bridge settings and diagnostic reports in the user's config directory
// (~/.config/spellink-bridge on Linux, ~/Library/Application Support/spellink-bridge on macOS).
package config

import (
	"cmp"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/lantls"
	"io/fs"
	"math"
	mrand "math/rand/v2"
	"os"
	"path/filepath"
	"slices"
	"strings"
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
	// Provider chooses the AI the bridge asks (ADR 0015). Without it: Claude Code, with the
	// claudeExecutable and model above (kept so existing config files keep working).
	Provider *ProviderConfig `json:"provider,omitempty"`
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
	// TiltDegrees is this agent's handwriting slope (positive rises to the right), picked at
	// random the first time so every agent has its own habit. Edit to taste.
	TiltDegrees *float64 `json:"tiltDegrees,omitempty"`
	// ThinkingAnimation defaults to off (it saves and re-reads the note, which can stall the note
	// app on large notes); nil means "not set".
	ThinkingAnimation *bool `json:"thinkingAnimation,omitempty"`
	// AvoidOverwriting defaults to off (it saves and reads the whole page for every question).
	AvoidOverwriting bool `json:"avoidOverwriting,omitempty"`
	// BusyNotice defaults to on; nil means "not set".
	BusyNotice *bool `json:"busyNotice,omitempty"`
	// Letter spacing in percent (see protocol.Settings); nil means the default.
	FullWidthSpacing *int `json:"fullWidthSpacing,omitempty"`
	HalfWidthSpacing *int `json:"halfWidthSpacing,omitempty"`
	// Timeouts bound how long the AI may stay silent (bridge-run spec, R13). Zero fields use the
	// defaults.
	Timeouts *Timeouts `json:"timeouts,omitempty"`
	// History controls what the bridge remembers of each note's conversation (ADR 0012).
	History *History `json:"history,omitempty"`
	// LAN enables direct wss:// at home with a Let's Encrypt certificate for a name that points
	// at this machine's LAN address (internal/lantls). Empty disables it.
	LAN *lantls.Config `json:"lan,omitempty"`
	// Pairing tunes how devices are added (ADR 0016). Zero fields use the defaults.
	Pairing *Pairing `json:"pairing,omitempty"`
}

// Pairing holds the limits of the pairing ceremony.
type Pairing struct {
	// WindowSec is how long the owner has to confirm a code once a device shows it (default 90).
	WindowSec int `json:"windowSec,omitempty"`
	// TTLHours is how long a device's first secret stays valid before it is paired (default 24).
	TTLHours int `json:"ttlHours,omitempty"`
	// PluginPackage is the public Spellink.snplg that `device package` adds a device's setup to.
	PluginPackage string `json:"pluginPackage,omitempty"`
}

const (
	defaultWindowSec = 90
	defaultTTLHours  = 24
	minWindowSec     = 10
	maxWindowSec     = 600
	maxTTLHours      = 24 * 30
)

// PairingWindow is how long the owner has to confirm a code.
func (c *Config) PairingWindow() time.Duration {
	if c.Pairing == nil || c.Pairing.WindowSec == 0 {
		return defaultWindowSec * time.Second
	}
	return time.Duration(c.Pairing.WindowSec) * time.Second
}

// PairingTTL is how long a device's first secret stays valid.
func (c *Config) PairingTTL() time.Duration {
	if c.Pairing == nil || c.Pairing.TTLHours == 0 {
		return defaultTTLHours * time.Hour
	}
	return time.Duration(c.Pairing.TTLHours) * time.Hour
}

// PluginPackage is the configured base package, or the empty string.
func (c *Config) PluginPackage() string {
	if c.Pairing == nil {
		return ""
	}
	return c.Pairing.PluginPackage
}

// ProviderConfig names an AI and how to reach it. Authentication is the provider's own: a CLI
// uses its own login, an API provider will keep its key here, on this machine only.
type ProviderConfig struct {
	// Kind is "claude-code" (the only one so far).
	Kind string `json:"kind,omitempty"`
	// Model is passed to the AI (e.g. "haiku"); empty uses its default.
	Model string `json:"model,omitempty"`
	// Executable is the CLI to run, for a CLI provider; empty means its usual name.
	Executable string `json:"executable,omitempty"`
}

// KnownProviders are the values `provider.kind` may have.
var KnownProviders = []string{"claude-code"}

// ProviderSettings returns the effective kind, model and executable: provider.* wins over the
// older top-level claudeExecutable / model.
func (c *Config) ProviderSettings() (kind, model, executable string) {
	kind, model, executable = "claude-code", c.Model, c.ClaudeExecutable
	if p := c.Provider; p != nil {
		kind = cmp.Or(p.Kind, kind)
		model = cmp.Or(p.Model, model)
		executable = cmp.Or(p.Executable, executable)
	}
	return kind, model, cmp.Or(executable, "claude")
}

// Timeouts are in seconds.
type Timeouts struct {
	// FirstOutputSec is how long the model may take to show its first sign of work after the
	// process started (default 10; measured: about 1.5 s).
	FirstOutputSec int `json:"firstOutputSec,omitempty"`
	// GapSec is the longest silence once it works (default 10): nothing at all, thinking included.
	GapSec int `json:"gapSec,omitempty"`
	// TotalSec bounds the whole answer (default 30). The silence limits above already stop an AI
	// that has stopped; this only ends one that keeps trickling. A real answer takes under 10 s.
	TotalSec int `json:"totalSec,omitempty"`
}

const (
	defaultFirstOutputSec = 10
	defaultGapSec         = 10
	defaultTotalSec       = 30
	maxTimeoutSec         = 3600
)

// ProviderTimeouts returns the effective silence limits.
func (c *Config) ProviderTimeouts() (first, gap, total time.Duration) {
	t := c.Timeouts
	if t == nil {
		t = &Timeouts{}
	}
	pick := func(v, def int) time.Duration {
		if v == 0 {
			v = def
		}
		return time.Duration(v) * time.Second
	}
	return pick(t.FirstOutputSec, defaultFirstOutputSec), pick(t.GapSec, defaultGapSec), pick(t.TotalSec, defaultTotalSec)
}

// History is how much of a note's earlier conversation is handed to the model with a new
// question. Everything is kept on disk regardless; these only bound what is sent.
type History struct {
	// ContextChars is the size (characters of questions plus answers) of the most recent
	// conversation sent along; 0 means the default, negative turns the memory off.
	ContextChars int `json:"contextChars,omitempty"`
	// Compaction is what happens to conversation older than ContextChars. Only "none" (it is
	// simply not sent) exists so far; "summarize" is reserved for when long notes need it.
	Compaction string `json:"compaction,omitempty"`
}

const defaultHistoryChars = 8000

// HistoryChars is the size of conversation to send, or 0 when the memory is off.
func (c *Config) HistoryChars() int {
	switch {
	case c.History == nil || c.History.ContextChars == 0:
		return defaultHistoryChars
	case c.History.ContextChars < 0:
		return 0
	}
	return c.History.ContextChars
}

func (c *Config) validate(file string) error {
	if kind, _, _ := c.ProviderSettings(); !slices.Contains(KnownProviders, kind) {
		return fmt.Errorf("%s: provider.kind %q is not one this version knows (%s)", file, kind, strings.Join(KnownProviders, ", "))
	}
	if c.Port < 0 || c.Port > 65535 {
		return fmt.Errorf("%s: port %d is not a valid port (1-65535)", file, c.Port)
	}
	if t := c.Timeouts; t != nil {
		for name, v := range map[string]int{"firstOutputSec": t.FirstOutputSec, "gapSec": t.GapSec, "totalSec": t.TotalSec} {
			if v < 0 || v > maxTimeoutSec {
				return fmt.Errorf("%s: timeouts.%s %d must be between 0 (default) and %d seconds", file, name, v, maxTimeoutSec)
			}
		}
	}
	if p := c.Pairing; p != nil {
		if p.WindowSec != 0 && (p.WindowSec < minWindowSec || p.WindowSec > maxWindowSec) {
			return fmt.Errorf("%s: pairing.windowSec %d must be 0 (default) or between %d and %d seconds", file, p.WindowSec, minWindowSec, maxWindowSec)
		}
		if p.TTLHours < 0 || p.TTLHours > maxTTLHours {
			return fmt.Errorf("%s: pairing.ttlHours %d must be between 0 (default) and %d", file, p.TTLHours, maxTTLHours)
		}
	}
	if h := c.History; h != nil && h.Compaction != "" && h.Compaction != "none" {
		return fmt.Errorf("%s: history.compaction %q is not supported yet (only \"none\")", file, h.Compaction)
	}
	return nil
}

// ensureTilt gives a new agent its writing habit: mostly gentle, sometimes more pronounced.
func (c *Config) ensureTilt() bool {
	if c.TiltDegrees != nil {
		return false
	}
	d := math.Round(max(-2.5, min(2.5, mrand.NormFloat64()*1.0))*10) / 10
	c.TiltDegrees = &d
	return true
}

const (
	defaultFullSpacing = 100
	defaultHalfSpacing = 110
)

// ThinkingAnimationOn reports the animation setting, off unless turned on.
func (c *Config) ThinkingAnimationOn() bool {
	return c.ThinkingAnimation != nil && *c.ThinkingAnimation
}

func orDefault(v *int, def int) int {
	if v == nil {
		return def
	}
	return *v
}

func (c *Config) FullSpacing() int { return orDefault(c.FullWidthSpacing, defaultFullSpacing) }
func (c *Config) HalfSpacing() int { return orDefault(c.HalfWidthSpacing, defaultHalfSpacing) }

// BusyNoticeOn reports the busy-notice setting, on unless turned off.
func (c *Config) BusyNoticeOn() bool { return c.BusyNotice == nil || *c.BusyNotice }

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

func HistoryDir() (string, error) { return path("history") }
func GlyphFile() (string, error)  { return path("glyphs.bin") }
func ReportFile() (string, error) { return path("reports.jsonl") }
func ConfigFile() (string, error) { return path("config.json") }

// DevicesFile is the ledger of paired devices; AdminTokenFile holds the bearer token the
// command line uses to talk to the running bridge.
func DevicesFile() (string, error)    { return path("devices.json") }
func AdminTokenFile() (string, error) { return path("admin.token") }

// AdminToken returns the command line's token, creating it (mode 0600, 32 random bytes) the first time.
func AdminToken() (string, error) {
	file, err := AdminTokenFile()
	if err != nil {
		return "", err
	}
	if b, err := os.ReadFile(file); err == nil {
		tok := strings.TrimSpace(string(b))
		if len(tok) >= 32 {
			return tok, nil
		}
		// a token that is too short was not written by us: replace it rather than trust it
	} else if !errors.Is(err, fs.ErrNotExist) {
		return "", err
	}
	raw := make([]byte, 32)
	if _, err := rand.Read(raw); err != nil {
		return "", err
	}
	tok := base64.RawURLEncoding.EncodeToString(raw)
	if err := os.MkdirAll(filepath.Dir(file), 0o700); err != nil {
		return "", err
	}
	if err := os.WriteFile(file, []byte(tok+"\n"), 0o600); err != nil {
		return "", err
	}
	// WriteFile keeps the mode of a file that already exists (the short token we replace).
	if err := os.Chmod(file, 0o600); err != nil {
		return "", err
	}
	return tok, nil
}

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
		if err := c.validate(file); err != nil {
			return nil, err
		}
		if c.Port == 0 {
			c.Port = 8765
		}
		if c.ClaudeExecutable == "" {
			c.ClaudeExecutable = "claude"
		}
		if c.ensureTilt() {
			return c, Save(c)
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
	c.ensureTilt()
	if err := os.MkdirAll(filepath.Dir(file), 0o700); err != nil {
		return nil, err
	}
	out, _ := json.MarshalIndent(c, "", "  ")
	return c, os.WriteFile(file, append(out, '\n'), 0o600)
}

// maxReportBytes is when reports.jsonl is moved aside to reports.jsonl.1 (one old copy is kept).
const maxReportBytes = 5 << 20

// AppendReport adds one diagnostics entry (from the plugin) to reports.jsonl.
func AppendReport(peer string, data any) error {
	file, err := ReportFile()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(file), 0o700); err != nil {
		return err
	}
	// The log is written by whoever holds the token: keep it from growing without bound.
	if info, err := os.Stat(file); err == nil && info.Size() > maxReportBytes {
		if err := os.Rename(file, file+".1"); err != nil {
			return err
		}
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
