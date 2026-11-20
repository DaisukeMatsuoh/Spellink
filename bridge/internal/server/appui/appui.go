// Package appui is the bridge's app page: the bridge's status and its devices in a browser at
// http://127.0.0.1:<port>/app. Like the pairing page it is in the binary and answers only to the person at this computer.
package appui

import (
	"embed"
	"encoding/json"
	"fmt"
	"net/http"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/localguard"
	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/server/pairpage"
)

//go:embed index.html app.js strings.tsv
var files embed.FS

func page(name, contentType string) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		b, err := files.ReadFile(name)
		if err != nil { // the files are embedded: this is a build mistake, not a request problem
			http.Error(w, "missing", http.StatusInternalServerError)
			return
		}
		w.Header().Set("Content-Type", contentType)
		_, _ = w.Write(b)
	}
}

// Handler serves the page under /app. It also needs the pairing page's pairlogic.js (served under /pair).
func Handler() (http.Handler, error) {
	raw, err := files.ReadFile("strings.tsv")
	if err != nil {
		return nil, err
	}
	strs, err := pairpage.ParseStrings(string(raw))
	if err != nil {
		return nil, fmt.Errorf("appui strings.tsv: %w", err)
	}
	asJSON, err := json.Marshal(strs)
	if err != nil {
		return nil, err
	}
	mux := http.NewServeMux()
	mux.HandleFunc("GET /app", page("index.html", "text/html; charset=utf-8"))
	mux.HandleFunc("GET /app/app.js", page("app.js", "text/javascript; charset=utf-8"))
	mux.HandleFunc("GET /app/strings.json", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json; charset=utf-8")
		_, _ = w.Write(asJSON)
	})
	return localguard.Only(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		h := w.Header()
		h.Set("Cache-Control", "no-store")
		h.Set("Referrer-Policy", "no-referrer")
		h.Set("X-Content-Type-Options", "nosniff")
		h.Set("Content-Security-Policy", "default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; img-src 'self' data:")
		mux.ServeHTTP(w, r)
	})), nil
}
