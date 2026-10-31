# Spellink

Supernote（e-ink 手帳）用プラグイン。手書きをラッソで囲むと Claude に質問し、回答を筆跡としてページに書き戻す。

- 作業開始時は必ず `docs/notes/progress/README.md` を読み、作業の区切りで更新する（セッションをまたいだ引き継ぎ用）。
- 設計判断は `docs/decisions/` に ADR として追加する。調査・メモは `docs/notes/`。
- ドキュメントは日本語で書く。
- ブリッジは Go（`bridge/`、ツールチェーンは `~/.local/go/bin`）。プラグインは React Native（`plugin/`、`npm run build`）。
- 特許・知財の分析は `docs/notes/ip/private/`（git 管理外）に置き、公開ドキュメントに書かない（`docs/notes/ip/README.md`）。
