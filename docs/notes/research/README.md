# 調査メモ（research）

調べた内容と、その結論。**各メモの先頭の「状態」を見れば、決着しているか、まだ開いているかが分かる。** 決着したものは、決めた先（ADR）を書いてある。決定は ADR（`docs/decisions/`）、仕様は `docs/spec/`、運営の手順は `docs/guides/`。

| メモ | 話題 | 状態 | 結論・決めた先 |
|---|---|---|---|
| [2026-10-30-competitive-landscape](2026-10-30-competitive-landscape.md) | 競合・類似 | 参考 | 「汎用 LLM の回答を手書きとして書く」は主流に無い |
| [2026-10-30-supernote-platform-survey](2026-10-30-supernote-platform-survey.md) | 拡張方式・SDK | **決着** | 公式プラグイン SDK（ADR 0001） |
| [2026-10-31-naming](2026-10-31-naming.md) | 名前 | **決着** | Spellink |
| [2026-10-31-page-overflow-and-canvas](2026-10-31-page-overflow-and-canvas.md) | 回答がページに収まらない | **決着** | 溢れたら次のページへ（ADR 0009） |
| [2026-10-31-remote-access](2026-10-31-remote-access.md) | 外出先からの接続 | **決着** | 運営の中継（ADR 0010）。自前トンネルは上級者向け |
| [2026-11-01-relay-hosting](2026-11-01-relay-hosting.md) | 中継の置き場所 | **決着** | Kagoya の VPS、scratch、自前の TLS（ADR 0019） |
| [2026-11-02-non-text-input](2026-11-02-non-text-input.md) | 文字以外の手書き | **未決（構想）** | 実装の予定なし |
| [2026-11-03-naming-round2](2026-11-03-naming-round2.md) | 改名の再検討 | **未決・保留** | 現行名のまま。ユーザーの判断待ち |
| [2026-11-08-pairing](2026-11-08-pairing.md) | ペアリング | **決着** | 端末ごとの B・K（ADR 0016） |
| [2026-11-20-bridge-app-draft](2026-11-20-bridge-app-draft.md) | Bridge App（トレイ常駐） | **未決（草案）** | 決めること（§9）待ち |
| [2026-11-20-direct-connection-options](2026-11-20-direct-connection-options.md) | 中継を通さない直結 | **決着** | 名前サービス＋各ブリッジの証明書（ADR 0017） |
| [2026-11-20-relay-decisions](2026-11-20-relay-decisions.md) | 中継の運営の判断 | **一部決着** | 規約は確認済み、置き場所は ADR 0019。**公開・費用・規約文書は未決** |
| [2026-11-21-relay-tls-in-scratch](2026-11-21-relay-tls-in-scratch.md) | scratch での TLS | **決着** | 中継が自分で TLS、暗号は aws-lc-rs（ADR 0019） |
| [2026-11-21-rust-measurement](2026-11-21-rust-measurement.md) | Rust の計測 | **決着** | 全面移植（ADR 0018）。本文の「Go のまま」は撤回 |

## 開いているもの（決めるのはユーザー）
1. 中継を**公開するか**、費用の持ち方、利用規約・プライバシーの文書（`2026-11-20-relay-decisions`）。
2. 改名の再検討（`2026-11-03-naming-round2`）。
3. Bridge App の「決めること」（`2026-11-20-bridge-app-draft` §9）。
4. 文字以外の手書き（`2026-11-02-non-text-input`）を扱うか。

## 書き方
新しい調査メモを足したら、この表に 1 行足し、メモの先頭に「状態」を書く。決着したら、状態を直し、決めた先（ADR）を書く。過去の本文は直さず、必要なら状態の行で「撤回」と書く。
