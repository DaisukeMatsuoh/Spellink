package devices

import (
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func secret(n byte) string {
	b := make([]byte, secretBytes)
	for i := range b {
		b[i] = n
	}
	return base64.RawURLEncoding.EncodeToString(b)
}

func goodRecord(state string) Record {
	r := Record{ID: "0123456789abcdef", Name: "Nomad", Gen: 1, State: state, IssuedAt: time.Date(2026, 11, 20, 0, 0, 0, 0, time.UTC)}
	switch state {
	case "I":
		r.B = secret(1)
	case "G":
		r.B, r.K = secret(1), secret(2)
	case "A":
		r.K = secret(2)
	}
	return r
}

func ledgerOf(recs ...Record) []byte {
	b, err := encode(recs)
	if err != nil {
		panic(err)
	}
	return b
}

func TestValidators(t *testing.T) {
	names := map[string]bool{"Nomad": true, "ノート": true, strings.Repeat("x", 64): true, "": false, strings.Repeat("x", 65): false, "a\nb": false, "a\x00b": false, "\xff": false}
	for n, want := range names {
		if got := ValidName(n); got != want {
			t.Errorf("ValidName(%q) = %v", n, got)
		}
	}
	secrets := map[string]bool{secret(7): true, "": false, "short": false, secret(7) + "=": false, strings.Repeat("A", 44): false, strings.Repeat("!", 43): false,
		// 43 characters whose last one leaves bits set that a 32-byte value cannot have: not canonical
		strings.Repeat("A", 42) + "B": false}
	for s, want := range secrets {
		if got := ValidSecret(s); got != want {
			t.Errorf("ValidSecret(%q) = %v", s, got)
		}
	}
	codes := map[string]bool{"12345678": true, "00000000": true, "1234567": false, "123456789": false, "1234567a": false, "": false, "１２３４５６７８": false}
	for c, want := range codes {
		if got := ValidCode(c); got != want {
			t.Errorf("ValidCode(%q) = %v", c, got)
		}
	}
	ids := map[string]bool{"0123456789abcdef": true, "0123456789ABCDEF": false, "0123456789abcde": false, "0123456789abcdeg": false}
	for i, want := range ids {
		if got := ValidID(i); got != want {
			t.Errorf("ValidID(%q) = %v", i, got)
		}
	}
}

func TestDecodeKeepsGoodRecordsOfEveryState(t *testing.T) {
	recs, skipped, err := decode(ledgerOf(goodRecord("I"), func() Record { r := goodRecord("G"); r.ID, r.Name = "1123456789abcdef", "g"; return r }(),
		func() Record { r := goodRecord("A"); r.ID, r.Name = "2123456789abcdef", "a"; return r }(),
		func() Record { r := goodRecord("X"); r.ID, r.Name = "3123456789abcdef", "x"; return r }(),
		func() Record { r := goodRecord("R"); r.ID, r.Name = "4123456789abcdef", "r"; return r }()))
	if err != nil || len(skipped) != 0 || len(recs) != 5 {
		t.Fatalf("recs %d skipped %v err %v", len(recs), skipped, err)
	}
}

func TestDecodeLeavesOutUnusableRecordsAndSaysWhy(t *testing.T) {
	mut := func(f func(*Record)) Record { r := goodRecord("A"); f(&r); return r }
	bad := map[string]Record{
		"bad id":             mut(func(r *Record) { r.ID = "xyz" }),
		"bad name":           mut(func(r *Record) { r.Name = "" }),
		"bad gen":            mut(func(r *Record) { r.Gen = 0 }),
		"gen too large":      mut(func(r *Record) { r.Gen = maxGen + 1 }),
		"bad B":              mut(func(r *Record) { r.B = "nope" }),
		"bad K":              mut(func(r *Record) { r.K = "nope" }),
		"bad state":          mut(func(r *Record) { r.State = "Q" }),
		"issued without B":   mut(func(r *Record) { r.State, r.K = "I", "" }),
		"issued with K":      mut(func(r *Record) { r.State, r.B = "I", secret(1) }),
		"granting without K": mut(func(r *Record) { r.State, r.B, r.K = "G", secret(1), "" }),
		"active without K":   mut(func(r *Record) { r.K = "" }),
		"expired with a B":   mut(func(r *Record) { r.State, r.B, r.K = "X", secret(1), "" }),
		"revoked with a K":   mut(func(r *Record) { r.State = "R" }),
	}
	for name, rec := range bad {
		recs, skipped, err := decode(ledgerOf(rec))
		if err != nil || len(recs) != 0 || len(skipped) != 1 {
			t.Errorf("%s: recs %d skipped %v err %v", name, len(recs), skipped, err)
		}
	}
}

func TestDecodeDropsALeftoverBFromAnActiveRecord(t *testing.T) {
	r := goodRecord("A")
	r.B = secret(9)
	recs, skipped, err := decode(ledgerOf(r))
	if err != nil || len(skipped) != 0 || len(recs) != 1 || recs[0].B != "" || recs[0].K != secret(2) {
		t.Fatalf("recs %+v skipped %v err %v", recs, skipped, err)
	}
}

func TestDecodeRefusesDuplicatesAndTooMany(t *testing.T) {
	a := goodRecord("I")
	sameID := goodRecord("I")
	sameID.Name = "other"
	sameName := goodRecord("I")
	sameName.ID = "ffffffffffffffff"
	recs, skipped, err := decode(ledgerOf(a, sameID, sameName))
	if err != nil || len(recs) != 1 || len(skipped) != 2 {
		t.Fatalf("recs %d skipped %v err %v", len(recs), skipped, err)
	}
	var many []Record
	for i := 0; i < MaxDevices+2; i++ {
		r := goodRecord("I")
		r.ID = fmt.Sprintf("%016x", i+1)
		r.Name = fmt.Sprintf("d%d", i)
		many = append(many, r)
	}
	recs, skipped, err = decode(ledgerOf(many...))
	if err != nil || len(recs) != MaxDevices || len(skipped) != 2 {
		t.Fatalf("recs %d skipped %v err %v", len(recs), skipped, err)
	}
}

func TestDecodeRefusesAFileThatIsNotALedger(t *testing.T) {
	for name, data := range map[string]string{"empty": "", "json array": "[]", "garbage": "{{{", "wrong version": `{"version":99,"devices":[]}`, "no version": `{"devices":[]}`} {
		if _, _, err := decode([]byte(data)); err == nil {
			t.Errorf("%s: no error", name)
		}
	}
}

func TestNewReadsTheLedgerLikeARestart(t *testing.T) {
	h := newHarness(t)
	issued := h.add("issued")
	waiting := h.add("waiting")
	h.request(waiting, "11111111")
	granted, _ := h.granted("granted", "22222222")
	active, _ := h.active("active", "33333333")
	revoked := h.add("revoked")
	h.reg.Revoke(revoked)

	h.clock.Advance(5 * time.Second)
	r2 := h.reload()
	got := map[string]string{}
	for _, in := range r2.List() {
		got[in.ID] = in.Status
	}
	want := map[string]string{issued: "issued", waiting: "issued", granted: "granting", active: "active", revoked: "revoked"}
	for id, st := range want {
		if got[id] != st {
			t.Errorf("after restart %s is %q, want %q", id, got[id], st)
		}
	}
	// the waiting request was only in memory: confirming finds nobody
	if _, err := r2.Confirm("11111111"); !errors.Is(err, ErrNoPending) {
		t.Errorf("Confirm after restart: %v", err)
	}
	// a grant in progress is given a full new wait for the device's ack
	h.clock.Advance(h.reg.cfg.AckTimeout - time.Second)
	r2.Tick()
	if st := statusIn(r2, granted); st != "granting" {
		t.Errorf("the restart must restart the wait: %s", st)
	}
	h.clock.Advance(time.Second)
	r2.Tick()
	if st := statusIn(r2, granted); st != "issued" {
		t.Errorf("after the wait: %s", st)
	}
}

func statusIn(r *Registry, id string) string {
	for _, in := range r.List() {
		if in.ID == id {
			return in.Status
		}
	}
	return "gone"
}

func TestNewReportsRecordsItSkipsAndRefusesAnUnreadableLedger(t *testing.T) {
	bad := goodRecord("A")
	bad.K = ""
	store := &memStore{data: ledgerOf(goodRecord("I"), bad)}
	r, skipped, err := New(Config{}, store, Hooks{})
	if err != nil || len(skipped) != 1 || len(r.List()) != 1 {
		t.Fatalf("skipped %v err %v list %d", skipped, err, len(r.List()))
	}
	if _, _, err := New(Config{}, &memStore{data: []byte("{{")}, Hooks{}); err == nil {
		t.Error("an unreadable ledger must stop the bridge")
	}
	if _, _, err := New(Config{}, &memStore{loadErr: errors.New("io error")}, Hooks{}); err == nil {
		t.Error("a ledger that cannot be read must stop the bridge")
	}
	if r, skipped, err := New(Config{}, &memStore{}, Hooks{}); err != nil || skipped != nil || len(r.List()) != 0 {
		t.Errorf("no ledger yet: %v %v", skipped, err)
	}
}

func TestALeftoverBIsDroppedAndTheNextWriteSavesIt(t *testing.T) {
	r := goodRecord("A")
	r.B = secret(9)
	store := &memStore{data: ledgerOf(r)}
	reg, skipped, err := New(Config{}, store, Hooks{})
	if err != nil || len(skipped) != 0 {
		t.Fatal(err, skipped)
	}
	if b, _, _ := reg.Secrets(r.ID); b != "" {
		t.Error("B must not be valid for an active device")
	}
	if _, err := reg.Add("another"); err != nil {
		t.Fatal(err)
	}
	var f ledgerFile
	json.Unmarshal(store.data, &f)
	if f.Devices[0].B != "" {
		t.Error("the leftover B was written back")
	}
}

func TestFileStoreWritesAtomicallyWithPrivatePermissions(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "sub")
	s := FileStore{Path: filepath.Join(dir, "devices.json")}
	if data, err := s.Load(); data != nil || err != nil {
		t.Fatalf("Load of a missing file = %v, %v", data, err)
	}
	if err := s.Save([]byte("one")); err != nil {
		t.Fatal(err)
	}
	if err := s.Save([]byte("two")); err != nil {
		t.Fatal(err)
	}
	if data, _ := s.Load(); string(data) != "two" {
		t.Errorf("Load = %q", data)
	}
	st, err := os.Stat(s.Path)
	if err != nil || st.Mode().Perm() != 0o600 {
		t.Errorf("mode %v err %v", st.Mode(), err)
	}
	entries, _ := os.ReadDir(dir)
	if len(entries) != 1 {
		t.Errorf("temporary files left behind: %v", entries)
	}
}

func TestFileStoreFailures(t *testing.T) {
	root := t.TempDir()
	// the directory cannot be created: its parent is a file
	file := filepath.Join(root, "file")
	os.WriteFile(file, nil, 0o600)
	if err := (FileStore{Path: filepath.Join(file, "x", "devices.json")}).Save([]byte("x")); err == nil {
		t.Error("MkdirAll under a file should fail")
	}
	// the target is a directory: the rename fails, and the temporary file is cleaned up
	dir := filepath.Join(root, "dir")
	os.MkdirAll(filepath.Join(dir, "devices.json", "keep"), 0o700)
	if err := (FileStore{Path: filepath.Join(dir, "devices.json")}).Save([]byte("x")); err == nil {
		t.Error("renaming over a directory should fail")
	}
	if left, _ := filepath.Glob(filepath.Join(dir, ".devices-*")); len(left) != 0 {
		t.Errorf("temporary file left: %v", left)
	}
	// Load reports an error that is not "does not exist"
	if _, err := (FileStore{Path: dir}).Load(); err == nil {
		t.Error("reading a directory should fail")
	}
	// no write permission in the directory: the temporary file cannot be created
	ro := filepath.Join(root, "ro")
	os.MkdirAll(ro, 0o500)
	if os.Geteuid() != 0 {
		if err := (FileStore{Path: filepath.Join(ro, "devices.json")}).Save([]byte("x")); err == nil {
			t.Error("a read-only directory should fail")
		}
	}
}

func TestFileStoreChecksWhatItReadsBack(t *testing.T) {
	p := filepath.Join(t.TempDir(), "devices.json")
	wrong := FileStore{Path: p, read: func(string) ([]byte, error) { return []byte("something else"), nil }}
	if err := wrong.Save([]byte("x")); err == nil || !strings.Contains(err.Error(), "read back") {
		t.Errorf("a differing read-back must fail: %v", err)
	}
	broken := FileStore{Path: p, read: func(string) ([]byte, error) { return nil, errors.New("gone") }}
	if err := broken.Save([]byte("x")); err == nil {
		t.Error("an unreadable read-back must fail")
	}
}

func TestSecretsAndIdsNeedWorkingRandomness(t *testing.T) {
	if _, err := newID(failRand{}.fill); err == nil {
		t.Error("newID")
	}
	if _, err := newSecret(failRand{}.fill); err == nil {
		t.Error("newSecret")
	}
	id, err := newID((&counterRand{}).fill)
	if err != nil || !ValidID(id) {
		t.Errorf("%q %v", id, err)
	}
	s, err := newSecret((&counterRand{}).fill)
	if err != nil || !ValidSecret(s) {
		t.Errorf("%q %v", s, err)
	}
}

func (failRand) fill(b []byte) error       { _, err := failRand{}.Read(b); return err }
func (r *counterRand) fill(b []byte) error { _, err := r.Read(b); return err }
