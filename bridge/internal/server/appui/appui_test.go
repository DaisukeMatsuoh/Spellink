package appui

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"regexp"
	"strings"
	"testing"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/server/pairpage"
)

func words(t *testing.T) pairpage.Strings {
	t.Helper()
	raw, err := files.ReadFile("strings.tsv")
	if err != nil {
		t.Fatal(err)
	}
	s, err := pairpage.ParseStrings(string(raw))
	if err != nil {
		t.Fatal(err)
	}
	return s
}

func TestTheWordsAreCompleteInEveryLanguage(t *testing.T) {
	s := words(t)
	for _, lang := range []string{"en", "ja"} {
		if len(s[lang]) == 0 || len(s[lang]) != len(s["en"]) {
			t.Errorf("%s has %d messages, English %d", lang, len(s[lang]), len(s["en"]))
		}
	}
	raw, _ := files.ReadFile("strings.tsv")
	for _, line := range strings.Split(string(raw), "\n") {
		if line == "" || strings.HasPrefix(line, "#") || strings.HasPrefix(line, "key\t") {
			continue
		}
		if cells := strings.Split(line, "\t"); cells[2] == "" {
			t.Errorf("%s has no Japanese text", cells[0])
		}
	}
}

// What the page's script asks for must exist in the words, or a person would see a bare key.
func TestEveryMessageThePageUsesExists(t *testing.T) {
	js, err := os.ReadFile("app.js")
	if err != nil {
		t.Fatal(err)
	}
	s := words(t)["en"]
	used := map[string]bool{}
	for _, re := range []*regexp.Regexp{
		regexp.MustCompile(`\bt\('(\w+)'[,)]`),               // t('relayOn') but not t('action' + ...)
		regexp.MustCompile(`say\('(\w+)'[,)]`),               // say('nameEmpty')
		regexp.MustCompile(`doneKey|'done_(\w+)'`),           // 'done_save'
		regexp.MustCompile(`step\d`),                         // 'step1'
		regexp.MustCompile(`confirm(Revoke|Reissue|Forget)`), // {revoke: 'confirmRevoke', …}
	} {
		for _, m := range re.FindAllStringSubmatch(string(js), -1) {
			switch {
			case len(m) > 1 && m[1] != "" && strings.HasPrefix(m[0], "confirm"):
				used["confirm"+m[1]] = true
			case len(m) > 1 && m[1] != "":
				used[m[1]] = true
			case strings.HasPrefix(m[0], "step"):
				used[m[0]] = true
			}
		}
	}
	// the messages built from a state or an action: state_<status>, action<Action>
	for _, st := range []string{"issued", "waiting", "granting", "active", "expired", "revoked"} {
		used["state"+strings.ToUpper(st[:1])+st[1:]] = true
	}
	for _, a := range []string{"Pair", "Save", "Revoke", "Reissue", "Forget"} {
		used["action"+a] = true
	}
	for _, a := range []string{"Revoke", "Reissue", "Forget"} { // the answer to each action
		used["done"+a] = true
	}
	for _, k := range []string{"title", "devicesTitle", "addTitle", "addButton", "addPlaceholder"} { // set straight from the words
		used[k] = true
	}
	for k := range used {
		if _, ok := s[k]; !ok {
			t.Errorf("the page uses the message %q, which strings.tsv does not have", k)
		}
	}
	if len(used) < 25 {
		t.Errorf("found only %d messages in use: the check itself is broken", len(used))
	}
	// and every message in the words is used by something (no dead words)
	for k := range s {
		if !used[k] && !strings.Contains(string(js), "'"+k+"'") && !strings.Contains(string(js), "t('"+k+"'") && !strings.Contains(string(js), "strings."+k) {
			t.Errorf("the message %q is in strings.tsv but the page never uses it", k)
		}
	}
}

func serve(t *testing.T, path, host string) *httptest.ResponseRecorder {
	t.Helper()
	h, err := Handler()
	if err != nil {
		t.Fatal(err)
	}
	req := httptest.NewRequest("GET", path, nil)
	req.Host = host
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

func TestTheHandlerServesThePageAndItsParts(t *testing.T) {
	for path, want := range map[string]string{"/app": "text/html", "/app/app.js": "text/javascript", "/app/strings.json": "application/json"} {
		rec := serve(t, path, "127.0.0.1:8765")
		if rec.Code != 200 || !strings.HasPrefix(rec.Header().Get("Content-Type"), want) || rec.Body.Len() == 0 {
			t.Errorf("%s: %d %q", path, rec.Code, rec.Header().Get("Content-Type"))
		}
		if rec.Header().Get("Cache-Control") != "no-store" || !strings.Contains(rec.Header().Get("Content-Security-Policy"), "default-src 'none'") {
			t.Errorf("%s: security headers: %v", path, rec.Header())
		}
	}
	if rec := serve(t, "/app/other.js", "127.0.0.1:8765"); rec.Code != 404 {
		t.Errorf("an unknown file: %d", rec.Code)
	}
	var got map[string]map[string]string
	if err := json.Unmarshal(serve(t, "/app/strings.json", "localhost").Body.Bytes(), &got); err != nil || got["ja"]["actionRevoke"] == "" {
		t.Errorf("strings.json: %v", err)
	}
}

func TestThePageIsOnlyForThisComputer(t *testing.T) {
	for _, host := range []string{"evil.example:8765", "192.168.2.180:8765", "bridge.spellink.uk"} {
		if rec := serve(t, "/app", host); rec.Code != http.StatusForbidden {
			t.Errorf("Host %s: %d", host, rec.Code)
		}
	}
}

func TestThePageLoadsNothingFromOutside(t *testing.T) {
	html := serve(t, "/app", "127.0.0.1").Body.String()
	if strings.Contains(html, "http://") || strings.Contains(html, "https://") {
		t.Error("the page must not load anything from outside")
	}
	for _, src := range regexp.MustCompile(`src="([^"]+)"`).FindAllStringSubmatch(html, -1) {
		if !strings.HasPrefix(src[1], "/app/") && !strings.HasPrefix(src[1], "/pair/pairlogic.js") {
			t.Errorf("script %s is not served by the bridge", src[1])
		}
	}
}
