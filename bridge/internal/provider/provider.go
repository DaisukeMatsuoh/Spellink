// Package provider is the bridge's boundary to "the AI": what is asked, and the one thing an AI
// has to be able to do — answer, streaming text and showing signs of life (ADR 0015). The rest of
// the bridge (runs, page layout, memory, reconnection) does not know which AI is behind it.
//
// Authentication belongs to the provider and never leaves the machine: a CLI provider uses the
// CLI's own login (the bridge never touches it); an API provider keeps its key in the bridge's
// config file. The plugin on the device is told only the provider's display name.
package provider

import (
	"context"
	"fmt"
	"strings"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/history"
)

// Request is one question for the AI.
type Request struct {
	// System says how to answer (plain sentences, how long); it does not depend on the AI.
	System string
	// Earlier is the conversation so far in the same note, oldest first.
	Earlier []history.Turn
	// Question is what was read from the handwriting.
	Question string
	// Lang is "ja" or "en", the language of the question.
	Lang string
}

// Sink is where an answer goes as it is produced.
type Sink struct {
	// Text receives each piece of the answer text.
	Text func(string)
	// Activity reports a sign of life. model is false for "the process runs", true once the model
	// itself has begun (thinking counts, not only visible text). A provider that cannot tell may
	// call it only with true, from Text.
	Activity func(model bool)
}

// Provider answers questions.
type Provider interface {
	// Name is shown to the user as the connection's destination ("Claude Code").
	Name() string
	// Ask answers r, streaming through s, and returns the whole answer. It must return when ctx
	// is cancelled. The caller owns the silence limits; the provider only reports activity.
	Ask(ctx context.Context, r Request, s Sink) (string, error)
}

// Flat is the request as one piece of text, for a provider that is given a single prompt: the
// earlier conversation in front of the question.
func (r Request) Flat() string { return history.Transcript(r.Earlier, r.Question, r.Lang) }

// Func adapts a function to a Provider; q is the Flat text. It is for tests and for providers
// that are a single function.
func Func(name string, f func(ctx context.Context, q, system string, s Sink) (string, error)) Provider {
	return funcProvider{name, f}
}

type funcProvider struct {
	name string
	f    func(ctx context.Context, q, system string, s Sink) (string, error)
}

func (p funcProvider) Name() string { return p.name }

func (p funcProvider) Ask(ctx context.Context, r Request, s Sink) (string, error) {
	if s.Text == nil {
		s.Text = func(string) {}
	}
	if s.Activity == nil {
		s.Activity = func(bool) {}
	}
	return p.f(ctx, r.Flat(), r.System, s)
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
