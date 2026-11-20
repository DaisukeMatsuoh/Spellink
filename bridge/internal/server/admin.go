package server

import (
	"crypto/sha256"
	"crypto/subtle"
	"encoding/json"
	"errors"
	"net/http"
	"strings"
	"time"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/devices"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/localguard"
)

// AdminDeps is what the bridge's own command line needs: the ledger, a way to build a device's
// .snplg, and a bearer token. It is served under /admin/ and only to a caller that did not come
// through a proxy or tunnel and knows the token (stored in the bridge's config directory, mode 0600).
type AdminDeps struct {
	Devices *devices.Registry
	// Token is the bearer token of the command line (high-entropy; never logged).
	Token string
	// Package returns the .snplg for a device, built from the data the ledger gives (REPACK).
	Package func(devices.PackageData) ([]byte, error)
	// Status says what the app page shows about the bridge itself (nil: only the device counts).
	Status func() StatusInfo
}

// StatusInfo is what the bridge knows about itself that the ledger does not.
type StatusInfo struct {
	Version string `json:"version"`
	Name    string `json:"name"`
	// Provider is the AI the bridge asks: its name, and whether its program can be found here.
	Provider ProviderInfo `json:"provider"`
	// Relay is true when the bridge registers with a relay (devices can reach it away from home).
	Relay bool `json:"relay"`
	// Addresses are the wss:// addresses a new device would be told.
	Addresses []string `json:"addresses"`
}

// ProviderInfo describes the AI program.
type ProviderInfo struct {
	Name  string `json:"name"`
	Found bool   `json:"found"`
	Path  string `json:"path,omitempty"`
}

type statusJSON struct {
	StatusInfo
	Devices struct {
		Total   int `json:"total"`
		Active  int `json:"active"`
		Waiting int `json:"waiting"`
	} `json:"devices"`
}

const maxAdminBody = 4 << 10

type deviceJSON struct {
	ID       string    `json:"id"`
	Name     string    `json:"name"`
	Gen      int       `json:"gen"`
	Status   string    `json:"status"`
	Waiting  int       `json:"waiting"`
	IssuedAt time.Time `json:"issuedAt"`
	Expires  time.Time `json:"expires,omitzero"`
	LastSeen time.Time `json:"lastSeen,omitzero"`
}

// Handler returns the /admin/ routes.
func (a *AdminDeps) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /admin/status", func(w http.ResponseWriter, r *http.Request) {
		var out statusJSON
		if a.Status != nil {
			out.StatusInfo = a.Status()
		}
		if out.Addresses == nil {
			out.Addresses = []string{}
		}
		for _, d := range a.Devices.List() {
			out.Devices.Total++
			if d.Status == "active" {
				out.Devices.Active++
			}
			out.Devices.Waiting += d.Waiting
		}
		writeJSON(w, http.StatusOK, out)
	})
	mux.HandleFunc("GET /admin/devices", func(w http.ResponseWriter, r *http.Request) {
		list := a.Devices.List()
		out := make([]deviceJSON, len(list))
		for i, in := range list {
			out[i] = deviceJSON{in.ID, in.Name, in.Gen, in.Status, in.Waiting, in.IssuedAt, in.Expires, in.LastSeen}
		}
		writeJSON(w, http.StatusOK, out)
	})
	mux.HandleFunc("POST /admin/devices", func(w http.ResponseWriter, r *http.Request) {
		var in struct {
			Name string `json:"name"`
		}
		if !readJSON(w, r, &in) {
			return
		}
		id, err := a.Devices.Add(in.Name)
		if err != nil {
			adminError(w, err)
			return
		}
		writeJSON(w, http.StatusCreated, map[string]string{"id": id, "name": in.Name})
	})
	for action, f := range map[string]func(string) error{
		"revoke": a.Devices.Revoke, "reissue": a.Devices.Reissue, "forget": a.Devices.Forget,
	} {
		mux.HandleFunc("POST /admin/devices/{id}/"+action, func(w http.ResponseWriter, r *http.Request) {
			if err := f(r.PathValue("id")); err != nil {
				adminError(w, err)
				return
			}
			w.WriteHeader(http.StatusNoContent)
		})
	}
	mux.HandleFunc("GET /admin/devices/{id}/package", func(w http.ResponseWriter, r *http.Request) {
		d, err := a.Devices.Repack(r.PathValue("id"))
		if err != nil {
			adminError(w, err)
			return
		}
		pkg, err := a.Package(d)
		if err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		w.Header().Set("Content-Type", "application/octet-stream")
		w.Header().Set("X-Device-Name", d.Name)
		_, _ = w.Write(pkg)
	})
	mux.HandleFunc("POST /admin/pair", func(w http.ResponseWriter, r *http.Request) {
		var in struct {
			Code string `json:"code"`
			// ID is the device's id when the code was read from its QR code; then only that device is considered.
			ID string `json:"id"`
		}
		if !readJSON(w, r, &in) {
			return
		}
		var name string
		var err error
		if in.ID != "" {
			name, err = a.Devices.ConfirmFor(in.ID, in.Code)
		} else {
			name, err = a.Devices.Confirm(in.Code)
		}
		if err != nil {
			adminError(w, err)
			return
		}
		writeJSON(w, http.StatusOK, map[string]string{"name": name})
	})
	return a.guard(mux)
}

// guard lets a request through only if it is local and carries the token.
func (a *AdminDeps) guard(next http.Handler) http.Handler {
	want := sha256.Sum256([]byte(a.Token))
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if !localguard.Local(r) {
			http.Error(w, "not available here", http.StatusForbidden)
			return
		}
		got := sha256.Sum256([]byte(strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer ")))
		if a.Token == "" || subtle.ConstantTimeCompare(got[:], want[:]) != 1 {
			http.Error(w, "unauthorized", http.StatusUnauthorized)
			return
		}
		next.ServeHTTP(w, r)
	})
}

func readJSON(w http.ResponseWriter, r *http.Request, v any) bool {
	dec := json.NewDecoder(http.MaxBytesReader(w, r.Body, maxAdminBody))
	if err := dec.Decode(v); err != nil {
		http.Error(w, "bad request", http.StatusBadRequest)
		return false
	}
	return true
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}

// adminError maps the ledger's errors to what the command line shows. The message is the ledger's own
// sentence (never a secret, never which devices are waiting for a code).
func adminError(w http.ResponseWriter, err error) {
	status := http.StatusInternalServerError
	switch {
	case errors.Is(err, devices.ErrUnknownDevice), errors.Is(err, devices.ErrNoPending):
		status = http.StatusNotFound
	case errors.Is(err, devices.ErrBadName), errors.Is(err, devices.ErrBadCode):
		status = http.StatusBadRequest
	case errors.Is(err, devices.ErrCodeRejected):
		status = http.StatusForbidden
	case errors.Is(err, devices.ErrNameTaken), errors.Is(err, devices.ErrFull), errors.Is(err, devices.ErrAlreadyRevoked),
		errors.Is(err, devices.ErrRevoked), errors.Is(err, devices.ErrExpired), errors.Is(err, devices.ErrStillValid):
		status = http.StatusConflict
	}
	http.Error(w, err.Error(), status)
}
