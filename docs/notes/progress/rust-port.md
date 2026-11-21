# Rust への移植の進み具合（ADR 0018）

**どちらが正か**: 移植が終わるまで、動いているのは `bridge/`（Go）。`bridge-rs/`（Rust）は、Go と同じテストを写しながら作っている。パッケージごとに「写したテストの件数」を下に書き、全部そろったら `bridge/` を `bridge-rs/` で置き換える。

進め方（毎回同じ）: ①Go のパッケージとテストを読む ②Rust の同名のモジュールに写す ③**テストを先に写し**、通す ④変異（コードを壊す）で、テストが落ちることを確かめる ⑤コミット。相互接続（Go ⇄ Rust）は、両方がそろった段階で、`check` と中継で確かめる。表（状態遷移）は `docs/spec/` が元で、生成器を Rust の出力にも対応させる。

全部のパッケージを移し、`bridge/` を Rust にした（2026-11-21）。Go 版は git の履歴（このコミットの直前）。テスト: Rust 369 件（直結・名前サービスを含む）、端末側 858 件（Rust のブリッジに対して）。

| 項目 | 状態 |
|---|---|
| 筆跡の核・中継・暗号・設定・履歴・プロバイダ・台帳・サーバー・画面・疎通確認・グリフ生成・サービス・端末用コマンド・プレビュー | 済み |
| 結合テスト・上書き更新テスト・Go との相互接続 | 済み |
| `lan`（家の中の wss の証明書） | **ADR 0017 で作り直して実装済み**（名前サービス、ACME、2 つの状態機械）。Cloudflare トークンの方式は移さず廃止（ADR 0011 は Superseded）。実環境での確認は未 |
| scratch コンテナ（中継・名前サービス） | `bridge/Dockerfile.edge` と `deploy/vps/compose.yaml` を書いた。**docker のある環境で未確認** |
| `THIRD_PARTY_NOTICES.md` | Go の依存のまま。Rust の依存の一覧に作り直す（`cargo deny list` か `cargo about`） |
| CI（`bridge.yml`・`release.yml`・`plugin.yml`） | 書き換えた。**一度も動かしていない**（push の許可が要る） |
| 実機（Nomad）での確認 | 未。開発機のブリッジは、まだ Go の古い版が動いている |

残り: `docs/` の古い Go の記述（手順書・ADR の本文は、当時の記録なので直さない）、`THIRD_PARTY_NOTICES.md`、`device-test-guide.md` の手順（Go のビルドの所）。

## 実装メモ（次の担当へ）
- **Go のソースの読み方**: 出力が省略されることがある。`sed 's/^/| /' ファイル` のように行頭に印をつけると、全文が読める。
- 表は `plugin/scripts/gen-tables.js` が Go と Rust の両方に出す（`bridge-rs/src/devices/table.rs`・`src/server/run_table.rs`）。`match` が全マスを網羅するので、マスが欠けるとコンパイルが通らない。
- 暗号（`securechan`）は入出力を持たない。送信は、書き込み順＝暗号化の順でなければならないので、`seal` が持つ鍵の錠の中で、送信キューに積む。サーバーは、接続ごとに「書き手のタスク」を1つ持ち、キューを順に WebSocket へ書く。
- `Timestamp` は Go の `time.Time` と同じ RFC 3339 を読み書きする（ローカル時刻のオフセットも読める）。Go が書いた台帳は、Rust が読める（テストあり）。
- 予約語 `gen` は Rust で使えない: コードでは `generation`、JSON では `gen`。
