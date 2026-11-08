package service

import (
	"bytes"
	"errors"
	"strings"
	"testing"
)

func env(goos string) Env {
	return Env{GOOS: goos, Home: "/home/pat", User: "pat", UID: 1000, Exe: "/home/pat/.local/bin/spellink-bridge", ClaudeDir: "/opt/claude/bin"}
}

func TestLinuxUnitRunsTheBridgeByItsFullPathWithClaudeOnThePath(t *testing.T) {
	p, err := For(env("linux"))
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		"ExecStart=/home/pat/.local/bin/spellink-bridge serve",
		"Restart=on-failure",
		"Environment=PATH=/opt/claude/bin:/home/pat/.local/bin:/usr/local/bin:/usr/bin:/bin",
		"WantedBy=default.target",
	} {
		if !strings.Contains(p.Content, want) {
			t.Errorf("the unit lacks %q:\n%s", want, p.Content)
		}
	}
	if p.File != "/home/pat/.config/systemd/user/spellink-bridge.service" {
		t.Errorf("a per-user file, not a system one: %s", p.File)
	}
	joined := ""
	for _, c := range append(append([][]string{}, p.Install...), p.Uninstall...) {
		joined += strings.Join(c, " ") + "\n"
	}
	if strings.Contains(joined, "sudo") || strings.Contains(joined, "/etc/") {
		t.Errorf("nothing outside the user's account, no root: %s", joined)
	}
	if !strings.Contains(joined, "loginctl enable-linger pat") {
		t.Errorf("without linger it would stop when the user logs out: %s", joined)
	}
}

func TestThePathHasNoRepeatsAndSurvivesNoClaude(t *testing.T) {
	e := env("linux")
	e.ClaudeDir = "/home/pat/.local/bin" // already one of the usual directories
	p, _ := For(e)
	if strings.Count(p.Content, "/home/pat/.local/bin:") != 1 {
		t.Errorf("a directory appears once: %s", p.Content)
	}
	e.ClaudeDir = ""
	p, _ = For(e)
	if !strings.Contains(p.Content, "Environment=PATH=/home/pat/.local/bin:") {
		t.Errorf("without claude found, the usual directories: %s", p.Content)
	}
}

func TestMacAgentIsAValidPlistWithEscapedValues(t *testing.T) {
	e := env("darwin")
	e.Home, e.Exe = "/Users/pat", "/Users/pat/bin & tools/spellink-bridge"
	p, err := For(e)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(p.Content, "<string>/Users/pat/bin &amp; tools/spellink-bridge</string>") {
		t.Errorf("XML special characters must be escaped:\n%s", p.Content)
	}
	for _, want := range []string{"app.spellink.bridge", "<key>RunAtLoad</key><true/>", "<key>KeepAlive</key><true/>"} {
		if !strings.Contains(p.Content, want) {
			t.Errorf("the agent lacks %q", want)
		}
	}
	if p.File != "/Users/pat/Library/LaunchAgents/app.spellink.bridge.plist" {
		t.Error(p.File)
	}
	if got := strings.Join(p.Install[1], " "); got != "launchctl bootstrap gui/1000 /Users/pat/Library/LaunchAgents/app.spellink.bridge.plist" {
		t.Error(got)
	}
}

func TestWindowsUsesAScheduledTaskAndQuotesThePath(t *testing.T) {
	e := env("windows")
	e.Exe = `/Program Files/Spellink/spellink-bridge.exe`
	p, err := For(e)
	if err != nil {
		t.Fatal(err)
	}
	if p.File != "" {
		t.Error("a scheduled task keeps no file of ours")
	}
	got := strings.Join(p.Install[0], "\x00")
	if !strings.Contains(got, "/TR\x00\""+e.Exe+"\" serve") || !strings.Contains(got, "ONLOGON") || !strings.Contains(got, "LIMITED") {
		t.Errorf("limited rights, at logon, the path quoted: %q", p.Install[0])
	}
}

func TestPathsThatCannotBeWrittenSafelyAreRefused(t *testing.T) {
	for name, mutate := range map[string]func(*Env){
		"a relative path":          func(e *Env) { e.Exe = "spellink-bridge" },
		"an empty path":            func(e *Env) { e.Exe = "" },
		"a line break in the path": func(e *Env) { e.Exe = "/a\nExecStart=/bin/evil" },
		"a quote in the path":      func(e *Env) { e.Exe = `/a"b` },
		"a line break in claude's": func(e *Env) { e.ClaudeDir = "/x\n[Service]" },
	} {
		e := env("linux")
		mutate(&e)
		if _, err := For(e); err == nil {
			t.Errorf("%s must be refused", name)
		}
	}
	if _, err := For(env("plan9")); err == nil || !strings.Contains(err.Error(), "serve") {
		t.Errorf("an unsupported system says what to do instead: %v", err)
	}
}

type recorder struct {
	ran     []string
	failOn  string
	written map[string]string
	removed []string
}

func (r *recorder) run(args []string) ([]byte, error) {
	c := strings.Join(args, " ")
	r.ran = append(r.ran, c)
	if r.failOn != "" && strings.Contains(c, r.failOn) {
		return []byte("it did not work"), errors.New("exit status 1")
	}
	return []byte("active (running)"), nil
}
func (r *recorder) write(p, c string) error {
	if r.written == nil {
		r.written = map[string]string{}
	}
	r.written[p] = c
	return nil
}
func (r *recorder) remove(p string) error { r.removed = append(r.removed, p); return nil }

func TestInstallWritesTheFileThenRunsTheCommandsInOrder(t *testing.T) {
	p, _ := For(env("linux"))
	r := &recorder{}
	var out bytes.Buffer
	if err := Do("install", p, false, r.run, r.write, r.remove, &out); err != nil {
		t.Fatal(err)
	}
	if r.written[p.File] != p.Content {
		t.Error("the unit must be written")
	}
	if len(r.ran) != 3 || !strings.HasPrefix(r.ran[0], "systemctl --user daemon-reload") || !strings.HasPrefix(r.ran[2], "loginctl enable-linger") {
		t.Errorf("commands: %v", r.ran)
	}
	if !strings.Contains(out.String(), "spellink-bridge check") {
		t.Errorf("the person is told how to check it worked: %s", out.String())
	}
}

func TestDryRunChangesNothingAndShowsEverything(t *testing.T) {
	p, _ := For(env("linux"))
	r := &recorder{}
	var out bytes.Buffer
	for _, action := range []string{"install", "uninstall", "status"} {
		if err := Do(action, p, true, r.run, r.write, r.remove, &out); err != nil {
			t.Fatal(err)
		}
	}
	if len(r.ran) != 0 || len(r.written) != 0 || len(r.removed) != 0 {
		t.Fatalf("a dry run must do nothing: ran %v wrote %v removed %v", r.ran, r.written, r.removed)
	}
	for _, want := range []string{"write " + p.File, "ExecStart=", "run   systemctl --user enable --now spellink-bridge", "remove " + p.File} {
		if !strings.Contains(out.String(), want) {
			t.Errorf("a dry run shows %q", want)
		}
	}
}

func TestAFailingCommandStopsTheInstallAndNamesItself(t *testing.T) {
	p, _ := For(env("linux"))
	r := &recorder{failOn: "enable --now"}
	err := Do("install", p, false, r.run, r.write, r.remove, &bytes.Buffer{})
	if err == nil || !strings.Contains(err.Error(), "enable --now") || !strings.Contains(err.Error(), "it did not work") {
		t.Fatalf("the error says which command and why: %v", err)
	}
	if len(r.ran) != 2 {
		t.Errorf("nothing runs after the one that failed: %v", r.ran)
	}
}

func TestFirstInstallOnAMacIgnoresThatThereWasNothingToTakeDown(t *testing.T) {
	p, _ := For(env("darwin"))
	r := &recorder{failOn: "bootout"}
	if err := Do("install", p, false, r.run, r.write, r.remove, &bytes.Buffer{}); err != nil {
		t.Fatalf("a first install has nothing loaded to boot out: %v", err)
	}
	if len(r.ran) != 2 || !strings.Contains(r.ran[1], "bootstrap") {
		t.Errorf("it goes on to load the agent: %v", r.ran)
	}
}

func TestUninstallTakesItDownAndRemovesTheFile(t *testing.T) {
	p, _ := For(env("linux"))
	r := &recorder{}
	if err := Do("uninstall", p, false, r.run, r.write, r.remove, &bytes.Buffer{}); err != nil {
		t.Fatal(err)
	}
	if len(r.removed) != 1 || r.removed[0] != p.File {
		t.Errorf("removed %v", r.removed)
	}
	if !strings.HasPrefix(r.ran[0], "systemctl --user disable --now") {
		t.Errorf("stop it before deleting its file: %v", r.ran)
	}
}

func TestStatusShowsTheSystemsAnswerAndAnUnknownActionIsRefused(t *testing.T) {
	p, _ := For(env("linux"))
	var out bytes.Buffer
	if err := Do("status", p, false, (&recorder{}).run, nil, nil, &out); err != nil || !strings.Contains(out.String(), "active (running)") {
		t.Fatalf("%v %q", err, out.String())
	}
	if err := Do("restart", p, false, nil, nil, nil, &out); err == nil {
		t.Fatal("an unknown action must be refused")
	}
}
