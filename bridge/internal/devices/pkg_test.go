package devices

import (
	"archive/zip"
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"strings"
	"testing"
	"time"
)

func makeZip(t *testing.T, files [][2]string) []byte {
	t.Helper()
	var buf bytes.Buffer
	zw := zip.NewWriter(&buf)
	mod := time.Date(2026, 11, 8, 19, 9, 0, 0, time.UTC)
	for _, f := range files {
		w, err := zw.CreateHeader(&zip.FileHeader{Name: f[0], Method: zip.Deflate, Modified: mod})
		if err != nil {
			t.Fatal(err)
		}
		io.WriteString(w, f[1])
	}
	if err := zw.Close(); err != nil {
		t.Fatal(err)
	}
	return buf.Bytes()
}

func entries(t *testing.T, data []byte) map[string]string {
	t.Helper()
	zr, err := zip.NewReader(bytes.NewReader(data), int64(len(data)))
	if err != nil {
		t.Fatal(err)
	}
	out := map[string]string{}
	for _, f := range zr.File {
		b, err := readEntry(f)
		if err != nil {
			t.Fatal(err)
		}
		out[f.Name] = string(b)
	}
	return out
}

func goodData() PackageData {
	return PackageData{ID: "0123456789abcdef", Name: "Nomad", Gen: 3, B: secret(5), Window: 90 * time.Second}
}

var goodURLs = []string{"wss://bridge.example/v1?d=0123456789abcdef", "wss://relay.example/v1/relay/client?room=abc"}

func baseZip(t *testing.T) []byte {
	return makeZip(t, [][2]string{{"PluginConfig.json", `{"name":"Spellink"}`}, {BundleName, "var __BUNDLE_START_TIME__=1;"}, {"icon.png", "PNG"}})
}

func TestBuildPackagePutsTheSetupAtTheTopOfTheBundleAndChangesNothingElse(t *testing.T) {
	out, err := BuildPackage(baseZip(t), goodData(), goodURLs)
	if err != nil {
		t.Fatal(err)
	}
	got := entries(t, out)
	if got["PluginConfig.json"] != `{"name":"Spellink"}` || got["icon.png"] != "PNG" {
		t.Errorf("other files changed: %v", got)
	}
	bundle := got[BundleName]
	line, rest, _ := strings.Cut(bundle, "\n")
	if rest != "var __BUNDLE_START_TIME__=1;" || !strings.HasPrefix(line, pairMarker) || !strings.HasSuffix(line, ";") {
		t.Fatalf("bundle = %q", bundle)
	}
	var s pairSetup
	if err := json.Unmarshal([]byte(strings.TrimSuffix(strings.TrimPrefix(line, pairMarker), ";")), &s); err != nil {
		t.Fatal(err)
	}
	d := goodData()
	if s.V != 1 || s.ID != d.ID || s.Gen != 3 || s.B != d.B || s.Name != "Nomad" || s.WindowSec != 90 || len(s.URLs) != 2 {
		t.Errorf("setup = %+v", s)
	}
	zr, _ := zip.NewReader(bytes.NewReader(out), int64(len(out)))
	order := []string{}
	for _, f := range zr.File {
		order = append(order, f.Name)
	}
	if strings.Join(order, ",") != "PluginConfig.json,"+BundleName+",icon.png" {
		t.Errorf("entry order changed: %v", order)
	}
}

func TestBuildPackageForAPairedDeviceCarriesNoSecret(t *testing.T) {
	d := goodData()
	d.B = ""
	out, err := BuildPackage(baseZip(t), d, goodURLs)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(entries(t, out)[BundleName], `"b"`) {
		t.Error("a paired device's package must not carry B")
	}
}

func TestBuildPackageEscapesWhatItWrites(t *testing.T) {
	d := goodData()
	d.Name = "</script>  &"
	out, err := BuildPackage(baseZip(t), d, goodURLs)
	if err != nil {
		t.Fatal(err)
	}
	line, _, _ := strings.Cut(entries(t, out)[BundleName], "\n")
	for _, bad := range []string{"</script>", " ", " "} {
		if strings.Contains(line, bad) {
			t.Errorf("the setup line contains %q raw", bad)
		}
	}
}

func TestBuildPackageRefusesBadInput(t *testing.T) {
	cases := map[string]func(*PackageData, *[]string){
		"bad id":   func(d *PackageData, _ *[]string) { d.ID = "x" },
		"bad name": func(d *PackageData, _ *[]string) { d.Name = "" },
		"gen 0":    func(d *PackageData, _ *[]string) { d.Gen = 0 },
		"bad B":    func(d *PackageData, _ *[]string) { d.B = "short" },
		"no URLs":  func(_ *PackageData, u *[]string) { *u = nil },
		"too many URLs": func(_ *PackageData, u *[]string) {
			*u = make([]string, maxURLs+1)
			for i := range *u {
				(*u)[i] = "wss://x"
			}
		},
		"plain ws":         func(_ *PackageData, u *[]string) { *u = []string{"ws://x/v1"} },
		"https":            func(_ *PackageData, u *[]string) { *u = []string{"https://x/v1"} },
		"quote in URL":     func(_ *PackageData, u *[]string) { *u = []string{`wss://x/"`} },
		"newline in URL":   func(_ *PackageData, u *[]string) { *u = []string{"wss://x/\n"} },
		"URL too long":     func(_ *PackageData, u *[]string) { *u = []string{"wss://" + strings.Repeat("a", maxURLChars)} },
		"window too short": func(d *PackageData, _ *[]string) { d.Window = 5 * time.Second },
		"window too long":  func(d *PackageData, _ *[]string) { d.Window = 11 * time.Minute },
	}
	for name, f := range cases {
		d, u := goodData(), append([]string(nil), goodURLs...)
		f(&d, &u)
		if _, err := BuildPackage(baseZip(t), d, u); err == nil {
			t.Errorf("%s: no error", name)
		}
	}
}

func TestBuildPackageRefusesABaseItCannotUse(t *testing.T) {
	if _, err := BuildPackage([]byte("not a zip"), goodData(), goodURLs); err == nil {
		t.Error("not a zip")
	}
	if _, err := BuildPackage(makeZip(t, [][2]string{{"icon.png", "x"}}), goodData(), goodURLs); err == nil || !strings.Contains(err.Error(), "not found") {
		t.Errorf("no bundle: %v", err)
	}
	once, err := BuildPackage(baseZip(t), goodData(), goodURLs)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := BuildPackage(once, goodData(), goodURLs); err == nil || !strings.Contains(err.Error(), "already") {
		t.Errorf("a base that already has a setup: %v", err)
	}
}

func TestBuildPackageRefusesAnOversizedEntry(t *testing.T) {
	old := maxPackage
	maxPackage = 10
	defer func() { maxPackage = old }()
	if _, err := BuildPackage(baseZip(t), goodData(), goodURLs); err == nil {
		t.Error("an entry above the limit must be refused")
	}
}

func TestBuildPackageRefusesAnEntryItCannotOpen(t *testing.T) {
	var buf bytes.Buffer
	zw := zip.NewWriter(&buf)
	w, _ := zw.CreateRaw(&zip.FileHeader{Name: BundleName, Method: 99, CompressedSize64: 3, UncompressedSize64: 3})
	w.Write([]byte("xyz"))
	zw.Close()
	if _, err := BuildPackage(buf.Bytes(), goodData(), goodURLs); err == nil {
		t.Error("an entry with an unknown compression method must be refused")
	}
}

func TestBuildPackageDoesNotReturnBytesThatFailTheirCheck(t *testing.T) {
	old := verify
	verify = func([]byte, *zip.Reader, string) error { return errors.New("injected") }
	defer func() { verify = old }()
	if out, err := BuildPackage(baseZip(t), goodData(), goodURLs); err == nil || out != nil {
		t.Errorf("out %d bytes, err %v", len(out), err)
	}
}

func TestVerifyPackageCatchesEveryKindOfDamage(t *testing.T) {
	base := baseZip(t)
	br, _ := zip.NewReader(bytes.NewReader(base), int64(len(base)))
	line := "L;\n"
	good := func() []byte {
		var buf bytes.Buffer
		zw := zip.NewWriter(&buf)
		for _, f := range br.File {
			if f.Name != BundleName {
				zw.Copy(f)
				continue
			}
			body, _ := readEntry(f)
			w, _ := zw.CreateHeader(&zip.FileHeader{Name: f.Name, Method: zip.Deflate})
			w.Write(append([]byte(line), body...))
		}
		zw.Close()
		return buf.Bytes()
	}
	if err := verifyPackage(good(), br, line); err != nil {
		t.Fatalf("a correct package: %v", err)
	}
	if err := verifyPackage([]byte("junk"), br, line); err == nil {
		t.Error("not a zip")
	}
	if err := verifyPackage(makeZip(t, [][2]string{{"a", "x"}}), br, line); err == nil {
		t.Error("wrong number of entries")
	}
	if err := verifyPackage(makeZip(t, [][2]string{{"x", "1"}, {BundleName, line + "var __BUNDLE_START_TIME__=1;"}, {"icon.png", "PNG"}}), br, line); err == nil {
		t.Error("wrong entry name")
	}
	if err := verifyPackage(makeZip(t, [][2]string{{"PluginConfig.json", "CHANGED"}, {BundleName, line + "var __BUNDLE_START_TIME__=1;"}, {"icon.png", "PNG"}}), br, line); err == nil {
		t.Error("a changed file")
	}
	if err := verifyPackage(makeZip(t, [][2]string{{"PluginConfig.json", `{"name":"Spellink"}`}, {BundleName, "no setup line"}, {"icon.png", "PNG"}}), br, line); err == nil {
		t.Error("a bundle without the line")
	}
}

// storedZip is a zip whose entries are stored uncompressed, so a test can damage one on purpose.
func storedZip(t *testing.T, files [][2]string) []byte {
	t.Helper()
	var buf bytes.Buffer
	zw := zip.NewWriter(&buf)
	for _, f := range files {
		w, _ := zw.CreateHeader(&zip.FileHeader{Name: f[0], Method: zip.Store})
		io.WriteString(w, f[1])
	}
	zw.Close()
	return buf.Bytes()
}

func TestBuildPackageRefusesADamagedBase(t *testing.T) {
	data := storedZip(t, [][2]string{{"PluginConfig.json", "{}"}, {BundleName, "BUNDLEBUNDLEBUNDLE"}})
	damaged := bytes.Replace(data, []byte("BUNDLEBUNDLEBUNDLE"), []byte("BUNDLEBUNDLEBUNDLX"), 1) // the checksum no longer matches
	if _, err := BuildPackage(damaged, goodData(), goodURLs); err == nil {
		t.Error("a bundle that fails its checksum must be refused")
	}
}

func TestVerifyPackageNoticesAnUnreadableEntry(t *testing.T) {
	base := baseZip(t)
	br, _ := zip.NewReader(bytes.NewReader(base), int64(len(base)))
	bad := storedZip(t, [][2]string{{"PluginConfig.json", "ABCD"}, {BundleName, "x"}, {"icon.png", "PNG"}})
	bad = bytes.Replace(bad, []byte("ABCD"), []byte("ABCE"), 1)
	if err := verifyPackage(bad, br, "L"); err == nil {
		t.Error("an entry that fails its checksum in the result")
	}
	// and one in the base itself
	good := storedZip(t, [][2]string{{"PluginConfig.json", `{"name":"Spellink"}`}, {BundleName, "L;\nvar __BUNDLE_START_TIME__=1;"}, {"icon.png", "PNG"}})
	badBase := storedZip(t, [][2]string{{"PluginConfig.json", "WXYZ"}, {BundleName, "var __BUNDLE_START_TIME__=1;"}, {"icon.png", "PNG"}})
	badBase = bytes.Replace(badBase, []byte("WXYZ"), []byte("WXYQ"), 1)
	bbr, _ := zip.NewReader(bytes.NewReader(badBase), int64(len(badBase)))
	if err := verifyPackage(good, bbr, "L;\n"); err == nil {
		t.Error("a base entry that fails its checksum")
	}
}
