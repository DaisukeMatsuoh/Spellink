# 進捗（引き継ぎ用の入口）

新しいセッションのエージェントは、**まずこのファイル**を読む。次に `CLAUDE.md`（規約）。リリースまでの全体は `release-plan.md`。経緯が要るときだけ `history.md`。別のエージェントに作業を渡すときは `handoff.md`。

最終更新: 2026-11-28（ADR 0022 をブリッジに実装。それ以前: 2026-11-26 VPS 2 台が動き始めた。それ以前: 2026-11-24 表の状態名の改名、Bridge App の確認、名前の確定、寄付は Ko-fi。それ以前: 2026-11-22 レビューの修正、運営の形〔ADR 0020〕、計測〔ADR 0021〕、直結の週の枠、診断の追加。実機のペアリングは 2026-11-23 にユーザーが試す）

## このファイルの管理ルール
- ここには「**いまの状態・次にやること・未決事項・手順**」だけを書く。経緯や実機での試行錯誤は `log/<年-月>.md` に**追記**する（追記専用。過去の記述は直さず、誤りは新しい項目で訂正）。`history.md` は、まとめ直した読み物。
- 決めたことは ADR（`docs/decisions/`）。仕様（状態遷移表・インターフェース台帳）は `docs/spec/`。調査・比較は `docs/notes/research/`。ユーザー向けの手順は `docs/guides/`。重複して書かず、リンクで示す。
- 区切りごとに: ①この文書の「現在の状態」と「次にやること」を書き換える ②`log/` に日付つきで経緯を足す。1 項目は 1〜3 行。

## ゴール
Supernote のノートで、手書きをラッソで囲む → AI（現状は Claude）に質問 → 回答が**手書きの筆跡として**ページに浮かび上がる。ユーザーはコードを書かない。実装はエージェントが行い、説明は日本語で行う。

## 前提（変わらないもの）
- 端末: Nomad（Chauvet 3.31.45）。Manta にも対応する設計。
- AI は **API キーを使わず**、ユーザーのサブスクリプションで、**公式の Claude Code CLI をそのまま**使う（ADR 0005）。プラグイン内にログインを持たない。
- 構成: プラグイン（RN）⇄ wss（Cloudflare トンネル／リレー／LAN）⇄ ブリッジ（**Rust**。ADR 0018）⇄ `claude -p`。端末とブリッジの間は Noise で暗号化（ADR 0007）。
- **push はユーザーの指示があるまでしない**（コミットは可）。特許・知財の分析は `docs/notes/ip/private/`（git 管理外）にだけ書く。
- **開発ポリシー（ADR 0014）**: 戻り値は必ず確認／全ケースを洗い出す／状態遷移表を先に書く（表がコードの元データ）／信頼境界の値は必ず検証／場当たり的な再試行をしない。

## 現在の状態（2026-11-22）
- **コード**: `stg` ブランチ、**未 push**（push はユーザーの指示まで。コミットは可）。直近: `c383821` レビューの修正、`0842f1d` 診断、`f243244` ADR 0021 ほか、`f6cd426` 計測、`66a5ad4` 直結の週の枠、続けて文書の更新。テスト（2026-11-22 時点）: Rust は lib 275 と統合テスト全部緑、clippy `-D warnings` 緑、端末は **31 スイート・865 件**緑、ダッシュボードの Node テスト 5 件緑。**件数だけでなくスイート数を確認する。** `dropping_the_answer_kills_the_cli`（claudecode）は、負荷が高いと一度だけ落ちたことがある（単独では通る。もとからのタイミング依存）。CI（`.github/workflows/`）は一度も動かしていない（push が要る。`macos-15-intel` のラベルは未確認）。
- **構成**: プラグイン（RN）⇄ wss（自前のトンネル／運営の中継／家の直結）⇄ ブリッジ（Rust。ADR 0018）⇄ `claude -p`。ブリッジ・中継・名前サービスは 1 つのバイナリ。端末とブリッジの間は Noise（ADR 0007）。運営の入口は ADR 0019（Cloudflare Tunnel をやめ、自前の TLS と DNS、scratch）。
- **ペアリング（ADR 0016・0020）**: QR と 8 桁。`device add` が、端末ごとの期限つきの設置用 `.snplg` を作る。**プラグインのパッケージはバイナリに埋め込み**（`build.rs`、ビルド時の環境変数 `SPELLINK_BASE_PACKAGE`。作り方は `bridge/scripts/build-with-plugin.sh`）。**プラグインは、開発者のブリッジの住所もトークンも持たない**（ビルド時に埋める仕組みは廃止）。利用者向けの手順は `docs/guides/install-guide.md`。
- **運営の形（ADR 0020、決定 2026-11-22）**: 中継は運営する。費用は最初は寄付（Buy Me a Coffee・暗号通貨。**アカウント名と送金先は、ユーザーが用意する**。私は作らない）。対応 OS は Windows・macOS・Linux。VPS は**日本が Kagoya（借りた）、米国が Scaleway（これから借りる）**。中継は `jp.relay.spellink.uk`・`us.relay.spellink.uk`（ADR 0019 の追記）。ブリッジは `relayUrls` の**すべての中継に登録**し、端末が競争で近いほうを使う（ADR 0019 の追記。実装済み 2026-11-22）。
- **直結の週の枠（ADR 0017 の追記。決定 45 と 5）**: 名前サービスが、新しい ID の最初の注文を、直近 7 日に 45 件までにする。中継・名前サービスの ID は `--infra-ids`（5 件の予約）。断られたら、空く時刻まで中継で動く。`lan.spellink.uk` の PSL への登録は、利用者の実績ができてから。ドメインは `spellink.uk` の 1 つ（別ドメインにはしない）。仕様は `docs/spec/lan-cert.md` の L9。
- **計測（ADR 0021）**: 中継が時間と件数だけをヒストグラムで数え、`GET /v1/stats` で公開。`scripts/usage-dashboard.mjs` と `.github/workflows/usage.yml`（週 1 回）が `docs/usage/usage-dashboard.md` を作る（中継の一覧 `docs/usage/relays.json` は空）。**国の引きは未実装。**
- **中継経由の質問は通った（2026-11-27）**。ペアリングは、接続先を断片に切って保存する形に直して通った（ADR 0016 の追記）。**複数ページの回答で、新しいページの最初の行が消える不具合が、実機で確認されている**（原因は未確定。計測 `ask.verify` を入れた。ログの 2026-11-27）。
- **レビュー（2026-11-22）**: セキュリティとコードのレビューを実施、直せるものは直した（`docs/notes/review-2026-11-22.md`）。重大な指摘はなかった。残り: 週の枠の乱用への備え（上）、名前サービスの保存のロック、TLS の入口の本文の期限と接続元ごとの上限（可用性。広く公開する前に）。
- **gitleaks（2026-11-22）**: `~/.local/bin/gitleaks`（v8.30.1、チェックサム確認済み）を導入。全履歴（115 コミット、全ブランチ）で**検出 0 件**。push の直前に、もう一度実行する（`gitleaks git --redact -v .`）。トークンの照合は 2026-11-21 に済み。
- **診断（端末）**: 「3. Network」が各 URL の往復時間（初回と、2 回目以降の最小・中央値・最大）を出す。「12b. Network channels」は、WebRTC などのグローバルの有無を出す。「12. SDK names」と合わせて、実機で押す。
- **未確認（実環境）**: VPS、本物の Let's Encrypt（まず staging）、NS の委任、端末が家の中で `<id>.lan.spellink.uk:8766` につながること、端末での新しいペアリング、CI とリリース、中継の負荷（机上の見積りのみ）。確認済みなのは、docker でのイメージのビルドと実行（強化した設定）、CA 証明書つきで Let's Encrypt の staging に注文できること。
- **開発機の状態**: ブリッジ（`~/.local/bin/spellink-bridge`）は**レビューの修正より前の版**。入れ替えは `bridge/scripts/build-with-plugin.sh` でビルド → `install`。`config.json` の `pairing.pluginPackage` は、埋め込みがあれば不要。端末は、いまも Cloudflare のトンネル（`wss://bridge.spellink.uk/v1`）で届いている。**運営の中継が動くまで切らない**（切るときは、ユーザーが `sudo systemctl stop cloudflared`、DNS の `bridge.spellink.uk` を消す）。`~/Spellink-nomad-test.snplg` は古い版から作ったので、`device reissue` と `device package` で作り直す。
- **実機で確認済み**（〜11-07）: ラッソ → 認識 → 回答、複数ページ、会話の記憶（ADR 0012）、wss＋Noise、設定画面、ネイティブのエラーダイアログ。2026-11-19〜20: 書き換えた `.snplg` の入れ替え、ディレクトリ名の保存が再起動をまたぐ、QR が PC のカメラで読める。2026-11-22: 端末が Rust 版のブリッジとトンネルで通信できる（ユーザー報告）。
- **実機で未確認**: ペアリングの全体（`device-test-guide.md`・`install-guide.md`）、`/app`・`/pair` の操作、切断後の再開（ADR 0013）、再接続、`busyNotice`、`avoidOverwriting`、空き領域の確認の実測。
- **決めたこと**: 署名はしない（公式サイトに SHA-256。2026-11-20）。Anthropic の規約は「利用者自身のパソコンで、改変しない Claude Code を、自分のログインで、本人だけが使う」形（初回に同意を取る）。端末の言語は本体の 4 言語から。表の状態名・イベント名は読める名前の形にそろえた（2026-11-24。質問の流れ・波線・ページ追加。host-requests も）。名前は **Spellink で確定**（2026-11-24）。寄付の入口は当面 **Ko-fi のみ**（`.github/FUNDING.yml` の `ko_fi`）。Bridge App の寿命の表を確認済み（終了の待ち 10 秒、確認の間隔 15 秒、失敗でもトレイを残す）。
- ADR の一覧と根拠: `docs/decisions/README.md`（0001〜0021）。

## 実機で見てほしいこと（ユーザー）
**まず、ペアリング**（`device-test-guide.md` の 6）。以下は、それ以前の確認項目。
新しい `Spellink.snplg` を端末に入れ直し、**新しいノート**で。
1. 普通の質問が、最後まで書かれるか。ページをまたぐ長い回答。
2. トンネルを一時的に止める／ブリッジを止めて、「再接続中 (n/3)」が出て、戻れば続きが書かれるか。3 回で諦めたときのエラー。
3. 回答中にもう一度ボタンを押して、通知が出るか（設定で切れるか）。
4. 回答中に別のノートを開いて、別のノートに書かれないか。
5. 設定「既存の字を避ける」を on にして、下に字があるとき、重ならないか、どのくらい遅いか。
6. ブリッジを止める等で、10 秒で失敗が出るか。
7. NOTE が固まる・ページが空に見える現象が、まだ起きるか。起きたら診断の「Send log to bridge」。

## 次にやること（優先順）
0. **[U→A] 公開の手順**: `docs/guides/github-operations.md`。**2026-11-23 に済み**: 初回の push（`main`・`stg`）、CI 緑（bridge: fmt・clippy・テスト・audit・deny、plugin: 型・lint・851 件・パッケージ）、イメージ `ghcr.io/daisukematsuoh/spellink-edge:0.1.0`・`latest`（タグ `edge-v0.1.0`、認証なしで取れる）、セキュリティ設定（全部オン）。Actions の許可は「GitHub のもの＋`docker/*`・`Swatinem/rust-cache`・`dtolnay/rust-toolchain`」だけ。残り: `main` の保護（必須チェックに bridge・plugin の test）、Dependabot の PR 10 本の仕分け（React Native の大きな更新は SDK に合わせる）、`release.yml` は未実行。push は、これからも指示があるときだけ。Dependabot は有効にしてよい（`.github/dependabot.yml` を用意した）。診断は、利用者向け（確認・送る）と、五回タップで開く開発者向けに分けた。実機の調査が済んだら、開発者向けの画面の実験は消す。
**[A] AI への環境の情報と許可リスト（ADR 0022・`docs/spec/ai-tools.md`）— ブリッジに実装済み（2026-11-28）**: 設定 `ai.environment`・`ai.tools.allow`（検証、引数の組み立て、日時・OS のシステムプロンプト、プロセスグループごとの停止、道具の記録はログのみ）。Rust 292＋統合テスト緑、clippy 緑。実際の `claude` で確認済み。****`ai.instructions`（常設の指示）と、`/app` の「AI の設定」画面も入った（2026-11-28。画面はブラウザで確認済み）。** 残り: [U] 端末から「今は?」と聞いて答えるか（`config.json` に `"ai":{"tools":{"allow":["Bash(date)"]}}` を書いて再起動）。履歴への道具の記録は見送り。**
**[U]** = ユーザー、**[A]** = エージェント。担当の分け方は `handoff.md`（古い部分がある。この文書が優先）。
1. **[U] 実機でペアリングを試す（2026-11-23 の予定）。** 新しいバイナリを `bridge/scripts/build-with-plugin.sh` で作って入れ、`device reissue nomad-test`・`device package nomad-test`、手順は `docs/guides/install-guide.md`。診断の「3. Network」「12. SDK names」「12b. Network channels」の結果を共有。結果を見て、表の気になったセルを直す。
2. **[U→A] VPS（日本 Kagoya・米国 Scaleway Chicago）— 2026-11-26 に両方が動いた。** 外からの確認（[A]）: `https://jp.relay.spellink.uk`・`https://us.relay.spellink.uk` の `/health` が `{"ok":true,"bridges":0}`、`/v1/stats` が `{"v":1,…}`、証明書は Let's Encrypt の**本番**（staging ではない）、`https://names.spellink.uk:8443/health` が `{"ok":true,"records":3}`、`us.relay` の `_acme-challenge` CNAME も引ける。**未確認（VPS の中身。[U] が確かめる）**: 米国の ID が日本の `INFRA_IDS` にあるか、再起動後に自動で上がるか、`unattended-upgrades`・chrony・`ufw`（米国は 22・443 のみ）、バックアップ（identity）、外から `53/udp`（日本。**DNS を横取りしない回線から**）。手元の操作マニュアルは `docs/private/vps-runbook.md`（git 管理外）。
3. **[U→A] 段階 C（`vps-checklist.md` の 4）**: ブリッジの `relayUrls` に 2 つを書いて再起動 → `device add` で新しい設置用ファイル → **実機で診断の「3. Network」を見る**（`jp.relay`・`us.relay` の往復時間）→ 動いたら、既定値（`config.rs`）、`docs/usage/relays.json`、`install-guide.md` の「準備中」を直す（[A]）→ 最後に、トンネル（`cloudflared`、`bridge.spellink.uk`）を切る（[U]）。チェックリストは、米国を OVHcloud と書いたまま（実際は Scaleway。直す）。
4. **[A] 計測の続き（ADR 0021）**: 国の引き（GeoIP の国データベースを設定で渡す。件数 20 未満は出さない）。中継の負荷試験の道具。中継が動いたら `docs/usage/relays.json` に URL を足す。
5. **寄付の入口**: `.github/FUNDING.yml` に Ko-fi（`beatsgroove`）を設定済み（ユーザー、2026-11-24）。当面 Ko-fi のみ。**[A]** README・ブリッジの案内にも Ko-fi のリンクを置く（Supernote のプラグインストアの規約で、寄付のリンクが許されるかも確認）。
6. **[U] 公開の前に埋める文書**: `docs/legal/` のプライバシーポリシー・利用規約の `[ ]`。名前サービスが ID と LAN の IP を見ること、証明書の公開記録に名前が載ること、**中継の集計（時間と件数のみ）を取ること**を入れる。`README.md`・`CONTRIBUTING.md`・`SECURITY.md`・`THIRD_PARTY_NOTICES.md`（Rust の依存も）。専門家の確認を勧める。
7. **[U] Anthropic の規約を、公開の前と変更のたびに確認**（`claude -p` の扱いの変更の報道あり。`support.claude.com` で最新を）。
8. **[A] 公開前の残り**: 名前サービスの保存のロック、TLS の入口の本文の期限と接続元ごとの上限（`review-2026-11-22.md`）。CI とリリースを動かす（**push の許可が要る**。`macos-15-intel` のラベルを確認。手元の Intel の Mac で実行する案もある。公開リポジトリの自前ランナーは、タグのリリースだけに）。push の直前に gitleaks。
9. **[A] Bridge App**: 寿命の表は確認済み（2026-11-24）→ 実装（トレイ、自動起動）。表を `npm run spec` の対象に入れ、各マスにテストを対応させる。
10. **[A]** 多言語（zh_CN・zh_TW の列。グリフは文字体系ごとにデータが要る）。他社の AI の追加（規約の確認が先）。文字以外の入力。NOTE が回答中に固まる件の再検証（原因未特定）。
11. **[U] 公開の時期。** 名前は Spellink で確定（2026-11-24）。

## 未決（ユーザーの判断）
- **名前**: **Spellink で確定**（2026-11-24）。`spellink.uk` を取得済み。`pluginID` は変更禁止。別案の記録は `docs/notes/research/2026-11-03-naming-round2.md`。
- 公開の時期と形、**push の許可**。（寄付で始めることは決定。ADR 0020。入口のアカウントは未用意）
- 中継の公開の時期、可用性の約束（運営・置き場所・名前サービス・費用の持ち方は決定。ADR 0017・0019・0020）。
- 他社の AI（ChatGPT など）を足す順番と、各社の規約の確認（ADR 0015）。
- 保留: 履歴の削除と実行中の回答の関係、ホストのライフサイクルの意味、ページ数の変化の扱い、波線の消し残しの通知。

## 手順（再開時）
**ローカルの IDE で再開するとき**: リポジトリは `~/GitWorkspaces/Spellink`、ブランチは `stg`（`git log --oneline | head -30` で、この期間のコミットが見える）。まず、このファイルと `CLAUDE.md`、次に `log/2026-11.md` の末尾（11-19〜20 の項）。テストは下の手順で、Rust（`~/.cargo/bin`）と Node が要る。**ブリッジは 2026-11-21 に Go から Rust に置き換えた（ADR 0018、`rust-port.md`）**。結合テストは `zip`・`unzip` も使う。**開発機のブリッジは古い版**（上の「実行中」）。
```bash
export PATH=$HOME/.cargo/bin:$PATH
# ブリッジ: テスト → ビルド → 入れ替え → 再起動 → 疎通
cd ~/GitWorkspaces/Spellink/bridge && cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked
# 設置用の .snplg を作るバイナリは、プラグインを埋め込んで作る: bridge/scripts/build-with-plugin.sh（下の素の cargo build は、埋め込みなし）
cargo build --release --locked && cp target/release/spellink-bridge /tmp/spellink-bridge-dev && pkill -f "^$HOME/.local/bin/spellink-bridge serve"; sleep 1
install -m 755 /tmp/spellink-bridge-dev ~/.local/bin/spellink-bridge   # 設定・台帳・履歴はそのまま使える（上書き更新のテストあり）
(cd /tmp && nohup ~/.local/bin/spellink-bridge serve > /tmp/spellink-bridge.log 2>&1 &)
~/.local/bin/spellink-bridge check            # 端末と同じ道（LAN と、トンネル）で、暗号化の握手まで確かめる
# プラグイン: 表の生成 → 型 → テスト → ビルド（成果物 plugin/build/outputs/Spellink.snplg）
cd ../plugin && npm run spec && npx tsc --noEmit && npx jest && npm run build
# 端末からの診断ログ
tail -n 5 ~/.config/spellink-bridge/reports.jsonl
```
- 設定: `~/.config/spellink-bridge/config.json`（`model: haiku`、`extraUrls`、`timeouts`、`history`、`tiltDegrees` など）。文字データ `glyphs.bin`（KanjiVG から生成。再配布しない）、診断 `reports.jsonl`、会話の記憶 `history/`。JSON のカンマ抜けでブリッジが起動しなくなる（エラーにファイル名が出る）。
- 端末へのインストール: `spellink-bridge device add <名前>` が作る設置用の `.snplg` を端末に入れ、端末の画面の QR と 8 桁でペアリングする（`install-guide.md`）。**OS 更新前に作ったノートではボタンが出ない。新しいノートで試す**。接続先は、ビルドには埋め込まない。
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
| 調査・比較 | `docs/notes/research/`（11-20: `relay-decisions`〔中継の運営と Anthropic の規約〕、`direct-connection-options`〔直結〕、`bridge-app-draft`〕 |
| 公開する文書の草案（プライバシーポリシー・利用規約） | `docs/legal/` |
| 中継の運営の手順・systemd 単位 | `docs/guides/relay-operations.md`、`deploy/relay/` |
| 実機テストの手順 | `docs/notes/device-test-guide.md` |
| 知財のルール | `docs/notes/ip/README.md`（分析本体は private/ で git 管理外） |
| プロジェクトの規約 | `CLAUDE.md` |
