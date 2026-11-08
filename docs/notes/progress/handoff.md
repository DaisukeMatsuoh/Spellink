# 別のエージェントへの引き継ぎ（遠隔での作業用）

2026-11-08。ユーザーが外出し、**この開発機の Claude Code は、遠隔操作（Remote Control）では使えない**ため、一部の作業を別のエージェントが行う。先に `CLAUDE.md`（規約）と `README.md`（現状）を読み、経緯が要るときだけ `history.md` を読む。

## 守ること（要約。詳しくは `CLAUDE.md`、ADR 0014）
1. **push しない。** コミットは可。コミットの末尾に `Co-Authored-By: <使っているモデル> <noreply@anthropic.com>` を付ける。
2. **表が先。** 状態を持つ処理は、`docs/spec/` の表を直す → `cd plugin && npm run spec` → ハンドラを書く → テスト。生成された `*.generated.ts` / `runtable_gen.go` は手で直さない。
3. 戻り値・境界の値は必ず検証する。場当たり的な再試行や、回数・時間での帳尻合わせをしない。
4. **テストは、件数だけでなくスイート数を確認**する（端末 15 スイート、Go は 10 パッケージ）。本番の入口（ラッソなら `AskFlow.run()`）から通すテストを書く。壊して落ちることも確かめる。
5. 作業の区切りで、`README.md`（現状・次にやること）と `log/2026-11.md`（日付つきの追記）を更新する。判断を伴うことは ADR。
6. ドキュメントは日本語。ユーザーへの説明は、短く、日本語。
7. 特許・知財の分析は `docs/notes/ip/private/`（git 管理外）にだけ書く。

## 確認コマンド
```bash
export PATH=$HOME/.local/go/bin:$PATH
cd ~/GitWorkspaces/Cloute/bridge  && gofmt -l . ; go vet ./... && go test -race -count=1 ./...   # 全パッケージ ok
cd ~/GitWorkspaces/Cloute/plugin  && npx tsc --noEmit && npx jest                                  # 15 スイート・約 450 件
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
優先度の高い順:
1. **CI に、表の生成物が古くないことの確認を足す**: `.github/workflows/plugin.yml` で `npm run spec` のあと `git diff --exit-code`、`bridge.yml` でも同様（Go の生成物は `plugin/` の `npm run spec` が作る）。まだ push していないので CI は動いていない。
2. **表の「テストなし」（⬜）を埋める**: `docs/spec/*.md` の処理の表で「テスト」が ⬜ のもの（run の掃除の保持期限は済み。多重 ask、重複 ID の細部、保持件数の上限、など）。
3. **ADR 0015 の実装**: 呼称を「AIに聞く」/ "Ask AI" に（`plugin/src/i18n.ts`、README）。ブリッジに `internal/provider` の境界を作り、`claude` パッケージをその実装にする。`config.json` に `provider`。プロトコルは変えない。先に `docs/spec/bridge-run.md` の B10 / B16 の呼び名との整合を見る。
4. **会話の記憶の拡張**（ADR 0012 の予約分）: ノート単位の削除、圧縮（`history.compaction: "summarize"`）。
5. **配布の準備の文書**: README、CONTRIBUTING、SECURITY.md、ライセンスの整理（KanjiVG は CC BY-SA で、同梱せず利用者のパソコンで生成）。`docs/guides/bridge-and-relay-for-users.md` を元に。
6. **lantls の点検**（`docs/spec/interfaces.md` の I21）、リレーの接続元ごとの回数制限（方針の決定と一緒に）。
7. 文字以外の入力（楽譜など）の調査の続き（`docs/notes/research/2026-11-02-non-text-input.md`。画像は外部に送らない方針）。

### 先にユーザーに聞くこと（勝手に決めない）
- ページ数の変化の検出（回答の最中に、利用者がページを挿入・削除したとき）。仕様の確認待ち（`docs/spec/page-turn.md`）。
- 履歴の削除と、実行中の回答の関係。ホストのライフサイクルの意味（実機で調べる）。
- 波線の消し残しを、利用者に知らせる方法。

## 遠隔のエージェントを動かすときの注意
- **この機械の `~/.claude/settings.json` に、caveman のプロキシ（`ANTHROPIC_BASE_URL`）があり、Remote Control が使えない。** 使うなら、別の設定ディレクトリで起動する（ログイン情報だけコピーし、hooks と `env` を持たない）:
  ```bash
  mkdir -p ~/.claude-rc && cp ~/.claude/.credentials.json ~/.claude-rc/
  cd ~/GitWorkspaces/Cloute && CLAUDE_CONFIG_DIR=~/.claude-rc claude remote-control
  ```
  シェルに `ANTHROPIC_API_KEY` があると優先されるので、`env -u ANTHROPIC_API_KEY` を付ける。ユーザーが「家にいるときはやりにくい」と判断済み。
- **コミットは push していない**（`stg` ブランチ）。別の機械のエージェントは、このリポジトリのコミットを見られない。同じ機械で動かすか、ユーザーが push を許可するか（または `git bundle` で渡す）。
- 作業ツリーに、ユーザーの未コミットの変更が 1 つある（`.gitignore` に `docs/References/SPI_Monitor.zip`）。触らない。
- 実行中のもの: ブリッジ（`~/.local/bin/spellink-bridge serve`。**再起動すると自動では起動しない**）、cloudflared（システムのサービス、起動時に自動）。ブリッジを止める・入れ替える作業は、ローカルのエージェントかユーザーが行う。
