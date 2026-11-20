package devicecli

import (
	"bytes"
	"io"
	"log"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/devices"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/ink"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/protocol"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/provider"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/server"
)

const token = "cli-test-token-0123456789abcdef0123456789"

// env is a real bridge (the server's admin door on a real ledger) and a client for it.
type env struct {
	t   *testing.T
	reg *devices.Registry
	srv *httptest.Server
	c   *Client
	dir string
}

func newEnv(t *testing.T) *env {
	t.Helper()
	reg, _, err := devices.New(devices.Config{}, devices.FileStore{Path: filepath.Join(t.TempDir(), "devices.json")}, devices.Hooks{})
	if err != nil {
		t.Fatal(err)
	}
	d := server.Deps{
		Name: "t", Token: "x", Font: &ink.Font{Em: 100, Glyphs: map[rune]ink.Glyph{}}, Provider: provider.Func("t", nil),
		Report: func(string, any) {}, Settings: func() protocol.Settings { return protocol.Settings{} }, SetSettings: func(protocol.Settings) error { return nil },
		Log: log.New(io.Discard, "", 0), Devices: reg,
		Admin: &server.AdminDeps{Devices: reg, Token: token, Package: func(p devices.PackageData) ([]byte, error) {
			return []byte("PACKAGE:" + p.Name + ":" + p.B), nil
		}},
	}
	srv := httptest.NewServer(server.Handler(d))
	t.Cleanup(srv.Close)
	return &env{t: t, reg: reg, srv: srv, c: &Client{Base: srv.URL, Token: token}, dir: t.TempDir()}
}

func (e *env) run(args ...string) (string, error) {
	return e.runWith("", args...)
}

func (e *env) runWith(stdin string, args ...string) (string, error) {
	var out bytes.Buffer
	err := Run(e.c, args, strings.NewReader(stdin), &out, e.dir)
	return out.String(), err
}

func (e *env) mustRun(args ...string) string {
	e.t.Helper()
	out, err := e.run(args...)
	if err != nil {
		e.t.Fatalf("%v: %v", args, err)
	}
	return out
}

func TestAddWritesAPrivateSetupFileAndSaysWhatToDoNext(t *testing.T) {
	e := newEnv(t)
	out := e.mustRun("add", "Nomad")
	path := filepath.Join(e.dir, "Spellink-Nomad.snplg")
	st, err := os.Stat(path)
	if err != nil || st.Mode().Perm() != 0o600 {
		t.Fatalf("file: %v %v", st, err)
	}
	data, _ := os.ReadFile(path)
	if !strings.HasPrefix(string(data), "PACKAGE:Nomad:") {
		t.Errorf("content %q", data)
	}
	for _, want := range []string{path, "Copy the file", "tap Pair", "device pair", "Keep it private"} {
		if !strings.Contains(out, want) {
			t.Errorf("output lacks %q:\n%s", want, out)
		}
	}
	// into another directory
	other := t.TempDir()
	e.mustRun("add", "Manta", other)
	if _, err := os.Stat(filepath.Join(other, "Spellink-Manta.snplg")); err != nil {
		t.Error(err)
	}
}

func TestAddRefusesWhatItCannotDo(t *testing.T) {
	e := newEnv(t)
	e.mustRun("add", "Nomad")
	if _, err := e.run("add", "Nomad"); err == nil || !strings.Contains(err.Error(), "already exists") {
		t.Errorf("duplicate: %v", err)
	}
	if _, err := e.run("add", ""); err == nil {
		t.Error("an empty name")
	}
	if _, err := e.run("add"); err == nil || !strings.Contains(err.Error(), "usage") {
		t.Errorf("no name: %v", err)
	}
	// two names that make the same file name: the second device is added, its file is not written over the first
	e.mustRun("add", "A B")
	_, err := e.run("add", "A_B")
	if err == nil || !strings.Contains(err.Error(), "device package") {
		t.Errorf("clash: %v", err)
	}
	if data, _ := os.ReadFile(filepath.Join(e.dir, "Spellink-A_B.snplg")); !strings.Contains(string(data), ":A B:") {
		t.Errorf("the first file was overwritten: %q", data)
	}
	// a directory that does not exist
	if _, err := e.run("add", "Third", filepath.Join(e.dir, "missing")); err == nil {
		t.Error("missing directory")
	}
}

func TestListShowsEachStateWithoutSecrets(t *testing.T) {
	e := newEnv(t)
	if out := e.mustRun("list"); !strings.Contains(out, "No devices yet") {
		t.Errorf("empty: %s", out)
	}
	e.mustRun("add", "issued")
	waiting, _ := e.reg.Add("waiting")
	e.reg.Request(waiting, 1, "11111111", &stubConn{})
	active, _ := e.reg.Add("active")
	e.reg.Request(active, 1, "22222222", &stubConn{})
	e.reg.Confirm("22222222")
	e.reg.Ack(active, 1)
	e.reg.ConnectK(active)
	revoked, _ := e.reg.Add("revoked")
	e.reg.Revoke(revoked)
	out := e.mustRun("list")
	for _, want := range []string{"issued", "waiting", "active", "revoked", "1 waiting for a code", "last seen", "setup file valid until"} {
		if !strings.Contains(out, want) {
			t.Errorf("list lacks %q:\n%s", want, out)
		}
	}
	for _, d := range e.reg.List() {
		b, _ := e.reg.Repack(d.ID)
		if b.B != "" && strings.Contains(out, b.B) {
			t.Error("a secret was printed")
		}
	}
}

type stubConn struct{ denied []string }

func (c *stubConn) Grant(string, []string) error { return nil }
func (c *stubConn) Deny(r string)                { c.denied = append(c.denied, r) }

func TestPairWithTheCodeOnTheCommandLineOrAskedFor(t *testing.T) {
	e := newEnv(t)
	id, _ := e.reg.Add("Nomad")
	e.reg.Request(id, 1, "12345678", &stubConn{})
	if out := e.mustRun("pair", "12345678"); !strings.Contains(out, `"Nomad" is being paired`) {
		t.Errorf("output: %s", out)
	}
	id2, _ := e.reg.Add("Manta")
	e.reg.Request(id2, 1, "87654321", &stubConn{})
	out, err := e.runWith("87654321\n", "pair")
	if err != nil || !strings.Contains(out, "Code shown on the device") || !strings.Contains(out, `"Manta"`) {
		t.Errorf("asked: %q %v", out, err)
	}
}

func TestPairExplainsEachWayItCanFail(t *testing.T) {
	e := newEnv(t)
	if _, err := e.run("pair", "12345678"); err == nil || !strings.Contains(err.Error(), "no device is showing a code") {
		t.Errorf("nobody waiting: %v", err)
	}
	id, _ := e.reg.Add("Nomad")
	e.reg.Request(id, 1, "12345678", &stubConn{})
	if _, err := e.run("pair", "00000000"); err == nil || !strings.Contains(err.Error(), "not accepted") || strings.Contains(err.Error(), "Nomad") {
		t.Errorf("wrong code: %v", err)
	}
	if _, err := e.run("pair", "12"); err == nil || !strings.Contains(err.Error(), "eight digits") {
		t.Errorf("short code: %v", err)
	}
	if _, err := e.runWith("", "pair"); err == nil || !strings.Contains(err.Error(), "no code") {
		t.Errorf("no input: %v", err)
	}
	if _, err := e.runWith("12345678", "pair"); err != nil { // the last line may have no newline
		t.Errorf("input without a newline: %v", err)
	}
}

func TestPackageWritesTheFileAgainOnlyOnce(t *testing.T) {
	e := newEnv(t)
	id, _ := e.reg.Add("Nomad")
	if out := e.mustRun("package", "Nomad"); !strings.Contains(out, "Spellink-Nomad.snplg") {
		t.Errorf("output: %s", out)
	}
	if _, err := e.run("package", "Nomad"); err == nil || !strings.Contains(err.Error(), "remove it first") {
		t.Errorf("second time: %v", err)
	}
	if _, err := e.run("package", id, t.TempDir()); err != nil { // by id, into another directory
		t.Error(err)
	}
	if _, err := e.run("package", "nobody"); err == nil || !strings.Contains(err.Error(), "no device named") {
		t.Errorf("unknown: %v", err)
	}
	if _, err := e.run("package"); err == nil {
		t.Error("usage")
	}
	e.reg.Revoke(id)
	if _, err := e.run("package", "Nomad", t.TempDir()); err == nil || !strings.Contains(err.Error(), "revoked") {
		t.Errorf("a revoked device has no package: %v", err)
	}
}

func TestRevokeReissueForget(t *testing.T) {
	e := newEnv(t)
	e.reg.Add("Nomad")
	if out := e.mustRun("reissue", "Nomad"); !strings.Contains(out, "pair it again") {
		t.Errorf("reissue: %s", out)
	}
	if out := e.mustRun("revoke", "Nomad"); !strings.Contains(out, "cannot connect any more") {
		t.Errorf("revoke: %s", out)
	}
	if _, err := e.run("revoke", "Nomad"); err == nil || !strings.Contains(err.Error(), "already revoked") {
		t.Errorf("twice: %v", err)
	}
	if out := e.mustRun("forget", "Nomad"); !strings.Contains(out, "Forgot") {
		t.Errorf("forget: %s", out)
	}
	if _, err := e.run("forget", "Nomad"); err == nil {
		t.Error("forgetting twice")
	}
	for _, c := range []string{"revoke", "reissue", "forget"} {
		if _, err := e.run(c); err == nil || !strings.Contains(err.Error(), "usage") {
			t.Errorf("%s without a name: %v", c, err)
		}
	}
}

func TestUsage(t *testing.T) {
	e := newEnv(t)
	for _, args := range [][]string{nil, {"frobnicate"}} {
		if _, err := e.run(args...); err == nil || !strings.Contains(err.Error(), "usage: spellink-bridge device") {
			t.Errorf("%v: %v", args, err)
		}
	}
}

func TestTheBridgeBeingDownOrRefusingTheTokenIsExplained(t *testing.T) {
	e := newEnv(t)
	e.srv.Close()
	if _, err := e.run("list"); err == nil || !strings.Contains(err.Error(), "is it running") {
		t.Errorf("down: %v", err)
	}
	e2 := newEnv(t)
	e2.c.Token = "wrong"
	if _, err := e2.run("list"); err == nil || !strings.Contains(err.Error(), "admin token") {
		t.Errorf("wrong token: %v", err)
	}
	if _, _, err := (&Client{Base: "http://[::1", Token: token}).do("GET", "/", nil); err == nil {
		t.Error("an unusable address")
	}
}

func TestAnAnswerItCannotReadIsReportedNotTrusted(t *testing.T) {
	garbage := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { io.WriteString(w, "not json") }))
	defer garbage.Close()
	c := &Client{Base: garbage.URL, Token: token}
	for _, args := range [][]string{{"list"}, {"add", "x"}, {"pair", "12345678"}, {"revoke", "x"}} {
		if err := Run(c, args, strings.NewReader(""), io.Discard, t.TempDir()); err == nil {
			t.Errorf("%v: an unreadable answer was accepted", args)
		}
	}
}

func TestFileNames(t *testing.T) {
	for name, want := range map[string]string{
		"Nomad": "Spellink-Nomad.snplg", "My Nomad!": "Spellink-My_Nomad.snplg", "../../etc/passwd": "Spellink-etc_passwd.snplg",
		"ノート": "Spellink-0123.snplg", "...": "Spellink-0123.snplg",
	} {
		if got := fileName(device{ID: "0123", Name: name}); got != want {
			t.Errorf("fileName(%q) = %q, want %q", name, got, want)
		}
	}
}

func TestAnAnswerThatIsCutOffIsAnError(t *testing.T) {
	cut := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Length", "100")
		io.WriteString(w, "short")
		conn, _, _ := w.(http.Hijacker).Hijack()
		conn.Close()
	}))
	defer cut.Close()
	if _, _, err := (&Client{Base: cut.URL, Token: token}).do("GET", "/", nil); err == nil {
		t.Error("a truncated answer must not be taken as complete")
	}
}
