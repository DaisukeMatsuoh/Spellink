# 進捗（引き継ぎ用の入口）

新しいセッションのエージェントは、**このファイルだけ読めば再開できる**ように書いてある。詳しい経緯が要るときだけ、下の「参照先」をたどる。

最終更新: 2026-11-03

> **2026-11-03: 開発機のブリッジとクイックトンネルは停止済み**（ユーザーが遠隔の Claude Code に切り替えるため）。再開時は下の「手順」で起動し直す。

## このファイルの管理ルール
- ここには「**いまの状態・次にやること・未決事項・手順**」だけを書く。**経緯や実機での試行錯誤は `log/` に追記**する（追記専用。過去の記述は直さず、誤りは新しい項目で訂正する）。
- 区切りごとに: ①「現在の状態」と「次にやること」を書き換える ②`log/<年-月>.md` の末尾に日付つきで経緯を足す。
- 決めたことは ADR（`docs/decisions/`）。調査・比較は `docs/notes/research/`。ユーザー向けの手順は `docs/guides/`。ここに重複して書かず、リンクで示す。
- 完了した項目は「現在の状態」へ移し、「次にやること」から消す。1 項目は 1〜3 行に収める。

## ゴール
Supernote のノートで、手書きをラッソで囲む → Claude に質問 → 回答が**手書きの筆跡として**ページに浮かび上がる（「魔法の紙」）。
ユーザーはコードを書かない。実装はエージェントが行い、説明は日本語で行う。

## 前提（変わらないもの）
- 端末: Nomad（Chauvet 3.31.45）。Manta にも対応する設計。
- Claude は **API キーを使わず**、ユーザーのサブスクリプションで、**公式の Claude Code CLI をそのまま**使う（ADR 0005）。プラグイン内にログインを持たない。
- 構成: プラグイン（React Native）⇄ wss（Cloudflare トンネル／LAN）⇄ ブリッジ（Go）⇄ `claude -p`。通信は Noise で端末とブリッジの間を暗号化（ADR 0007）。
- push はユーザーの指示があるまでしない（コミットは可）。特許・知財の分析は `docs/notes/ip/private/`（git 管理外）にだけ書く。
- 識別は確実な ID で行い、**場当たり的な再試行やヒューリスティックで帳尻を合わせない**（ユーザー方針）。

## 現在の状態（2026-11-03）
動いている（実機確認済み）: ラッソ → 端末内 OCR → ブリッジ → Claude → 筆跡の書き戻し、複数ページ、余白なしで新ページ、既存の字を避ける、待機中の波線アニメ（既定オフ）、設定画面（字間・傾き・進捗表示）、ネイティブの小さなエラーダイアログ。
詳細は `log/2026-11.md`（実機）と `log/2026-10.md`（初期）。

- コード: `bridge/`（Go）、`plugin/`（RN）。テスト: ブリッジ `go test -race ./...`、プラグイン jest 75 件・9 スイート。**合格数だけでなくスイート数も確認する**（スイートが落ちると件数が減る）。
- コミットは `stg` ブランチ。最新の区切りは `0a9f51f`。未 push。CI（`.github/workflows/`）は一度も動かしていない。
- ADR: 一覧と状態の根拠は `docs/decisions/README.md`（2026-11-07 に整理。Accepted: 0001〜0003・0005〜0008・0012・0014。Proposed: 0009〜0011・0013・0015）。

## 次にやること（優先順）
00000. **利用者向けの整理**: `docs/guides/bridge-and-relay-for-users.md`（利用者が知ること・やること、運営側の作業、未決一覧）。ユーザーが整理する。
00000. **AI プロバイダの差し替え設計（ADR 0015）**: 呼称を「AIに聞く」/ "Ask AI" に、問い合わせ先と認証をプロバイダとしてモジュール化。ユーザーが表を確認し終えてから実装する。
0000. **状態遷移表の確認**（表から出た穴の修正は 2026-11-07 に実装済み。残りは、表のコード化〔表の確認後〕、テストの補完、保留項目）（2026-11-07 に、ユーザーの決定を表へ反映済み。決定の一覧は `docs/spec/README.md` の「決定の記録」）（`docs/spec/`、ADR 0014）。表は初版。ユーザーの確認が要る項目（❓）: 二重押下の表示、無音の時間切れの値（端末 120/60 秒案、ブリッジ 180 秒案）、接続成立後・`ask` 送信前の切断の仕様、回答中のページ数の変化の扱い、波線の消し残しの通知、実行中の履歴削除の扱い、ホストのライフサイクルの意味。
   確認が要らない穴（先に直す）: ブリッジ→プラグインのフレームの検証（`line.page` 範囲・`strokes` の形）、Claude への問い合わせの時間切れ、`insertPageAfter` の `ensureOpen`、SDK 戻り値の型検証、権限・パネル呼び出しを共通処理へ、設定値の範囲、診断ファイルの総量、未点検の境界（relay、lantls、glyphs.bin、bridges.json）。
000. **SDK 呼び出しの点検（ADR 0014）の実機確認**: 保存・ページ挿入の `result` と、ページ数での確認、依頼の直列化。ページ追加が `getNoteTotalPageNum` の戻り値の違いで止まらないか見る（止まったらメッセージに `page count` / `unexpected answer`）。`reports.jsonl` の trace に、ブール値の `result` が記録される。
00. **`closed (1006)` の再開機能（ADR 0013）を実機で確認**（ユーザー）。新しい `Spellink.snplg` の入れ直しが必要。ページをまたぐ長い回答を何度か。起きたら `/tmp/spellink-bridge.log`（`write ... failed` / `disconnected` / `resume`）と `reports.jsonl` の trace（`*.begin`＝重い呼び出しの開始）を読む。原因（ページ追加でホストが止まる？）は未解明。
0. **会話の引き継ぎ（ADR 0012）を実機で確認**（ユーザー）。同じノートで「1+1は？」→「それを3倍して」と続けて書き、前を踏まえて答えるか。実装済み: ブリッジ（`internal/history`、`ask.note`、`clearHistory`、`config.history`）とプラグイン（`note` 送信、設定画面の「会話を忘れさせる」）。**新しい `Spellink.snplg` の入れ直しが必要**。未実装: 圧縮（`history.compaction: "summarize"`）、ノート単位の削除。
1. **名前を決める**（ユーザー判断待ち。製品名は未決定で、現行は Spellink）。ドメインは `spellink.uk`（Cloudflare Registrar で取得。2026-11-03。先に取った `spell-ink.com` は使わない）。`spellink.com` が取得済みのため再検討中。候補とドメイン空き状況: `docs/notes/research/2026-11-03-naming-round2.md`。エージェントの推奨は `dancink`。決まったら改名を一括で行う（範囲は同ノート末尾。`pluginID` は変更禁止）。決定前に改名作業を始めない。
2. **名前付きトンネル: 疎通済み**（2026-11-07）。`https://bridge.spellink.uk/health` が通る。Cloudflare 側のルートの Service は **HTTP**（HTTPS だと 502）、URL は `localhost:8765`。cloudflared は公式の `service install` 方式（`/etc/cloudflared/token`）で動いている。
   `extraUrls` は `wss://bridge.spellink.uk/v1` に更新、`pair` 作り直し、プラグイン再ビルド済み（`plugin/build/outputs/Spellink.snplg`）。クイックトンネルは停止済み。
   - **残り**: ①端末へ新しい `Spellink.snplg` を入れ、トンネル経由で質問が通るか確認（ユーザー）②ブリッジの自動起動（systemd ユーザーサービス）。後者は自動モードの権限判定で拒否されたため、**ユーザーが自分のターミナルで実行する**: `mkdir -p ~/.config/systemd/user && cp bridge/contrib/spellink-bridge.service ~/.config/systemd/user/ && systemctl --user daemon-reload && systemctl --user enable --now spellink-bridge && loginctl enable-linger $USER`（実行前に、手で起動中のブリッジを `pkill -f 'spellink-bridge serve'` で止める）。
3. **NOTE アプリが回答中に固まる件の再検証**（ユーザーの再現待ち）。原因未特定。フライトレコーダーで「SDK 呼び出しが返らないまま止まる」ことまで分かっている。
   - ユーザーに頼むこと: 待機アニメーション OFF のまま、長い回答中にページを切り替えて再現するか。診断「10. Cache number」と「Send log to bridge」の実行。
   - エージェントがやること: `~/.config/spellink-bridge/reports.jsonl` の trace を読む。診断 10 でキャッシュから番号が読めるなら、波線の後始末から「保存＋ページ全体の読み取り」を除く。
   - 注意: 「ページが消えた」は誤認だった（NOTE の描画が止まっていただけ）。
4. LAN 直結 wss（ADR 0011）の実証明書での確認。必要: ドメイン＋Cloudflare API トークン（Zone → DNS → 編集、対象ドメインのみ）。
5. 文字以外（楽譜など）の入力。方針: **画像を外部に送らず**、ストロークデータから意味を取る（`docs/notes/research/2026-11-02-non-text-input.md` の 2・3）。未実装。
6. 配布準備: README・CONTRIBUTING・SECURITY.md、CI の実行確認（push 後）、macOS の公証、Windows の動作確認。弁理士への相談は当面しない。

## 未解決の判断（ユーザー）
- 名前（上記 1）。ドメインの方針（丸ごと移す／専用を別に取る）。
- 公開の時期と形（OSS・寄付か SaaS か）。push の許可。

## 手順（再開時）
```bash
# ツールチェーン
export PATH=$HOME/.local/go/bin:$PATH
# ブリッジ: テスト → ビルド → 入れ替え → 再起動
cd ~/GitWorkspaces/Cloute/bridge && go vet ./... && go test -race ./...
go build -o /tmp/spellink-bridge-dev . && pkill -f '^/home/daisuke/.local/bin/spellink-bridge serve'; sleep 0.5
install -m 755 /tmp/spellink-bridge-dev ~/.local/bin/spellink-bridge
(cd /tmp && nohup ~/.local/bin/spellink-bridge serve > /tmp/spellink-bridge.log 2>&1 &)
# プラグイン: テスト → ビルド（成果物 plugin/build/outputs/Spellink.snplg）
cd ../plugin && npx jest && npm run build
# 端末からの診断ログを読む
tail -n 5 ~/.config/spellink-bridge/reports.jsonl
```
- 実行中（開発機 ROCK 5B+、aarch64）: ブリッジ `~/.local/bin/spellink-bridge serve`（ポート 8765、ログ `/tmp/spellink-bridge.log`）。クイックトンネル `/tmp/cfd/cloudflared tunnel --url http://127.0.0.1:8765`（URL は `/tmp/cfd/tunnel.log`。数十秒おきに切れる。検証専用）。再起動後は手動で立ち上げ直す。
- 設定: `~/.config/spellink-bridge/config.json`（`model: haiku`、`extraUrls`、`tiltDegrees` など）。文字データ `glyphs.bin`（KanjiVG から生成。再配布しない）、診断 `reports.jsonl`。JSON のカンマ抜けでブリッジが起動しなくなる（エラーにファイル名が出る）。
- 端末へのインストール: `Spellink.snplg` を端末に送って入れる。**OS 更新前に作ったノートではボタンが出ない。新しいノートで試す**。接続先はビルド時に埋め込む（git 管理外の `plugin/src/bridges.local.json`）。
- 実機で気を付けること: 平文の ws/http は PluginHost が禁止（wss＋公的 CA が必須）。`.local` は引けない。`INTERNET` 権限の許可が要る。
- SDK の落とし穴（コードに反映済み。詳細は `log/2026-11.md`）: ファイル系 API（`insertNotePage`・`getElements` など）の前に `saveCurrentNote`。コメント系 API は「いま開いているファイル」に作用するので、書く前に開いているノートを確認する。`insertPageElements` はキャッシュにない要素を黙ってスキップする。

## 参照先
| 知りたいこと | 場所 |
|---|---|
| 経緯（実機の結果・バグと修正） | `log/2026-11.md`、`log/2026-10.md` |
| 設計判断 | `docs/decisions/README.md`（0001〜0011） |
| 調査・比較（競合、SDK、名前、中継、非テキスト入力） | `docs/notes/research/` |
| 実機テストの手順 | `docs/notes/device-test-guide.md` |
| Cloudflare トンネルの手順 | `docs/guides/cloudflare-tunnel-setup.md` |
| 知財のルール | `docs/notes/ip/README.md`（分析本体は private/ で git 管理外） |
| プロジェクトの規約 | `CLAUDE.md` |
