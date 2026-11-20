// Package devices is the bridge's ledger of paired devices (ADR 0016, docs/spec/pairing.md).
//
// Each device has a record. A record holds a first secret B (put into the device's own
// .snplg, good only for pairing, with an expiry) and, once the owner has confirmed the pairing,
// a second secret K that the device keeps and uses from then on; B is thrown away when K is
// known to have arrived. The record's state machine is the table in docs/spec/pairing.md,
// generated into table_gen.go.
package devices

import (
	"bytes"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"time"
	"unicode"
	"unicode/utf8"
)

const (
	ledgerVersion = 1
	secretBytes   = 32
	idBytes       = 8
	maxNameRunes  = 64
	maxGen        = 1 << 30
	// MaxDevices is how many records the ledger holds.
	MaxDevices = 16
)

var (
	idRe   = regexp.MustCompile(`^[0-9a-f]{16}$`)
	codeRe = regexp.MustCompile(`^[0-9]{8}$`)
)

// Record is one device in the ledger as it is stored. Requests waiting for the owner's
// confirmation (state Q) are never stored: after a restart the record is simply issued again.
type Record struct {
	ID        string    `json:"id"`
	Name      string    `json:"name"`
	Gen       int       `json:"gen"`
	State     string    `json:"state"` // issued, granted, active, expired or revoked (see stateCodes)
	B         string    `json:"b,omitempty"`
	K         string    `json:"k,omitempty"`
	IssuedAt  time.Time `json:"issuedAt"`
	RevokedAt time.Time `json:"revokedAt,omitzero"`
	LastSeen  time.Time `json:"lastSeen,omitzero"`
}

type ledgerFile struct {
	Version int      `json:"version"`
	Devices []Record `json:"devices"`
}

// The names a record's state is stored under. They are the ledger's file format, so they do not
// follow the names in the code (dsIssued, …): a rename in the code must not change the file.
// A record that is waiting for the owner's code (Requested) is stored as issued.
const (
	stateIssued  = "issued"
	stateGranted = "granted"
	stateActive  = "active"
	stateExpired = "expired"
	stateRevoked = "revoked"
)

var stateCodes = map[devState]string{dsIssued: stateIssued, dsRequested: stateIssued, dsGranted: stateGranted, dsActive: stateActive, dsExpired: stateExpired, dsRevoked: stateRevoked}

var codeStates = map[string]devState{stateIssued: dsIssued, stateGranted: dsGranted, stateActive: dsActive, stateExpired: dsExpired, stateRevoked: dsRevoked}

// ValidName reports whether s is a usable device name: 1 to 64 characters, none of them control.
func ValidName(s string) bool {
	n := utf8.RuneCountInString(s)
	if n < 1 || n > maxNameRunes || !utf8.ValidString(s) {
		return false
	}
	for _, r := range s {
		if unicode.IsControl(r) {
			return false
		}
	}
	return true
}

// ValidSecret reports whether s is a 32-byte secret in unpadded base64url.
func ValidSecret(s string) bool {
	b, err := base64.RawURLEncoding.DecodeString(s)
	return err == nil && len(b) == secretBytes && base64.RawURLEncoding.EncodeToString(b) == s
}

// ValidCode reports whether s is a pairing code: exactly eight digits.
func ValidCode(s string) bool { return codeRe.MatchString(s) }

// ValidID reports whether s is a device id: sixteen lowercase hex digits.
func ValidID(s string) bool { return idRe.MatchString(s) }

// check validates one stored record. It returns the record to use (B is dropped from an active
// record, ADR 0016) or an error that says what is wrong.
func check(r Record) (Record, error) {
	switch {
	case !ValidID(r.ID):
		return r, errors.New("bad id")
	case !ValidName(r.Name):
		return r, errors.New("bad name")
	case r.Gen < 1 || r.Gen > maxGen:
		return r, fmt.Errorf("bad gen %d", r.Gen)
	case r.B != "" && !ValidSecret(r.B):
		return r, errors.New("bad B")
	case r.K != "" && !ValidSecret(r.K):
		return r, errors.New("bad K")
	}
	hasB, hasK := r.B != "", r.K != ""
	var ok bool
	switch r.State {
	case stateIssued:
		ok = hasB && !hasK
	case stateGranted:
		ok = hasB && hasK
	case stateActive:
		ok = hasK // B left over from an interrupted switch is dropped below
		r.B = ""
	case stateExpired, stateRevoked:
		ok = !hasB && !hasK
	default:
		return r, fmt.Errorf("bad state %q", r.State)
	}
	if !ok {
		return r, fmt.Errorf("state %s does not fit the secrets it holds (B %v, K %v)", r.State, hasB, hasK)
	}
	return r, nil
}

// decode reads a ledger. A file that is not a ledger at all is an error (the bridge does not start
// with a ledger it cannot read: that would serve nobody, silently). Single records that are not
// usable are left out and reported, so one bad record cannot hide the rest.
func decode(data []byte) (recs []Record, skipped []string, err error) {
	var f ledgerFile
	dec := json.NewDecoder(bytes.NewReader(data))
	if err := dec.Decode(&f); err != nil {
		return nil, nil, fmt.Errorf("not a device ledger: %w", err)
	}
	if f.Version != ledgerVersion {
		return nil, nil, fmt.Errorf("ledger version %d is not supported (this bridge reads %d)", f.Version, ledgerVersion)
	}
	ids, names := map[string]bool{}, map[string]bool{}
	for i, r := range f.Devices {
		r, err := check(r)
		switch {
		case err != nil:
			skipped = append(skipped, fmt.Sprintf("record %d: %v", i, err))
		case ids[r.ID]:
			skipped = append(skipped, fmt.Sprintf("record %d: duplicate id", i))
		case names[r.Name]:
			skipped = append(skipped, fmt.Sprintf("record %d: duplicate name", i))
		case len(recs) >= MaxDevices:
			skipped = append(skipped, fmt.Sprintf("record %d: more than %d devices", i, MaxDevices))
		default:
			ids[r.ID], names[r.Name] = true, true
			recs = append(recs, r)
		}
	}
	return recs, skipped, nil
}

func encode(recs []Record) ([]byte, error) {
	return json.MarshalIndent(ledgerFile{Version: ledgerVersion, Devices: recs}, "", "  ")
}

// Store keeps the ledger's bytes somewhere.
type Store interface {
	// Load returns the stored bytes; a ledger that was never written is (nil, nil).
	Load() ([]byte, error)
	// Save replaces the stored bytes. It returns only when the new bytes can be read back.
	Save([]byte) error
}

// FileStore keeps the ledger in one file (mode 0600): written to a temporary file next to it and
// renamed over it, so a power cut leaves the old or the new ledger, and read back to check.
type FileStore struct {
	Path string
	read func(string) ([]byte, error) // nil: os.ReadFile (a test replaces it)
}

// Load implements Store.
func (s FileStore) Load() ([]byte, error) {
	b, err := os.ReadFile(s.Path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	return b, err
}

// Save implements Store.
func (s FileStore) Save(data []byte) error {
	dir := filepath.Dir(s.Path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(dir, ".devices-*.tmp")
	if err != nil {
		return err
	}
	name := tmp.Name()
	defer os.Remove(name) // after a successful rename the name is gone and this does nothing
	if err := tmp.Chmod(0o600); err != nil {
		tmp.Close()
		return err
	}
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	if err := os.Rename(name, s.Path); err != nil {
		return err
	}
	read := s.read
	if read == nil {
		read = os.ReadFile
	}
	back, err := read(s.Path)
	if err != nil {
		return err
	}
	if !bytes.Equal(back, data) {
		return errors.New("ledger read back differs from what was written")
	}
	return nil
}

// newID returns a fresh device id from the random source.
func newID(rnd func([]byte) error) (string, error) {
	b := make([]byte, idBytes)
	if err := rnd(b); err != nil {
		return "", err
	}
	return hex.EncodeToString(b), nil
}

// newSecret returns a fresh 32-byte secret in unpadded base64url.
func newSecret(rnd func([]byte) error) (string, error) {
	b := make([]byte, secretBytes)
	if err := rnd(b); err != nil {
		return "", err
	}
	return base64.RawURLEncoding.EncodeToString(b), nil
}
