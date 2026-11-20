// Package pairpage is the bridge's pairing page: it reads the QR code a device shows, with the
// computer's camera, and confirms the pairing (docs/spec/pairing.md). Everything it needs is in
// the binary, so it works without internet access.
package pairpage

import (
	"embed"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"regexp"
	"sort"
	"strings"
)

//go:embed index.html pair.js pairlogic.js jsQR.js strings.tsv
var files embed.FS

var (
	keyRe  = regexp.MustCompile(`^[a-zA-Z][a-zA-Z0-9]*$`)
	langRe = regexp.MustCompile(`^[a-z]{2,3}(_[A-Z]{2})?$`)
	phRe   = regexp.MustCompile(`\{(\w+)\}`)
)

// Strings maps a language to its messages.
type Strings map[string]map[string]string

func placeholders(s string) string {
	var out []string
	for _, m := range phRe.FindAllStringSubmatch(s, -1) {
		out = append(out, m[1])
	}
	sort.Strings(out)
	return strings.Join(out, ",")
}

func unescape(cell string) (string, error) {
	var b strings.Builder
	for i := 0; i < len(cell); i++ {
		if cell[i] != '\\' {
			b.WriteByte(cell[i])
			continue
		}
		i++
		if i >= len(cell) {
			return "", errors.New("a cell ends with a backslash")
		}
		switch cell[i] {
		case 'n':
			b.WriteByte('\n')
		case 't':
			b.WriteByte('\t')
		case '\\':
			b.WriteByte('\\')
		default:
			return "", fmt.Errorf("unknown escape \\%c", cell[i])
		}
	}
	return b.String(), nil
}

// ParseStrings reads the page's words: a TSV with `key<TAB>en<TAB>…` as its header (the same
// rules as plugin/locales/strings.tsv). An empty cell outside English uses the English text.
func ParseStrings(tsv string) (Strings, error) {
	var langs []string
	out := Strings{}
	seen := map[string]bool{}
	for n, raw := range strings.Split(tsv, "\n") {
		line := strings.TrimSuffix(raw, "\r")
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		cells := strings.Split(line, "\t")
		where := fmt.Sprintf("line %d", n+1)
		if langs == nil {
			if len(cells) < 2 || cells[0] != "key" || cells[1] != "en" {
				return nil, fmt.Errorf("%s: the header must start with key<TAB>en", where)
			}
			dup := map[string]bool{}
			for _, l := range cells[1:] {
				if !langRe.MatchString(l) {
					return nil, fmt.Errorf("%s: %q is not a language code", where, l)
				}
				if dup[l] {
					return nil, fmt.Errorf("%s: language %s appears twice", where, l)
				}
				dup[l] = true
				out[l] = map[string]string{}
			}
			langs = cells[1:]
			continue
		}
		if len(cells) != len(langs)+1 {
			return nil, fmt.Errorf("%s: %d columns, expected %d", where, len(cells), len(langs)+1)
		}
		key := cells[0]
		if !keyRe.MatchString(key) {
			return nil, fmt.Errorf("%s: %q is not a valid key", where, key)
		}
		if seen[key] {
			return nil, fmt.Errorf("%s: key %q appears twice", where, key)
		}
		seen[key] = true
		texts := make([]string, len(langs))
		for i, c := range cells[1:] {
			t, err := unescape(c)
			if err != nil {
				return nil, fmt.Errorf("%s, %s: %w", where, langs[i], err)
			}
			texts[i] = t
		}
		if texts[0] == "" {
			return nil, fmt.Errorf("key %q: the English text is required", key)
		}
		for i, l := range langs {
			if texts[i] == "" {
				texts[i] = texts[0]
			}
			if placeholders(texts[i]) != placeholders(texts[0]) {
				return nil, fmt.Errorf("key %q, %s: placeholders {%s} differ from English {%s}", key, l, placeholders(texts[i]), placeholders(texts[0]))
			}
			out[l][key] = texts[i]
		}
	}
	if langs == nil {
		return nil, errors.New("no header line")
	}
	return out, nil
}

// page serves one embedded file.
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

// Handler serves the pairing page under /pair. It is only for the person at this computer: a
// request that came through a proxy or tunnel is refused. The page has no secret in it; it reads the
// command line's token from the part of the address after # (which a browser never sends).
func Handler() (http.Handler, error) {
	raw, err := files.ReadFile("strings.tsv")
	if err != nil {
		return nil, err
	}
	strs, err := ParseStrings(string(raw))
	if err != nil {
		return nil, fmt.Errorf("pairpage strings.tsv: %w", err)
	}
	asJSON, err := json.Marshal(strs)
	if err != nil {
		return nil, err
	}
	mux := http.NewServeMux()
	mux.HandleFunc("GET /pair", page("index.html", "text/html; charset=utf-8"))
	mux.HandleFunc("GET /pair/pair.js", page("pair.js", "text/javascript; charset=utf-8"))
	mux.HandleFunc("GET /pair/pairlogic.js", page("pairlogic.js", "text/javascript; charset=utf-8"))
	mux.HandleFunc("GET /pair/jsQR.js", page("jsQR.js", "text/javascript; charset=utf-8"))
	mux.HandleFunc("GET /pair/strings.json", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json; charset=utf-8")
		_, _ = w.Write(asJSON)
	})
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Cf-Connecting-Ip") != "" || r.Header.Get("X-Forwarded-For") != "" {
			http.Error(w, "not available here", http.StatusForbidden)
			return
		}
		h := w.Header()
		h.Set("Cache-Control", "no-store")
		h.Set("Referrer-Policy", "no-referrer")
		h.Set("X-Content-Type-Options", "nosniff")
		h.Set("Content-Security-Policy", "default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'unsafe-inline'; media-src 'self' blob:; connect-src 'self'; img-src 'self' data:")
		mux.ServeHTTP(w, r)
	}), nil
}
