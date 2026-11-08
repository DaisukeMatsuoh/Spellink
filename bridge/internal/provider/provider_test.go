package provider

import (
	"context"
	"strings"
	"testing"

	"github.com/DaisukeMatsuoh/Spellink/bridge/internal/history"
)

func TestSystemPromptCarriesTheLengthBudget(t *testing.T) {
	if p := SystemPrompt("ja", 300); !strings.Contains(p, "300字以内") {
		t.Errorf("ja prompt: %q", p)
	}
	if p := SystemPrompt("en", 300); !strings.Contains(p, "60 words") {
		t.Errorf("en prompt: %q", p)
	}
	if p := SystemPrompt("ja", 1); !strings.Contains(p, "60字以内") {
		t.Errorf("budget must have a floor: %q", p)
	}
}

func TestFlatPutsTheEarlierConversationFirst(t *testing.T) {
	r := Request{Question: "next", Lang: "en", Earlier: []history.Turn{{Q: "q1", A: "a1"}}}
	flat := r.Flat()
	if !strings.Contains(flat, "q1") || !strings.Contains(flat, "a1") || !strings.HasSuffix(flat, "next") {
		t.Fatalf("got %q", flat)
	}
	if (Request{Question: "alone"}).Flat() != "alone" {
		t.Fatal("without history the question is the whole prompt")
	}
}

func TestFuncAdaptsAFunctionAndFillsInASilentSink(t *testing.T) {
	p := Func("test", func(_ context.Context, q, system string, s Sink) (string, error) {
		s.Text("x")      // a nil sink must not crash it
		s.Activity(true) // …neither for activity
		return q + "|" + system, nil
	})
	if p.Name() != "test" {
		t.Fatal(p.Name())
	}
	got, err := p.Ask(context.Background(), Request{System: "S", Question: "Q"}, Sink{})
	if err != nil || got != "Q|S" {
		t.Fatalf("%q %v", got, err)
	}
}
