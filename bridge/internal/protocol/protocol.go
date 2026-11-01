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
//   - "ask":    ID, Question, Lang ("ja" | "en"), Layout
//   - "cancel": ID
//   - "report": Data — free-form diagnostics appended to the bridge's report log
//   - "setSettings": Settings — change user settings stored on the bridge
type ClientMessage struct {
	Type     string      `json:"type"`
	Token    string      `json:"token,omitempty"`
	Client   string      `json:"client,omitempty"`
	ID       string      `json:"id,omitempty"`
	Question string      `json:"question,omitempty"`
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
}

// SettingsReply confirms a setSettings.
type SettingsReply struct {
	Type     string   `json:"type"` // "settings"
	Settings Settings `json:"settings"`
}

type Welcome struct {
	Type     string   `json:"type"` // "welcome"
	Bridge   string   `json:"bridge"`
	Version  string   `json:"version"`
	Protocol int      `json:"protocol"`
	Settings Settings `json:"settings"`
}

type Status struct {
	Type  string `json:"type"` // "status"
	ID    string `json:"id"`
	Stage string `json:"stage"` // "thinking" | "writing"
}

type Line struct {
	Type string `json:"type"` // "line"
	ID   string `json:"id"`
	ink.Line
}

type Done struct {
	Type      string   `json:"type"` // "done"
	ID        string   `json:"id"`
	Text      string   `json:"text"`
	Missing   []string `json:"missing"`
	Truncated bool     `json:"truncated"`
	Pages     int      `json:"pages"`
	ElapsedMs int64    `json:"elapsedMs"`
}

// Error codes: unauthorized, bad_request, busy, claude_failed, cancelled.
type Error struct {
	Type    string `json:"type"` // "error"
	ID      string `json:"id,omitempty"`
	Code    string `json:"code"`
	Message string `json:"message"`
}
