# Rust への移植の進み具合（ADR 0018）

**どちらが正か**: 移植が終わるまで、動いているのは `bridge/`（Go）。`bridge-rs/`（Rust）は、Go と同じテストを写しながら作っている。パッケージごとに「写したテストの件数」を下に書き、全部そろったら `bridge/` を `bridge-rs/` で置き換える。

進め方（毎回同じ）: ①Go のパッケージとテストを読む ②Rust の同名のモジュールに写す ③**テストを先に写し**、通す ④変異（コードを壊す）で、テストが落ちることを確かめる ⑤コミット。相互接続（Go ⇄ Rust）は、両方がそろった段階で、`check` と中継で確かめる。表（状態遷移）は `docs/spec/` が元で、生成器を Rust の出力にも対応させる。

| Go のパッケージ | Rust のモジュール | 状態 |
|---|---|---|
| `internal/ink`（random・glyphs・handwriting・layout） | `random`・`glyphs`・`handwriting`・`layout` | 済み（Go と出力が1バイトも違わない。12 場面） |
| `internal/relay`（サーバー） | `relay` | 済み（17 件。変異で確認。musl・静的 1.4 MB） |
| `internal/relay`（connector・RoomSet） | `relay::connector` | 未 |
| `internal/securechan` | `securechan` | 済み（10 件。Go との相互接続は、サーバーがそろってから `check` で確認） |
| `internal/protocol` | `protocol` | 未 |
| `internal/config` | `config` | 済み（20 件。未知のキーの保持、0600、tmp+rename を含む） |
| `internal/history` | `history` | 済み（9 件） |
| `internal/provider`・`claudecode` | `provider`・`provider::claudecode` | 済み（14 件。取消と drop で CLI を止める） |
| `internal/devices`（ledger・registry・pkg・表） | `devices` | 済み（82 件。表は `npm run spec` が `devices/table.rs` に生成。変異で確認） |
| `internal/server`（server・runs・pairing・admin・表） | `server` | 未 |
| `internal/server/appui`・`pairpage`・`localguard` | `ui` | 未 |
| `internal/lantls` | `lantls` | 未（instant-acme） |
| `internal/probe`・`check` | `probe` | 未 |
| `internal/glyphbuild` | `glyphbuild` | 未 |
| `internal/service`・`service.go` | `service` | 未 |
| `internal/devicecli`・`devices.go` | `devicecli` | 未 |
| `main.go`・`preview.go`・`compose.go` | `main`・`cli` | 一部（relay・compose・ask・bench・idle） |
| 結合テスト・上書き更新テスト | `tests/` | 未 |

置き換えるときにやること: `CLAUDE.md`・CI・`release.yml`・手順書の Go の記述を直す。実測（待機のメモリ・バイナリ・起動時間）を取り直す。`cargo audit`・`cargo deny` を CI に置く。

## 実装メモ（次の担当へ）
- **Go のソースの読み方**: 出力が省略されることがある。`sed 's/^/| /' ファイル` のように行頭に印をつけると、全文が読める。
- 表は `plugin/scripts/gen-tables.js` が Go と Rust の両方に出す（`bridge-rs/src/devices/table.rs`・`src/server/run_table.rs`）。`match` が全マスを網羅するので、マスが欠けるとコンパイルが通らない。
- 暗号（`securechan`）は入出力を持たない。送信は、書き込み順＝暗号化の順でなければならないので、`seal` が持つ鍵の錠の中で、送信キューに積む。サーバーは、接続ごとに「書き手のタスク」を1つ持ち、キューを順に WebSocket へ書く。
- `Timestamp` は Go の `time.Time` と同じ RFC 3339 を読み書きする（ローカル時刻のオフセットも読める）。Go が書いた台帳は、Rust が読める（テストあり）。
- 予約語 `gen` は Rust で使えない: コードでは `generation`、JSON では `gen`。
