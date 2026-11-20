package main

import (
	"archive/zip"
	"bytes"
	"context"
	"encoding/json"
	"io"
	"log"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/config"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/devices"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/ink"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/lantls"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/protocol"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/provider"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/relay"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/securechan"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/server"
)

func isolate(t *testing.T) {
	t.Helper()
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	t.Setenv("HOME", dir)
}

func TestDeviceURLsAreWssOnlyNameTheDeviceAndAddTheRelayRoom(t *testing.T) {
	cfg := &config.Config{
		LAN:       &lantls.Config{Host: "rock.lan.example.com"},
		ExtraURLs: []string{"wss://bridge.example.com/v1", "ws://plain.example/v1", "not a url \x7f", "wss://q.example/v1?x=1"},
		RelayURL:  "wss://relay.example.com/",
	}
	got := deviceURLs(cfg, "0123456789abcdef", "SECRET")
	want := []string{
		"wss://rock.lan.example.com:8766/v1?d=0123456789abcdef",
		"wss://bridge.example.com/v1?d=0123456789abcdef",
		"wss://q.example/v1?d=0123456789abcdef&x=1",
		relay.ClientURL("wss://relay.example.com/", "SECRET"),
	}
	if strings.Join(got, "\n") != strings.Join(want, "\n") {
		t.Errorf("got\n%s\nwant\n%s", strings.Join(got, "\n"), strings.Join(want, "\n"))
	}
	if got := deviceURLs(&config.Config{RelayURL: "wss://relay.example.com"}, "id", ""); len(got) != 0 {
		t.Errorf("a relay room needs a secret: %v", got)
	}
	if got := deviceURLs(&config.Config{}, "id", "S"); len(got) != 0 {
		t.Errorf("nothing configured: %v", got)
	}
	if !strings.Contains(describeURLs(&config.Config{}), "none") || !strings.Contains(describeURLs(cfg), "wss://bridge.example.com") {
		t.Error("describeURLs")
	}
}

func TestRoomsForGivesEverySecretItsRoomAndItsDevice(t *testing.T) {
	var served []string
	want := roomsFor([]devices.Secret{{ID: "a", Secret: "s1"}, {ID: "a", Secret: "s2", IsK: true}, {ID: "b", Secret: "s3"}},
		func(id string) relay.ServeFunc {
			return func(context.Context, *websocket.Conn, string) { served = append(served, id) }
		})
	if len(want) != 3 {
		t.Fatalf("rooms: %d", len(want))
	}
	for secret, id := range map[string]string{"s1": "a", "s2": "a", "s3": "b"} {
		want[relay.RoomID(secret)](nil, nil, "")
		if served[len(served)-1] != id {
			t.Errorf("room of %s served as %s", secret, served[len(served)-1])
		}
	}
}

func snplg(t *testing.T) string {
	t.Helper()
	var buf bytes.Buffer
	zw := zip.NewWriter(&buf)
	for _, f := range [][2]string{{"PluginConfig.json", "{}"}, {devices.BundleName, "var x=1;"}} {
		w, _ := zw.Create(f[0])
		io.WriteString(w, f[1])
	}
	zw.Close()
	path := filepath.Join(t.TempDir(), "Spellink.snplg")
	os.WriteFile(path, buf.Bytes(), 0o600)
	return path
}

func TestPackageBuilder(t *testing.T) {
	d := devices.PackageData{ID: "0123456789abcdef", Name: "Nomad", Gen: 1, Window: 90 * time.Second}
	if _, err := packageBuilder(&config.Config{})(d); err != errNoBasePackage {
		t.Errorf("no package configured: %v", err)
	}
	cfg := &config.Config{Pairing: &config.Pairing{PluginPackage: filepath.Join(t.TempDir(), "missing.snplg")}}
	if _, err := packageBuilder(cfg)(d); err == nil || !strings.Contains(err.Error(), "missing.snplg") {
		t.Errorf("missing file: %v", err)
	}
	cfg = &config.Config{Name: "Rock 5B", Pairing: &config.Pairing{PluginPackage: snplg(t)}, ExtraURLs: []string{"wss://bridge.example.com/v1"}}
	out, err := packageBuilder(cfg)(d)
	if err != nil || !bytes.Contains(out, []byte("PK")) {
		t.Fatalf("package: %v", err)
	}
	if got := bundleLine(t, out); !strings.Contains(got, `"name":"Rock 5B"`) || strings.Contains(got, "Nomad") {
		t.Errorf("the setup must name the bridge, not the device: %s", got)
	}
	// a bridge name that cannot be shown is replaced, not passed on
	cfg.Name = strings.Repeat("x", 65)
	out, err = packageBuilder(cfg)(d)
	if err != nil || !strings.Contains(bundleLine(t, out), `"name":"bridge"`) {
		t.Errorf("unusable bridge name: %v", err)
	}
	// a device that still has to pair needs an address to connect to
	d.B = strings.Repeat("A", 43)
	if _, err := packageBuilder(&config.Config{Pairing: cfg.Pairing})(d); err == nil {
		t.Error("no address configured")
	}
}

// bundleLine is the first line of the bundle inside a package.
func bundleLine(t *testing.T, pkg []byte) string {
	t.Helper()
	zr, err := zip.NewReader(bytes.NewReader(pkg), int64(len(pkg)))
	if err != nil {
		t.Fatal(err)
	}
	for _, f := range zr.File {
		if f.Name == devices.BundleName {
			rc, _ := f.Open()
			defer rc.Close()
			b, _ := io.ReadAll(rc)
			line, _, _ := strings.Cut(string(b), "\n")
			return line
		}
	}
	t.Fatal("no bundle")
	return ""
}

func testDeps() server.Deps {
	return server.Deps{
		Name: "t", Version: "0", Token: "tok", Font: &ink.Font{Em: 100, Glyphs: map[rune]ink.Glyph{}}, Provider: provider.Func("t", nil),
		Report: func(string, any) {}, Settings: func() protocol.Settings { return protocol.Settings{} },
		SetSettings: func(protocol.Settings) error { return nil }, Log: log.New(io.Discard, "", 0),
	}
}

func TestStartDevicesGivesTheServerALedgerAndTheAdminDoor(t *testing.T) {
	isolate(t)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	deps := testDeps()
	if err := startDevices(ctx, &config.Config{Port: 8765}, log.New(io.Discard, "", 0), &deps); err != nil {
		t.Fatal(err)
	}
	if deps.Devices == nil || deps.Sessions == nil || deps.Admin == nil || len(deps.Admin.Token) < 32 {
		t.Fatalf("deps: %+v", deps)
	}
	if _, err := deps.Devices.Add("Nomad"); err != nil {
		t.Fatal(err)
	}
	file, _ := config.DevicesFile()
	if st, err := os.Stat(file); err != nil || st.Mode().Perm() != 0o600 {
		t.Errorf("ledger file: %v %v", st, err)
	}
}

func TestStartDevicesRefusesALedgerItCannotRead(t *testing.T) {
	isolate(t)
	file, _ := config.DevicesFile()
	os.MkdirAll(filepath.Dir(file), 0o700)
	os.WriteFile(file, []byte("{{ not a ledger"), 0o600)
	deps := testDeps()
	err := startDevices(context.Background(), &config.Config{}, log.New(io.Discard, "", 0), &deps)
	if err == nil || !strings.Contains(err.Error(), "devices.json") {
		t.Errorf("the error must name the file: %v", err)
	}
}

func TestStartDevicesReportsRecordsItLeavesOut(t *testing.T) {
	isolate(t)
	file, _ := config.DevicesFile()
	os.MkdirAll(filepath.Dir(file), 0o700)
	os.WriteFile(file, []byte(`{"version":1,"devices":[{"id":"bad"}]}`), 0o600)
	var logs strings.Builder
	deps := testDeps()
	if err := startDevices(context.Background(), &config.Config{}, log.New(&logs, "", 0), &deps); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(logs.String(), "skipped") {
		t.Errorf("not reported: %q", logs.String())
	}
}

// dialRoom reaches a device session through the relay, waiting for the room to be registered.
func dialRoom(t *testing.T, ctx context.Context, relayURL, secret string) (*websocket.Conn, *securechan.Channel) {
	t.Helper()
	var lastErr error
	for i := 0; i < 100; i++ {
		c, _, err := websocket.Dial(ctx, relay.ClientURL(relayURL, secret), nil)
		if err == nil {
			ch, err := securechan.Dial(ctx, c, securechan.PSK(secret))
			if err == nil {
				t.Cleanup(func() { c.CloseNow() })
				return c, ch
			}
			c.CloseNow()
			lastErr = err
		} else {
			lastErr = err
		}
		time.Sleep(50 * time.Millisecond)
	}
	t.Fatalf("could not reach the room through the relay: %v", lastErr)
	return nil, nil
}

func readFrame(t *testing.T, ctx context.Context, ch *securechan.Channel) map[string]any {
	t.Helper()
	b, err := ch.Read(ctx)
	if err != nil {
		t.Fatalf("read: %v", err)
	}
	var m map[string]any
	if err := json.Unmarshal(b, &m); err != nil {
		t.Fatal(err)
	}
	return m
}

// The whole thing through a real relay: a device with B opens the pairing channel in the room that
// B opens, the owner confirms, the device ends up with K and a session in the room that K opens.
func TestADeviceCanPairThroughTheRelay(t *testing.T) {
	isolate(t)
	relaySrv := httptest.NewServer(relay.NewServer(log.New(io.Discard, "", 0)).Handler())
	defer relaySrv.Close()
	relayURL := "ws" + strings.TrimPrefix(relaySrv.URL, "http")

	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	deps := testDeps()
	if err := startDevices(ctx, &config.Config{RelayURL: relayURL}, log.New(io.Discard, "", 0), &deps); err != nil {
		t.Fatal(err)
	}
	id, err := deps.Devices.Add("Nomad")
	if err != nil {
		t.Fatal(err)
	}
	pk, _ := deps.Devices.Repack(id)

	_, ch := dialRoom(t, ctx, relayURL, pk.B)
	req, _ := json.Marshal(map[string]any{"type": "pairRequest", "id": id, "gen": 1, "code": "12345678"})
	if err := ch.Write(ctx, req); err != nil {
		t.Fatal(err)
	}
	for i := 0; ; i++ {
		if deps.Devices.List()[0].Waiting == 1 {
			break
		}
		if i > 200 {
			t.Fatal("the request never reached the ledger")
		}
		time.Sleep(10 * time.Millisecond)
	}
	if name, err := deps.Devices.Confirm("12345678"); err != nil || name != "Nomad" {
		t.Fatalf("Confirm: %q %v", name, err)
	}
	grant := readFrame(t, ctx, ch)
	key, _ := grant["key"].(string)
	if grant["type"] != "pairGrant" || !devices.ValidSecret(key) {
		t.Fatalf("grant: %v", grant)
	}
	ack, _ := json.Marshal(map[string]any{"type": "pairAck", "id": id, "gen": 1})
	if err := ch.Write(ctx, ack); err != nil {
		t.Fatal(err)
	}

	_, kch := dialRoom(t, ctx, relayURL, key)
	if w := readFrame(t, ctx, kch); w["type"] != "welcome" {
		t.Fatalf("a session in K's room must start with welcome: %v", w)
	}
	if got := deps.Devices.List()[0].Status; got != "active" {
		t.Errorf("status %s", got)
	}
}

func TestDeviceCommandReportsABridgeThatIsNotRunning(t *testing.T) {
	isolate(t)
	// a port nothing listens on: the default one may be a real bridge on this machine
	file, _ := config.ConfigFile()
	os.MkdirAll(filepath.Dir(file), 0o700)
	os.WriteFile(file, []byte(`{"name":"t","port":9,"token":"x"}`), 0o600)
	if err := deviceCmd([]string{"list"}); err == nil || !strings.Contains(err.Error(), "is it running") {
		t.Errorf("not running: %v", err)
	}
	if err := deviceCmd(nil); err == nil || !strings.Contains(err.Error(), "usage") {
		t.Errorf("no command: %v", err)
	}
}

func TestOpenCommandPerPlatform(t *testing.T) {
	for goos, want := range map[string]string{"darwin": "open", "windows": "rundll32", "linux": "xdg-open", "freebsd": "xdg-open"} {
		name, args := openCommand(goos, "http://127.0.0.1:8765/pair#t")
		if name != want || args[len(args)-1] != "http://127.0.0.1:8765/pair#t" {
			t.Errorf("%s: %s %v", goos, name, args)
		}
	}
}

func TestOpenBrowserReportsACommandThatCannotStart(t *testing.T) {
	t.Setenv("PATH", t.TempDir()) // nothing to run: xdg-open / open / rundll32 are not there
	if err := openBrowser("http://127.0.0.1:1/pair"); err == nil {
		t.Error("a missing command must be an error, not a silent success")
	}
}
