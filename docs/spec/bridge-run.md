# ブリッジ: 回答 1 件（run）の寿命（`bridge/internal/server/runs.go`、`server.go`）

対象: ブリッジが `ask` を受けてから、回答のフレームを端末に渡し終え、忘れるまで（ADR 0013）。
run は WebSocket より長く生きる。フレーム（`line` / `done` / 回答を終わらせる `error`）には 0 から始まる通し番号 `seq` が付き、run が保存する。
対象は、特定の AI に依存しない部分（ADR 0015）。AI への問い合わせは「プロバイダ」が行い、run はその結果（文字・完了・失敗）だけを扱う。

**この文書の表が、コードの表の元データ**（`npm run spec`。書式は ask-flow.md と同じ。生成物は `bridge/internal/server/runtable_gen.go`）。状態 NoRun の行は、run がまだ存在しない場合の規則で、run の登録簿（`Runs`）が実行する。N 以外の行は、run の状態機械が実行する。

## 不変条件（どの状態でも成り立つこと）
- I1: フレームの `seq` は 0 から連続している。欠けも重複もない。
- I2: 回答を終わらせるフレーム（`done` か、`seq` つきの `error`）は、1 つの run に**ちょうど 1 つ**。それより後にフレームは作られない。
- I3: 再送と新しいフレームは、同じロックの下で順に送る。再送の途中に新しいフレームが割り込んで、順序が崩れることはない。
- I4: 端末が `seq` N まで持っていると言えば、N 以降だけを送る。N が範囲外でも、壊れずに、範囲内の分だけ送る。
- I5: 取り消された・失敗した質問は、会話の記憶（ADR 0012）に残さない。

## 状態
| 状態 | 意味 |
|---|---|
| NoRun | その ID の run はない（まだ、または忘れた） |
| AskingLive | AI に聞いている最中。端末が接続している |
| AskingAway | AI に聞いている最中。端末は接続していない（猶予タイマー作動中） |
| DoneLive | 回答は完成済み。端末が接続している（保持中） |
| DoneAway | 回答は完成済み。端末は接続していない（保持中） |

## イベント
| イベント | 意味 |
|---|---|
| Ask | 新しい ID の質問（検証に通った） |
| AskRepeats | すでにある ID と同じ ID の質問 |
| AskWhileBusy | この接続がすでに別の質問を待っているときの質問 |
| Cancel | 端末から取消 |
| DeviceLeaves | 端末の接続が終わった |
| DeviceResumes | 端末が `resume{id,next}` を送った |
| GraceEnds | 猶予（既定 30 秒。端末の再接続は最大 3 回×5 秒）が切れた |
| TextArrives | AI（プロバイダ）から文字が届いた |
| AiFinishes | AI が正常に終わった |
| AiFails | AI が失敗した（起動できない、未ログイン、異常終了、取消、時間切れを含む） |
| PagesRunOut | 回答が、使えるページ数に収まらなくなった |
| Swept | 保持期限（10 分）切れ、または件数上限（16）の掃除（次の質問の開始時に実行） |
| BridgeStops | ブリッジの停止・再起動 |
| SendFails | 端末への書き込みに失敗した |
| HistoryCleared | 端末が会話の削除を頼んだ |
| AiGoesSilent | AI が、最初の出力まで 10 秒、または出力の間 10 秒（設定で変更可）、何も返さない。全体の上限は 30 秒 |
| KeepaliveTick | 生存通知の周期（約 5 秒）。端末が、接続の死活を見分けるため |

## 状態遷移表

| 状態＼イベント | Ask | AskRepeats | AskWhileBusy | Cancel | DeviceLeaves | DeviceResumes |
|---|---|---|---|---|---|---|
| **NoRun** | R1→AskingLive | —a | —a | ×b | —c | E1 |
| **AskingLive** | —a | E2 | E3 | R2→DoneLive | R3→AskingAway | R4 |
| **AskingAway** | —a | E2 | —f | R2→DoneAway | —c | R4→AskingLive |
| **DoneLive** | —a | E2 | —f | ×g | R7→DoneAway | R4 |
| **DoneAway** | —a | E2 | —f | ×g | —c | R4→DoneLive |

| 状態＼イベント | GraceEnds | TextArrives | AiFinishes | AiFails | PagesRunOut | Swept |
|---|---|---|---|---|---|---|
| **NoRun** | —d | —e | —e | —e | —e | —i |
| **AskingLive** | —d | R5 | R8→DoneLive | R10→DoneLive | R11→DoneLive | ×k |
| **AskingAway** | R6→DoneAway | R5 | R8→DoneAway | R10→DoneAway | R11→DoneAway | ×k |
| **DoneLive** | —d | ×h | ×h | ×h | ×h | R14→NoRun |
| **DoneAway** | —d | ×h | ×h | ×h | ×h | R14→NoRun |

| 状態＼イベント | BridgeStops | SendFails | HistoryCleared | AiGoesSilent | KeepaliveTick |
|---|---|---|---|---|---|
| **NoRun** | ×l | —j | R9 | —e | —e |
| **AskingLive** | ×l | R12 | R9 | R13→DoneLive | R15 |
| **AskingAway** | ×l | —j | R9 | R13→DoneAway | ×n |
| **DoneLive** | ×l | R12 | R9 | ×h | ×h |
| **DoneAway** | ×l | —j | R9 | ×h | ×h |

## 処理（Action）
| 処理 | 内容 | 現状 | テスト |
|---|---|---|---|
| R1 | run を作り、端末に接続（attach）して AI に聞く。会話の記憶を読んで質問に付ける | ✅ | `server_test` |
| R2 | AI を止める。`error`（`cancelled`）フレームを 1 つ作る | ✅ | `TestCancelStopsTheModel` |
| R3 | 接続を外し（detach）、猶予タイマーを始める | ✅ | `TestAnswerNobodyComesBackForIsCancelled` |
| R4 | `next` 以降のフレームを再送し、接続先をこの接続にする（猶予タイマーを止める）。すでに別の接続が受け取っていれば、最新の接続が受け取る | ✅ | `TestDroppedConnection…`、`TestResumingBeforeTheGraceEnds…`、`TestTheNewestConnectionReceivesTheAnswer` |
| R5 | フレームを作って保存し、接続中なら送る | ✅ | `server_test` |
| R6 | 猶予が切れた: AI を止める。`error`（`cancelled`）を保存 | ✅ | `TestAnswerNobodyComesBackFor…` |
| R7 | 接続を外す。保持期限は完了時から数える | ✅ | `TestTheShapeOfTheTable`（完了は FA↔FD のみ） |
| R8 | 会話の記憶に追記し、`done` フレームを 1 つ作る | ✅ | `TestFollowUpInTheSameNote…` |
| R9 | 会話の記憶をすべて消し、`historyCleared` を返す | ✅ | `TestClearHistory…` |
| R10 | 回答を終わらせる `error` フレームを 1 つ作る。記憶には残さない | ✅ | `TestFailedOrCancelled…` |
| R11 | AI を止め（これ以上は課金しない）、`truncated` つきの `done` を作る | ✅ | `server_test` |
| R12 | 最初の失敗だけログに残す。フレームは保存済みなので、再接続で取り戻せる | ✅ | — |
| R13 | **AI の無応答の時間切れ**: 止めて、`error`（`provider_timeout`、理由つき）を 1 つ作る。最初の出力までの時間も、1 質問ごとにログに記録する | ✅ | `TestAnAIThatNeverStartsIsStopped` ほか |
| R14 | run を忘れる | ✅ | `TestFinishedAnswersAreForgotten…` |
| R15 | 生存通知（`status`、`stage: alive`、`phase`、`idleMs`、保存しない）を約 5 秒おきに送る | ✅ | `TestARunningAnswerSaysItIsAlive` |
| E1 | `error` `unknown_ask` を返す | ✅ | `TestResumeOfAnUnknownAnswerSaysSo` |
| E2 | `error` `bad_request`（duplicate id）を返す。既存の run には触れない | ✅ | `TestTheSameIdTwice…` |
| E3 | `error` `busy` を返す | ✅ | `TestTheSameIdTwice…` |

## 注
- a: 新しい ID でなければ Ask ではない（AskRepeats になる）。この ID の run があるのに NoRun ではない、という矛盾は起きない。
- b: 知らない ID の取消は無害なので無視する。
- c: 接続していないのに切断は起きない。
- d: 猶予タイマーは AskingAway だけで動く。NoRun・DoneLive・DoneAway には対象がない。
- e: run がないので AI も動いていない。
- f: 別の接続の質問は新しい run になる。この run には関係しない。
- g: すでに終わっているので何も止めるものがない。
- h: 完了後にフレームは作らない（不変条件 I2）。完了のあとに届く、AI の遅れた結果・生存通知の最後の 1 回は、無視する（取消のあとにも、AI は結果を返す）。
- i: 掃除は、完了した run だけが対象で、NoRun には対象がない。
- j: 接続先がないので書き込みは起きない。
- k: 掃除は完了した run だけを忘れる（走っている run は忘れない）。
- l: メモリ上のみ。ブリッジの停止で run は失われ、端末の `resume` は `unknown_ask` になる（ADR 0013 の限界）。
- n: 端末が接続していないので、生存通知を送る相手がいない。
- 実行中に履歴を消しても、その run の完了時に、その 1 往復が、消した後の履歴に入る。**保留**（ユーザー判断待ち、2026-11-07）。現状のまま。
- 掃除（Swept）は次の質問の開始時にだけ走る。保持の上限（16 件）はメモリの上限でもある。

## 履歴
- 2026-11-07: 表を機械が読む書式にした（イベント名を B1〜B17 から名前に）。全体の時間切れを 30 秒、猶予を 30 秒に短縮した。

