# ブリッジ: 回答 1 件（run）の寿命（`bridge/internal/server/runs.go`、`server.go`）

対象: ブリッジが `ask` を受けてから、回答のフレームを端末に渡し終え、忘れるまで（ADR 0013）。
run は WebSocket より長く生きる。フレーム（`line` / `done` / 回答を終わらせる `error`）には 0 から始まる通し番号 `seq` が付き、run が保存する。

## 不変条件（どの状態でも成り立つこと）
- I1: フレームの `seq` は 0 から連続している。欠けも重複もない。
- I2: 回答を終わらせるフレーム（`done` か、`seq` つきの `error`）は、1 つの run に**ちょうど 1 つ**。それより後にフレームは作られない。
- I3: 再送と新しいフレームは、同じロックの下で順に送る。再送の途中に新しいフレームが割り込んで、順序が崩れることはない。
- I4: 端末が `seq` N まで持っていると言えば、N 以降だけを送る。N が範囲外でも、壊れずに、範囲内の分だけ送る。
- I5: 取り消された・失敗した質問は、会話の記憶（ADR 0012）に残さない。

## 状態
| 記号 | 状態 | 意味 |
|---|---|---|
| N | NONE | その ID の run はない（まだ、または忘れた） |
| RA | RUN-ATT | Claude に聞いている最中。端末が接続している |
| RD | RUN-DET | Claude に聞いている最中。端末は接続していない（猶予タイマー作動中） |
| FA | FIN-ATT | 回答は完成済み。端末が接続している（保持中） |
| FD | FIN-DET | 回答は完成済み。端末は接続していない（保持中） |

## イベント
| 記号 | イベント |
|---|---|
| B1 | ASK: 新しい ID の質問（検証に通った） |
| B2 | ASK: すでにある ID と同じ ID |
| B3 | ASK: この接続がすでに別の質問を待っている |
| B4 | CANCEL: 端末から取消 |
| B5 | DISCONNECT: 端末の接続が終わった |
| B6 | RESUME: 端末が `resume{id,next}` を送った |
| B7 | GRACE: 猶予（既定 90 秒）が切れた |
| B8 | DELTA: Claude から文字が届いた |
| B9 | CLAUDE-DONE: Claude が正常に終わった |
| B10 | CLAUDE-FAIL: Claude が失敗した（起動できない、異常終了、取消含む） |
| B11 | FULL: 回答が、使えるページ数に収まらなくなった |
| B12 | SWEEP: 保持期限（10 分）切れ、または件数上限（16）の掃除（次の質問の開始時に実行） |
| B13 | SHUTDOWN: ブリッジの停止・再起動 |
| B14 | SEND-FAIL: 端末への書き込みに失敗した |
| B15 | CLEARHISTORY: 端末が会話の削除を頼んだ |
| B16 | HANG: Claude が長時間、何も返さない・終わらない |

## 状態遷移表
| 状態＼イベント | B1 ASK新 | B2 ASK重複 | B3 ASK多重 | B4 CANCEL | B5 DISC | B6 RESUME | B7 GRACE | B8 DELTA |
|---|---|---|---|---|---|---|---|---|
| **N** | R1→RA | —a | — | ×b | —c | E1 | —d | —e |
| **RA** | —a | E2 | E3 | R2→FA | R3→RD | R4（最新の接続が受け取る）→RA | —d | R5→RA |
| **RD** | —a | E2 | —f | R2→FD | —c | R4→RA | R6→FD | R5（保存のみ）→RD |
| **FA** | —a | E2 | —f | ×g | R7→FD | R4（全再送）→FA | —d | —h |
| **FD** | —a | E2 | —f | ×g | —c | R4（全再送）→FA | —d | —h |

| 状態＼イベント | B9 DONE | B10 FAIL | B11 FULL | B12 SWEEP | B13 SHUTDOWN | B14 SEND-FAIL | B15 CLEAR | B16 HANG |
|---|---|---|---|---|---|---|---|---|
| **N** | —e | —e | —e | —i | — | —j | R9 | —e |
| **RA** | R8→FA | R10→FA | R11→FA | ×k | 失われるl | R12 | R9（この run の組は完了後に追記される m） | ！ R13→FA ⚠ |
| **RD** | R8→FD | R10→FD | R11→FD | ×k | 失われるl | —j | R9 | ！ R13→FD ⚠ |
| **FA** | —h | —h | —h | R14（期限・上限超過）→N | 失われるl | R12 | R9 | —h |
| **FD** | —h | —h | —h | R14→N | 失われるl | —j | R9 | —h |

## 処理（Action）
| 記号 | 処理 | 現状 | テスト |
|---|---|---|---|
| R1 | run を作り、端末に接続（attach）して Claude に聞く。会話の記憶を読んで質問に付ける | ✅ | `server_test` |
| R2 | Claude を止める。`error`（`cancelled`）フレームを 1 つ作る | ✅ | `TestCancelStopsTheModel` |
| R3 | 接続を外し（detach）、猶予タイマーを始める | ✅ | `TestAnswerNobodyComesBackForIsCancelled` |
| R4 | `next` 以降のフレームを再送し、接続先をこの接続にする（猶予タイマーを止める） | ✅（RA での「奪い取り」は ❌ テストなし） | `TestDroppedConnection…`、`TestResumingBeforeTheGraceEnds…` |
| R5 | フレームを作って保存し、接続中なら送る | ✅ | `server_test` |
| R6 | Claude を止める。`error`（`cancelled`）を保存 | ✅ | `TestAnswerNobodyComesBackFor…` |
| R7 | 接続を外す。保持期限は完了時から数える | 🟡 | — |
| R8 | 会話の記憶に追記し、`done` フレームを 1 つ作る | ✅ | `TestFollowUpInTheSameNote…` |
| R9 | 会話の記憶をすべて消し、`historyCleared` を返す | ✅（実行中の run との関係は決めていない ❓） | `TestClearHistory…` |
| R10 | 回答を終わらせる `error` フレームを 1 つ作る。記憶には残さない | ✅ | `TestFailedOrCancelled…` |
| R11 | Claude を止め（これ以上は課金しない）、`truncated` つきの `done` を作る | ✅ | `ink` テスト、`server_test` |
| R12 | 最初の失敗だけログに残す。フレームは保存済みなので、再接続で取り戻せる | ✅ | — |
| R13 | 全体の時間切れ（案: 180 秒）で Claude を止め、`error`（`claude_failed`、理由つき）を作る | ❌ **時間切れが一切ない** | — |
| R14 | run を忘れる | ✅ | ❌ 掃除のテストなし |
| E1 | `error` `unknown_ask` を返す | ✅ | `TestResumeOfAnUnknownAnswerSaysSo` |
| E2 | `error` `bad_request`（duplicate id）を返す。既存の run には触れない | ✅ | ❌ テストなし |
| E3 | `error` `busy` を返す | ✅ | ❌ テストなし |

## 注
- a: 新しい ID でなければ B1 ではない（B2 になる）。
- b: 知らない ID の取消は無害なので無視する。
- c: 接続していないのに切断は起きない。
- d: 猶予タイマーは RD だけで動く。
- e: run がないので Claude も動いていない。
- f: 別の接続の質問は新しい run になる。この run には関係しない。
- g: すでに終わっているので何も止めるものがない。
- h: 完了後にフレームは作らない（I2）。
- i: 掃除は、完了した run だけが対象で、N には対象がない。
- j: 接続先がないので書き込みは起きない。
- k: 掃除は完了した run だけを忘れる（走っている run は忘れない）。
- l: メモリ上のみ。ブリッジの停止で run は失われ、端末の `resume` は `unknown_ask` になる（ADR 0013 の限界）。
- m: 実行中に履歴を消しても、その run の完了時に、その 1 往復が、消した後の履歴に入る。許容するか ❓。
- 掃除（B12）は次の質問の開始時にだけ走る。保持の上限（16 件）はメモリの上限でもある。

## この表から出る修正（優先順）
1. B16 / R13: Claude への問い合わせに全体の時間切れを入れる。
2. B2・B3・B12・RA での B6 のテストを足す。
3. B15 と実行中の run の関係を決める。
