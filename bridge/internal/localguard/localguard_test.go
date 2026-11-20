package localguard

import (
	"net/http"
	"net/http/httptest"
	"testing"
)

func req(host string, headers map[string]string) *http.Request {
	r := httptest.NewRequest("GET", "http://"+host+"/x", nil)
	r.Host = host
	for k, v := range headers {
		r.Header.Set(k, v)
	}
	return r
}

func TestLoopbackNamesAreLocal(t *testing.T) {
	for _, host := range []string{"127.0.0.1:8765", "127.0.0.1", "localhost:8765", "LOCALHOST", "[::1]:8765", "[::1]", "127.1.2.3:80"} {
		if !Local(req(host, nil)) {
			t.Errorf("%s should be local", host)
		}
	}
}

func TestOtherNamesAreNotLocalWhateverTheyResolveTo(t *testing.T) {
	for _, host := range []string{
		"evil.example:8765", "evil.example", "bridge.spellink.uk", "192.168.2.180:8765", "10.0.0.1", "[2001:db8::1]:80",
		"localhost.evil.example", "127.0.0.1.evil.example", "", "0.0.0.0:8765", "[::]:80",
	} {
		if Local(req(host, nil)) {
			t.Errorf("%q must not be local (DNS rebinding)", host)
		}
	}
}

func TestAProxyOrTunnelIsNeverLocal(t *testing.T) {
	for _, h := range []string{"Cf-Connecting-Ip", "X-Forwarded-For", "Forwarded"} {
		if Local(req("127.0.0.1:8765", map[string]string{h: "203.0.113.9"})) {
			t.Errorf("%s must make a request non-local", h)
		}
	}
}

func TestOnlyAnswersAForeignRequestWith403AndNeverRunsTheHandler(t *testing.T) {
	ran := 0
	h := Only(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { ran++ }))
	for host, want := range map[string]int{"127.0.0.1:1": 200, "evil.example": 403} {
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, req(host, nil))
		if rec.Code != want {
			t.Errorf("%s: %d, want %d", host, rec.Code, want)
		}
	}
	if ran != 1 {
		t.Errorf("the handler ran %d times", ran)
	}
}
