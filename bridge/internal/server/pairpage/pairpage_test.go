package pairpage

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"regexp"
	"strings"
	"testing"
)

const header = "key\ten\tja\n"

func TestParseStringsReadsLanguagesAsColumns(t *testing.T) {
	s, err := ParseStrings("# note\n\n" + header + "hello\tHi {name}\\nthere\\there\\\\\tこんにちは {name}\r\nbye\tBye\t\n")
	if err != nil {
		t.Fatal(err)
	}
	if s["en"]["hello"] != "Hi {name}\nthere\there\\" || s["ja"]["hello"] != "こんにちは {name}" {
		t.Errorf("hello: %q / %q", s["en"]["hello"], s["ja"]["hello"])
	}
	if s["ja"]["bye"] != "Bye" {
		t.Errorf("an empty cell must use the English text, got %q", s["ja"]["bye"])
	}
	more, err := ParseStrings("key\ten\tja\tzh_CN\nx\tA\tB\tC\n")
	if err != nil || more["zh_CN"]["x"] != "C" {
		t.Errorf("a third language: %v %v", more, err)
	}
}

func TestParseStringsRefusesWhatIsWrong(t *testing.T) {
	for name, c := range map[string]struct{ in, want string }{
		"no header":             {"x\tA\tB\n", "header"},
		"nothing at all":        {"", "no header"},
		"English not second":    {"key\tja\ten\n", "header"},
		"one column header":     {"key\n", "header"},
		"a bad language":        {"key\ten\tJapanese\n", "language code"},
		"a language twice":      {"key\ten\tja\tja\n", "twice"},
		"a missing cell":        {header + "x\tA\n", "columns"},
		"a bad key":             {header + "1x\tA\tB\n", "valid key"},
		"a key twice":           {header + "x\tA\tB\nx\tA\tB\n", "twice"},
		"an unknown escape":     {header + "x\tA\\q\tB\n", "escape"},
		"a trailing backslash":  {header + "x\tA\\\tB\n", "backslash"},
		"no English text":       {header + "x\t\tB\n", "English text is required"},
		"a missing placeholder": {header + "x\tHi {n}\tこんにちは\n", "placeholders"},
		"an extra placeholder":  {header + "x\tHi\tこんにちは {n}\n", "placeholders"},
	} {
		if _, err := ParseStrings(c.in); err == nil || !strings.Contains(err.Error(), c.want) {
			t.Errorf("%s: %v (want %q)", name, err, c.want)
		}
	}
}

func TestTheEmbeddedWordsAreCompleteInEveryLanguage(t *testing.T) {
	raw, err := files.ReadFile("strings.tsv")
	if err != nil {
		t.Fatal(err)
	}
	s, err := ParseStrings(string(raw))
	if err != nil {
		t.Fatal(err)
	}
	for _, lang := range []string{"en", "ja"} {
		if len(s[lang]) == 0 {
			t.Fatalf("no %s", lang)
		}
		if len(s[lang]) != len(s["en"]) {
			t.Errorf("%s has %d messages, English has %d", lang, len(s[lang]), len(s["en"]))
		}
	}
	// no gap falls back to English silently: every Japanese cell is really Japanese text
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
	js, err := os.ReadFile("pair.js")
	if err != nil {
		t.Fatal(err)
	}
	raw, _ := files.ReadFile("strings.tsv")
	s, _ := ParseStrings(string(raw))
	used := map[string]bool{}
	collect := func(src []byte, re *regexp.Regexp) {
		for _, m := range re.FindAllStringSubmatch(string(src), -1) {
			for _, k := range strings.Split(strings.ReplaceAll(m[1], "'", ""), ", ") {
				used[k] = true
			}
		}
	}
	collect(js, regexp.MustCompile(`say\('(\w+)'`))                     // say('hint')
	collect(js, regexp.MustCompile(`\[('\w+'(?:, '\w+')*)\]\.forEach`)) // ['title', …].forEach
	logic, _ := os.ReadFile("pairlogic.js")
	collect(logic, regexp.MustCompile(`case \d+: return '(\w+)'`)) // the messages of messageFor
	for k := range used {
		if _, ok := s["en"][k]; !ok {
			t.Errorf("the page uses the message %q, which strings.tsv does not have", k)
		}
	}
	if len(used) < 8 {
		t.Errorf("found only %d messages in use: the check itself is broken", len(used))
	}
}

func serve(t *testing.T, path string, headers map[string]string) *httptest.ResponseRecorder {
	t.Helper()
	h, err := Handler()
	if err != nil {
		t.Fatal(err)
	}
	req := httptest.NewRequest("GET", path, nil)
	for k, v := range headers {
		req.Header.Set(k, v)
	}
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

func TestTheHandlerServesThePageAndItsParts(t *testing.T) {
	for path, want := range map[string]string{
		"/pair": "text/html", "/pair/pair.js": "text/javascript", "/pair/pairlogic.js": "text/javascript",
		"/pair/jsQR.js": "text/javascript", "/pair/strings.json": "application/json",
	} {
		rec := serve(t, path, nil)
		if rec.Code != 200 || !strings.HasPrefix(rec.Header().Get("Content-Type"), want) || rec.Body.Len() == 0 {
			t.Errorf("%s: %d %q", path, rec.Code, rec.Header().Get("Content-Type"))
		}
		if rec.Header().Get("Cache-Control") != "no-store" || rec.Header().Get("Referrer-Policy") != "no-referrer" || !strings.Contains(rec.Header().Get("Content-Security-Policy"), "default-src 'none'") {
			t.Errorf("%s: security headers missing: %v", path, rec.Header())
		}
	}
	if rec := serve(t, "/pair/nothing.js", nil); rec.Code != 404 {
		t.Errorf("an unknown file: %d", rec.Code)
	}
}

func TestTheStringsAreServedAsJSONForEveryLanguage(t *testing.T) {
	rec := serve(t, "/pair/strings.json", nil)
	var got map[string]map[string]string
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if got["en"]["title"] == "" || got["ja"]["title"] == "" || got["ja"]["title"] == got["en"]["title"] {
		t.Errorf("titles: %q / %q", got["en"]["title"], got["ja"]["title"])
	}
}

func TestThePageIsNotForAnyoneBehindAProxy(t *testing.T) {
	for _, h := range []string{"Cf-Connecting-Ip", "X-Forwarded-For"} {
		if rec := serve(t, "/pair", map[string]string{h: "203.0.113.9"}); rec.Code != http.StatusForbidden {
			t.Errorf("%s: %d", h, rec.Code)
		}
	}
}

func TestThePageHasNoSecretAndNoOutsideScript(t *testing.T) {
	html := serve(t, "/pair", nil).Body.String()
	if strings.Contains(html, "http://") || strings.Contains(html, "https://") {
		t.Error("the page must not load anything from outside (it works offline, and nothing outside may run in it)")
	}
	for _, src := range regexp.MustCompile(`src="([^"]+)"`).FindAllStringSubmatch(html, -1) {
		if !strings.HasPrefix(src[1], "/pair/") {
			t.Errorf("script %s is not served by the bridge", src[1])
		}
	}
}
