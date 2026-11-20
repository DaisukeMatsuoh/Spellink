package devices

import (
	"errors"
	"log"
	"strings"
	"testing"
	"time"
)

func TestAddIssuesARecordWithOnlyB(t *testing.T) {
	h := newHarness(t)
	id := h.add("Nomad")
	if !ValidID(id) {
		t.Fatalf("id %q", id)
	}
	b, k, ok := h.reg.Secrets(id)
	if !ok || !ValidSecret(b) || k != "" {
		t.Fatalf("Secrets = %q, %q, %v: want B only", b, k, ok)
	}
	recs := h.ledger()
	if len(recs) != 1 || recs[0].ID != id || recs[0].State != "I" || recs[0].Gen != 1 || recs[0].B != b || recs[0].K != "" {
		t.Fatalf("stored record = %+v", recs)
	}
	if got := h.reg.List(); len(got) != 1 || got[0].Status != "issued" || !got[0].Expires.Equal(h.clock.Now().Add(h.reg.cfg.TTL)) {
		t.Fatalf("List = %+v", got)
	}
}

func TestAddRefusesBadNamesDuplicatesAndAFullLedger(t *testing.T) {
	h := newHarness(t)
	for _, name := range []string{"", strings.Repeat("あ", 65), "tab\there", "bad\xffutf8"} {
		if _, err := h.reg.Add(name); !errors.Is(err, ErrBadName) {
			t.Errorf("Add(%q) = %v, want ErrBadName", name, err)
		}
	}
	h.add("one")
	if _, err := h.reg.Add("one"); !errors.Is(err, ErrNameTaken) {
		t.Errorf("duplicate name: %v", err)
	}
	for i := 1; i < MaxDevices; i++ {
		h.add("dev-" + strings.Repeat("x", i))
	}
	saves := h.store.saves
	if _, err := h.reg.Add("one too many"); !errors.Is(err, ErrFull) {
		t.Errorf("17th device: %v", err)
	}
	if len(h.reg.List()) != MaxDevices || h.store.saves != saves {
		t.Errorf("a refused Add changed the ledger")
	}
}

func TestAddSurvivesAnIdCollisionButNotFourInARow(t *testing.T) {
	first := make([]byte, idBytes)
	for i := range first {
		first[i] = byte(i)
	}
	sr := &scriptRand{}
	h := newHarness(t, func(c *Config) { c.Rand = sr })
	one := h.add("one") // takes bytes 0..7 as its id
	if one != "0001020304050607" {
		t.Fatalf("test assumption: id %s", one)
	}
	sr.queue = [][]byte{first, first} // two collisions, then a fresh id
	two := h.add("two")
	if two == one {
		t.Fatal("id collision was not resolved")
	}
	sr.queue = [][]byte{first, first, first, first}
	if _, err := h.reg.Add("three"); err == nil || !strings.Contains(err.Error(), "free device id") {
		t.Errorf("four collisions: %v", err)
	}
	if len(h.reg.List()) != 2 {
		t.Error("a failed Add left a record behind")
	}
}

func TestAddRollsBackWhenRandomnessOrTheLedgerFails(t *testing.T) {
	h := newHarness(t, func(c *Config) { c.Rand = failRand{} })
	if _, err := h.reg.Add("x"); err == nil {
		t.Error("id: no randomness should fail")
	}
	// the id comes first; make only the secret fail
	sr := &scriptRand{}
	h = newHarness(t, func(c *Config) { c.Rand = sr })
	h.reg.cfg.Rand = &afterN{n: idBytes}
	if _, err := h.reg.Add("x"); err == nil {
		t.Error("B: no randomness should fail")
	}
	if len(h.reg.List()) != 0 {
		t.Error("a failed Add left a record behind")
	}
	h = newHarness(t)
	h.store.saveErr = errors.New("disk full")
	if _, err := h.reg.Add("x"); err == nil || len(h.reg.List()) != 0 || h.store.data != nil {
		t.Errorf("a ledger that cannot be written must not gain a record: %v", err)
	}
}

// afterN serves n bytes and then fails.
type afterN struct{ n int }

func (a *afterN) Read(p []byte) (int, error) {
	if a.n <= 0 {
		return 0, errors.New("out of randomness")
	}
	n := min(len(p), a.n)
	a.n -= n
	return n, nil
}

func TestPairingFromRequestToActive(t *testing.T) {
	h := newHarness(t)
	id := h.add("Nomad")
	conn := h.request(id, "12345678")
	h.mustStatus(id, "waiting")

	if name, err := h.reg.Confirm("12345678"); err != nil || name != "Nomad" {
		t.Fatalf("Confirm = %q, %v", name, err)
	}
	h.mustStatus(id, "granting")
	if len(conn.grants) != 1 || !ValidSecret(conn.grants[0].key) {
		t.Fatalf("grants = %+v", conn.grants)
	}
	k := conn.grants[0].key
	if got := conn.grants[0].eps; len(got) != 1 || got[0] != "wss://bridge.example/v1?d="+id {
		t.Errorf("endpoints sent with K: %v", got)
	}
	if len(h.hooks.endpoints) != 1 || h.hooks.endpoints[0] != [2]string{id, k} {
		t.Errorf("Endpoints hook saw %v", h.hooks.endpoints)
	}
	// K must be in the ledger before the device is told (a crash in between must not lose it).
	atGrant, _, err := decode(conn.atGrant)
	if err != nil || len(atGrant) != 1 || atGrant[0].State != "G" || atGrant[0].K != k || atGrant[0].B == "" {
		t.Fatalf("ledger at the moment of the grant: %+v %v", atGrant, err)
	}
	b, k2, _ := h.reg.Secrets(id)
	if b == "" || k2 != k {
		t.Errorf("while granting both secrets are valid: B=%q K=%q", b, k2)
	}

	if err := h.reg.Ack(id, h.gen(id)); err != nil {
		t.Fatal(err)
	}
	h.mustStatus(id, "active")
	b, k2, _ = h.reg.Secrets(id)
	if b != "" || k2 != k {
		t.Errorf("after the ack B is gone and K stays: B=%q K=%q", b, k2)
	}
	if recs := h.ledger(); recs[0].State != "A" || recs[0].B != "" || recs[0].K != k {
		t.Errorf("stored: %+v", recs[0])
	}
	if h.hooks.changed == 0 {
		t.Error("the relay was not told the secrets changed")
	}
}

func TestAConnectionWithKCountsAsTheAck(t *testing.T) {
	h := newHarness(t)
	id, _ := h.granted("Nomad", "12345678")
	if err := h.reg.ConnectK(id); err != nil {
		t.Fatal(err)
	}
	h.mustStatus(id, "active")
	if b, _, _ := h.reg.Secrets(id); b != "" {
		t.Error("B must be gone once K is in use")
	}
}

func TestConnectKErrors(t *testing.T) {
	h := newHarness(t)
	if err := h.reg.ConnectK("0000000000000000"); !errors.Is(err, ErrUnknownDevice) {
		t.Errorf("unknown: %v", err)
	}
	id := h.add("Nomad")
	if err := h.reg.ConnectK(id); !errors.Is(err, ErrNotEligible) {
		t.Errorf("K before it exists: %v", err)
	}
}

func TestConnectingWithKOnlyWritesLastSeenOncePerMinute(t *testing.T) {
	h := newHarness(t)
	id, _ := h.active("Nomad", "12345678")
	saves := h.store.saves
	h.clock.Advance(lastSeenEvery - time.Second)
	if err := h.reg.ConnectK(id); err != nil {
		t.Fatal(err)
	}
	if h.store.saves != saves {
		t.Error("a connection within the minute rewrote the ledger")
	}
	h.clock.Advance(2 * time.Second)
	if err := h.reg.ConnectK(id); err != nil {
		t.Fatal(err)
	}
	if h.store.saves != saves+1 || h.ledger()[0].LastSeen.IsZero() {
		t.Error("a connection after the minute must store the time")
	}
}

func TestAckOnlyCountsForTheRightGeneration(t *testing.T) {
	h := newHarness(t)
	id, _ := h.granted("Nomad", "12345678")
	if err := h.reg.Ack(id, h.gen(id)+1); !errors.Is(err, ErrUnknownDevice) {
		t.Errorf("wrong gen: %v", err)
	}
	if err := h.reg.Ack("ffffffffffffffff", 1); !errors.Is(err, ErrUnknownDevice) {
		t.Errorf("unknown id: %v", err)
	}
	h.mustStatus(id, "granting")
	// an ack in a state that is not granting is a late or repeated frame: nothing happens
	other := h.add("Other")
	if err := h.reg.Ack(other, 1); err != nil {
		t.Errorf("stray ack: %v", err)
	}
	h.mustStatus(other, "issued")
}

func TestRequestValidatesWhatItIsGiven(t *testing.T) {
	h := newHarness(t)
	id := h.add("Nomad")
	for _, c := range []struct {
		id, code string
		gen      int
	}{
		{"bad", "12345678", 1}, {id, "1234567", 1}, {id, "1234567a", 1}, {id, "12345678", 0},
	} {
		conn := &fakeConn{}
		if err := h.reg.Request(c.id, c.gen, c.code, conn); !errors.Is(err, ErrBadRequest) || conn.reasons() != DenyUnknownDevice {
			t.Errorf("%+v: err %v denied %q", c, err, conn.reasons())
		}
	}
	conn := &fakeConn{}
	if err := h.reg.Request("ffffffffffffffff", 1, "12345678", conn); !errors.Is(err, ErrUnknownDevice) || conn.reasons() != DenyUnknownDevice {
		t.Errorf("unknown device: %v %q", err, conn.reasons())
	}
	conn = &fakeConn{}
	if err := h.reg.Request(id, 2, "12345678", conn); !errors.Is(err, ErrUnknownDevice) || conn.reasons() != DenyUnknownDevice {
		t.Errorf("old package (wrong gen): %v %q", err, conn.reasons())
	}
	h.mustStatus(id, "issued")
}

func TestRequestFromADeviceThatCannotPairIsRefused(t *testing.T) {
	h := newHarness(t)
	active, _ := h.active("A", "11111111")
	revoked := h.add("R")
	if err := h.reg.Revoke(revoked); err != nil {
		t.Fatal(err)
	}
	for _, id := range []string{active, revoked} {
		conn := &fakeConn{}
		if err := h.reg.Request(id, h.gen(id), "22222222", conn); err != nil || conn.reasons() != DenyUnknownDevice {
			t.Errorf("%s: err %v denied %q", id, err, conn.reasons())
		}
	}
	if !strings.Contains(h.logs.String(), "devices.never") {
		t.Error("the impossible request was not recorded")
	}
}

func TestRequestWhileGrantingIsRefused(t *testing.T) {
	h := newHarness(t)
	id, _ := h.granted("Nomad", "12345678")
	conn := &fakeConn{}
	if err := h.reg.Request(id, h.gen(id), "87654321", conn); err != nil || conn.reasons() != DenyGrantPending {
		t.Errorf("err %v denied %q", err, conn.reasons())
	}
	h.mustStatus(id, "granting")
}

func TestTheSameCodeTwiceCancelsBothAndNeverGrantsEither(t *testing.T) {
	h := newHarness(t)
	id := h.add("Nomad")
	first := h.request(id, "12345678")
	second := h.request(id, "12345678") // someone who saw the code on the screen
	if first.reasons() != DenyCodeInUse || second.reasons() != DenyCodeInUse {
		t.Errorf("first %q second %q", first.reasons(), second.reasons())
	}
	h.mustStatus(id, "issued") // nobody is left waiting
	if _, err := h.reg.Confirm("12345678"); !errors.Is(err, ErrNoPending) {
		t.Errorf("Confirm after the taint: %v", err)
	}
}

func TestTheSameCodeAmongSeveralRequestsKeepsTheOthers(t *testing.T) {
	h := newHarness(t)
	id := h.add("Nomad")
	a := h.request(id, "11111111")
	b := h.request(id, "22222222")
	c := h.request(id, "11111111")
	if a.reasons() != DenyCodeInUse || c.reasons() != DenyCodeInUse || b.reasons() != "" {
		t.Errorf("a %q b %q c %q", a.reasons(), b.reasons(), c.reasons())
	}
	h.mustStatus(id, "waiting")
	if _, err := h.reg.Confirm("22222222"); err != nil || len(b.grants) != 1 {
		t.Errorf("the untouched request must still be grantable: %v", err)
	}
}

func TestTooManyWaitingRequestsAreRefused(t *testing.T) {
	h := newHarness(t, func(c *Config) { c.MaxPending = 2 })
	id := h.add("Nomad")
	h.request(id, "11111111")
	h.request(id, "22222222")
	third := h.request(id, "33333333")
	if third.reasons() != DenyPairBusy {
		t.Errorf("third: %q", third.reasons())
	}
	h.mustStatus(id, "waiting")
	if got := h.reg.List()[0].Waiting; got != 2 {
		t.Errorf("waiting = %d", got)
	}
}

func TestConfirmRefusesWhatItCannotUse(t *testing.T) {
	h := newHarness(t)
	if _, err := h.reg.Confirm("12345"); !errors.Is(err, ErrBadCode) {
		t.Errorf("short code: %v", err)
	}
	if _, err := h.reg.Confirm("12345678"); !errors.Is(err, ErrNoPending) {
		t.Errorf("nobody waiting: %v", err)
	}
	id := h.add("Nomad")
	h.request(id, "12345678")
	if _, err := h.reg.Confirm("00000000"); !errors.Is(err, ErrCodeRejected) {
		t.Errorf("wrong code: %v", err)
	}
	h.mustStatus(id, "waiting")
}

func TestFiveWrongCodesCloseTheWindow(t *testing.T) {
	h := newHarness(t)
	id := h.add("Nomad")
	conn := h.request(id, "12345678")
	for i := 0; i < 4; i++ {
		if _, err := h.reg.Confirm("00000000"); !errors.Is(err, ErrCodeRejected) {
			t.Fatalf("attempt %d: %v", i+1, err)
		}
		h.mustStatus(id, "waiting")
	}
	if _, err := h.reg.Confirm("00000000"); !errors.Is(err, ErrCodeRejected) {
		t.Fatal(err)
	}
	h.mustStatus(id, "issued")
	if conn.reasons() != DenyWindowClosed {
		t.Errorf("the waiting request was told %q", conn.reasons())
	}
	// the right code no longer works, but a new request opens a fresh window with a fresh count
	if _, err := h.reg.Confirm("12345678"); !errors.Is(err, ErrNoPending) {
		t.Errorf("after the window closed: %v", err)
	}
	h.request(id, "87654321")
	if _, err := h.reg.Confirm("87654321"); err != nil {
		t.Errorf("a new window starts with no failures: %v", err)
	}
}

func TestACodeThatTwoDevicesShowIsAmbiguousAndGrantsNeither(t *testing.T) {
	h := newHarness(t)
	one, two := h.add("one"), h.add("two")
	c1, c2 := h.request(one, "12345678"), h.request(two, "12345678")
	if _, err := h.reg.Confirm("12345678"); !errors.Is(err, ErrCodeRejected) {
		t.Errorf("ambiguous code: %v", err)
	}
	if len(c1.grants)+len(c2.grants) != 0 {
		t.Error("an ambiguous code granted a device")
	}
	h.mustStatus(one, "waiting")
	h.mustStatus(two, "waiting")
}

func TestConfirmPicksTheRightDeviceAmongSeveral(t *testing.T) {
	h := newHarness(t)
	one, two := h.add("one"), h.add("two")
	c1, c2 := h.request(one, "11111111"), h.request(two, "22222222")
	if name, err := h.reg.Confirm("22222222"); err != nil || name != "two" {
		t.Fatalf("Confirm = %q %v", name, err)
	}
	if len(c1.grants) != 0 || len(c2.grants) != 1 {
		t.Errorf("grants: one %d two %d", len(c1.grants), len(c2.grants))
	}
	h.mustStatus(one, "waiting")
}

func TestGrantingIsUndoneWhenTheLedgerCannotBeWritten(t *testing.T) {
	h := newHarness(t)
	id := h.add("Nomad")
	conn := h.request(id, "12345678")
	h.store.saveErr = errors.New("disk full")
	if _, err := h.reg.Confirm("12345678"); err == nil {
		t.Fatal("Confirm must fail")
	}
	h.mustStatus(id, "waiting")
	if len(conn.grants) != 0 {
		t.Error("K was sent although it was not stored")
	}
	if b, k, _ := h.reg.Secrets(id); b == "" || k != "" {
		t.Errorf("secrets after the rollback: B=%q K=%q", b, k)
	}
	h.store.saveErr = nil // the owner can try again
	if _, err := h.reg.Confirm("12345678"); err != nil || len(conn.grants) != 1 {
		t.Errorf("retry: %v", err)
	}
}

func TestGrantingFailsCleanlyWithoutRandomness(t *testing.T) {
	h := newHarness(t)
	id := h.add("Nomad")
	conn := h.request(id, "12345678")
	h.reg.cfg.Rand = failRand{}
	if _, err := h.reg.Confirm("12345678"); err == nil || len(conn.grants) != 0 {
		t.Errorf("err %v grants %d", err, len(conn.grants))
	}
	h.mustStatus(id, "waiting")
}

func TestAGrantThatCannotBeSentReturnsTheDeviceToIssued(t *testing.T) {
	h := newHarness(t)
	id := h.add("Nomad")
	conn := &fakeConn{grantErr: errors.New("connection reset"), store: h.store}
	if err := h.reg.Request(id, 1, "12345678", conn); err != nil {
		t.Fatal(err)
	}
	if _, err := h.reg.Confirm("12345678"); err != nil {
		t.Fatal(err)
	}
	h.mustStatus(id, "issued")
	if _, k, _ := h.reg.Secrets(id); k != "" {
		t.Error("K must be dropped when it could not be sent")
	}
	if recs := h.ledger(); recs[0].State != "I" || recs[0].K != "" {
		t.Errorf("stored: %+v", recs[0])
	}
}

func TestDisconnectDropsOnlyThatRequest(t *testing.T) {
	h := newHarness(t)
	id := h.add("Nomad")
	a, b := h.request(id, "11111111"), h.request(id, "22222222")
	h.reg.Disconnect(a)
	h.mustStatus(id, "waiting")
	if _, err := h.reg.Confirm("11111111"); !errors.Is(err, ErrCodeRejected) {
		t.Errorf("the dropped request must not be confirmable: %v", err)
	}
	h.reg.Disconnect(b)
	h.mustStatus(id, "issued")
	h.reg.Disconnect(&fakeConn{}) // a connection nobody knows
	h.mustStatus(id, "issued")
}

func TestTheWindowClosesByItself(t *testing.T) {
	h := newHarness(t)
	id := h.add("Nomad")
	conn := h.request(id, "12345678")
	h.clock.Advance(h.reg.cfg.Window - time.Second)
	h.reg.Tick()
	h.mustStatus(id, "waiting")
	h.clock.Advance(time.Second)
	h.reg.Tick()
	h.mustStatus(id, "issued")
	if conn.reasons() != DenyWindowClosed {
		t.Errorf("request told %q", conn.reasons())
	}
	if _, err := h.reg.Confirm("12345678"); !errors.Is(err, ErrNoPending) {
		t.Errorf("after the window: %v", err)
	}
}

func TestAnUnconfirmedGrantFallsBackToIssued(t *testing.T) {
	h := newHarness(t)
	id, _ := h.granted("Nomad", "12345678")
	h.clock.Advance(h.reg.cfg.AckTimeout)
	h.reg.Tick()
	h.mustStatus(id, "issued")
	if b, k, _ := h.reg.Secrets(id); b == "" || k != "" {
		t.Errorf("B must stay and K must go: B=%q K=%q", b, k)
	}
	h.request(id, "87654321") // the device may ask again
	h.mustStatus(id, "waiting")
}

func TestBExpiresAfterItsTimeToLive(t *testing.T) {
	// the window is longer than the TTL here, so the TTL is what ends the waiting
	h := newHarness(t, func(c *Config) { c.TTL = time.Minute; c.Window = time.Hour })
	issued, queued := h.add("issued"), h.add("queued")
	conn := h.request(queued, "12345678")
	h.clock.Advance(h.reg.cfg.TTL)
	h.reg.Tick()
	h.mustStatus(issued, "expired")
	h.mustStatus(queued, "expired")
	if conn.reasons() != DenyExpired {
		t.Errorf("waiting request told %q", conn.reasons())
	}
	if _, _, ok := h.reg.Secrets(issued); ok {
		t.Error("an expired device still has a valid secret")
	}
	if recs := h.ledger(); recs[0].State != "X" || recs[0].B != "" {
		t.Errorf("stored: %+v", recs[0])
	}
}

func TestAGrantInProgressOutlivesTheTTLAndThenExpires(t *testing.T) {
	h := newHarness(t, func(c *Config) { c.TTL = time.Minute })
	id, _ := h.granted("Nomad", "12345678")
	h.clock.Advance(10 * time.Second)
	h.reg.Tick()
	h.mustStatus(id, "granting") // TTL has not passed yet
	h.clock.Advance(time.Minute)
	h.reg.Tick() // TTL and the ack timeout are both due: the ack timeout returns it to issued, which expires at once
	h.mustStatus(id, "expired")
	if _, _, ok := h.reg.Secrets(id); ok {
		t.Error("expired device keeps a secret")
	}
}

func TestAWindowThatEndsAfterTheTTLLeavesTheDeviceExpired(t *testing.T) {
	h := newHarness(t, func(c *Config) { c.TTL = time.Hour; c.Window = 2 * time.Hour })
	id := h.add("Nomad")
	h.request(id, "12345678")
	h.clock.Advance(3 * time.Hour)
	h.reg.Tick()
	h.mustStatus(id, "expired")
}

func TestAnActiveDeviceNeverExpires(t *testing.T) {
	h := newHarness(t)
	id, _ := h.active("Nomad", "12345678")
	h.clock.Advance(1000 * time.Hour)
	h.reg.Tick()
	h.mustStatus(id, "active")
}

func TestRevokeRemovesBothSecretsAndEndsEverySession(t *testing.T) {
	h := newHarness(t)
	id, _ := h.active("Nomad", "12345678")
	if err := h.reg.Revoke(id); err != nil {
		t.Fatal(err)
	}
	h.mustStatus(id, "revoked")
	if _, _, ok := h.reg.Secrets(id); ok {
		t.Error("a revoked device keeps a secret")
	}
	if len(h.hooks.closed) != 1 || h.hooks.closed[0] != id {
		t.Errorf("sessions closed: %v", h.hooks.closed)
	}
	if recs := h.ledger(); recs[0].State != "R" || recs[0].K != "" || recs[0].RevokedAt.IsZero() {
		t.Errorf("stored: %+v", recs[0])
	}
	if err := h.reg.Revoke(id); !errors.Is(err, ErrAlreadyRevoked) {
		t.Errorf("second revoke: %v", err)
	}
	if err := h.reg.Revoke("ffffffffffffffff"); !errors.Is(err, ErrUnknownDevice) {
		t.Errorf("unknown: %v", err)
	}
}

func TestRevokeTellsWaitingRequests(t *testing.T) {
	h := newHarness(t)
	id := h.add("Nomad")
	conn := h.request(id, "12345678")
	if err := h.reg.Revoke(id); err != nil || conn.reasons() != DenyUnknownDevice {
		t.Errorf("err %v told %q", err, conn.reasons())
	}
}

func TestRevokeHoldsEvenWhenTheLedgerCannotBeWritten(t *testing.T) {
	h := newHarness(t)
	id, _ := h.active("Nomad", "12345678")
	h.store.saveErr = errors.New("disk full")
	err := h.reg.Revoke(id)
	if !errors.Is(err, ErrNotPersisted) {
		t.Fatalf("err = %v", err)
	}
	h.mustStatus(id, "revoked") // kept in memory: a revoked device must not be served
	if len(h.hooks.closed) != 1 {
		t.Error("sessions must be closed even though the write failed")
	}
	if recs := h.ledger(); recs[0].State != "A" {
		t.Errorf("the file still says %s: the owner has been told it was not saved", recs[0].State)
	}
}

func TestReissueGivesANewBAndDropsK(t *testing.T) {
	h := newHarness(t)
	id, _ := h.active("Nomad", "12345678")
	oldB := h.ledger()[0].B
	if err := h.reg.Reissue(id); err != nil {
		t.Fatal(err)
	}
	h.mustStatus(id, "issued")
	b, k, _ := h.reg.Secrets(id)
	if b == "" || b == oldB || k != "" || h.gen(id) != 2 {
		t.Errorf("B=%q (old %q) K=%q gen=%d", b, oldB, k, h.gen(id))
	}
	if len(h.hooks.closed) != 1 || h.hooks.closed[0] != id {
		t.Errorf("the old K's sessions must end: %v", h.hooks.closed)
	}
	// the old package (gen 1) is refused
	conn := &fakeConn{}
	if err := h.reg.Request(id, 1, "11111111", conn); !errors.Is(err, ErrUnknownDevice) {
		t.Errorf("old generation: %v", err)
	}
}

func TestReissueFromExpiredAndFromWaiting(t *testing.T) {
	h := newHarness(t)
	expired := h.add("expired")
	h.clock.Advance(h.reg.cfg.TTL)
	h.reg.Tick()
	if err := h.reg.Reissue(expired); err != nil {
		t.Fatal(err)
	}
	h.mustStatus(expired, "issued")
	if b, _, ok := h.reg.Secrets(expired); !ok || b == "" {
		t.Error("a reissued device must have a B again")
	}
	waiting := h.add("waiting")
	conn := h.request(waiting, "12345678")
	if err := h.reg.Reissue(waiting); err != nil || conn.reasons() != DenySuperseded {
		t.Errorf("err %v told %q", err, conn.reasons())
	}
}

func TestReissueRefusalsAndRollback(t *testing.T) {
	h := newHarness(t)
	id, _ := h.active("Nomad", "12345678")
	if err := h.reg.Reissue("ffffffffffffffff"); !errors.Is(err, ErrUnknownDevice) {
		t.Errorf("unknown: %v", err)
	}
	h.reg.cfg.Rand = failRand{}
	if err := h.reg.Reissue(id); err == nil {
		t.Error("no randomness should fail")
	}
	h.reg.cfg.Rand = &counterRand{n: 200}
	h.store.saveErr = errors.New("disk full")
	if err := h.reg.Reissue(id); err == nil {
		t.Error("an unwritable ledger should fail")
	}
	h.mustStatus(id, "active")
	if h.gen(id) != 1 || len(h.hooks.closed) != 0 {
		t.Error("a failed reissue must change nothing")
	}
	h.store.saveErr = nil
	if err := h.reg.Revoke(id); err != nil {
		t.Fatal(err)
	}
	if err := h.reg.Reissue(id); !errors.Is(err, ErrRevoked) {
		t.Errorf("a revoked device cannot be reissued: %v", err)
	}
}

func TestForgetOnlyDeletesRevokedOrExpiredRecords(t *testing.T) {
	h := newHarness(t)
	active, _ := h.active("A", "11111111")
	if err := h.reg.Forget(active); !errors.Is(err, ErrStillValid) {
		t.Errorf("active: %v", err)
	}
	if err := h.reg.Forget("ffffffffffffffff"); !errors.Is(err, ErrUnknownDevice) {
		t.Errorf("unknown: %v", err)
	}
	if err := h.reg.Revoke(active); err != nil {
		t.Fatal(err)
	}
	expired := h.add("X")
	h.clock.Advance(h.reg.cfg.TTL)
	h.reg.Tick()
	for _, id := range []string{active, expired} {
		if err := h.reg.Forget(id); err != nil {
			t.Fatalf("Forget(%s): %v", id, err)
		}
	}
	if len(h.reg.List()) != 0 || len(h.ledger()) != 0 {
		t.Error("forgotten records are still there")
	}
	// the name is free again
	h.add("A")
}

func TestForgetIsUndoneWhenTheLedgerCannotBeWritten(t *testing.T) {
	h := newHarness(t)
	id := h.add("one")
	h.add("two")
	if err := h.reg.Revoke(id); err != nil {
		t.Fatal(err)
	}
	h.store.saveErr = errors.New("disk full")
	if err := h.reg.Forget(id); err == nil {
		t.Fatal("Forget must fail")
	}
	list := h.reg.List()
	if len(list) != 2 || list[0].ID != id {
		t.Errorf("the record must come back where it was: %+v", list)
	}
}

func TestRepackChangesNothing(t *testing.T) {
	h := newHarness(t)
	id := h.add("Nomad")
	saves := h.store.saves
	d, err := h.reg.Repack(id)
	if err != nil || d.ID != id || d.Name != "Nomad" || d.Gen != 1 || !ValidSecret(d.B) || d.Window != h.reg.cfg.Window {
		t.Fatalf("Repack = %+v, %v", d, err)
	}
	h.request(id, "12345678")
	if d, err = h.reg.Repack(id); err != nil || d.B == "" {
		t.Errorf("while waiting the package still carries B: %+v %v", d, err)
	}
	h.reg.Confirm("12345678")
	if d, err = h.reg.Repack(id); err != nil || d.B == "" {
		t.Errorf("while granting the package still carries B: %+v %v", d, err)
	}
	h.reg.Ack(id, 1)
	if d, err = h.reg.Repack(id); err != nil || d.B != "" {
		t.Errorf("after pairing the package carries no secret: %+v %v", d, err)
	}
	if h.store.saves != saves+3 { // only the confirm, the ack and the request's ledger changes... see below
		// the saves come from Confirm (G) and Ack (A); a request changes nothing stored
		if h.store.saves != saves+2 {
			t.Errorf("Repack wrote the ledger: saves %d → %d", saves, h.store.saves)
		}
	}
}

func TestRepackRefusals(t *testing.T) {
	h := newHarness(t)
	if _, err := h.reg.Repack("ffffffffffffffff"); !errors.Is(err, ErrUnknownDevice) {
		t.Errorf("unknown: %v", err)
	}
	expired := h.add("X")
	h.clock.Advance(h.reg.cfg.TTL)
	h.reg.Tick()
	if _, err := h.reg.Repack(expired); !errors.Is(err, ErrExpired) {
		t.Errorf("expired: %v", err)
	}
	revoked := h.add("R")
	h.reg.Revoke(revoked)
	if _, err := h.reg.Repack(revoked); !errors.Is(err, ErrRevoked) {
		t.Errorf("revoked: %v", err)
	}
}

func TestActiveSecretsListsEveryValidSecret(t *testing.T) {
	h := newHarness(t)
	issued := h.add("issued")
	granted, _ := h.granted("granted", "11111111")
	active, _ := h.active("active", "22222222")
	revoked := h.add("revoked")
	h.reg.Revoke(revoked)
	got := map[string]int{}
	for _, s := range h.reg.ActiveSecrets() {
		if !ValidSecret(s.Secret) {
			t.Errorf("bad secret for %s", s.ID)
		}
		key := s.ID + "/B"
		if s.IsK {
			key = s.ID + "/K"
		}
		got[key]++
	}
	want := map[string]int{issued + "/B": 1, granted + "/B": 1, granted + "/K": 1, active + "/K": 1}
	if len(got) != len(want) {
		t.Fatalf("got %v want %v", got, want)
	}
	for k := range want {
		if got[k] != 1 {
			t.Errorf("missing %s in %v", k, got)
		}
	}
}

func TestSecretsOfAnUnknownDevice(t *testing.T) {
	h := newHarness(t)
	if _, _, ok := h.reg.Secrets("ffffffffffffffff"); ok {
		t.Error("an unknown device has no secrets")
	}
}

func TestChangedIsCalledOnlyWhenSomethingChanged(t *testing.T) {
	h := newHarness(t)
	id := h.add("Nomad")
	n := h.hooks.changed
	h.reg.Tick() // nothing is due
	h.reg.Disconnect(&fakeConn{})
	if _, err := h.reg.Confirm("12345678"); err == nil {
		t.Fatal("nobody is waiting")
	}
	if h.hooks.changed != n {
		t.Errorf("Changed was called %d extra time(s) for no change", h.hooks.changed-n)
	}
	h.request(id, "12345678")
	if h.hooks.changed == n {
		t.Error("a new request changes the state: Changed expected")
	}
}

func TestRegistryWorksWithoutHooksAndWithDefaults(t *testing.T) {
	r, _, err := New(Config{}, &memStore{}, Hooks{})
	if err != nil {
		t.Fatal(err)
	}
	id, err := r.Add("Nomad")
	if err != nil {
		t.Fatal(err)
	}
	c := &fakeConn{}
	if err := r.Request(id, 1, "12345678", c); err != nil {
		t.Fatal(err)
	}
	if _, err := r.Confirm("12345678"); err != nil || len(c.grants) != 1 || c.grants[0].eps != nil {
		t.Errorf("Confirm: %v grants %+v", err, c.grants)
	}
	if err := r.Revoke(id); err != nil {
		t.Error(err)
	}
	if cfg := (Config{}).withDefaults(); cfg.Window != 90*time.Second || cfg.TTL != 24*time.Hour || cfg.AckTimeout != 30*time.Second || cfg.MaxPending != 4 || cfg.MaxAttempts != 5 {
		t.Errorf("defaults: %+v", cfg)
	}
}

func TestRegistryIsSafeForConcurrentUse(t *testing.T) {
	h := newHarness(t)
	id := h.add("Nomad")
	done := make(chan struct{})
	for i := 0; i < 8; i++ {
		go func(i int) {
			defer func() { done <- struct{}{} }()
			for j := 0; j < 50; j++ {
				c := &fakeConn{}
				_ = h.reg.Request(id, 1, "1234567"+string(rune('0'+i)), c)
				h.reg.Disconnect(c)
				h.reg.List()
				h.reg.Tick()
				h.reg.Secrets(id)
			}
		}(i)
	}
	for i := 0; i < 8; i++ {
		<-done
	}
}

func TestConfirmingOneRequestTellsTheOthersTheyWereSuperseded(t *testing.T) {
	h := newHarness(t)
	id := h.add("Nomad")
	a, b := h.request(id, "11111111"), h.request(id, "22222222")
	if _, err := h.reg.Confirm("22222222"); err != nil {
		t.Fatal(err)
	}
	if a.reasons() != DenySuperseded || len(b.grants) != 1 || b.reasons() != "" {
		t.Errorf("a %q, b grants %d denied %q", a.reasons(), len(b.grants), b.reasons())
	}
}

func TestAnOldGrantFailureDoesNotTouchANewerState(t *testing.T) {
	h := newHarness(t)
	id, _ := h.granted("Nomad", "12345678")
	gen := h.gen(id)
	for name, f := range map[string]func() error{
		"unknown device": func() error { return h.reg.timeoutGrant("ffffffffffffffff", gen) },
		"another gen":    func() error { return h.reg.timeoutGrant(id, gen+1) },
	} {
		if err := h.reg.locked(f); err != nil {
			t.Errorf("%s: %v", name, err)
		}
		h.mustStatus(id, "granting")
	}
	if err := h.reg.Ack(id, gen); err != nil {
		t.Fatal(err)
	}
	if err := h.reg.locked(func() error { return h.reg.timeoutGrant(id, gen) }); err != nil {
		t.Fatal(err)
	}
	h.mustStatus(id, "active") // a late failure report must not undo an acknowledged grant
}

// A write that fails after an action that normally writes nothing: these tests force the ledger to
// differ from what is stored so that the error from the store reaches the caller.
func TestStoreErrorsReachTheCaller(t *testing.T) {
	h := newHarness(t)
	id := h.add("Nomad")
	h.request(id, "12345678")
	h.reg.saved = []byte("stale")
	h.store.saveErr = errors.New("disk full")
	if _, err := h.reg.Confirm("00000000"); err == nil || errors.Is(err, ErrCodeRejected) {
		t.Errorf("Confirm with a failing store: %v", err)
	}
	h2 := newHarness(t)
	id2 := h2.add("Nomad")
	h2.reg.saved = []byte("stale")
	h2.store.saveErr = errors.New("disk full")
	if _, err := h2.reg.Repack(id2); err == nil {
		t.Error("Repack with a failing store")
	}
}

func TestADateTheLedgerCannotWriteIsRefusedAndRolledBack(t *testing.T) {
	h := newHarness(t)
	h.clock.t = time.Date(10000, 1, 1, 0, 0, 0, 0, time.UTC) // JSON cannot hold a year above 9999
	if _, err := h.reg.Add("Nomad"); err == nil {
		t.Fatal("encoding must fail")
	}
	if len(h.reg.List()) != 0 {
		t.Error("a record that cannot be stored must not stay")
	}
}

func TestStartingWithALedgerThatNeedsRewritingAndCannotBeWrittenStillServes(t *testing.T) {
	r := goodRecord("A")
	r.B = secret(9)
	store := &memStore{data: ledgerOf(r), saveErr: errors.New("read-only file system")}
	var logs strings.Builder
	reg, _, err := New(Config{Log: log.New(&logWriter{b: &logs}, "", 0)}, store, Hooks{})
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	if _, k, ok := reg.Secrets(r.ID); !ok || k != secret(2) {
		t.Error("an active device must still be served")
	}
	if !strings.Contains(logs.String(), "could not rewrite") {
		t.Errorf("not logged: %q", logs.String())
	}
}
