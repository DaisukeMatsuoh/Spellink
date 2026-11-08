package history

import (
	"strings"
	"testing"
	"time"
)

func turn(q, a string) Turn { return Turn{At: time.Unix(0, 0).UTC(), Q: q, A: a} }

func TestRecentKeepsNewestWithinTheBudgetAndOldestFirst(t *testing.T) {
	s := &Store{Dir: t.TempDir()}
	for _, q := range []string{"a", "b", "c"} {
		if err := s.Append("note", turn(q, strings.Repeat("x", 9))); err != nil { // 10 chars each
			t.Fatal(err)
		}
	}
	got, err := s.Recent("note", 25)
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 2 || got[0].Q != "b" || got[1].Q != "c" {
		t.Fatalf("want [b c], got %+v", got)
	}
	if got, _ := s.Recent("note", 5); len(got) != 0 {
		t.Fatalf("a turn larger than the budget must not be cut: %+v", got)
	}
}

func TestNotesAreSeparateAndUnknownNotesAreEmpty(t *testing.T) {
	s := &Store{Dir: t.TempDir()}
	_ = s.Append("one", turn("q1", "a1"))
	_ = s.Append("two", turn("q2", "a2"))
	if got, _ := s.Recent("one", 1000); len(got) != 1 || got[0].Q != "q1" {
		t.Fatalf("one: %+v", got)
	}
	if got, err := s.Recent("never", 1000); err != nil || len(got) != 0 {
		t.Fatalf("never: %+v %v", got, err)
	}
}

func TestSurvivesRestartAndSkipsDamagedLines(t *testing.T) {
	dir := t.TempDir()
	first := &Store{Dir: dir}
	_ = first.Append("n", turn("q1", "a1"))
	// a half-written line (e.g. power cut) must not lose the other turns
	f, _ := first.openForTest("n")
	_, _ = f.WriteString("{\"q\":\"torn\n")
	_ = f.Close()
	_ = first.Append("n", turn("q2", "a2"))
	got, err := (&Store{Dir: dir}).Recent("n", 1000)
	if err != nil || len(got) != 2 || got[1].Q != "q2" {
		t.Fatalf("%+v %v", got, err)
	}
}

func TestClearForgetsEverything(t *testing.T) {
	s := &Store{Dir: t.TempDir()}
	_ = s.Append("a", turn("q", "a"))
	_ = s.Append("b", turn("q", "a"))
	if n, err := s.Clear(); err != nil || n != 2 {
		t.Fatalf("cleared %d, %v", n, err)
	}
	if got, _ := s.Recent("a", 1000); len(got) != 0 {
		t.Fatalf("still there: %+v", got)
	}
}

func TestNilStoreAndEmptyNoteRememberNothing(t *testing.T) {
	var s *Store
	if err := s.Append("n", turn("q", "a")); err != nil {
		t.Fatal(err)
	}
	if got, err := s.Recent("n", 100); err != nil || got != nil {
		t.Fatalf("%v %v", got, err)
	}
	real := &Store{Dir: t.TempDir()}
	_ = real.Append("", turn("q", "a"))
	if got, _ := real.Recent("", 100); got != nil {
		t.Fatalf("empty note key stored: %v", got)
	}
}

func TestTranscript(t *testing.T) {
	if got := Transcript(nil, "今日は？", "ja"); got != "今日は？" {
		t.Fatalf("without history the question is sent as is, got %q", got)
	}
	got := Transcript([]Turn{turn("1+1は？", "2です。")}, "それを3倍して", "ja")
	for _, want := range []string{"[質問] 1+1は？", "[回答] 2です。", "今回の質問", "それを3倍して"} {
		if !strings.Contains(got, want) {
			t.Fatalf("missing %q in %q", want, got)
		}
	}
	if !strings.HasSuffix(got, "それを3倍して") {
		t.Fatalf("the new question must come last: %q", got)
	}
	if en := Transcript([]Turn{turn("q", "a")}, "next", "en"); !strings.Contains(en, "[Question] q") || !strings.HasSuffix(en, "next") {
		t.Fatalf("en: %q", en)
	}
}

func TestClearNoteForgetsOnlyThatNote(t *testing.T) {
	s := &Store{Dir: t.TempDir()}
	_ = s.Append("a", turn("qa", "aa"))
	_ = s.Append("b", turn("qb", "ab"))
	if n, err := s.ClearNote("a"); err != nil || n != 1 {
		t.Fatalf("cleared %d, %v", n, err)
	}
	if got, _ := s.Recent("a", 1000); len(got) != 0 {
		t.Fatalf("a is still there: %+v", got)
	}
	if got, _ := s.Recent("b", 1000); len(got) != 1 {
		t.Fatalf("b must be untouched: %+v", got)
	}
	if n, err := s.ClearNote("never"); err != nil || n != 0 {
		t.Fatalf("a note with no conversation: %d, %v", n, err)
	}
	if n, err := s.ClearNote(""); err != nil || n != 0 {
		t.Fatalf("no key, nothing to forget: %d, %v", n, err)
	}
	var nilStore *Store
	if n, err := nilStore.ClearNote("a"); err != nil || n != 0 {
		t.Fatalf("a nil store remembers nothing: %d, %v", n, err)
	}
}
