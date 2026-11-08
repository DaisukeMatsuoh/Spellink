// Package claudecode is the provider that asks the locally installed, unmodified Claude Code CLI
// — signed in with the user's own account — and streams the answer back. The bridge never
// touches credentials (ADR 0005).
package claudecode

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"time"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/provider"
)

type Runner struct {
	Executable string
	Model      string
	workDir    string
}

// NewRunner prepares an empty working directory so Claude Code never picks up a project's
// CLAUDE.md or files.
func NewRunner(executable, model string) (*Runner, error) {
	dir, err := os.MkdirTemp("", "spellink-bridge-")
	if err != nil {
		return nil, err
	}
	return &Runner{Executable: executable, Model: model, workDir: dir}, nil
}

// Name is what the settings screen shows as the destination.
func (r *Runner) Name() string { return "Claude Code" }

// Ask runs `claude -p` and streams the answer. The question goes in on standard input; the
// earlier conversation of the note is put in front of it. Signs of life: onActivity(false) for
// anything the process emits (it started), onActivity(true) once the model itself has begun
// (message and thinking events, not only visible text). A process that has started but whose
// model never answers — the network to the AI is down, the service is overloaded — is told apart
// from one that is working, long before any text exists.
func (r *Runner) Ask(ctx context.Context, req provider.Request, sink provider.Sink) (string, error) {
	question, systemPrompt := req.Flat(), req.System
	onText, onActivity := sink.Text, sink.Activity
	if onText == nil {
		onText = func(string) {}
	}
	if onActivity == nil {
		onActivity = func(bool) {}
	}
	args := []string{
		"-p",
		"--output-format", "stream-json",
		"--verbose",
		"--include-partial-messages",
		"--tools", "",
		"--strict-mcp-config",
		"--setting-sources", "",
		"--no-session-persistence",
		"--system-prompt", systemPrompt,
	}
	if r.Model != "" {
		args = append(args, "--model", r.Model)
	}
	cmd := exec.CommandContext(ctx, r.Executable, args...)
	cmd.Dir = r.workDir
	// A killed process can leave children holding stdout open, which would block Wait forever.
	cmd.WaitDelay = 2 * time.Second
	cmd.Stdin = strings.NewReader(question)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return "", err
	}
	if err := cmd.Start(); err != nil {
		return "", err
	}

	var answer strings.Builder
	var resultErr string
	sc := bufio.NewScanner(stdout)
	sc.Buffer(make([]byte, 64*1024), 4*1024*1024)
	for sc.Scan() {
		var ev struct {
			Type    string `json:"type"`
			IsError bool   `json:"is_error"`
			Result  string `json:"result"`
			Subtype string `json:"subtype"`
			Event   struct {
				Type  string `json:"type"`
				Delta struct {
					Type string `json:"type"`
					Text string `json:"text"`
				} `json:"delta"`
			} `json:"event"`
		}
		if json.Unmarshal(sc.Bytes(), &ev) != nil {
			continue
		}
		onActivity(ev.Type == "stream_event" || ev.Type == "assistant" || ev.Type == "result")
		switch {
		case ev.Type == "stream_event" && ev.Event.Type == "content_block_delta" && ev.Event.Delta.Type == "text_delta":
			answer.WriteString(ev.Event.Delta.Text)
			onText(ev.Event.Delta.Text)
		case ev.Type == "result" && ev.IsError:
			resultErr = ev.Result
			if resultErr == "" {
				resultErr = ev.Subtype
			}
		}
	}
	waitErr := cmd.Wait()
	switch {
	case ctx.Err() != nil:
		return answer.String(), ctx.Err()
	case resultErr != "":
		return answer.String(), fmt.Errorf("%s", resultErr)
	case waitErr != nil:
		tail := stderr.String()
		if len(tail) > 2000 {
			tail = tail[len(tail)-2000:]
		}
		return answer.String(), fmt.Errorf("claude: %v: %s", waitErr, strings.TrimSpace(tail))
	}
	return answer.String(), nil
}
