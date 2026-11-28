# 端末: 質問から回答まで（`plugin/src/askFlow.ts`、`askController.ts`）

対象: ラッソのボタン（「AIに聞く」）を押してから、回答を書き終える（または止まる）まで。1 回の質問が 1 つの `AskFlow`（状態機械）。

**この文書の表が、コードの表の元データ。** `npm run spec` が、この表から `plugin/src/askTable.generated.ts` を作る（全マスが埋まっていないと、生成が失敗する）。表を直す → `npm run spec` → ハンドラ（処理）を足す、の順。テスト（`askMachine.test`）が、文書とコードの表が一致していること、全マスが定義されていること、遷移先が実在することを確かめる。

## 書式（機械が読む。守ること）
- マス: `—<注>`（起こりえない）、`×<注>`（起きるが無視する）、`×A10`（無視するが、処理 A10 で記録する）、`A4→Failed`（処理して、遷移）、`A2`（処理して、その状態のまま）、`A9→Reconnecting [more] / A4→Failed [spent]`（再接続の回数が残っているかで分かれる）。
- 状態名・イベント名・処理 ID は、下の定義の表にあるものだけ。
- 注は小文字 1 文字（下の「注」）。

## 状態
| 状態 | 意味 |
|---|---|
| Idle | 質問なし。ここから、ボタンの押下で CheckingPermission へ |
| CheckingPermission | 権限の確認中 |
| Recognizing | ノートの場所・ラッソの範囲を読み、端末内の文字認識中 |
| Preparing | ペン・波線・（設定が on なら）ページの読み取り・ブリッジへの接続・（必要なら）新しいページへの移動 |
| Streaming | `ask` を送り、`line` を受けて書いている |
| Reconnecting | 完了前に接続が切れ、再接続を試している（試行回数 n は 0〜3。画面に「再接続中 (n/3)」） |
| Draining | `done` を受け取った。書き込みの列に残った行を書き切るところ |
| Completed | 完了（終了状態） |
| Failed | 失敗。理由を利用者に伝える（終了状態） |
| Cancelled | 利用者が取り消した（終了状態） |

## イベント
| イベント | 意味 |
|---|---|
| Pressed | 処理中に、ラッソの「AIに聞く」をもう一度選んだ（連打、または回答中の再操作）。新しい質問の開始は、新しい `AskFlow` が Idle から受ける |
| CancelRequested | 進捗パネルの取消・閉じる |
| Granted | 必要な権限がすべて許可された |
| Denied | 権限が拒否された |
| Recognized | ラッソの範囲から、質問の文字が得られた |
| NoText | 認識結果が空または null（楽譜・絵など） |
| Refused | SDK の呼び出しが失敗、または `result: false`（拒否）で、続けられない |
| NoAnswer | ノートアプリが時間内に答えない |
| NoteChanged | いま開いているノートが、質問したノートと違う |
| SoftFailed | なくても答えられる読み取り（空き領域の判定）の失敗 |
| Asked | ブリッジに接続でき、`ask` を送った |
| NoBridge | どのブリッジにも接続できない（最初の接続） |
| Line | 正常な `line` フレーム |
| Status | `status` フレーム（考え中・書き始め・生存通知） |
| BadFrame | 検査に通らないフレーム（値が不正、ページが範囲外、必須項目なし） |
| DoneFrame | `done` フレーム |
| ErrorFrame | 回答を終わらせる `error` フレーム（AI の失敗・時間切れを含む） |
| Closed | 接続が切れた（`done` の前）、または `ask` の送信に失敗した |
| Resumed | 再接続できて、`resume`（または、未送信だった `ask`）を送った |
| ResumeFailed | 再接続の 1 回が失敗した、またはブリッジがその回答を覚えていない（`unknown_ask`） |
| Quiet | 接続は開いているが、ブリッジからの通信（フレームも生存通知も）が一定時間まったく来ない = 接続が死んでいる |
| Drained | `done` のあと、書き込みの列が空になった |
| Late | 終わったあとに届く、遅れた切断やフレーム |
| Lifecycle | プラグインのホストのライフサイクル通知（停止・再開・破棄） |
| ScreenOpened | 設定画面・診断画面を開いた |

## 状態遷移表
行が状態、列がイベント。

| 状態＼イベント | Pressed | CancelRequested | Granted | Denied | Recognized | NoText | Refused |
|---|---|---|---|---|---|---|---|
| **Idle** | A1→CheckingPermission | ×a | —b | —b | —b | —b | —b |
| **CheckingPermission** | A2 | A3→Cancelled | A16→Recognizing | A4→Failed | —d | —d | A4→Failed |
| **Recognizing** | A2 | A3→Cancelled | —d | —d | A16→Preparing | A5→Failed | A4→Failed |
| **Preparing** | A2 | A3→Cancelled | —d | —d | —d | —d | A4→Failed |
| **Streaming** | A2 | A3→Cancelled | —d | —d | —d | —d | A12→Failed |
| **Reconnecting** | A2 | A3→Cancelled | —d | —d | —d | —d | A12→Failed |
| **Draining** | A2 | A3→Cancelled | —d | —d | —d | —d | A12→Failed |
| **Completed** | ×q | ×a | ×A10 | ×A10 | ×A10 | ×A10 | ×A10 |
| **Failed** | ×q | ×a | ×A10 | ×A10 | ×A10 | ×A10 | ×A10 |
| **Cancelled** | ×q | ×a | ×A10 | ×A10 | ×A10 | ×A10 | ×A10 |

| 状態＼イベント | NoAnswer | NoteChanged | SoftFailed | Asked | NoBridge | Line | Status |
|---|---|---|---|---|---|---|---|
| **Idle** | —b | —b | —b | —b | —b | —b | —b |
| **CheckingPermission** | A4→Failed | —d | —d | —d | —d | —e | —e |
| **Recognizing** | A4→Failed | ×f | —d | —d | —d | —e | —e |
| **Preparing** | A4→Failed | A12→Failed | A6 | A16→Streaming | A4→Failed | —e | —e |
| **Streaming** | A12→Failed | A12→Failed | —d | —d | —d | A15 | A18 |
| **Reconnecting** | A12→Failed | A12→Failed | —d | —d | —d | —i | —i |
| **Draining** | A12→Failed | A12→Failed | —d | —d | —d | ×A10 | ×A10 |
| **Completed** | ×A10 | ×A10 | ×A10 | ×A10 | ×A10 | ×A10 | ×A10 |
| **Failed** | ×A10 | ×A10 | ×A10 | ×A10 | ×A10 | ×A10 | ×A10 |
| **Cancelled** | ×A10 | ×A10 | ×A10 | ×A10 | ×A10 | ×A10 | ×A10 |

| 状態＼イベント | BadFrame | DoneFrame | ErrorFrame | Closed | Resumed | ResumeFailed | Quiet |
|---|---|---|---|---|---|---|---|
| **Idle** | —b | —b | —b | —b | —b | —b | —b |
| **CheckingPermission** | —e | —e | —e | —e | —k | —k | —e |
| **Recognizing** | —e | —e | —e | —e | —k | —k | —e |
| **Preparing** | —e | —e | —e | A9b→Reconnecting | —k | —k | —e |
| **Streaming** | A7→Failed | A8→Draining | A4→Failed | A9→Reconnecting | —k | —k | A9→Reconnecting |
| **Reconnecting** | —i | —i | —i | —i | A16→Streaming | A9→Reconnecting [more] / A4→Failed [spent] | —i |
| **Draining** | ×A10 | ×A11 | ×A10 | ×A10 | —k | —k | —l |
| **Completed** | ×A10 | ×A11 | ×A10 | ×A10 | ×A10 | ×A10 | —l |
| **Failed** | ×A10 | ×A11 | ×A10 | ×A10 | ×A10 | ×A10 | —l |
| **Cancelled** | ×A10 | ×A11 | ×A10 | ×A10 | ×A10 | ×A10 | —l |

| 状態＼イベント | Drained | Late | Lifecycle | ScreenOpened |
|---|---|---|---|---|
| **Idle** | —b | ×A10 | A13 | ×m |
| **CheckingPermission** | —r | ×A10 | A13 | ×m |
| **Recognizing** | —r | ×A10 | A13 | ×m |
| **Preparing** | —r | ×A10 | A13 | ×m |
| **Streaming** | —r | ×A10 | A13 | ×m |
| **Reconnecting** | —r | ×A10 | A13 | ×m |
| **Draining** | A17→Completed | ×A10 | A13 | ×m |
| **Completed** | ×A10 | ×A10 | A13 | ×m |
| **Failed** | ×A10 | ×A10 | A13 | ×m |
| **Cancelled** | ×A10 | ×A10 | A13 | ×m |

## 処理（Action）
| 処理 | 内容 | 現状 | テスト |
|---|---|---|---|
| A1 | 開始: 最初のメッセージ（権限の確認中）を出す | ✅ | `askController` |
| A2 | **二重押下への反応**: 新しい質問は始めない。設定 `busyNotice`（既定オン）がオンなら短い通知を出す。`ask.pressedWhileBusy` を記録 | ✅ | `askController` |
| A3 | 取消: 接続を閉じ、ID があればブリッジへ `cancel`。待っているものを解決。**書いた分は残す（巻き戻さない）** | ✅ | `askFlow` cancel 系 |
| A4 | 失敗: 理由を記録し、利用者に伝える（小さなダイアログ。出せなければパネル）。接続を閉じる | ✅ | `askController`、`askFlow` |
| A5 | 認識結果なし: 「文字として読み取れない内容です」を伝える | ✅ | `askController` |
| A6 | 空き領域の判定の失敗は、記録して続行（空き領域なしで書く） | ✅ | `sdkSafety` |
| A7 | 異常なフレーム: ブリッジへ `cancel` を送り、失敗にする（理由つき） | ✅ | `frames.test`、`askMachine` |
| A8 | `done` を受けた: 無音の監視を止め、書き込みの列が空になったら `Drained` を出す | ✅ | `askFlow` |
| A9 | **再接続**: 最大 3 回（連続）。各回は接続の確立を最大 5 秒待つ。画面に「再接続中 (n/3)」。成功は `Resumed`、失敗は `ResumeFailed`。続きのフレームを受けたら回数を 0 に戻す | ✅ | `askFlow`（reconnecting） |
| A9b | `ask` を送る前に切れた場合の再接続: 接続からやり直し、`ask` を送る（A9 と同じ回数） | ✅ | `askFlow`（reconnecting） |
| A10 | 無視して記録（終わったもの・遅れた通知） | ✅ | `askMachine` |
| A11 | 重複したフレームを捨てる（記録） | ✅ | `askFlow` |
| A12 | 書き込みを止める。書いた分は残す。ブリッジへ `cancel` を送り、失敗にする | ✅ | `askFlow`（拒否・空振り・時間切れ） |
| A13 | ホストのライフサイクルを記録（意味は未調査・保留） | ✅ 記録のみ | — |
| A15 | `line` の書き込みを列に積む（ページ挿入が要れば先に） | ✅ | `askFlow` |
| A16 | 次の段階へ進む（状態の移動だけ。やることは呼び出し側の手順が続ける） | ✅ | `askMachine` |
| A17 | 完了: 完了を伝え、待っているものを解決 | ✅ | `askFlow` |
| A18 | 生存通知または状態の通知: 「考え中」の表示を更新。（どのフレームも、無音の監視を延ばし、再接続の回数を 0 に戻す） | ✅ | `askFlow` |

## 注
- a: 取り消す対象がない／すでに終わっている。
- b: 質問が始まっていないので、そのイベントを生む処理がない。
- d: その状態では、そのイベントを生む処理が動いていない。
- e: 接続がまだない、または `ask` をまだ送っていないので、フレームは来ない。
- f: 認識の間にノートが変わっても、読み取りだけなので害はない。以降の書き込み前に `ensureOpen` が検出する（A12）。
- i: 再接続中は、受け取る接続がない。接続できたら `Resumed` で Streaming に戻り、フレームはそこで受ける。
- k: 再接続は Reconnecting の間だけ。
- l: 終了後・完了後の無音の判定は意味がない（タイマーは止めてある）。
- m: 設定・診断を開く操作は、回答に影響しない。回答は続き、画面は要求どおり開く。パネルの取消ボタンだけが取消（CancelRequested）。
- q: 質問ごとに新しい `AskFlow` を作る。終わったものは相手にしない（新しい質問は新しい `AskFlow` が Idle から始める）。
- r: `Drained` は `done` を受けた後（Draining）だけ。

## 補足（表に載せない共通の決まり）
- **どのフレームを受けても**（`status` の生存通知を含む）、無音の監視を延ばし、再接続の回数を 0 に戻す（受信の入口の共通処理）。
- `seq` が処理済み以下のフレームは、再送の重複なので、入口で捨てる（A11 の記録）。
- 入口が 2 つある: ラッソのボタン（Idle から）と、診断画面の「質問を直接送る」（権限と認識を飛ばして Preparing から。`runWithQuestion`）。
- **別のノートに移ったとき（NoteChanged）は、いまのまま止める**（決定 2026-11-07）。回答を保留してブリッジに残し、戻ったときに続きを書く案は、将来の候補。
- 時間切れ: SDK 呼び出しは通常 15 秒、保存・読み取り・ページ挿入・認識は 60 秒、権限ダイアログの待ちは 5 分（`sdk.ts`）。再接続の確立は 5 秒（`connectFirst`）。無音の判定は 15 秒（ブリッジが約 5 秒おきに生存通知を送る）。
- AI の無応答の時間切れは、ブリッジが持つ（bridge-run.md の R13）。
- 不明な型のフレームは、記録して読み飛ばす（`frames.ts`）。

## 履歴
- 2026-11-07: 進行のイベント（Granted / Recognized / Asked / Resumed / Drained）と Status・SoftFailed・ResumeFailed を、表に加えた。以前の表は、異常のイベントだけで、正常に進む遷移がなかった。終了状態での、遅れた結果のイベント（Refused / NoAnswer / NoteChanged など）は「起こりえない」ではなく「無視」に直した（取消のあとにも、実行中の処理は結果を返す）。
- 再接続を最大 3 回・成功で 0 に戻す。二重押下は反応を返す（設定で通知を切り替え）。無音は接続の死活（端末）と AI の無応答（ブリッジ）に分けた。

