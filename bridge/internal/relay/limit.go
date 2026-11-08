package relay

import (
	"net"
	"net/http"
	"strings"
	"sync"
	"time"
)

// limiter allows each source address a number of attempts per window. It is a fixed window per
// address: simple, and enough to keep one address from using a relay up. Addresses that have
// gone quiet are forgotten, so the table cannot grow without bound either.
type limiter struct {
	mu     sync.Mutex
	max    int
	window time.Duration
	hits   map[string]*bucket
}

type bucket struct {
	start time.Time
	n     int
}

func newLimiter(max int, window time.Duration) *limiter {
	return &limiter{max: max, window: window, hits: map[string]*bucket{}}
}

// allow records an attempt from key at now and reports whether it is within the limit.
func (l *limiter) allow(key string, now time.Time) bool {
	l.mu.Lock()
	defer l.mu.Unlock()
	if len(l.hits) > 4096 {
		for k, b := range l.hits {
			if now.Sub(b.start) > l.window {
				delete(l.hits, k)
			}
		}
	}
	b := l.hits[key]
	if b == nil || now.Sub(b.start) >= l.window {
		b = &bucket{start: now}
		l.hits[key] = b
	}
	b.n++
	return b.n <= l.max
}

// sourceOf names who is connecting. Behind a proxy or tunnel (Cloudflare) the connection comes
// from the proxy, so the real address is in a header — but only if the operator said the proxy
// can be trusted: a header sent by anyone else would let a client pick its own identity.
func sourceOf(r *http.Request, trustProxy bool) string {
	if trustProxy {
		if ip := strings.TrimSpace(r.Header.Get("Cf-Connecting-Ip")); ip != "" {
			return ip
		}
		if xff := r.Header.Get("X-Forwarded-For"); xff != "" {
			return strings.TrimSpace(strings.Split(xff, ",")[0])
		}
	}
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		return r.RemoteAddr
	}
	return host
}

// admit applies a limit before anything is allocated for a connection. Over the limit: 429.
func (s *Server) admit(w http.ResponseWriter, r *http.Request, l *limiter) bool {
	if l.allow(sourceOf(r, s.TrustProxy), time.Now()) {
		return true
	}
	w.Header().Set("Retry-After", "60")
	http.Error(w, "too many connections from this address", http.StatusTooManyRequests)
	return false
}
