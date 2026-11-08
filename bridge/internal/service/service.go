// Package service makes the bridge start by itself when the computer does — the one thing a
// person has to do once so that "the PC is on" is enough. It only writes a per-user service file
// and runs the system's own commands for it (systemd user service, launchd agent, a Windows
// scheduled task): no root, nothing outside the user's account. Everything it would do can be
// shown first with --dry-run.
package service

import (
	"fmt"
	"io"
	"os/exec"
	"path/filepath"
	"strings"
)

// Env is what the service files are made from.
type Env struct {
	GOOS string
	Home string
	User string
	UID  int
	// Exe is the bridge's own path: the service runs this very file.
	Exe string
	// ClaudeDir is where the `claude` command lives ("" if not found). A service starts with a
	// minimal PATH, so that directory has to be named in the service itself.
	ClaudeDir string
}

// Plan is what install / uninstall / status do on one system.
type Plan struct {
	// File is the service file to write ("" when the system keeps no such file).
	File    string
	Content string
	// Commands, as argument lists, for each action.
	Install, Uninstall, Status [][]string
	// Notes are for the person, printed after an install.
	Notes []string
}

// pathList joins the directories the bridge's `claude` must be found in, without repeats.
func pathList(env Env, sep string, dirs ...string) string {
	seen := map[string]bool{}
	var out []string
	for _, d := range append([]string{env.ClaudeDir}, dirs...) {
		if d != "" && !seen[d] {
			seen[d] = true
			out = append(out, d)
		}
	}
	return strings.Join(out, sep)
}

// For returns the plan for env.GOOS.
func For(env Env) (Plan, error) {
	if env.Exe == "" || !filepath.IsAbs(env.Exe) {
		return Plan{}, fmt.Errorf("the bridge's path %q is not absolute", env.Exe)
	}
	if strings.ContainsAny(env.Exe, "\n\r\"") || strings.ContainsAny(env.ClaudeDir, "\n\r\"") {
		return Plan{}, fmt.Errorf("a path with a line break or a quote cannot go into a service file")
	}
	switch env.GOOS {
	case "linux":
		return linux(env), nil
	case "darwin":
		return darwin(env), nil
	case "windows":
		return windows(env), nil
	}
	return Plan{}, fmt.Errorf("no service support for %s: run `spellink-bridge serve` from your own startup mechanism", env.GOOS)
}

func linux(env Env) Plan {
	path := pathList(env, ":", filepath.Join(env.Home, ".local/bin"), "/usr/local/bin", "/usr/bin", "/bin")
	unit := fmt.Sprintf(`# Written by `+"`spellink-bridge service install`"+`; safe to delete with `+"`service uninstall`"+`.
[Unit]
Description=Spellink bridge (Supernote and an AI)
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=%s serve
Restart=on-failure
RestartSec=5s
Environment=PATH=%s

[Install]
WantedBy=default.target
`, env.Exe, path)
	return Plan{
		File:    filepath.Join(env.Home, ".config/systemd/user/spellink-bridge.service"),
		Content: unit,
		Install: [][]string{
			{"systemctl", "--user", "daemon-reload"},
			{"systemctl", "--user", "enable", "--now", "spellink-bridge"},
			// without this, a user service stops when the user logs out and starts only at login
			{"loginctl", "enable-linger", env.User},
		},
		Uninstall: [][]string{
			{"systemctl", "--user", "disable", "--now", "spellink-bridge"},
			{"systemctl", "--user", "daemon-reload"},
		},
		Status: [][]string{{"systemctl", "--user", "--no-pager", "status", "spellink-bridge"}},
		Notes:  []string{"Log: journalctl --user -u spellink-bridge", "Check the connection: spellink-bridge check"},
	}
}

func darwin(env Env) Plan {
	path := pathList(env, ":", filepath.Join(env.Home, ".local/bin"), "/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin")
	logFile := filepath.Join(env.Home, "Library/Logs/spellink-bridge.log")
	plist := fmt.Sprintf(`<?xml version="1.0" encoding="UTF-8"?>
<!-- Written by spellink-bridge service install; remove with service uninstall. -->
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>app.spellink.bridge</string>
  <key>ProgramArguments</key>
  <array>
    <string>%s</string>
    <string>serve</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key><string>%s</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>%s</string>
  <key>StandardErrorPath</key><string>%s</string>
</dict>
</plist>
`, xmlEscape(env.Exe), xmlEscape(path), xmlEscape(logFile), xmlEscape(logFile))
	file := filepath.Join(env.Home, "Library/LaunchAgents/app.spellink.bridge.plist")
	domain := fmt.Sprintf("gui/%d", env.UID)
	return Plan{
		File:    file,
		Content: plist,
		Install: [][]string{
			{"launchctl", "bootout", domain + "/app.spellink.bridge"}, // ignore "not loaded": a re-install replaces it
			{"launchctl", "bootstrap", domain, file},
		},
		Uninstall: [][]string{{"launchctl", "bootout", domain + "/app.spellink.bridge"}},
		Status:    [][]string{{"launchctl", "print", domain + "/app.spellink.bridge"}},
		Notes:     []string{"Log: " + logFile, "Check the connection: spellink-bridge check"},
	}
}

func windows(env Env) Plan {
	const task = "Spellink bridge"
	return Plan{
		Install: [][]string{{
			"schtasks", "/Create", "/F", "/SC", "ONLOGON", "/RL", "LIMITED", "/TN", task,
			"/TR", `"` + env.Exe + `" serve`,
		}, {"schtasks", "/Run", "/TN", task}},
		Uninstall: [][]string{{"schtasks", "/End", "/TN", task}, {"schtasks", "/Delete", "/F", "/TN", task}},
		Status:    [][]string{{"schtasks", "/Query", "/TN", task, "/V", "/FO", "LIST"}},
		Notes: []string{
			"The bridge starts when you log on, and runs while you are logged on.",
			"Windows is untested: if something does not work, run `spellink-bridge serve` in a terminal and tell us.",
		},
	}
}

func xmlEscape(s string) string {
	return strings.NewReplacer("&", "&amp;", "<", "&lt;", ">", "&gt;").Replace(s)
}

// Runner runs one command; tests replace it.
type Runner func(args []string) ([]byte, error)

// ExecRunner runs the command for real.
func ExecRunner(args []string) ([]byte, error) {
	return exec.Command(args[0], args[1:]...).CombinedOutput()
}

// Do carries out one action of the plan, writing what it does to w. With dry it only shows.
// write stores the service file (nil: not needed); remove deletes it.
func Do(action string, p Plan, dry bool, run Runner, write func(path, content string) error, remove func(path string) error, w io.Writer) error {
	var cmds [][]string
	switch action {
	case "install":
		cmds = p.Install
	case "uninstall":
		cmds = p.Uninstall
	case "status":
		cmds = p.Status
	default:
		return fmt.Errorf("unknown action %q (install, uninstall, status)", action)
	}
	if action == "install" && p.File != "" {
		fmt.Fprintf(w, "write %s\n", p.File)
		if dry {
			fmt.Fprintf(w, "----\n%s----\n", p.Content)
		} else if err := write(p.File, p.Content); err != nil {
			return fmt.Errorf("writing %s: %w", p.File, err)
		}
	}
	for _, c := range cmds {
		fmt.Fprintf(w, "run   %s\n", strings.Join(c, " "))
		if dry {
			continue
		}
		out, err := run(c)
		if s := strings.TrimSpace(string(out)); s != "" && action == "status" {
			fmt.Fprintln(w, s)
		}
		// Taking down something that is not there (launchd bootout on a first install) is fine.
		if err != nil && !(action == "install" && len(c) > 1 && c[1] == "bootout") {
			return fmt.Errorf("%s: %w: %s", strings.Join(c, " "), err, strings.TrimSpace(string(out)))
		}
	}
	if action == "uninstall" && p.File != "" {
		fmt.Fprintf(w, "remove %s\n", p.File)
		if !dry {
			if err := remove(p.File); err != nil {
				return err
			}
		}
	}
	if action == "install" {
		for _, n := range p.Notes {
			fmt.Fprintln(w, n)
		}
	}
	return nil
}
