// Package protocol is the wire format between the plugin and the bridge: JSON text frames over
// one WebSocket. The plugin keeps its own copy in plugin/src/protocol.ts — change both together.
package protocol

import "github.com/DaisukeMatsuoh/Spellink/bridge/internal/ink"

const (
	Version = 1
	WSPath  = "/v1"
)

// ClientMessage is any frame the plugin sends; Type selects which fields are set.
//   - "hello":  Token, Client — must be the first frame
//   - "ask":    ID, Question, Lang ("ja" | "en"), Layout, Note (key of the open note; earlier
//     questions asked in the same note are handed to the model, ADR 0012)
//   - "cancel": ID
//   - "resume": ID, Next — after a dropped connection: send the answer's frames from number
//     Next on (every line/done/error frame of an answer carries Seq, counting from 0; ADR 0013)
//   - "report": Data — free-form diagnostics appended to the bridge's report log
//   - "setSettings": Settings — change user settings stored on the bridge
//   - "clearHistory": forget the conversations of every note, or only of Note if it is set
type ClientMessage struct {
	Type     string      `json:"type"`
	Token    string      `json:"token,omitempty"`
	Client   string      `json:"client,omitempty"`
	ID       string      `json:"id,omitempty"`
	Question string      `json:"question,omitempty"`
	Note     string      `json:"note,omitempty"`
	Next     int         `json:"next,omitempty"`
	Lang     string      `json:"lang,omitempty"`
	Layout   *ink.Layout `json:"layout,omitempty"`
	Data     any         `json:"data,omitempty"`
	Settings *Settings   `json:"settings,omitempty"`
}

// Settings are the user's plugin preferences. The plugin cannot persist data on the device,
// so the bridge stores them and hands them over in every welcome.
type Settings struct {
	// ShowProgress opens the progress panel while answering (debug view). Off: the answer is
	// just written onto the page; the panel appears only on errors.
	ShowProgress bool `json:"showProgress"`
	// LineTilt applies the agent's writing slope (TiltDegrees) to answers.
	LineTilt bool `json:"lineTilt"`
	// TiltDegrees is the agent's own habit: positive rises to the right. Chosen once per
	// bridge; read-only for the plugin.
	TiltDegrees float64 `json:"tiltDegrees"`
	// ThinkingAnimation draws a wave on the page while Claude thinks.
	ThinkingAnimation bool `json:"thinkingAnimation"`
	// FullWidthSpacing and HalfWidthSpacing are letter spacing in percent of the glyph's natural
	// advance, for full-width (kanji, kana) and half-width (Latin, digits) characters. 100 is
	// the tightest the glyph data allows.
	FullWidthSpacing int `json:"fullWidthSpacing"`
	HalfWidthSpacing int `json:"halfWidthSpacing"`
	// AvoidOverwriting reads the page before answering so the answer stays clear of existing
	// writing (and moves to a new page when there is no room). It costs a save and a read of the
	// whole page, so it is off unless turned on.
	AvoidOverwriting bool `json:"avoidOverwriting"`
	// BusyNotice shows a short notice when the user asks again while an answer is being written.
	BusyNotice bool `json:"busyNotice"`
}

// Limits of the spacing sliders (percent).
const (
	SpacingMin     = 100
	FullSpacingMax = 140
	HalfSpacingMax = 160
)

// SettingsReply confirms a setSettings.
type SettingsReply struct {
	Type     string   `json:"type"` // "settings"
	Settings Settings `json:"settings"`
}

// HistoryCleared confirms a clearHistory.
type HistoryCleared struct {
	Type  string `json:"type"` // "historyCleared"
	Notes int    `json:"notes"`
}

type Welcome struct {
	Type   string `json:"type"` // "welcome"
	Bridge string `json:"bridge"`
	// Provider is the AI the bridge asks ("Claude Code"), shown in the plugin's settings.
	Provider string   `json:"provider,omitempty"`
	Version  string   `json:"version"`
	Protocol int      `json:"protocol"`
	Settings Settings `json:"settings"`
}

type Status struct {
	Type  string `json:"type"` // "status"
	ID    string `json:"id"`
	Stage string `json:"stage"` // "thinking" | "writing" | "alive" (a keep-alive: the answer is still being produced)
	// Phase and IdleMs accompany a keep-alive: where the AI is ("starting": the process runs but
	// the model has not begun; "working": it has begun, no text yet; "writing") and how long
	// since it last showed a sign of life.
	Phase string `json:"phase,omitempty"`
	// IdleMs is a pointer so that a keep-alive always carries it, even when it is 0.
	IdleMs *int64 `json:"idleMs,omitempty"`
}

type Line struct {
	Type string `json:"type"` // "line"
	ID   string `json:"id"`
	Seq  int    `json:"seq"`
	ink.Line
}

type Done struct {
	Type      string   `json:"type"` // "done"
	ID        string   `json:"id"`
	Seq       int      `json:"seq"`
	Text      string   `json:"text"`
	Missing   []string `json:"missing"`
	Truncated bool     `json:"truncated"`
	Pages     int      `json:"pages"`
	ElapsedMs int64    `json:"elapsedMs"`
}

// Error codes: unauthorized, bad_request, busy, provider_failed, cancelled, unknown_ask, provider_timeout.
type Error struct {
	Type    string `json:"type"` // "error"
	ID      string `json:"id,omitempty"`
	Seq     *int   `json:"seq,omitempty"` // set when the error ends an answer, like line and done
	Code    string `json:"code"`
	Message string `json:"message"`
}
