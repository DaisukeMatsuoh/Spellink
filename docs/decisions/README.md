# Architecture Decision Records

番号付きで追記していく。書式は各 ADR を参照（状態 / 背景 / 決定 / 結果）。
状態: `Proposed`（実機検証・ユーザー確認待ち）→ `Accepted` / `Rejected` / `Superseded by NNNN`。

## 運用ルール
- 決定は ADR に書き、**状態を変えるときは、根拠（いつ・何で確認したか）を、その ADR の末尾に日付つきで追記する**。過去の記述は直さず、追記で更新する。
- `Accepted` にするのは、実機で確認した、またはユーザーが確定した決定だけ。実装しただけでは `Proposed`。
- 設計の「仕様」（状態遷移表・インターフェース台帳）は ADR ではなく `docs/spec/` に置く。ADR はなぜそう決めたか、`docs/spec/` は何をどう処理するか。
- 開発の作法は ADR 0014（`CLAUDE.md` にも要約）。

## 一覧（2026-11-07 時点）
| # | タイトル | 状態 | 確認の根拠 |
|---|---|---|---|
| [0001](0001-use-official-plugin-sdk.md) | 拡張方式に公式プラグイン SDK を採用する | Accepted | — |
| [0002](0002-ocr-on-device-first.md) | 手書きのテキスト化は端末内認識を第一にする | Accepted | 実機で認識を確認（2026-11） |
| [0003](0003-render-answer-as-strokes.md) | 回答は単線フォント由来のストロークとしてページに書き込む | Accepted | 実機で挿入・表示・速度を確認（2026-11） |
| [0004](0004-direct-claude-api-byok-for-mvp.md) | MVP は端末から Claude API を直接呼ぶ（BYOK） | Superseded by 0005 | — |
| [0005](0005-claude-code-bridge-on-user-machine.md) | 自宅機で公式 Claude Code を動かす「ブリッジ」経由で Claude を使う | Accepted | — |
| [0006](0006-localization-strategy.md) | UI と筆跡出力のローカライズ方針 | Accepted（ja/en の範囲） | ja/en を実装・実機使用 |
| [0007](0007-relay-transport-and-security.md) | プラグイン⇔ブリッジ間の通信方式とセキュリティ | Accepted | wss + Noise を実機で確認（2026-11-07）。平文 hello の削除が残る |
| [0008](0008-bridge-in-go.md) | ブリッジを Go で実装し、単一バイナリで配る | Accepted | — |
| [0009](0009-long-answers-continue-on-new-pages.md) | 長い回答は字数を合わせ、溢れたら直後に挿入したページへ続ける | Proposed | ページ追加の堅牢化（0013・0014）の実機確認待ち |
| [0010](0010-rendezvous-relay.md) | 外出先からの接続は Spellink の中継サーバ（ランデブー）で受け持つ | Proposed | 実装済み。端末経由の確認・運営方針が未決定 |
| [0011](0011-lan-direct-wss.md) | 家の中は Let's Encrypt 証明書つきの wss:// で直接つなぐ | Proposed | 実証明書での確認待ち（API トークンが要る） |
| [0012](0012-note-conversation-memory.md) | 同じノートでの質問は、前のやり取りを踏まえて答える（ブリッジが会話を記憶する） | Accepted | 実機で確認（2026-11-07） |
| [0013](0013-resumable-answers.md) | 回答は接続から切り離し、番号つきのフレームで再開できるようにする | Proposed | 実機で、切断後の再開を確認待ち |
| [0014](0014-engineering-policy.md) | 開発ポリシー: 戻り値は必ず確認し、全ケースを表にしてから実装する | Accepted | ユーザー指示（2026-11-07） |
| [0015](0015-ai-provider-abstraction.md) | 問い合わせ先の AI を差し替え可能な「プロバイダ」にし、利用者向けの呼称を中立にする | Accepted | 実装済み（2026-11-08）。実機未確認 |

## 関連する仕様
- 状態遷移表・インターフェース台帳: [`docs/spec/`](../spec/README.md)
