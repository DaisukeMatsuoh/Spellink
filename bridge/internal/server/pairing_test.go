package server

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"log"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/devices"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/ink"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/protocol"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/securechan"
)

const adminToken = "admin-token-for-tests"

type pairEnv struct {
	t        *testing.T
	reg      *devices.Registry
	sessions *Sessions
	base     string // ws://…/v1
	http     string // http://…
	store    *failingStore
}

// failingStore wraps the real file store so a test can make the ledger unwritable.
type failingStore struct {
	devices.FileStore
	fail error
}

func (s *failingStore) Save(b []byte) error {
	if s.fail != nil {
		return s.fail
	}
	return s.FileStore.Save(b)
}

func newPairEnv(t *testing.T, tweak ...func(*Deps)) *pairEnv {
	t.Helper()
	e := &pairEnv{t: t, sessions: NewSessions(), store: &failingStore{FileStore: devices.FileStore{Path: filepath.Join(t.TempDir(), "devices.json")}}}
	var err error
	e.reg, _, err = devices.New(devices.Config{}, e.store, devices.Hooks{
		Endpoints: func(id, k string) []string { return []string{"wss://bridge.example/v1?d=" + id} },
		CloseAll:  e.sessions.CloseDevice,
	})
	if err != nil {
		t.Fatal(err)
	}
	font := &ink.Font{Em: 100, Glyphs: map[rune]ink.Glyph{'あ': {Advance: 100, Strokes: [][]int16{{50, 0, 50, 100}}}}}
	d := Deps{
		Name: "test", Version: "0", Token: "secret", Font: font, Provider: asProvider(nil, nil),
		Report:   func(string, any) {},
		Settings: func() protocol.Settings { return protocol.Settings{} }, SetSettings: func(protocol.Settings) error { return nil },
		Log:     log.New(io.Discard, "", 0),
		Devices: e.reg, Sessions: e.sessions,
		Admin: &AdminDeps{Devices: e.reg, Token: adminToken, Package: func(d devices.PackageData) ([]byte, error) {
			return []byte("package for " + d.Name + " gen " + string(rune('0'+d.Gen)) + " B=" + d.B), nil
		}},
	}
	for _, f := range tweak {
		f(&d)
	}
	srv := httptest.NewServer(Handler(d))
	t.Cleanup(srv.Close)
	e.http = srv.URL
	e.base = "ws" + strings.TrimPrefix(srv.URL, "http") + "/v1"
	return e
}

// device is a plugin's end of a device session.
type device struct {
	t  *testing.T
	c  *websocket.Conn
	ch *securechan.Channel
}

func (e *pairEnv) dial(id, secret string) *device {
	e.t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	c, _, err := websocket.Dial(ctx, e.base+"?d="+id, nil)
	if err != nil {
		e.t.Fatal(err)
	}
	e.t.Cleanup(func() { c.CloseNow() })
	ch, err := securechan.Dial(ctx, c, securechan.PSK(secret))
	if err != nil {
		e.t.Fatalf("handshake: %v", err)
	}
	return &device{t: e.t, c: c, ch: ch}
}

func (d *device) send(v any) {
	d.t.Helper()
	b, _ := json.Marshal(v)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := d.ch.Write(ctx, b); err != nil {
		d.t.Fatalf("send: %v", err)
	}
}

// recv returns the next frame, or nil if the session ended.
func (d *device) recv() msg {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	b, err := d.ch.Read(ctx)
	if err != nil {
		return nil
	}
	var m msg
	_ = json.Unmarshal(b, &m)
	return m
}

func (e *pairEnv) admin(method, path string, body any) (int, []byte) {
	e.t.Helper()
	var rd io.Reader
	if body != nil {
		b, _ := json.Marshal(body)
		rd = bytes.NewReader(b)
	}
	req, _ := http.NewRequest(method, e.http+path, rd)
	req.Header.Set("Authorization", "Bearer "+adminToken)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		e.t.Fatal(err)
	}
	defer resp.Body.Close()
	out, _ := io.ReadAll(resp.Body)
	return resp.StatusCode, out
}

func (e *pairEnv) addDevice(name string) (id, b string) {
	e.t.Helper()
	id, err := e.reg.Add(name)
	if err != nil {
		e.t.Fatal(err)
	}
	d, err := e.reg.Repack(id)
	if err != nil {
		e.t.Fatal(err)
	}
	return id, d.B
}

func (e *pairEnv) waitWaiting(id string, n int) {
	e.t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		for _, in := range e.reg.List() {
			if in.ID == id && in.Waiting == n {
				return
			}
		}
		time.Sleep(5 * time.Millisecond)
	}
	e.t.Fatalf("device %s never had %d waiting request(s)", id, n)
}

// pairFully takes a device through the whole ceremony and returns its K.
func (e *pairEnv) pairFully(name, code string) (id, k string) {
	e.t.Helper()
	id, b := e.addDevice(name)
	d := e.dial(id, b)
	d.send(msg{"type": "pairRequest", "id": id, "gen": 1, "code": code})
	e.waitWaiting(id, 1)
	if status, _ := e.admin("POST", "/admin/pair", msg{"code": code}); status != 200 {
		e.t.Fatalf("confirm: %d", status)
	}
	g := d.recv()
	if g["type"] != "pairGrant" {
		e.t.Fatalf("expected pairGrant, got %v", g)
	}
	d.send(msg{"type": "pairAck", "id": id, "gen": 1})
	if d.recv() != nil {
		e.t.Fatal("the pairing channel must end after the ack")
	}
	return id, g["key"].(string)
}

func TestPairingCeremonyEndToEndOverTheWire(t *testing.T) {
	e := newPairEnv(t)
	id, b := e.addDevice("Nomad")
	d := e.dial(id, b)
	d.send(msg{"type": "pairRequest", "id": id, "gen": 1, "code": "12345678"})
	e.waitWaiting(id, 1)

	status, body := e.admin("POST", "/admin/pair", msg{"code": "12345678"})
	if status != 200 || !strings.Contains(string(body), "Nomad") {
		t.Fatalf("confirm: %d %s", status, body)
	}
	g := d.recv()
	key, _ := g["key"].(string)
	eps, _ := g["endpoints"].([]any)
	if g["type"] != "pairGrant" || !devices.ValidSecret(key) || len(eps) != 1 {
		t.Fatalf("grant = %v", g)
	}
	d.send(msg{"type": "pairAck", "id": id, "gen": 1})
	if d.recv() != nil {
		t.Fatal("the channel must end after the ack")
	}

	// from now on only K works, and B no longer opens anything
	k := e.dial(id, key)
	if w := k.recv(); w["type"] != "welcome" {
		t.Fatalf("a session with K must start with welcome, got %v", w)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	c, _, err := websocket.Dial(ctx, e.base+"?d="+id, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer c.CloseNow()
	if _, err := securechan.Dial(ctx, c, securechan.PSK(b)); err == nil {
		t.Error("B must be refused once K has been acknowledged")
	}
}

func TestAConnectionWithKCountsAsTheAckOverTheWire(t *testing.T) {
	e := newPairEnv(t)
	id, b := e.addDevice("Nomad")
	d := e.dial(id, b)
	d.send(msg{"type": "pairRequest", "id": id, "gen": 1, "code": "12345678"})
	e.waitWaiting(id, 1)
	e.admin("POST", "/admin/pair", msg{"code": "12345678"})
	key := d.recv()["key"].(string)
	// the device stored K but its ack never arrived: it connects with K instead
	if w := e.dial(id, key).recv(); w["type"] != "welcome" {
		t.Fatalf("got %v", w)
	}
	if got := e.reg.List()[0].Status; got != "active" {
		t.Errorf("status %s", got)
	}
}

func TestThePairingChannelAllowsNothingButPairing(t *testing.T) {
	e := newPairEnv(t)
	id, b := e.addDevice("Nomad")
	d := e.dial(id, b)
	d.send(msg{"type": "ask", "id": "q", "question": "hi", "lang": "ja", "layout": layout})
	if m := d.recv(); m["type"] != "error" || m["code"] != "bad_request" {
		t.Fatalf("got %v", m)
	}
	if d.recv() != nil {
		t.Error("the channel must be closed after a forbidden frame")
	}
}

func TestPairingRequestRules(t *testing.T) {
	e := newPairEnv(t)
	id, b := e.addDevice("Nomad")
	other, _ := e.addDevice("Other")

	d := e.dial(id, b) // a request for another device
	d.send(msg{"type": "pairRequest", "id": other, "gen": 1, "code": "11111111"})
	if m := d.recv(); m["code"] != "bad_request" {
		t.Errorf("another device: %v", m)
	}
	d = e.dial(id, b) // two requests on one channel
	d.send(msg{"type": "pairRequest", "id": id, "gen": 1, "code": "11111111"})
	e.waitWaiting(id, 1)
	d.send(msg{"type": "pairRequest", "id": id, "gen": 1, "code": "22222222"})
	if m := d.recv(); m["code"] != "bad_request" {
		t.Errorf("second request: %v", m)
	}
	d = e.dial(id, b) // a malformed code
	d.send(msg{"type": "pairRequest", "id": id, "gen": 1, "code": "12"})
	if m := d.recv(); m["type"] != "pairDenied" || m["reason"] != devices.DenyUnknownDevice {
		t.Errorf("bad code: %v", m)
	}
	d = e.dial(id, b) // a stale package (old generation)
	d.send(msg{"type": "pairRequest", "id": id, "gen": 7, "code": "12345678"})
	if m := d.recv(); m["type"] != "pairDenied" {
		t.Errorf("old generation: %v", m)
	}
	d = e.dial(id, b) // an ack for another device
	d.send(msg{"type": "pairAck", "id": other, "gen": 1})
	if m := d.recv(); m["code"] != "bad_request" {
		t.Errorf("ack for another device: %v", m)
	}
}

func TestTheSameCodeFromTwoChannelsDeniesBoth(t *testing.T) {
	e := newPairEnv(t)
	id, b := e.addDevice("Nomad")
	first := e.dial(id, b)
	first.send(msg{"type": "pairRequest", "id": id, "gen": 1, "code": "12345678"})
	e.waitWaiting(id, 1)
	second := e.dial(id, b)
	second.send(msg{"type": "pairRequest", "id": id, "gen": 1, "code": "12345678"})
	for i, d := range []*device{first, second} {
		if m := d.recv(); m["type"] != "pairDenied" || m["reason"] != devices.DenyCodeInUse {
			t.Errorf("channel %d: %v", i, m)
		}
	}
	if status, _ := e.admin("POST", "/admin/pair", msg{"code": "12345678"}); status != http.StatusNotFound {
		t.Errorf("nothing may be left to confirm: %d", status)
	}
}

func TestAChannelThatNeverRequestsIsClosed(t *testing.T) {
	e := newPairEnv(t, func(d *Deps) { d.PairRequestWait = 100 * time.Millisecond })
	id, b := e.addDevice("Nomad")
	if e.dial(id, b).recv() != nil {
		t.Error("a silent pairing channel must be closed")
	}
}

func TestAPairingChannelHasAMaximumLife(t *testing.T) {
	e := newPairEnv(t, func(d *Deps) { d.PairingMax = 150 * time.Millisecond })
	id, b := e.addDevice("Nomad")
	d := e.dial(id, b)
	d.send(msg{"type": "pairRequest", "id": id, "gen": 1, "code": "12345678"})
	if d.recv() != nil {
		t.Error("the channel must end when its life is over")
	}
}

func TestDeviceConnectionsAreRefusedUnlessTheyAreProperlyEncrypted(t *testing.T) {
	e := newPairEnv(t)
	id, b := e.addDevice("Nomad")
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	try := func(name, url string, f func(c *websocket.Conn) error) {
		c, _, err := websocket.Dial(ctx, url, nil)
		if err != nil {
			t.Fatal(err)
		}
		defer c.CloseNow()
		if err := f(c); err == nil {
			// the server must close: the next read fails
			if _, _, err := c.Read(ctx); err == nil {
				t.Errorf("%s: the session stayed open", name)
			}
		}
	}
	try("plaintext hello", e.base+"?d="+id, func(c *websocket.Conn) error {
		return c.Write(ctx, websocket.MessageText, []byte(`{"type":"hello","token":"secret"}`))
	})
	try("unknown device", e.base+"?d=ffffffffffffffff", func(c *websocket.Conn) error {
		_, err := securechan.Dial(ctx, c, securechan.PSK(b))
		return err
	})
	try("wrong secret", e.base+"?d="+id, func(c *websocket.Conn) error {
		_, err := securechan.Dial(ctx, c, securechan.PSK("not the secret"))
		return err
	})
	try("no secret at all", e.base+"?d="+id, func(c *websocket.Conn) error {
		return c.Write(ctx, websocket.MessageBinary, []byte("garbage"))
	})
	// a ledger-less bridge refuses every device
	plain := startTest(t, nil)
	try("no ledger", plain+"?d="+id, func(c *websocket.Conn) error {
		_, err := securechan.Dial(ctx, c, securechan.PSK(b))
		return err
	})
}

func TestRevokingADeviceCutsOffItsLiveSession(t *testing.T) {
	e := newPairEnv(t)
	id, k := e.pairFully("Nomad", "12345678")
	live := e.dial(id, k)
	if live.recv()["type"] != "welcome" {
		t.Fatal("no welcome")
	}
	if status, _ := e.admin("POST", "/admin/devices/"+id+"/revoke", nil); status != http.StatusNoContent {
		t.Fatalf("revoke: %d", status)
	}
	if live.recv() != nil {
		t.Error("the live session of a revoked device must end")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	c, _, err := websocket.Dial(ctx, e.base+"?d="+id, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer c.CloseNow()
	if _, err := securechan.Dial(ctx, c, securechan.PSK(k)); err == nil {
		t.Error("a revoked device's K must not open a session")
	}
}

func TestSessionsTable(t *testing.T) {
	tab := NewSessions()
	tab.CloseDevice("nobody") // nothing to close
	s := &session{}
	tab.add("a", s)
	tab.add("a", &session{})
	tab.remove("a", s)
	tab.remove("a", &session{}) // not registered: nothing happens
	tab.remove("b", s)
	if len(tab.byID["a"]) != 1 {
		t.Errorf("table: %v", tab.byID)
	}
}

// ---- the command line's door ----

func TestAdminRefusesWhatIsNotLocalAndAuthenticated(t *testing.T) {
	e := newPairEnv(t)
	do := func(token string, headers map[string]string) int {
		req, _ := http.NewRequest("GET", e.http+"/admin/devices", nil)
		if token != "" {
			req.Header.Set("Authorization", "Bearer "+token)
		}
		for k, v := range headers {
			req.Header.Set(k, v)
		}
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		resp.Body.Close()
		return resp.StatusCode
	}
	if got := do("", nil); got != 401 {
		t.Errorf("no token: %d", got)
	}
	if got := do("wrong", nil); got != 401 {
		t.Errorf("wrong token: %d", got)
	}
	if got := do(adminToken, map[string]string{"Cf-Connecting-Ip": "203.0.113.9"}); got != 403 {
		t.Errorf("through the tunnel: %d", got)
	}
	if got := do(adminToken, map[string]string{"X-Forwarded-For": "203.0.113.9"}); got != 403 {
		t.Errorf("through a proxy: %d", got)
	}
	if got := do(adminToken, nil); got != 200 {
		t.Errorf("good request: %d", got)
	}
	// a bridge configured with no token must refuse everything, not accept the empty one
	empty := newPairEnv(t, func(d *Deps) { d.Admin.Token = "" })
	req, _ := http.NewRequest("GET", empty.http+"/admin/devices", nil)
	resp, _ := http.DefaultClient.Do(req)
	resp.Body.Close()
	if resp.StatusCode != 401 {
		t.Errorf("empty configured token: %d", resp.StatusCode)
	}
}

func TestAdminDeviceLifecycle(t *testing.T) {
	e := newPairEnv(t)
	status, body := e.admin("POST", "/admin/devices", msg{"name": "Nomad"})
	var created struct{ ID, Name string }
	json.Unmarshal(body, &created)
	if status != 201 || !devices.ValidID(created.ID) || created.Name != "Nomad" {
		t.Fatalf("add: %d %s", status, body)
	}
	if status, _ := e.admin("POST", "/admin/devices", msg{"name": "Nomad"}); status != 409 {
		t.Errorf("duplicate: %d", status)
	}
	if status, _ := e.admin("POST", "/admin/devices", msg{"name": ""}); status != 400 {
		t.Errorf("bad name: %d", status)
	}
	if status, _ := e.admin("POST", "/admin/devices", "not an object"); status != 400 {
		t.Errorf("bad body: %d", status)
	}
	status, body = e.admin("GET", "/admin/devices", nil)
	var list []deviceJSON
	json.Unmarshal(body, &list)
	if status != 200 || len(list) != 1 || list[0].Status != "issued" || list[0].Expires.IsZero() {
		t.Fatalf("list: %d %s", status, body)
	}
	if strings.Contains(string(body), "\"b\"") || strings.Contains(string(body), "\"k\"") {
		t.Error("the list must never carry a secret")
	}
	status, body = e.admin("GET", "/admin/devices/"+created.ID+"/package", nil)
	if status != 200 || !strings.HasPrefix(string(body), "package for Nomad gen 1 B=") {
		t.Errorf("package: %d %s", status, body)
	}
	if status, _ := e.admin("POST", "/admin/devices/"+created.ID+"/reissue", nil); status != 204 {
		t.Errorf("reissue: %d", status)
	}
	if status, _ := e.admin("POST", "/admin/devices/"+created.ID+"/forget", nil); status != 409 {
		t.Errorf("forget while valid: %d", status)
	}
	if status, _ := e.admin("POST", "/admin/devices/"+created.ID+"/revoke", nil); status != 204 {
		t.Errorf("revoke: %d", status)
	}
	if status, _ := e.admin("POST", "/admin/devices/"+created.ID+"/revoke", nil); status != 409 {
		t.Errorf("revoke twice: %d", status)
	}
	if status, _ := e.admin("POST", "/admin/devices/"+created.ID+"/reissue", nil); status != 409 {
		t.Errorf("reissue a revoked one: %d", status)
	}
	if status, _ := e.admin("GET", "/admin/devices/"+created.ID+"/package", nil); status != 409 {
		t.Errorf("package of a revoked one: %d", status)
	}
	if status, _ := e.admin("POST", "/admin/devices/"+created.ID+"/forget", nil); status != 204 {
		t.Errorf("forget: %d", status)
	}
	for _, p := range []string{"revoke", "reissue", "forget"} {
		if status, _ := e.admin("POST", "/admin/devices/ffffffffffffffff/"+p, nil); status != 404 {
			t.Errorf("%s of an unknown device: %d", p, status)
		}
	}
	if status, _ := e.admin("GET", "/admin/devices/ffffffffffffffff/package", nil); status != 404 {
		t.Errorf("package of an unknown device: %d", status)
	}
}

func TestAdminPairAndItsErrors(t *testing.T) {
	e := newPairEnv(t)
	if status, _ := e.admin("POST", "/admin/pair", msg{"code": "1"}); status != 400 {
		t.Errorf("short code: %d", status)
	}
	if status, _ := e.admin("POST", "/admin/pair", msg{"code": "12345678"}); status != 404 {
		t.Errorf("nobody waiting: %d", status)
	}
	if status, _ := e.admin("POST", "/admin/pair", 5); status != 400 {
		t.Errorf("bad body: %d", status)
	}
	id, b := e.addDevice("Nomad")
	d := e.dial(id, b)
	d.send(msg{"type": "pairRequest", "id": id, "gen": 1, "code": "12345678"})
	e.waitWaiting(id, 1)
	if status, body := e.admin("POST", "/admin/pair", msg{"code": "00000000"}); status != 403 || strings.Contains(string(body), "Nomad") {
		t.Errorf("wrong code: %d %s (must not name the waiting device)", status, body)
	}
}

func TestAdminReportsAnUnwritableLedgerAndABrokenPackage(t *testing.T) {
	e := newPairEnv(t)
	e.store.fail = errors.New("disk full")
	if status, _ := e.admin("POST", "/admin/devices", msg{"name": "Nomad"}); status != 500 {
		t.Errorf("unwritable ledger: %d", status)
	}
	e.store.fail = nil
	id, _ := e.addDevice("Nomad")
	broken := newPairEnv(t, func(d *Deps) {
		d.Admin.Package = func(devices.PackageData) ([]byte, error) { return nil, errors.New("no base package") }
	})
	bid, _ := broken.addDevice("Nomad")
	if status, _ := broken.admin("GET", "/admin/devices/"+bid+"/package", nil); status != 500 {
		t.Errorf("broken package: %d", status)
	}
	_ = id
}

func TestAdminIsOffUnlessConfigured(t *testing.T) {
	url := httptestURL(t, func(d *Deps) { d.Admin = nil })
	resp, err := http.Get(url + "/admin/devices")
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != 404 {
		t.Errorf("status %d", resp.StatusCode)
	}
}

func httptestURL(t *testing.T, tweak func(*Deps)) string {
	e := newPairEnv(t, tweak)
	return e.http
}

func TestTheLegacySharedTokenStillWorks(t *testing.T) {
	e := newPairEnv(t)
	got := converse(t, e.base, []any{hello}, func(m msg) bool { return m["type"] == "welcome" })
	if types(got) != "welcome" {
		t.Fatalf("got %s", types(got))
	}
}

func TestAnAckForTheWrongGenerationDoesNotActivateTheDevice(t *testing.T) {
	e := newPairEnv(t)
	id, b := e.addDevice("Nomad")
	d := e.dial(id, b)
	d.send(msg{"type": "pairRequest", "id": id, "gen": 1, "code": "12345678"})
	e.waitWaiting(id, 1)
	e.admin("POST", "/admin/pair", msg{"code": "12345678"})
	if d.recv()["type"] != "pairGrant" {
		t.Fatal("no grant")
	}
	d.send(msg{"type": "pairAck", "id": id, "gen": 99})
	if d.recv() != nil {
		t.Error("the channel ends after an ack, valid or not")
	}
	if got := e.reg.List()[0].Status; got != "granting" {
		t.Errorf("status %s: only a matching ack may activate", got)
	}
}

func TestAGrantWithoutEndpointsSendsAnEmptyList(t *testing.T) {
	var reg *devices.Registry
	e := newPairEnv(t, func(d *Deps) {
		var err error
		reg, _, err = devices.New(devices.Config{}, &failingStore{FileStore: devices.FileStore{Path: filepath.Join(t.TempDir(), "d.json")}}, devices.Hooks{})
		if err != nil {
			t.Fatal(err)
		}
		d.Devices, d.Admin.Devices = reg, reg
	})
	id, err := reg.Add("Nomad")
	if err != nil {
		t.Fatal(err)
	}
	pk, _ := reg.Repack(id)
	d := e.dial(id, pk.B)
	d.send(msg{"type": "pairRequest", "id": id, "gen": 1, "code": "12345678"})
	e.waitWaitingIn(reg, id, 1)
	if _, err := reg.Confirm("12345678"); err != nil {
		t.Fatal(err)
	}
	g := d.recv()
	if eps, ok := g["endpoints"].([]any); g["type"] != "pairGrant" || !ok || len(eps) != 0 {
		t.Errorf("grant = %v", g)
	}
}

func (e *pairEnv) waitWaitingIn(reg *devices.Registry, id string, n int) {
	e.t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		for _, in := range reg.List() {
			if in.ID == id && in.Waiting == n {
				return
			}
		}
		time.Sleep(5 * time.Millisecond)
	}
	e.t.Fatalf("device %s never had %d waiting request(s)", id, n)
}

func TestServeConnIsTheSharedTokenEntrance(t *testing.T) {
	font := &ink.Font{Em: 100, Glyphs: map[rune]ink.Glyph{'あ': {Advance: 100, Strokes: [][]int16{{50, 0, 50, 100}}}}}
	d := Deps{Name: "test", Version: "0", Token: "secret", Font: font, Provider: asProvider(nil, nil), Report: func(string, any) {},
		Settings: func() protocol.Settings { return protocol.Settings{} }, SetSettings: func(protocol.Settings) error { return nil }, Log: log.New(io.Discard, "", 0)}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		c, err := websocket.Accept(w, r, &websocket.AcceptOptions{InsecureSkipVerify: true})
		if err != nil {
			return
		}
		ServeConn(r.Context(), d, c, r.RemoteAddr, false)
	}))
	defer srv.Close()
	got := converse(t, "ws"+strings.TrimPrefix(srv.URL, "http"), []any{hello}, func(m msg) bool { return m["type"] == "welcome" })
	if types(got) != "welcome" {
		t.Fatalf("got %s", types(got))
	}
}

func TestAdminPairWithTheDevicesIdOnlyConsidersThatDevice(t *testing.T) {
	e := newPairEnv(t)
	one, b1 := e.addDevice("one")
	two, b2 := e.addDevice("two")
	d1, d2 := e.dial(one, b1), e.dial(two, b2)
	d1.send(msg{"type": "pairRequest", "id": one, "gen": 1, "code": "11111111"})
	d2.send(msg{"type": "pairRequest", "id": two, "gen": 1, "code": "22222222"})
	e.waitWaiting(one, 1)
	e.waitWaiting(two, 1)
	if status, _ := e.admin("POST", "/admin/pair", msg{"id": one, "code": "22222222"}); status != 403 {
		t.Errorf("the code of another device: %d", status)
	}
	if status, body := e.admin("POST", "/admin/pair", msg{"id": two, "code": "22222222"}); status != 200 || !strings.Contains(string(body), "two") {
		t.Errorf("the right id and code: %d %s", status, body)
	}
	if g := d2.recv(); g["type"] != "pairGrant" {
		t.Errorf("the device was not granted: %v", g)
	}
	if status, _ := e.admin("POST", "/admin/pair", msg{"id": "ffffffffffffffff", "code": "11111111"}); status != 404 {
		t.Errorf("an unknown device: %d", status)
	}
}

func TestThePairingPageIsServedWithTheAdminDoorAndOnlyThen(t *testing.T) {
	e := newPairEnv(t)
	resp, err := http.Get(e.http + "/pair")
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != 200 || !strings.HasPrefix(resp.Header.Get("Content-Type"), "text/html") {
		t.Errorf("/pair: %d %s", resp.StatusCode, resp.Header.Get("Content-Type"))
	}
	url := httptestURL(t, func(d *Deps) { d.Admin = nil })
	resp, err = http.Get(url + "/pair")
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != 404 {
		t.Errorf("without the admin door the page must not exist: %d", resp.StatusCode)
	}
}

// ---- the handwritten banner ----

var bannerLayout = ink.Layout{X: 0, Y: 0, Width: 600, PageHeight: 400, BottomMargin: 0, CharSize: 40}

func TestARequestWithABannerGetsItWrittenInHandwritingBeforeAnyGrant(t *testing.T) {
	e := newPairEnv(t)
	id, b := e.addDevice("Nomad")
	d := e.dial(id, b)
	d.send(msg{"type": "pairRequest", "id": id, "gen": 1, "code": "12345678", "banner": "ああ あ", "layout": bannerLayout})
	m := d.recv()
	strokes, _ := m["strokes"].([]any)
	if m["type"] != "pairBanner" || len(strokes) == 0 {
		t.Fatalf("expected a banner with strokes, got %v", m)
	}
	first := strokes[0].(map[string]any)
	p, w := first["p"].([]any), first["w"].([]any)
	if len(p) < 2 || len(p)%2 != 0 || len(w) != len(p)/2 {
		t.Errorf("a stroke must have matching points and pressures: %d coordinates, %d pressures", len(p), len(w))
	}
	for _, v := range p {
		if x := v.(float64); x < -50 || x > 700 {
			t.Errorf("a point outside the asked-for width: %v", x)
		}
	}
	e.waitWaiting(id, 1)
	e.admin("POST", "/admin/pair", msg{"code": "12345678"})
	if g := d.recv(); g["type"] != "pairGrant" {
		t.Errorf("after the banner the grant comes: %v", g)
	}
}

func TestABannerIsOnlyDecorationSoBadRequestsGetNoneAndStillPair(t *testing.T) {
	cases := map[string]msg{
		"no banner":                 {},
		"only spaces":               {"banner": "   ", "layout": bannerLayout},
		"too long":                  {"banner": strings.Repeat("あ", maxBannerRunes+1), "layout": bannerLayout},
		"no layout":                 {"banner": "ああ"},
		"a layout that is not sane": {"banner": "ああ", "layout": ink.Layout{Width: -1, PageHeight: 400, CharSize: 40}},
		"nothing it can draw":       {"banner": "xyz", "layout": bannerLayout}, // the test font has no such letters
	}
	for name, extra := range cases {
		e := newPairEnv(t)
		id, b := e.addDevice("Nomad")
		d := e.dial(id, b)
		req := msg{"type": "pairRequest", "id": id, "gen": 1, "code": "12345678"}
		for k, v := range extra {
			req[k] = v
		}
		d.send(req)
		e.waitWaiting(id, 1)
		e.admin("POST", "/admin/pair", msg{"code": "12345678"})
		if g := d.recv(); g["type"] != "pairGrant" {
			t.Errorf("%s: the first frame must be the grant (no banner), got %v", name, g)
		}
	}
}

func TestABridgeWithoutAFontStillPairs(t *testing.T) {
	e := newPairEnv(t, func(d *Deps) { d.Font = nil })
	id, b := e.addDevice("Nomad")
	d := e.dial(id, b)
	d.send(msg{"type": "pairRequest", "id": id, "gen": 1, "code": "12345678", "banner": "ああ", "layout": bannerLayout})
	e.waitWaiting(id, 1)
	e.admin("POST", "/admin/pair", msg{"code": "12345678"})
	if g := d.recv(); g["type"] != "pairGrant" {
		t.Errorf("got %v", g)
	}
}

func TestADeniedRequestGetsNoBanner(t *testing.T) {
	e := newPairEnv(t)
	id, b := e.addDevice("Nomad")
	d := e.dial(id, b)
	d.send(msg{"type": "pairRequest", "id": id, "gen": 99, "code": "12345678", "banner": "ああ", "layout": bannerLayout})
	if m := d.recv(); m["type"] != "pairDenied" {
		t.Errorf("a refused request is answered with the refusal and nothing else: %v", m)
	}
}

func TestTheLocalDoorsRefuseAForeignHostEvenWithTheRightToken(t *testing.T) {
	e := newPairEnv(t)
	for _, path := range []string{"/admin/devices", "/pair", "/pair/strings.json"} {
		req, _ := http.NewRequest("GET", e.http+path, nil)
		req.Host = "evil.example:8765" // a page that re-pointed its own name at 127.0.0.1
		req.Header.Set("Authorization", "Bearer "+adminToken)
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		resp.Body.Close()
		if resp.StatusCode != http.StatusForbidden {
			t.Errorf("%s with a foreign Host: %d", path, resp.StatusCode)
		}
	}
	// the same requests with a loopback name are served
	for _, host := range []string{"localhost", "127.0.0.1"} {
		req, _ := http.NewRequest("GET", e.http+"/admin/devices", nil)
		req.Host = host
		req.Header.Set("Authorization", "Bearer "+adminToken)
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		resp.Body.Close()
		if resp.StatusCode != 200 {
			t.Errorf("Host %s: %d", host, resp.StatusCode)
		}
	}
}

func TestAdminStatusCountsDevicesAndPassesOnWhatTheBridgeSaysAboutItself(t *testing.T) {
	e := newPairEnv(t, func(d *Deps) {
		d.Admin.Status = func() StatusInfo {
			return StatusInfo{Version: "1.2.3", Name: "Rock", Relay: true, Addresses: []string{"wss://x/v1"}, Provider: ProviderInfo{Name: "Claude Code", Found: true, Path: "/usr/bin/claude"}}
		}
	})
	e.pairFully("active", "11111111")
	id, b := e.addDevice("waiting")
	d := e.dial(id, b)
	d.send(msg{"type": "pairRequest", "id": id, "gen": 1, "code": "22222222"})
	e.waitWaiting(id, 1)
	e.addDevice("issued")

	status, body := e.admin("GET", "/admin/status", nil)
	var got struct {
		Version   string
		Name      string
		Relay     bool
		Provider  ProviderInfo
		Devices   struct{ Total, Active, Waiting int }
		Addresses []string
	}
	if err := json.Unmarshal(body, &got); err != nil || status != 200 {
		t.Fatalf("%d %s %v", status, body, err)
	}
	if got.Version != "1.2.3" || got.Name != "Rock" || !got.Relay || !got.Provider.Found || got.Provider.Path != "/usr/bin/claude" || len(got.Addresses) != 1 {
		t.Errorf("status = %+v", got)
	}
	if got.Devices.Total != 3 || got.Devices.Active != 1 || got.Devices.Waiting != 1 {
		t.Errorf("devices = %+v", got.Devices)
	}
}

func TestAdminStatusWithoutAStatusFunctionStillAnswers(t *testing.T) {
	e := newPairEnv(t)
	status, body := e.admin("GET", "/admin/status", nil)
	if status != 200 || !strings.Contains(string(body), `"addresses":[]`) || !strings.Contains(string(body), `"total":0`) {
		t.Errorf("%d %s", status, body)
	}
}

func TestTheAppPageIsServedWithTheAdminDoorAndOnlyThen(t *testing.T) {
	e := newPairEnv(t)
	for _, path := range []string{"/app", "/app/app.js", "/app/strings.json", "/pair/pairlogic.js"} {
		resp, err := http.Get(e.http + path)
		if err != nil {
			t.Fatal(err)
		}
		resp.Body.Close()
		if resp.StatusCode != 200 {
			t.Errorf("%s: %d", path, resp.StatusCode)
		}
	}
	url := httptestURL(t, func(d *Deps) { d.Admin = nil })
	resp, err := http.Get(url + "/app")
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != 404 {
		t.Errorf("without the admin door the app page must not exist: %d", resp.StatusCode)
	}
}
