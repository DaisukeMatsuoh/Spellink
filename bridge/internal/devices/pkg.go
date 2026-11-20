package devices

import (
	"archive/zip"
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"strings"
)

// BundleName is the JavaScript bundle inside a plugin package.
const BundleName = "Spellink.bundle"

const pairMarker = "globalThis.__SPELLINK_PAIR__="

// What the plugin accepts in the pairing setup (the plugin checks the same limits again).
const (
	maxURLs       = 8
	maxURLChars   = 512
	minWindowSecs = 10
	maxWindowSecs = 600
)

// maxPackage bounds one file inside a package (a variable so a test can lower it).
var maxPackage int64 = 64 << 20

// pairSetup is the line written at the top of the bundle: what the plugin needs to pair (ADR 0016).
type pairSetup struct {
	V         int      `json:"v"`
	ID        string   `json:"id"`
	Gen       int      `json:"gen"`
	B         string   `json:"b,omitempty"`
	Name      string   `json:"name"`
	URLs      []string `json:"urls"`
	WindowSec int      `json:"windowSec"`
}

// BuildPackage returns a copy of the plugin package `base` (.snplg, a zip) whose bundle starts with
// the device's pairing setup. Every other file is copied as it is. The result is read back and
// checked before it is returned.
func BuildPackage(base []byte, d PackageData, urls []string) ([]byte, error) {
	setup := pairSetup{V: 1, ID: d.ID, Gen: d.Gen, B: d.B, Name: d.Name, URLs: urls, WindowSec: int(d.Window.Seconds())}
	switch {
	case !ValidID(d.ID), !ValidName(d.Name), d.Gen < 1 || d.Gen > maxGen:
		return nil, errors.New("package: bad device data")
	case d.B != "" && !ValidSecret(d.B):
		return nil, errors.New("package: bad B")
	case len(urls) == 0 || len(urls) > maxURLs:
		return nil, fmt.Errorf("package: %d URLs (need 1 to %d)", len(urls), maxURLs)
	case setup.WindowSec < minWindowSecs || setup.WindowSec > maxWindowSecs:
		return nil, fmt.Errorf("package: window of %d s (need %d to %d)", setup.WindowSec, minWindowSecs, maxWindowSecs)
	}
	for _, u := range urls {
		if !strings.HasPrefix(u, "wss://") || len(u) > maxURLChars || strings.ContainsAny(u, "\"\\\n\r") {
			return nil, fmt.Errorf("package: URL %q is not usable (wss:// only, up to %d characters)", u, maxURLChars)
		}
	}
	js, err := json.Marshal(setup) // escapes <, > and the line separators, so it is safe inside a script
	if err != nil {
		return nil, err
	}
	line := pairMarker + string(js) + ";\n"

	zr, err := zip.NewReader(bytes.NewReader(base), int64(len(base)))
	if err != nil {
		return nil, fmt.Errorf("package: the base is not a zip: %w", err)
	}
	var out bytes.Buffer
	zw := zip.NewWriter(&out)
	found := false
	for _, f := range zr.File {
		if f.Name != BundleName {
			if err := zw.Copy(f); err != nil {
				return nil, err
			}
			continue
		}
		found = true
		body, err := readEntry(f)
		if err != nil {
			return nil, err
		}
		if bytes.Contains(body, []byte(pairMarker)) {
			return nil, errors.New("package: the base already carries a pairing setup; use the original plugin package")
		}
		w, err := zw.CreateHeader(&zip.FileHeader{Name: BundleName, Method: zip.Deflate, Modified: f.Modified})
		if err != nil {
			return nil, err
		}
		if _, err := w.Write(append([]byte(line), body...)); err != nil {
			return nil, err
		}
	}
	if !found {
		return nil, fmt.Errorf("package: %s not found in the base", BundleName)
	}
	if err := zw.Close(); err != nil {
		return nil, err
	}
	if err := verify(out.Bytes(), zr, line); err != nil {
		return nil, fmt.Errorf("package: written file does not check out: %w", err)
	}
	return out.Bytes(), nil
}

func readEntry(f *zip.File) ([]byte, error) {
	rc, err := f.Open()
	if err != nil {
		return nil, err
	}
	defer rc.Close()
	if f.UncompressedSize64 > uint64(maxPackage) {
		return nil, errors.New("package: an entry is too large")
	}
	// archive/zip itself fails a read that does not match the size in the header (and the header is
	// bounded above), so what is read here is bounded too.
	return io.ReadAll(rc)
}

// verify is verifyPackage; a test replaces it to see that a failed check returns an error, not the bytes.
var verify = verifyPackage

// verifyPackage reads the result back: same entries, the bundle starts with the line, the rest is unchanged.
func verifyPackage(data []byte, base *zip.Reader, line string) error {
	zr, err := zip.NewReader(bytes.NewReader(data), int64(len(data)))
	if err != nil {
		return err
	}
	if len(zr.File) != len(base.File) {
		return fmt.Errorf("%d entries, expected %d", len(zr.File), len(base.File))
	}
	for i, f := range zr.File {
		if f.Name != base.File[i].Name {
			return fmt.Errorf("entry %d is %q, expected %q", i, f.Name, base.File[i].Name)
		}
		got, err := readEntry(f)
		if err != nil {
			return err
		}
		want, err := readEntry(base.File[i])
		if err != nil {
			return err
		}
		if f.Name == BundleName {
			want = append([]byte(line), want...)
		}
		if !bytes.Equal(got, want) {
			return fmt.Errorf("entry %q differs", f.Name)
		}
	}
	return nil
}
