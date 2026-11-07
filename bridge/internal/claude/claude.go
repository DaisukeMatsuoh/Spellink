// Package claude asks the locally installed, unmodified Claude Code CLI — signed in with the
// user's own account — and streams the answer back. The bridge never touches credentials.
package claude

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

// SystemPrompt asks for plain sentences of roughly budget characters (ja) or budget/5 words (en).
func SystemPrompt(lang string, budget int) string {
	if lang == "en" {
		words := max(30, budget/5)
		return strings.Join([]string{
			"You answer questions inside a handwritten notebook.",
			"Your answer will be written onto the page as handwriting.",
			"Use plain sentences only: no Markdown, headings, bullet symbols, tables, emoji or code blocks.",
			fmt.Sprintf("Be concise: aim for at most %d words so the answer fits the space on the page. Use line breaks between paragraphs.", words),
			"The question was read by handwriting recognition and may contain recognition errors; infer the intent from context.",
		}, "\n")
	}
	return strings.Join([]string{
		"あなたは手書きノートの中で質問に答えるアシスタントです。",
		"あなたの回答は手書きの文字としてノートのページに書き込まれます。",
		"Markdown、見出し、箇条書き記号、表、絵文字、コードブロックは使わず、プレーンな文章で答えてください。",
		fmt.Sprintf("ページの余白に収まるよう、要点を簡潔に、%d字以内を目安に答えてください。段落を分けるときは改行を使ってください。", max(60, budget)),
		"質問は手書き文字認識で読み取ったものなので、誤認識が含まれることがあります。文脈から意図を汲み取ってください。",
	}, "\n")
}

// Ask runs `claude -p` and calls onText for every streamed text delta. It returns the full answer.
func (r *Runner) Ask(ctx context.Context, question, systemPrompt string, onText func(string)) (string, error) {
	return r.AskActive(ctx, question, systemPrompt, onText, func(bool) {})
}

// AskActive is Ask that also reports signs of life: onActivity(false) for anything the process
// emits (it started), onActivity(true) once the model itself has begun (message and thinking
// events, not only visible text). A process that has started but whose model never answers —
// the network to the AI is down, the service is overloaded — is told apart from one that is
// working, long before any text exists.
func (r *Runner) AskActive(ctx context.Context, question, systemPrompt string, onText func(string), onActivity func(model bool)) (string, error) {
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
