// Package localguard decides whether a request may reach the bridge's local-only doors (the
// command line's admin API, the pairing page, the app page): it must come from this computer's
// own browser or command line, not through a tunnel or proxy, and not from a web page that
// tricked the browser into talking to 127.0.0.1.
package localguard

import (
	"net"
	"net/http"
	"strings"
)

// Local reports whether r is a request from the person at this computer:
//   - no proxy or tunnel headers (a tunnel delivers every request from 127.0.0.1; these headers are how it shows), and
//   - the Host is a loopback name. A web page on another site can make a browser send requests to
//     127.0.0.1 after re-pointing its own name there ("DNS rebinding"); the Host it carries is still its own name.
func Local(r *http.Request) bool {
	if r.Header.Get("Cf-Connecting-Ip") != "" || r.Header.Get("X-Forwarded-For") != "" || r.Header.Get("Forwarded") != "" {
		return false
	}
	return loopbackHost(r.Host)
}

func loopbackHost(hostport string) bool {
	host := hostport
	if h, _, err := net.SplitHostPort(hostport); err == nil {
		host = h
	}
	host = strings.Trim(host, "[]")
	if strings.EqualFold(host, "localhost") {
		return true
	}
	ip := net.ParseIP(host)
	return ip != nil && ip.IsLoopback()
}

// Only wraps a handler so that it answers a non-local request with 403.
func Only(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if !Local(r) {
			http.Error(w, "not available here", http.StatusForbidden)
			return
		}
		next.ServeHTTP(w, r)
	})
}
