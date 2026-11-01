# 進捗（引き継ぎ用）

新しいセッションのエージェントは、まずこのファイル → `docs/decisions/README.md` → 最新の調査ノートの順に読むこと。

## ゴール
Supernote のノートで、手書きをラッソで囲む → Claude に質問 → 回答が**手書きの筆跡として**ページに浮かび上がる（「魔法の紙」）。
ユーザー自身はコードを書かない。実装はエージェントが行う。

## 確定した前提（2026-10-30 ユーザー回答）
- 端末: **Nomad（A6 X2）、Chauvet 3.31.45**。最低限 **Nomad と Manta** に対応する（画面: Nomad 1404×1872、Manta 1920×2560。座標はページ実寸基準なので解像度差は吸収できる見込み）。
- Claude の利用: **API キー（従量課金）は使わない**。手持ちのサブスクリプションで使いたい → 規約上、プラグインにログインを持たせることはできないので、
  **自宅機で公式 Claude Code を動かすブリッジ方式**（ADR 0005）。開発機 ROCK 5B+ に Claude Code 2.1.314 が入っている。
- 言語: 日本語中心、英語も同等。将来 中国語・スペイン語・韓国語など（ADR 0006）。

## 現在の状態（2026-10-31 深夜）
- フェーズ: **スパイク実装完了 / 実機検証待ち**。ブリッジは Go に移植済み。
- 決定: ADR 0001・0005・0008 は Accepted。0002 / 0003 / 0006 / 0007 / 0009 は Proposed（実機結果で確定）。

### できているもの
- `bridge/`（**Go**。依存は `github.com/coder/websocket` のみ。ADR 0008）
  - `serve`: WebSocket（`/v1`）+ `/health`。トークン認証 → `ask` → `claude -p --output-format stream-json`（ツール無効・空ディレクトリ・設定読み込みなし）→ 行ごとに筆跡ストロークへ変換して `line` で送る。
    空き字数を見積もってシステムプロンプトで回答の長さを指示し、溢れたら次ページへ（`line.page`、最大 3 ページ。ADR 0009）。
  - `pair`: プラグイン用の接続ファイル。`build-glyphs`: 文字データ再生成。`preview TEXT out.png [size] [pages]`: 筆跡を PNG で確認。
  - 設定・文字データ・診断レポートは `os.UserConfigDir()/spellink-bridge/`（Linux: `~/.config/spellink-bridge/`）。
  - テスト: `go test -race ./...` 緑（レイアウト・SVG パス・サーバ）。実 Claude Code での E2E（単ページ・複数ページ）確認済み。
  - メモリ約 17〜19MB（Node 版 129MB）、バイナリ約 7MB、文字データ 2.1MB。
  - クロスビルド: `scripts/build.sh`（darwin/linux/windows × arm64/amd64、この ARM 機から全部ビルド可）。CI: `.github/workflows/bridge.yml`（未実行。push していないため）。
  - 常駐化の雛形: `contrib/`（systemd / launchd）。
- `plugin/`（公式テンプレート + sn-plugin-lib 0.2.3、JS のみ、この ARM 機で `npm run build` → `build/outputs/Spellink.snplg`）
  - ラッソツールバー「Claudeに聞く」（ポップアップ）: 権限 → 端末内 OCR → ラッソ解除 → ブリッジに並列接続 → 行ごとに `insertPageElements`。
    `line.page` が増えたら直後にページを挿入（`insertNotePage`、元ページと同じテンプレート）→ `jumpToPage` → 書き込み。
  - サイドバー「Spellink 診断」: 環境・権限・ネットワーク・合成ストローク速度・テキストボックス・OCR 抜き質問・**新ページ挿入**・ログ送信。
  - ブリッジ接続先は `MyStyle/Spellink/bridges.json`（file:// 読み込みは未確認）→ だめならビルド時埋め込みの `src/bridges.local.json`（git 管理外）。

## 次のステップ
1. **ユーザーが実機で診断を実施**（手順: `docs/notes/device-test-guide.md`）→ `~/.config/spellink-bridge/reports.jsonl` を読む。確認したいこと:
   - 平文 ws:// / http:// が通るか（ADR 0007 の分岐）
   - `probeFillMode` が bulk か single か、200 ストロークの所要時間（遅ければ Java ネイティブモジュール化 → x86 ビルド環境が必要）
   - 挿入した線がすぐ画面に出るか（`reloadFile` 要否）、ペン太さの単位（`thickness` / `penWidth`）
   - **新ページ挿入と `jumpToPage`、表示中でないページへの `insertPageElements` が動くか**（ADR 0009）
   - `fetch('file://…')` で MyStyle の設定ファイルを読めるか
   - 日本語 OCR の精度
2. 結果を受けて: ADR 0002/0003/0007/0009 を確定、暗号化（Noise）、ペアリング UX。
3. 中継の本番運用: ユーザーが Cloudflare（ドメイン・トンネル）を用意 → エージェントが systemd ユニットと手順を整える。
   Noise・中継は実装済み（端末での動作は未確認）。ペアリング情報の端末側保存は未解決（現状はビルド時埋め込み）。
4. 知財: 弁理士への相談は当面しない方針。情報はローカルの `docs/notes/ip/private/`、ルールは `docs/notes/ip/README.md`。
5. 配布準備: CI でリリース、macOS の公証、CONTRIBUTING・SECURITY.md。

## 既知の制約・メモ
- スパイク版の通信は平文（信頼できる LAN 限定）。
- 1 行のストロークは JSON で 20〜40KB（長い回答で数百 KB）。LAN では問題ないが、必要なら差分符号化や WebSocket 圧縮を検討。
- 開発機ではブリッジを `~/.local/bin/spellink-bridge serve` でバックグラウンド起動したまま（ポート 8765、ログ `/tmp/spellink-bridge.log`）。再起動後は手動で起動するか `contrib/spellink-bridge.service` を有効化する。
- Go ツールチェーンは `~/.local/go`（PATH に入っていないので `export PATH=$HOME/.local/go/bin:$PATH`）。
- `plugin/PluginConfig.json` の `pluginID` は変更禁止。生成物 `plugin/build/`・`bridge/dist/` は git 管理外。

## 未解決の質問・ユーザーの方針
- 外出先: 利用者には何も求めない。**Spellink の中継サーバ（ADR 0010）を実装済み**。Go で運用する方針（ユーザー決定）。ホスティングはフェーズ 1 = この機械＋Cloudflare Tunnel を推奨（`docs/notes/research/2026-11-01-relay-hosting.md`）。ユーザーの Cloudflare 設定（ドメイン、`cloudflared tunnel login`）待ち。
- 名前: **Spellink に決定**（2026-10-31）。リポジトリ内は置換済み。GitHub リポジトリ名と Go モジュールパス（`github.com/DaisukeMatsuoh/Spellink/bridge`）はユーザーが GitHub で改名した後に変更。開発機の設定は `~/.config/spellink-bridge/`、バイナリは `~/.local/bin/spellink-bridge`。
- 弁理士: すぐには相談しない（情報として保持）。
- push: ユーザーの指示があるまでしない（まだ公開しない）。

## 作業ログ
- 2026-10-30: 公式ドキュメント全 154 ページ、sn-plugin-lib 0.2.3 / テンプレート 1.0.13 のソース、先行プラグイン（sn-copilot, sn-dictionary, supernote-plugin-research）を調査。docs 一式を作成。
- 2026-11-01: Noise による端末⇔ブリッジ間の暗号化（Go＋TS、相互接続確認）、中継サーバと中継経由接続を実装（実 Claude Code で確認）。ホスティング調査。
- 2026-10-31: 名前を Spellink に決定・置換。外出先接続の方針を中継サーバ（ADR 0010）に。
- 2026-10-31: 名前の候補出し、外出先アクセスの整理、ブリッジ設定 `extraUrls` を追加。
- 2026-10-31: ブリッジを Go に移植（ADR 0008）。長い回答の字数指示と次ページへの続き書き（ADR 0009）。特許調査（ローカルのみ）、名前と大きな紙の検討ノート。
- 2026-10-31: スパイク実装（bridge + plugin + 診断画面）。ARM 機でのプラグインビルド、実 Claude Code での E2E、接続レース処理を確認。ADR 0003/0007 に追記。
- 2026-10-30: ブリッジを Mac/Linux/Windows 対応・複数台のペアリング制に拡張（ADR 0005 追記）。通信は WebSocket + Noise に（ADR 0007）。Codex 等への差し替え余地を残す。
- 2026-10-30: ユーザー回答（Nomad / Chauvet 3.31.45、サブスク利用希望、ja/en）を反映。Anthropic の認証規約を確認し ADR 0004 → 0005（ブリッジ方式）へ変更。ADR 0006（l10n）と競合調査ノートを追加。
