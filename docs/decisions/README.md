# Architecture Decision Records

番号付きで追記していく。書式は各 ADR を参照（状態 / 背景 / 決定 / 結果）。
状態: `Proposed`（実機検証待ち等）→ `Accepted` / `Rejected` / `Superseded by NNNN`。

| # | タイトル | 状態 |
|---|---|---|
| [0001](0001-use-official-plugin-sdk.md) | 拡張方式に公式プラグイン SDK を採用する | Accepted |
| [0002](0002-ocr-on-device-first.md) | 手書きのテキスト化は端末内認識を第一にする | Proposed |
| [0003](0003-render-answer-as-strokes.md) | 回答は単線フォント由来のストロークとしてページに書き込む | Proposed |
| [0004](0004-direct-claude-api-byok-for-mvp.md) | MVP は端末から Claude API を直接呼ぶ（BYOK） | Superseded by 0005 |
| [0005](0005-claude-code-bridge-on-user-machine.md) | 自宅機で公式 Claude Code を動かす「ブリッジ」経由で Claude を使う | Accepted |
| [0006](0006-localization-strategy.md) | UI と筆跡出力のローカライズ方針 | Proposed |
| [0007](0007-relay-transport-and-security.md) | プラグイン⇔ブリッジ間の通信方式とセキュリティ | Proposed |
| [0008](0008-bridge-in-go.md) | ブリッジを Go で実装し、単一バイナリで配る | Accepted |
| [0009](0009-long-answers-continue-on-new-pages.md) | 長い回答は字数を合わせ、溢れたら直後に挿入したページへ続ける | Proposed |
| [0010](0010-rendezvous-relay.md) | 外出先からの接続は Spellink の中継サーバ（ランデブー）で受け持つ | Proposed |
| [0011](0011-lan-direct-wss.md) | 家の中は Let's Encrypt 証明書つきの wss:// で直接つなぐ | Proposed |
| [0012](0012-note-conversation-memory.md) | 同じノートでの質問は、前のやり取りを踏まえて答える（ブリッジが会話を記憶する） | Proposed |
| [0013](0013-resumable-answers.md) | 回答は接続から切り離し、番号つきのフレームで再開できるようにする | Proposed |
| [0014](0014-engineering-policy.md) | 開発ポリシー: 戻り値は必ず確認し、全ケースを表にしてから実装する | Accepted |
