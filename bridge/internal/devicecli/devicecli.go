// Package devicecli is `spellink-bridge device …`: the owner's commands for adding, pairing and
// removing devices. It talks to the running bridge over its local admin API (the pending pairing
// requests live in that process), never to the ledger file.
package devicecli

import (
	"bufio"
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"text/tabwriter"
	"time"
)

// Usage is printed for a command line that is not understood.
const Usage = `usage: spellink-bridge device <command>
  add <name> [dir]        add a device and write its setup file (Spellink-<name>.snplg) into dir
  list                    show every device and its state
  pair --web              open the pairing page: read the QR code a device shows with this computer's camera
  pair [code]             confirm the 8-digit code a device is showing (asked for if not given)
  package <name> [dir]    write the setup file again (nothing changes)
  revoke <name>           stop a device from connecting, now and for good
  reissue <name>          give a device a new first secret; it must be paired again
  forget <name>           delete a revoked or expired device from the list`

// Client is the command line's connection to the running bridge.
type Client struct {
	Base  string // e.g. http://127.0.0.1:8765
	Token string
	HTTP  *http.Client
	// Open shows an address in the user's browser (nil: only the address is printed).
	Open func(url string) error
}

type device struct {
	ID      string    `json:"id"`
	Name    string    `json:"name"`
	Gen     int       `json:"gen"`
	Status  string    `json:"status"`
	Waiting int       `json:"waiting"`
	Expires time.Time `json:"expires"`
	Last    time.Time `json:"lastSeen"`
}

// statusError is what the bridge answered when it refused.
type statusError struct {
	Status  int
	Message string
}

func (e *statusError) Error() string { return e.Message }

func (c *Client) do(method, path string, body any) (int, []byte, error) {
	var rd io.Reader
	if body != nil {
		b, err := json.Marshal(body)
		if err != nil {
			return 0, nil, err
		}
		rd = bytes.NewReader(b)
	}
	req, err := http.NewRequest(method, c.Base+path, rd)
	if err != nil {
		return 0, nil, err
	}
	req.Header.Set("Authorization", "Bearer "+c.Token)
	hc := c.HTTP
	if hc == nil {
		hc = &http.Client{Timeout: 30 * time.Second}
	}
	resp, err := hc.Do(req)
	if err != nil {
		return 0, nil, fmt.Errorf("cannot reach the bridge at %s (is it running? start it with `spellink-bridge serve` or the service): %w", c.Base, err)
	}
	defer resp.Body.Close()
	out, err := io.ReadAll(io.LimitReader(resp.Body, 64<<20))
	if err != nil {
		return 0, nil, err
	}
	if resp.StatusCode >= 300 {
		msg := strings.TrimSpace(string(out))
		if resp.StatusCode == http.StatusUnauthorized {
			msg = "the bridge did not accept the admin token (admin.token in its config directory)"
		}
		return resp.StatusCode, out, &statusError{resp.StatusCode, msg}
	}
	return resp.StatusCode, out, nil
}

// Run executes one `device` command. dir is where setup files are written when none is named.
func Run(c *Client, args []string, in io.Reader, out io.Writer, dir string) error {
	if len(args) == 0 {
		return errors.New(Usage)
	}
	cmd, rest := args[0], args[1:]
	switch cmd {
	case "add":
		if len(rest) < 1 || len(rest) > 2 {
			return errors.New("usage: spellink-bridge device add <name> [dir]")
		}
		return add(c, rest[0], pick(rest, 1, dir), out)
	case "list":
		return list(c, out)
	case "pair":
		if len(rest) == 1 && rest[0] == "--web" {
			return pairWeb(c, out)
		}
		code := ""
		if len(rest) > 0 {
			code = rest[0]
		}
		return pair(c, code, in, out)
	case "package":
		if len(rest) < 1 || len(rest) > 2 {
			return errors.New("usage: spellink-bridge device package <name> [dir]")
		}
		d, err := find(c, rest[0])
		if err != nil {
			return err
		}
		path, err := writePackage(c, d, pick(rest, 1, dir))
		if err != nil {
			return err
		}
		fmt.Fprintf(out, "Wrote %s\n", path)
		return nil
	case "revoke", "reissue", "forget":
		if len(rest) != 1 {
			return fmt.Errorf("usage: spellink-bridge device %s <name>", cmd)
		}
		return act(c, cmd, rest[0], out)
	}
	return errors.New(Usage)
}

func pick(args []string, i int, def string) string {
	if len(args) > i {
		return args[i]
	}
	return def
}

func fetch(c *Client) ([]device, error) {
	_, body, err := c.do("GET", "/admin/devices", nil)
	if err != nil {
		return nil, err
	}
	var list []device
	if err := json.Unmarshal(body, &list); err != nil {
		return nil, fmt.Errorf("the bridge sent a list this version cannot read: %w", err)
	}
	return list, nil
}

// find resolves a device by its exact name or its id.
func find(c *Client, nameOrID string) (device, error) {
	list, err := fetch(c)
	if err != nil {
		return device{}, err
	}
	for _, d := range list {
		if d.Name == nameOrID || d.ID == nameOrID {
			return d, nil
		}
	}
	return device{}, fmt.Errorf("no device named %q (see `spellink-bridge device list`)", nameOrID)
}

var unsafeName = regexp.MustCompile(`[^A-Za-z0-9._-]+`)

// fileName is the setup file's name: the device's name made safe for any file system.
func fileName(d device) string {
	n := strings.Trim(unsafeName.ReplaceAllString(d.Name, "_"), "._-")
	if n == "" {
		n = d.ID
	}
	return "Spellink-" + n + ".snplg"
}

// writePackage downloads the device's setup file into dir. It never overwrites a file: the file
// carries a secret until the device is paired, and a name clash must not destroy another.
func writePackage(c *Client, d device, dir string) (string, error) {
	_, data, err := c.do("GET", "/admin/devices/"+d.ID+"/package", nil)
	if err != nil {
		return "", err
	}
	path := filepath.Join(dir, fileName(d))
	f, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return "", fmt.Errorf("cannot write %s: %w (remove it first if you want a new one)", path, err)
	}
	if _, err := f.Write(data); err != nil {
		f.Close()
		os.Remove(path)
		return "", err
	}
	if err := f.Close(); err != nil {
		os.Remove(path)
		return "", err
	}
	// Success is checked by the result: the file must be the size that was sent.
	if st, err := os.Stat(path); err != nil || st.Size() != int64(len(data)) {
		return "", fmt.Errorf("%s was not written completely", path)
	}
	return path, nil
}

func add(c *Client, name, dir string, out io.Writer) error {
	_, body, err := c.do("POST", "/admin/devices", map[string]string{"name": name})
	if err != nil {
		return err
	}
	var created struct{ ID, Name string }
	if err := json.Unmarshal(body, &created); err != nil || created.ID == "" {
		return errors.New("the bridge's answer to \"add\" could not be read")
	}
	d := device{ID: created.ID, Name: created.Name}
	path, err := writePackage(c, d, dir)
	if err != nil {
		return fmt.Errorf("the device was added but its setup file could not be written: %w\n(write it again with `spellink-bridge device package %q`)", err, name)
	}
	fmt.Fprintf(out, "Added %q. Setup file: %s\n", name, path)
	fmt.Fprintln(out, "  1. Copy the file to the device (MyStyle folder) and install the plugin from it.")
	fmt.Fprintln(out, "  2. On the device, open Spellink and tap Pair: an 8-digit code appears.")
	fmt.Fprintln(out, "  3. Run `spellink-bridge device pair` and type the code.")
	fmt.Fprintln(out, "The file works only for this device, only until it is paired (24 hours by default). Keep it private until then.")
	return nil
}

func list(c *Client, out io.Writer) error {
	devs, err := fetch(c)
	if err != nil {
		return err
	}
	if len(devs) == 0 {
		fmt.Fprintln(out, "No devices yet. Add one with `spellink-bridge device add <name>`.")
		return nil
	}
	w := tabwriter.NewWriter(out, 0, 4, 2, ' ', 0)
	fmt.Fprintln(w, "NAME\tID\tSTATE\tNOTE")
	for _, d := range devs {
		note := ""
		switch d.Status {
		case "issued", "waiting", "granting":
			note = "setup file valid until " + d.Expires.Local().Format("2006-01-02 15:04")
			if d.Waiting > 0 {
				note = fmt.Sprintf("%d waiting for a code; %s", d.Waiting, note)
			}
		case "active":
			if !d.Last.IsZero() {
				note = "last seen " + d.Last.Local().Format("2006-01-02 15:04")
			}
		}
		fmt.Fprintf(w, "%s\t%s\t%s\t%s\n", d.Name, d.ID, d.Status, note)
	}
	return w.Flush()
}

func pair(c *Client, code string, in io.Reader, out io.Writer) error {
	if code == "" {
		fmt.Fprint(out, "Code shown on the device: ")
		line, err := bufio.NewReader(io.LimitReader(in, 256)).ReadString('\n')
		if err != nil && line == "" {
			return errors.New("no code was given")
		}
		code = strings.TrimSpace(line)
	}
	_, body, err := c.do("POST", "/admin/pair", map[string]string{"code": code})
	if err != nil {
		var se *statusError
		if errors.As(err, &se) && se.Status == http.StatusForbidden {
			return errors.New("that code was not accepted; check the device's screen and try again")
		}
		return err
	}
	var res struct{ Name string }
	if err := json.Unmarshal(body, &res); err != nil || res.Name == "" {
		return errors.New("the bridge's answer to \"pair\" could not be read")
	}
	fmt.Fprintf(out, "Code accepted: %q is being paired. The device finishes by itself.\n", res.Name)
	return nil
}

// pairWeb opens the bridge's pairing page.
func pairWeb(c *Client, out io.Writer) error {
	return openPage(c, "/pair", "Opened the pairing page in your browser. Show the QR code on the device to the camera.",
		"Open this address in a browser on this computer, then show the QR code on the device to the camera:", out)
}

// OpenApp opens the bridge's app page (status and devices) in the browser: `spellink-bridge open`.
func OpenApp(c *Client, out io.Writer) error {
	return openPage(c, "/app", "Opened the Spellink Bridge App in your browser.",
		"Open this address in a browser on this computer:", out)
}

// openPage opens one of the bridge's local pages. The command line's token travels in the part of the
// address after # — a browser never sends that to a server — so the page can use it and nothing else sees it.
func openPage(c *Client, path, opened, notOpened string, out io.Writer) error {
	// ask the bridge something first: a bridge that is not running should be said, not left to a blank browser tab
	if _, err := fetch(c); err != nil {
		return err
	}
	url := c.Base + path + "#" + c.Token
	if c.Open != nil {
		if err := c.Open(url); err == nil {
			fmt.Fprintln(out, opened)
			return nil
		}
	}
	fmt.Fprintln(out, notOpened)
	fmt.Fprintln(out, "  "+url)
	return nil
}

func act(c *Client, cmd, nameOrID string, out io.Writer) error {
	d, err := find(c, nameOrID)
	if err != nil {
		return err
	}
	if _, _, err := c.do("POST", "/admin/devices/"+d.ID+"/"+cmd, nil); err != nil {
		return err
	}
	switch cmd {
	case "revoke":
		fmt.Fprintf(out, "Revoked %q. It cannot connect any more.\n", d.Name)
	case "reissue":
		fmt.Fprintf(out, "Reissued %q: pair it again with `spellink-bridge device package %q`.\n", d.Name, d.Name)
	default:
		fmt.Fprintf(out, "Forgot %q.\n", d.Name)
	}
	return nil
}
