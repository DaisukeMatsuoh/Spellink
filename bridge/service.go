package main

import (
	"fmt"
	"os"
	"os/exec"
	"os/user"
	"path/filepath"
	"runtime"
	"strings"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/service"
)

// serviceCmd: `spellink-bridge service install|uninstall|status [--dry-run]`.
func serviceCmd(args []string) error {
	dry := false
	var rest []string
	for _, a := range args {
		if a == "--dry-run" || a == "-n" {
			dry = true
		} else {
			rest = append(rest, a)
		}
	}
	if len(rest) != 1 {
		return fmt.Errorf("usage: spellink-bridge service install|uninstall|status [--dry-run]")
	}
	exe, err := os.Executable()
	if err != nil {
		return err
	}
	if exe, err = filepath.EvalSymlinks(exe); err != nil {
		return err
	}
	// A binary from `go run` lives in a temporary directory and would be gone after a restart.
	if strings.Contains(exe, "go-build") || strings.HasPrefix(exe, os.TempDir()+string(os.PathSeparator)) {
		return fmt.Errorf("%s is a temporary build: install the program first (for example to ~/.local/bin) and run the service command from there", exe)
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return err
	}
	env := service.Env{GOOS: runtime.GOOS, Home: home, UID: os.Getuid(), Exe: exe}
	if u, err := user.Current(); err == nil {
		env.User = u.Username
	}
	if p, err := exec.LookPath("claude"); err == nil {
		env.ClaudeDir = filepath.Dir(p)
	}
	plan, err := service.For(env)
	if err != nil {
		return err
	}
	write := func(path, content string) error {
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			return err
		}
		return os.WriteFile(path, []byte(content), 0o644)
	}
	remove := func(path string) error {
		if err := os.Remove(path); err != nil && !os.IsNotExist(err) {
			return err
		}
		return nil
	}
	return service.Do(rest[0], plan, dry, service.ExecRunner, write, remove, os.Stdout)
}
