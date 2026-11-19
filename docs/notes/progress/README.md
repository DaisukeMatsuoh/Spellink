# 進捗（引き継ぎ用の入口）

新しいセッションのエージェントは、**まずこのファイル**を読む。次に `CLAUDE.md`（規約）。リリースまでの全体は `release-plan.md`。経緯が要るときだけ `history.md`。別のエージェントに作業を渡すときは `handoff.md`。

最終更新: 2026-11-08

## このファイルの管理ルール
- ここには「**いまの状態・次にやること・未決事項・手順**」だけを書く。経緯や実機での試行錯誤は `log/<年-月>.md` に**追記**する（追記専用。過去の記述は直さず、誤りは新しい項目で訂正）。`history.md` は、まとめ直した読み物。
- 決めたことは ADR（`docs/decisions/`）。仕様（状態遷移表・インターフェース台帳）は `docs/spec/`。調査・比較は `docs/notes/research/`。ユーザー向けの手順は `docs/guides/`。重複して書かず、リンクで示す。
- 区切りごとに: ①この文書の「現在の状態」と「次にやること」を書き換える ②`log/` に日付つきで経緯を足す。1 項目は 1〜3 行。

## ゴール
Supernote のノートで、手書きをラッソで囲む → AI（現状は Claude）に質問 → 回答が**手書きの筆跡として**ページに浮かび上がる。ユーザーはコードを書かない。実装はエージェントが行い、説明は日本語で行う。

## 前提（変わらないもの）
- 端末: Nomad（Chauvet 3.31.45）。Manta にも対応する設計。
- AI は **API キーを使わず**、ユーザーのサブスクリプションで、**公式の Claude Code CLI をそのまま**使う（ADR 0005）。プラグイン内にログインを持たない。
- 構成: プラグイン（RN）⇄ wss（Cloudflare トンネル／リレー／LAN）⇄ ブリッジ（Go）⇄ `claude -p`。端末とブリッジの間は Noise で暗号化（ADR 0007）。
- **push はユーザーの指示があるまでしない**（コミットは可）。特許・知財の分析は `docs/notes/ip/private/`（git 管理外）にだけ書く。
- **開発ポリシー（ADR 0014）**: 戻り値は必ず確認／全ケースを洗い出す／状態遷移表を先に書く（表がコードの元データ）／信頼境界の値は必ず検証／場当たり的な再試行をしない。

## 現在の状態（2026-11-08）
- **コード**: `stg` ブランチ、未 push。テスト: Go 10 パッケージ全緑（`-race`）、端末 18 スイート・約 466 件。**件数だけでなくスイート数を確認する**。CI（`.github/workflows/`）は一度も動かしていない。
- **表がコード**（`docs/spec/README.md`）: 端末の質問の流れ（`ask-flow.md`）、波線（`wave.md`）、ページ追加と書き込み（`page-turn.md`）、ブリッジの run（`bridge-run.md`）。文書を直す → `cd plugin && npm run spec` → ハンドラ → テスト。
- **実行中（開発機）**: ブリッジ `~/.local/bin/spellink-bridge serve`（ポート 8765、ログ `/tmp/spellink-bridge.log`。**nohup。再起動後は自動で起動しない**）。cloudflared はシステムのサービス（`/etc/cloudflared/token`、起動時に自動）。`https://bridge.spellink.uk/health` と、`spellink-bridge check`（実際の暗号化の握手まで）が通る。
- **配布物**: `plugin/build/outputs/Spellink.snplg`（接続先 `wss://bridge.spellink.uk/v1` を埋め込み済み。`plugin/src/bridges.local.json`、git 管理外）。
- **設定の既定**: 回答中の通知 on、既存の字を避ける（ページ全体の読み取り）off、待機中の波線 off。AI の無応答は 10 秒（始まらない・止まった）、全体 30 秒。
- **実機で確認済み**（〜11-07）: ラッソ → 認識 → 回答、複数ページ、会話の記憶（ADR 0012）、wss＋Noise、設定画面、ネイティブのエラーダイアログ。
- **実機で未確認**: 切断後の再開（ADR 0013）、再接続 3 回、フレームの検証、`busyNotice`、`avoidOverwriting`、表で動く新しい流れ全体、空き領域の確認の実測。
- ADR の一覧と根拠: `docs/decisions/README.md`（Accepted: 0001〜0003・0005〜0008・0012・0014）。

## 実機で見てほしいこと（ユーザー）
新しい `Spellink.snplg` を端末に入れ直し、**新しいノート**で。
1. 普通の質問が、最後まで書かれるか。ページをまたぐ長い回答。
2. トンネルを一時的に止める／ブリッジを止めて、「再接続中 (n/3)」が出て、戻れば続きが書かれるか。3 回で諦めたときのエラー。
3. 回答中にもう一度ボタンを押して、通知が出るか（設定で切れるか）。
4. 回答中に別のノートを開いて、別のノートに書かれないか。
5. 設定「既存の字を避ける」を on にして、下に字があるとき、重ならないか、どのくらい遅いか。
6. ブリッジを止める等で、10 秒で失敗が出るか。
7. NOTE が固まる・ページが空に見える現象が、まだ起きるか。起きたら診断の「Send log to bridge」。

## 次にやること（優先順）
担当の分け方と、遠隔でできる作業の一覧は `handoff.md`。
1. **実機の結果を見て、表の気になったセルだけを直す**（ユーザー）。
2. **ブリッジの自動起動**（ユーザー）: `~/.local/bin/spellink-bridge service install`（`--dry-run` で、先に何をするか見られる）。`sudo` は要らない。systemd のユーザーサービスを作り、`loginctl enable-linger` まで行う。エージェントは、権限判定で、常駐の登録を実行できない。
3. **公開用の文書を読んで直す**（ユーザー）: `README.md`、`CONTRIBUTING.md`、`SECURITY.md`（非公開の報告の連絡手段）、`THIRD_PARTY_NOTICES.md`。`LICENSE` は Apache-2.0（初期コミットから。仮置き）で、著作権者の表示はユーザーが決める。
4. **CI とリリースを動かす**（push の許可が要る）: `.github/workflows/`（`plugin.yml`、`bridge.yml`、`release.yml`）は、一度も動かしていない。
5. 会話の圧縮、他社の AI の追加（規約の確認が先）、多言語（遠隔可）。
6. **ペアリング**（公開版の最大のブロッカー）。方針は **ADR 0016（Proposed）**: ブリッジが端末ごとの 2 段の秘密（B: 端末専用の `.snplg` に入る初期の秘密、K: ペアリングで渡る本番の秘密）を発行し、台帳で管理。**状態遷移表 `docs/spec/pairing.md` をユーザーが確認する → 表をコードの表にする → 実装**。K の保存先は確認済み（ディレクトリ名 `k-<gen>-<K>`、再起動をまたいで残る。削除はしない）。中継の運営の方針（ADR 0010）はユーザーが決める。経緯と選択肢: `docs/notes/research/2026-11-08-pairing.md`。
7. NOTE が回答中に固まる件の再検証（原因未特定。フライトレコーダーとブリッジのログで証拠を取る）。
8. LAN 直結 wss（ADR 0011）の実証明書での確認（Cloudflare の API トークンが要る）。優先度は低い。
9. 文字以外（楽譜など）の入力（方針: 画像を外部に送らず、ストロークから意味を取る）。

## 未決（ユーザーの判断）
- **名前**: 現行は Spellink。`spellink.com` は取得済みで買えず、`spellink.uk` を取得。dancink などの案は `docs/notes/research/2026-11-03-naming-round2.md`。決めるまで改名しない（`pluginID` は変更禁止）。
- 公開の時期と形（OSS・寄付か有料か）、push の許可。
- リレーを運営するか、どこで、費用はどうするか。ペアリング情報を端末に入れる方法。（`docs/guides/bridge-and-relay-for-users.md` に整理）
- 他社の AI（ChatGPT など）を足す順番と、各社の規約の確認（ADR 0015）。
- 保留: 履歴の削除と実行中の回答の関係、ホストのライフサイクルの意味、ページ数の変化の扱い、波線の消し残しの通知。

## 手順（再開時）
```bash
export PATH=$HOME/.local/go/bin:$PATH
# ブリッジ: テスト → ビルド → 入れ替え → 再起動 → 疎通
cd ~/GitWorkspaces/Cloute/bridge && gofmt -l . ; go vet ./... && go test -race -count=1 ./...
go build -o /tmp/spellink-bridge-dev . && pkill -f '^/home/daisuke/.local/bin/spellink-bridge serve'; sleep 1
install -m 755 /tmp/spellink-bridge-dev ~/.local/bin/spellink-bridge
(cd /tmp && nohup ~/.local/bin/spellink-bridge serve > /tmp/spellink-bridge.log 2>&1 &)
~/.local/bin/spellink-bridge check            # 端末と同じ道（LAN と、トンネル）で、暗号化の握手まで確かめる
# プラグイン: 表の生成 → 型 → テスト → ビルド（成果物 plugin/build/outputs/Spellink.snplg）
cd ../plugin && npm run spec && npx tsc --noEmit && npx jest && npm run build
# 端末からの診断ログ
tail -n 5 ~/.config/spellink-bridge/reports.jsonl
```
- 設定: `~/.config/spellink-bridge/config.json`（`model: haiku`、`extraUrls`、`timeouts`、`history`、`tiltDegrees` など）。文字データ `glyphs.bin`（KanjiVG から生成。再配布しない）、診断 `reports.jsonl`、会話の記憶 `history/`。JSON のカンマ抜けでブリッジが起動しなくなる（エラーにファイル名が出る）。
- 端末へのインストール: `Spellink.snplg` を端末に送って入れる。**OS 更新前に作ったノートではボタンが出ない。新しいノートで試す**。接続先はビルド時に埋め込む（`spellink-bridge pair plugin/src/bridges.local.json`）。
- 実機の制約: 平文の ws/http は PluginHost が禁止（wss＋公的 CA が必須）。`.local` は引けない。`INTERNET` 権限の許可が要る。
- SDK の落とし穴（コードに反映済み）: ファイル系 API の前に `saveCurrentNote`。コメント系 API は「いま開いているファイル」に作用するので、書く前に開いているノートを確認。`insertPageElements` はキャッシュにない要素を黙ってスキップ。`success` と `result` は別（`result: false` は拒否）。
- トンネル: 502 は Service が HTTPS になっている／`1033` はコネクター未接続（`systemctl status cloudflared`、`/etc/cloudflared/token` の有無）。

## 参照先
| 知りたいこと | 場所 |
|---|---|
| 経緯（まとめ） | `history.md` |
| 別のエージェントへの引き継ぎ | `handoff.md` |
| リリースまでの計画 | `release-plan.md` |
| 日付ごとの記録 | `log/2026-11.md`、`log/2026-10.md` |
| 設計判断 | `docs/decisions/README.md` |
| 仕様（状態遷移表・インターフェース台帳） | `docs/spec/README.md` |
| 利用者が知ること・運営の作業 | `docs/guides/bridge-and-relay-for-users.md` |
| Cloudflare トンネルの手順 | `docs/guides/cloudflare-tunnel-setup.md` |
| 調査・比較 | `docs/notes/research/` |
| 実機テストの手順 | `docs/notes/device-test-guide.md` |
| 知財のルール | `docs/notes/ip/README.md`（分析本体は private/ で git 管理外） |
| プロジェクトの規約 | `CLAUDE.md` |
