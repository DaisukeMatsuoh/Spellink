# ページ追加と書き込みの手順（`plugin/src/sdk.ts` の `insertPageAfter`、`writeStrokesAt`）

対象: 回答が 1 ページに収まらないとき、直後に新しいページを挿入して表示を移す手順（**挿入**）と、ストロークを書く手順（**書き込み**）。
一連の手順なので、縦軸は**手順**（これから行う手順）、横軸はその手順の**結果**。**この文書の表が、コードの表の元データ**（`npm run spec`。書式は ask-flow.md と同じ。処理 X0〜X4）。
実行側は、手順ごとに SDK を呼んで結果を `Ok / Failed / Refused / NoAnswer / NoteChanged / Unexpected` に分類し、表を引いて、次の手順か、中断か、完了かを決める。ノートアプリへの依頼は、**ノートを変えるもの・状態に依存するものは 1 本ずつ**、手元の要素を読むだけの独立した読み取りだけが並列（最大 8。ADR 0014）。

## 結果
| 結果 | 意味 |
|---|---|
| Ok | 成功、期待どおりの値 |
| Failed | `success: false`（呼び出しが通らない）または例外 |
| Refused | `success: true` だが `result: false`（ホストが断った） |
| NoAnswer | ホストが時間内に答えない |
| NoteChanged | 開いているノートが、質問したノートと違う |
| Unexpected | 値が期待と違う（数値でない、ページ数が増えていない、名前がない など） |

## 手順（挿入）
| 手順 | 内容 |
|---|---|
| CheckNote | 開いているのが質問したノートか確認（`getCurrentFilePath`） |
| Save | 保存（`saveCurrentNote`）。保存前の編集は、ページ挿入でキャッシュが読み直されて失われる |
| CountBefore | 挿入前のページ数を読む（`getNoteTotalPageNum`） |
| GetTemplate | 元のページのテンプレートを調べる（`getNotePageTemplate`） |
| InsertPage | ページを挿入（`insertNotePage`） |
| CountAfter | 挿入後のページ数を読み、増えたか確かめる |
| Jump | そのページへ表示を移す（`jumpToPage`） |
| Done | 新しいページができて、表示も移った（終了状態） |
| AbortClean | 中断。ノートは変わっていない（終了状態） |
| AbortDirty | 中断。ノートが一部変わっている可能性がある（新しい空のページが 1 枚増えているかもしれない）。巻き戻さず、理由を伝える（終了状態） |

## 手順表（挿入）

| 状態＼イベント | Ok | Failed | Refused | NoAnswer | NoteChanged | Unexpected |
|---|---|---|---|---|---|---|
| **CheckNote** | X2→Save | X0→AbortClean | —a | X0→AbortClean | X0→AbortClean | X0→AbortClean |
| **Save** | X2→CountBefore | X0→AbortClean | X0→AbortClean | X0→AbortClean | X0→AbortClean | —a |
| **CountBefore** | X2→GetTemplate | X0→AbortClean | —a | X0→AbortClean | X0→AbortClean | X0→AbortClean |
| **GetTemplate** | X2→InsertPage | X3→InsertPage | —a | X0→AbortClean | —b | X3→InsertPage |
| **InsertPage** | X2→CountAfter | X0→AbortClean | X0→AbortClean | X1→AbortDirty | X0→AbortClean | —a |
| **CountAfter** | X2→Jump | X1→AbortDirty | —a | X1→AbortDirty | X1→AbortDirty | X1→AbortDirty |
| **Jump** | X4→Done | X1→AbortDirty | X3→Done | X1→AbortDirty | X1→AbortDirty | —a |
| **Done** | —c | —c | —c | —c | —c | —c |
| **AbortClean** | —c | —c | —c | —c | —c | —c |
| **AbortDirty** | —c | —c | —c | —c | —c | —c |

## 手順（書き込み）
| 手順 | 内容 |
|---|---|
| CheckNote | 開いているのが質問したノートか確認 |
| WriteStrokes | ストロークを書く（要素を作る → 点を入れる → 挿入 → 解放） |
| Written | 書けた（終了状態） |
| AbortClean | 中断。何も書いていない（終了状態） |
| AbortDirty | 中断。一部が書かれた可能性がある（終了状態） |

## 手順表（書き込み）

| 状態＼イベント | Ok | Failed | Refused | NoAnswer | NoteChanged | Unexpected |
|---|---|---|---|---|---|---|
| **CheckNote** | X2→WriteStrokes | X0→AbortClean | —a | X0→AbortClean | X0→AbortClean | X0→AbortClean |
| **WriteStrokes** | X4→Written | X1→AbortDirty | X1→AbortDirty | X1→AbortDirty | —b | X1→AbortDirty |
| **Written** | —c | —c | —c | —c | —c | —c |
| **AbortClean** | —c | —c | —c | —c | —c | —c |
| **AbortDirty** | —c | —c | —c | —c | —c | —c |

## 処理
| 処理 | 内容 | 現状 | テスト |
|---|---|---|---|
| X0 | 中断（ノートは変わっていない）。原因のエラーを、そのまま呼び出し側へ返す | ✅ | `sdkSafety` |
| X1 | 中断（ノートが一部変わっている可能性）。`pageTurn.dirty` を記録し、原因のエラーを返す。巻き戻さない | ✅ | `sdkSafety` |
| X2 | 次の手順へ進む | ✅ | `sdkSafety` |
| X3 | 記録して続ける（白紙のテンプレートで代用する、表示を移せなかった、など。書き込みには要らない） | ✅ | `sdkSafety` |
| X4 | 完了 | ✅ | `sdkSafety` |

## 注
- a: その手順の SDK 呼び出しは、その種類の答えを返さない（`result` が真偽値でない、など）。
- b: その手順は、開いているノートに依存しない（ファイルの内容を読むだけ）。
- c: 終了状態。手順は終わっていて、結果を生むものがない。

## 補足
- 手順の途中での取消は、手順の境目では検査しない。各手順は短く、完了してから止まる（回答の行の境目では検査する）。
- 書き込みは、ページ番号を指定する。利用者が別のページを見ていても、書き込み先は変わらない（診断 8 で確認済み）。ノートそのものを別のものに替えると、CheckNote の確認で止まる。
- 時間切れでも、ホストの中では挿入が実行されているかもしれない（InsertPage × NoAnswer → AbortDirty）。巻き戻さず、利用者に伝える。
- 表示を動かすのは見た目だけの話で、書き込みには要らない（Jump × Refused → 記録して続ける）。
- **未対応（仕様の確認待ち）**: 回答の最中に、利用者が（回答の対象より前に）ページを挿入・削除すると、ページ番号がずれて別のページに書く。開始時のページ数を記録し、挿入のたびに想定と比べれば検出できる。
- 実機の戻り値の確認: ブール値の `result` はトレースに記録される。`getNoteTotalPageNum` が数値以外を返す場合は Unexpected になり、複数ページの回答が止まる。

## 履歴
- 2026-11-07: 表を機械が読む書式にした。挿入と書き込みを別の表にし、表示の移動（Jump）を挿入の末尾の手順にした。結果の列から、利用者の操作（USR）と取消（CAN）を外した（USR はノートが変わる形で NoteChanged に現れる。CAN は手順の途中では検査しない）。

