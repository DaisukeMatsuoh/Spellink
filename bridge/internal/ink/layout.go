package ink

import (
	"math"
	"regexp"
	"strings"
)

// Layout says where and how large to write, in page pixels.
type Layout struct {
	X            float64 `json:"x"`            // left edge of the text block
	Y            float64 `json:"y"`            // top of the first line, on the starting page
	Width        float64 `json:"width"`        // the block wraps at this width (never wider than the page)
	PageHeight   float64 `json:"pageHeight"`   //
	BottomMargin float64 `json:"bottomMargin"` // lines never cross PageHeight-BottomMargin
	CharSize     float64 `json:"charSize"`     // height of one character's em box
	// MaxPages is how many pages the answer may use: 1 = only below the selection;
	// more = continue at ContinueTop on newly inserted pages. 0 means 1.
	MaxPages    int     `json:"maxPages,omitempty"`
	ContinueTop float64 `json:"continueTop,omitempty"`
	// Slope tilts the whole answer block (dy/dx; negative rises to the right), like a writer's
	// habit of drifting up or down without ruled lines. Set by the bridge from the agent's
	// handwriting settings, not by the plugin.
	Slope float64 `json:"-"`
}

// Line is one finished line of handwriting. Page is relative to the starting page (0, 1, …).
type Line struct {
	Index   int      `json:"index"`
	Page    int      `json:"page"`
	Text    string   `json:"text"`
	Strokes []Stroke `json:"strokes"`
}

const (
	lineHeight = 1.45 // × CharSize
	spaceWidth = 0.33 // × CharSize, ASCII space
)

// Characters that must not start a line (kinsoku); they hang past the right edge instead.
var noLineStart = runeSet("、。，．,.:;!?！？）」』】〕ー…‥・ぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮ")

// Markdown decoration the model may still emit despite the system prompt.
var dropped = runeSet("*`#_\r")

var wordChar = regexp.MustCompile(`^[A-Za-z0-9'’\-]$`)

func runeSet(s string) map[rune]bool {
	m := map[rune]bool{}
	for _, r := range s {
		m[r] = true
	}
	return m
}

type pending struct {
	ch      rune
	advance float64
}

// Composer accepts answer text as it streams in and returns each line, rendered into
// strokes, as soon as the line is complete.
type Composer struct {
	font   *Font
	layout Layout
	rng    *Random

	Missing   map[rune]bool
	Truncated bool // text no longer fits within MaxPages; further text is ignored
	Pages     int  // pages used so far

	line             []pending
	lineWidth        float64
	wordStart        int
	lineIndex        float64 // line slot on the current page; fractional after a paragraph gap
	page             int
	emitted          int
	atParagraphStart bool
}

func NewComposer(font *Font, layout Layout, seed uint32) *Composer {
	if layout.MaxPages < 1 {
		layout.MaxPages = 1
	}
	return &Composer{
		font: font, layout: layout, rng: NewRandom(seed),
		Missing: map[rune]bool{}, wordStart: -1, atParagraphStart: true, Pages: 1,
	}
}

// Push adds streamed text and returns the lines it completed.
func (c *Composer) Push(text string) []Line {
	var out []Line
	for _, ch := range text {
		if c.Truncated {
			break
		}
		if ch >= 0xff01 && ch <= 0xff5e { // full-width ASCII → half-width
			ch -= 0xfee0
		}
		switch {
		case dropped[ch]:
		case ch == '\n':
			c.flush(&out, true)
		default:
			c.add(ch, &out)
		}
	}
	return out
}

// Finish returns the last, partial line.
func (c *Composer) Finish() []Line {
	var out []Line
	c.flush(&out, false)
	return out
}

func (c *Composer) advanceOf(ch rune) float64 {
	size := c.layout.CharSize
	switch ch {
	case ' ':
		return spaceWidth * size
	case '　':
		return size
	}
	g, ok := c.font.Glyphs[ch]
	if !ok {
		c.Missing[ch] = true
		return 0.5 * size
	}
	return float64(g.Advance) / float64(c.font.Em) * size
}

func (c *Composer) add(ch rune, out *[]Line) {
	if ch == ' ' && len(c.line) == 0 {
		return // no leading spaces on a wrapped line
	}
	adv := c.advanceOf(ch)
	isWord := wordChar.MatchString(string(ch))
	overflows := c.lineWidth+adv > c.layout.Width && len(c.line) > 0
	if overflows && !noLineStart[ch] {
		if isWord && c.wordStart > 0 {
			// Carry the partial Latin word over to the next line.
			carried := append([]pending(nil), c.line[c.wordStart:]...)
			c.line = c.line[:c.wordStart]
			c.emit(out)
			for _, p := range carried {
				c.append(p.ch, p.advance, true)
			}
		} else {
			c.emit(out)
			if ch == ' ' {
				return
			}
		}
	}
	c.append(ch, adv, isWord)
}

func (c *Composer) append(ch rune, adv float64, isWord bool) {
	if isWord {
		if c.wordStart < 0 {
			c.wordStart = len(c.line)
		}
	} else {
		c.wordStart = -1
	}
	c.line = append(c.line, pending{ch, adv})
	c.lineWidth += adv
}

// flush ends the current line. A hard break on an empty line leaves half a line between paragraphs.
func (c *Composer) flush(out *[]Line, hardBreak bool) {
	if len(c.line) > 0 {
		c.emit(out)
	} else if hardBreak && !c.atParagraphStart {
		c.lineIndex += 0.5
		c.atParagraphStart = true
	}
}

func (c *Composer) top() float64 {
	start := c.layout.Y
	if c.page > 0 {
		start = c.layout.ContinueTop
	}
	return start + c.lineIndex*c.layout.CharSize*lineHeight
}

func (c *Composer) fits() bool {
	return c.top()+c.layout.CharSize <= c.layout.PageHeight-c.layout.BottomMargin
}

func (c *Composer) emit(out *[]Line) {
	if !c.fits() {
		if c.page+1 >= c.layout.MaxPages {
			c.Truncated = true
			c.resetLine()
			return
		}
		c.page++
		c.Pages = c.page + 1
		c.lineIndex = 0
		c.atParagraphStart = true
	}
	top := c.top()
	var strokes []Stroke
	var text strings.Builder
	x := c.layout.X
	for _, p := range c.line {
		if g, ok := c.font.Glyphs[p.ch]; ok {
			strokes = append(strokes, RenderGlyph(g, c.font.Em, x, top, c.layout.CharSize, c.rng)...)
		}
		x += p.advance
		text.WriteRune(p.ch)
	}
	if c.layout.Slope != 0 {
		for _, st := range strokes {
			for k := 0; k < len(st.P); k += 2 {
				st.P[k+1] += int(math.Round((float64(st.P[k]) - c.layout.X) * c.layout.Slope))
			}
		}
	}
	*out = append(*out, Line{Index: c.emitted, Page: c.page, Text: text.String(), Strokes: strokes})
	c.emitted++
	c.lineIndex++
	c.atParagraphStart = false
	c.resetLine()
}

func (c *Composer) resetLine() {
	c.line = c.line[:0]
	c.lineWidth = 0
	c.wordStart = -1
}

// Capacity estimates how many full-width characters fit on the starting page below Y,
// so the model can be asked for an answer that fits without turning the page.
func Capacity(l Layout) int {
	perLine := math.Floor(l.Width / l.CharSize)
	lines := math.Floor((l.PageHeight - l.BottomMargin - l.Y) / (l.CharSize * lineHeight))
	return int(math.Max(0, perLine*lines))
}
