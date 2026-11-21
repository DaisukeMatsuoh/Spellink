# 別のエージェントへの引き継ぎ（遠隔での作業用）

2026-11-08。ユーザーが外出し、**この開発機の Claude Code は、遠隔操作（Remote Control）では使えない**ため、一部の作業を別のエージェントが行う。先に `CLAUDE.md`（規約）と `README.md`（現状）を読み、経緯が要るときだけ `history.md` を読む。

## 守ること（要約。詳しくは `CLAUDE.md`、ADR 0014）
1. **push しない。** コミットは可。コミットの末尾に `Co-Authored-By: <使っているモデル> <noreply@anthropic.com>` を付ける。
2. **表が先。** 状態を持つ処理は、`docs/spec/` の表を直す → `cd plugin && npm run spec` → ハンドラを書く → テスト。生成された `*.generated.ts` / `runtable_gen.go` は手で直さない。
3. 戻り値・境界の値は必ず検証する。場当たり的な再試行や、回数・時間での帳尻合わせをしない。
4. **テストは、件数だけでなくスイート数を確認**する（端末 18 スイート、Go は 11 パッケージ（`probe` を含む））。本番の入口（ラッソなら `AskFlow.run()`）から通すテストを書く。壊して落ちることも確かめる。
5. 作業の区切りで、`README.md`（現状・次にやること）と `log/2026-11.md`（日付つきの追記）を更新する。判断を伴うことは ADR。
6. ドキュメントは日本語。ユーザーへの説明は、短く、日本語。
7. 特許・知財の分析は `docs/notes/ip/private/`（git 管理外）にだけ書く。

## 確認コマンド
```bash
export PATH=$HOME/.cargo/bin:$PATH
cd ~/GitWorkspaces/Cloute/bridge  && cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked   # 312 件 ok（Rust。ADR 0018）
cd ~/GitWorkspaces/Cloute/plugin  && npx tsc --noEmit && npx jest                                  # 18 スイート・30 スイート・858 件（統合テストは cargo が要る）
cd ~/GitWorkspaces/Cloute/plugin  && npm run spec && git diff --stat                                # 表を直したあと。差分は生成物だけ
```

## 担当の分け方
### ユーザーだけができること（実機・権限・判断）
- Supernote に `plugin/build/outputs/Spellink.snplg` を入れ直し、新しいノートで確認する。確認項目は README の「実機で見てほしいこと」。
- `sudo` が要ること（cloudflared のトークン、systemd）。ブリッジの自動起動のコマンドは README の「手順」。
- 判断: 名前、公開の時期と形、リレーの運営、他社の AI を足す順番（README の「未決」）。

### この開発機のエージェント（ローカル）でないとできないこと
- 動いているブリッジ・トンネルの確認と再起動（`spellink-bridge check`）、端末から送られた診断（`~/.config/spellink-bridge/reports.jsonl`）の読み取り、実 Claude Code を使う確認。

### 遠隔・別のエージェントでもできること（コードと文書だけ。端末も、この機械のサービスも要らない）
2026-11-08 に、CI の起動条件、表の穴の埋め直し、ADR 0015、ノート単位の削除、lantls の点検とリレーの回数制限、端末側の統合テスト、`service install`、公開用の文書の草案、リリースの自動化は済んだ。残り:
1. **会話の圧縮**（ADR 0012 の予約分。`history.compaction: "summarize"`。要約を作る AI の呼び出しをどうするか、先に設計）。
2. **他社の AI の追加**: `internal/provider/<名前>`、`main.go` の `newProvider`、`config.KnownProviders`。**規約と料金の確認が先**（ADR 0015）。
3. **多言語**（中国語・スペイン語・韓国語。文字データと UI 文字列。ADR 0006）。
4. 文字以外の入力（楽譜など）の調査の続き（`docs/notes/research/2026-11-02-non-text-input.md`。画像は外部に送らない方針）。
5. 利用者向けのセットアップ手順書の仕上げ（`docs/guides/bridge-and-relay-for-users.md` と `service install` / `check` を元に）。

### 先にユーザーに聞くこと（勝手に決めない）
- ページ数の変化の検出（回答の最中に、利用者がページを挿入・削除したとき）。仕様の確認待ち（`docs/spec/page-turn.md`）。
- 履歴の削除と、実行中の回答の関係。ホストのライフサイクルの意味（実機で調べる）。
- 波線の消し残しを、利用者に知らせる方法。

## 遠隔のエージェントを動かすときの注意
- **Remote Control は、`claude remote-control` をそのまま使うと、「公式の API でない」と断られる。** 理由: caveman のインストーラが `~/.claude/settings.json` の `env` に `ANTHROPIC_BASE_URL=http://127.0.0.1:8787/w/claude`（この機械の caveman のゲートウェイ）を書いており、Remote Control は、向き先が `api.anthropic.com` 以外だと使えないため（二重の中継ではなく、1 段。向き先を変えるだけで、上流は Anthropic）。`settings.json` の `env` は、起動時の環境変数より**優先される**（実測）ので、環境変数で上書きしても効かない。
- **起動は `claude-rc`**（`~/.local/bin/claude-rc [作業ディレクトリ]`）。`~/.claude-rc` という別の設定ディレクトリを使い、ログイン情報・`projects`（記憶や会話の履歴）は `~/.claude` と共有（シンボリックリンク）、caveman の hooks は残し、ゲートウェイの `env` だけを外してある。**初回だけ、端末で `claude-rc` を実行して `y` と答える**（「Enable Remote Control?」。一度答えれば、聞かれない）。caveman のゲートウェイ（入力・出力の圧縮の測定）は、この経路では使われない。
- `~/.claude/settings.json` を caveman が書き換えたら（更新や再設定）、`~/.claude-rc/settings.json` は自動では追従しない。hooks が変わったら、`env` を除いて作り直す。
- **コミットは push していない**（`stg` ブランチ）。別の機械のエージェントは、このリポジトリのコミットを見られない。同じ機械で動かすか、ユーザーが push を許可するか（または `git bundle` で渡す）。
- 作業ツリーに、ユーザーの未コミットの変更があれば（`git status`）、触らない。
- 実行中のもの: ブリッジ（`~/.local/bin/spellink-bridge serve`。**再起動すると自動では起動しない**）、cloudflared（システムのサービス、起動時に自動）。ブリッジを止める・入れ替える作業は、ローカルのエージェントかユーザーが行う。
