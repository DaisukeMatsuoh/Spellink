package lantls

import "testing"

func TestValid(t *testing.T) {
	cases := map[string]*Config{
		"lan.host is empty":            {},
		"lan.cloudflareToken is empty": {Host: "rock.lan.example.com"},
		"":                             {Host: "rock.lan.example.com", CloudflareToken: "t"},
	}
	for want, c := range cases {
		if got := c.Valid(); got != want {
			t.Errorf("%+v: got %q, want %q", c, got, want)
		}
	}
}

func TestBaseDomain(t *testing.T) {
	for in, want := range map[string]string{
		"rock.lan.example.com": "example.com",
		"example.com":          "example.com",
		"a.b.c.example.org":    "example.org",
	} {
		if got := baseDomain(in); got != want {
			t.Errorf("%s: got %s, want %s", in, got, want)
		}
	}
}
