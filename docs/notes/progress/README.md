# 進捗（引き継ぎ用の入口）

新しいセッションのエージェントは、**まずこのファイル**を読む。次に `CLAUDE.md`（規約）。リリースまでの全体は `release-plan.md`。経緯が要るときだけ `history.md`。別のエージェントに作業を渡すときは `handoff.md`。

最終更新: 2026-11-21（ブリッジを Rust に移植し、家の直結・運営の入口〔名前サービス、自前の TLS〕を実装した区切り）

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

## 現在の状態（2026-11-21）
- **2026-11-21 の区切り**（詳細は `log/2026-11.md`）: ①ブリッジと中継を **Rust に移植**（ADR 0018。`bridge/` は Rust、Go は git の履歴）。暗号は `aws-lc-rs`。②**家の直結を実装**（ADR 0017）: 名前サービス（`spellink-bridge names`。権威 DNS と署名つき API）、各ブリッジが自分の証明書を Let's Encrypt から取る（ACME DNS-01。**DNS 用のトークンは存在しない**）。状態は 2 つの表（`docs/spec/lan-cert.md`。登録・証明書。生成して使う）。既定は無効（`lan.enabled`）。③**運営の入口**（ADR 0019）: Cloudflare Tunnel をやめ、中継が自分で TLS（`relay --tls-domain …`）。`bridge/Dockerfile.edge`（scratch）と `deploy/vps/compose.yaml`。④構成図 `docs/guides/architecture.md`、運営の手順 `docs/guides/relay-operations.md`、調査メモの結論の一覧 `docs/notes/research/README.md`。
- **未確認（実環境）**: 本物の Let's Encrypt での注文（まず staging）、VPS への委任（NS）と DNS、docker でのイメージのビルド、端末が家の中で `<id>.lan.spellink.uk:8766` に直接つながること。偽の CA・偽の登録先での状態機械、TLS の受け口、DNS、API、バイナリでの登録→DNS の引きは、自動テストで確認済み。
- **push 前（ユーザー指定）**: セキュリティレビュー、コードレビュー、`gitleaks` での全履歴の検索（開発機に未導入）。トークンの照合は 2026-11-21 に済み（`docs/guides/architecture.md` §9）。
- **（以下は 2026-11-20 までの状態。Go を Rust に読み替える）**
- **コード**: `stg` ブランチ、**未 push**（この区切りでコミット済み。push はユーザーの指示まで）。テスト: Go 全パッケージ緑（`-race`）、端末 **30 スイート・855 件**緑。**件数だけでなくスイート数を確認する**。CI（`.github/workflows/`）は一度も動かしていない（本物のブリッジとの結合テストが `go`・`zip`・`unzip` を使うので、CI の端末のジョブにも要る）。
- **表がコード**（`docs/spec/README.md`）: 6 つ（端末の質問の流れ・波線・ページ追加と書き込み・ブリッジの run・**ペアリングの記録・ペアリングの端末**）。文書を直す → `cd plugin && npm run spec` → ハンドラ → テスト。**ペアリング（記録・端末）とブリッジの run の状態名・イベント名は、読める名前（先頭だけ大文字: `Issued`、`OwnerAdds` など）に変えた**（2026-11-20）。**質問の流れ・波線・ページ追加は、まだ ALL_CAPS の名前**（同じ形にそろえるかはユーザーの判断。コードの変更が大きい）。台帳の保存形式の状態名は、コードの名前と切り離した `issued` / `granted` / `active` / `expired` / `revoked`。
- **この期間に作ったもの**（詳細は `log/2026-11.md`）:
  - **ペアリング（ADR 0016）**: ブリッジが端末ごとに B（`.snplg` に入る初期の秘密）と K（ペアリングで渡る鍵。端末はディレクトリ名に保存）を発行し、台帳で管理。`spellink-bridge device add|list|pair [--web]|package|revoke|reissue|forget`。端末の画面に QR・**ブリッジが自分の字で書く案内文**・8 桁。鍵の拒否の検出。複数のブリッジ。
  - **QR を読むページ** `/pair`（ブラウザのカメラ。`jsQR` はバイナリに埋め込み）、**アプリ画面** `/app`（状態・端末の一覧・追加・解除・作り直し。`spellink-bridge open`）、`/admin/status`。ローカル専用の入口は **Host も検査**（DNS rebinding 対策）。
  - **中継の強化**: 無通信 15 分で閉じる、部屋ごとの帯域、ログに IP を書かないことをテストで固定。運営手順書・systemd 単位。
  - **文言は TSV**（`plugin/locales/strings.tsv`、ブリッジ側のページは各 `strings.tsv`。列を足せば言語が増える）。
  - **文書**: プライバシーポリシーと**利用規約（免責を含む）の草案**（`docs/legal/`）、Bridge App の寿命の表の草案（`docs/spec/bridge-app.md`）と仕様の草案（`docs/notes/research/2026-11-20-bridge-app-draft.md`）、中継の運営の判断材料（`2026-11-20-relay-decisions.md`）、**直結の選択肢**（`2026-11-20-direct-connection-options.md`）。
- **実行中（開発機）**: ブリッジは **古い版のまま**（`~/.local/bin/spellink-bridge`。systemd のユーザーサービスとして動いている）。**新しい機能（ペアリング・`/pair`・`/app`）を使うには、入れ替えと、`config.json` への `extraUrls`（例 `wss://bridge.spellink.uk/v1`）と `pairing.pluginPackage`（`plugin/build/outputs/Spellink.snplg` のパス）の追加が要る**（手順は `device-test-guide.md` の 6）。cloudflared は動いている。
- **配布物**: `plugin/build/outputs/Spellink.snplg`（開発者のブリッジの接続先を埋め込んだ、**個人用**。配らない）。
- **決めたこと（2026-11-20、ユーザー）**: ①**署名はしない**（macOS・Windows とも）。公式サイトに各配布物の SHA-256 を載せ、「公式」と示す。Windows は将来 SignPath Foundation の OSS 向け無料署名を検討。②**Anthropic の規約**: 「利用者自身のパソコンで、改変しない Claude Code を、自分のログインで、本人だけが使う」形で進める。初回の起動で、その同意を取る（`docs/legal/terms-of-service-draft.md`）。公式の記述を確認済み（`relay-decisions.md` の 1.5、ADR 0005 の追記）。③公式の中継は、運営する（Cloudflare。免責は全文を書く）。④端末の言語は、本体の 4 言語（en・ja・zh_CN・zh_TW。要・実機確認）から。
- **実機で確認済み**（〜11-07）: ラッソ → 認識 → 回答、複数ページ、会話の記憶（ADR 0012）、wss＋Noise、設定画面、ネイティブのエラーダイアログ。**2026-11-19〜20 に確認**: 書き換えた `.snplg` が入る・上書きで置き換わる、ディレクトリ名の保存が再起動をまたいで残る（`deleteDir` は使うとハングした）、QR（640px）が PC のカメラで一瞬で読める。
- **実機で未確認**: **ペアリングの全体**（`device-test-guide.md` の 6）、手書きの案内文の描画（短い線を並べる方式。e-ink での速さと見え方）、`/app`・`/pair` のブラウザでの操作、切断後の再開（ADR 0013）、再接続 3 回、`busyNotice`、`avoidOverwriting`、空き領域の確認の実測。
- ADR の一覧と根拠: `docs/decisions/README.md`（Accepted: 0001〜0003・0005〜0008・0012・0014。Proposed: 0010・0011・0013・**0016**）。

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
担当の分け方と、遠隔でできる作業の一覧は `handoff.md`。**[U]** = ユーザー、**[A]** = エージェント。
1. **[U] ブリッジを新しい版に入れ替え、`config.json` に `extraUrls` と `pairing.pluginPackage` を足し、実機でペアリングを試す**（`device-test-guide.md` の 6）。結果（特に、手書きの案内文の描画、QR とコード、`/pair`・`/app`）を見て、表の気になったセルを直す。
2. **[U] `docs/spec/bridge-app.md`（Bridge App の寿命の表）を読んで確認** → **[A]** コードの表にして実装（`/app` を土台に、`spellink-bridge app`、トレイ、自動起動。トレイは Mac・Windows の実機で試す必要がある）。仕様の草案: `docs/notes/research/2026-11-20-bridge-app-draft.md`。初回の同意の画面を含む。
3. **[U] 公開の前に埋める文書**: `docs/legal/privacy-policy-draft.md`・`terms-of-service-draft.md` の `[ ]`（前段のサービス名、保存期間、告知の場所、連絡先、準拠法・管轄）。専門家の確認を勧める。`README.md`・`CONTRIBUTING.md`・`SECURITY.md`・`THIRD_PARTY_NOTICES.md`（連絡手段、著作権者の表示）。
4. **[U] Anthropic の規約を、公開の前と変更のたびに確認**（特に、`claude -p` の利用が、2026-06-15 から別枠の月ごとのクレジットから引かれる、という報道。公式の最新の扱いを `support.claude.com` で確認）。必要なら Anthropic に書面で問い合わせる。
5. **[U] 中継の運営の判断**（`docs/notes/research/2026-11-20-relay-decisions.md`）: 運営と置き場所（Kagoya の VPS、scratch、自前の TLS。ADR 0019）は決定。残りは、公開の時期、費用の持ち方、可用性の約束、利用規約・プライバシーの文書（名前サービスが ID と LAN の IP を見ること、証明書透明性に名前が載ることを入れる）。**[A]** VPS での初回の運用の立ち会い（`docs/guides/relay-operations.md`。まず Let's Encrypt の staging）。
6. **[U→A] 家の中の直結（ADR 0017。実装済み 2026-11-21）の実環境での確認**: [U] VPS を用意し、DNS に `ns1` の A・`lan` の NS・2 つの CNAME を置く（`relay-operations.md` §3）。[A] staging で注文を通し、本番に切り替え、`lan.enabled` を既定にするか判断。端末で家の中の直結を確認。
7. **[U] 状態名・イベント名を、残りの表（質問の流れ・波線・ページ追加）も読める名前にそろえるか。** [A] は機械的にできる（生成器は対応済み）が、コードとテストの変更が大きい。
8. **[A] 多言語**: TSV に `zh_CN`・`zh_TW` の列を足す（翻訳は [A]、母語の方の確認を勧める）。筆跡のグリフは文字体系ごとにデータが要る（ADR 0006 の追記の表。ハングル・キリル・ギリシャ・アクセント付きラテン・中国語の漢字は未着手）。本体の表示言語は、実機の設定で確認する。
9. **[U] 配布**: 署名なし。公式サイトに SHA-256 を載せる（`release.yml` は、各配布物の `.sha256` を作る）。macOS・Windows の警告の回避の手順を、利用者向けの文書に書く。
10. **[U]** ブリッジの自動起動（`service install`）、CI とリリースを動かす（push の許可が要る）、公開用の文書。
11. 会話の圧縮、他社の AI の追加（規約の確認が先）。NOTE が回答中に固まる件の再検証（原因未特定）。文字以外の入力。
12. 鍵の拒否を、中継でも見分ける実装（失効した端末の部屋を、「拒否」で返す。必要になったとき）。ペアリング後に、端末が自分用のインストーラを消す（`deleteFile` は未確認）。

## 未決（ユーザーの判断）
- **名前**: 現行は Spellink。`spellink.com` は取得済みで買えず、`spellink.uk` を取得。dancink などの案は `docs/notes/research/2026-11-03-naming-round2.md`。決めるまで改名しない（`pluginID` は変更禁止）。
- 公開の時期と形（OSS・寄付〔Buy Me a Coffee など〕）、**push の許可**。
- 中継の公開の時期、費用の持ち方、可用性の約束（運営・置き場所・名前サービスは決定。ADR 0017・0019）。
- Bridge App の寿命の表の確認（`bridge-app.md` の末尾の 3 つ: 終了の待ちの上限、確認の間隔、失敗時もトレイを残すか）。
- 残りの表の状態名・イベント名を読める名前にそろえるか。
- 他社の AI（ChatGPT など）を足す順番と、各社の規約の確認（ADR 0015）。
- 保留: 履歴の削除と実行中の回答の関係、ホストのライフサイクルの意味、ページ数の変化の扱い、波線の消し残しの通知。

## 手順（再開時）
**ローカルの IDE で再開するとき**: リポジトリは `~/GitWorkspaces/Spellink`、ブランチは `stg`（`git log --oneline | head -30` で、この期間のコミットが見える）。まず、このファイルと `CLAUDE.md`、次に `log/2026-11.md` の末尾（11-19〜20 の項）。テストは下の手順で、Rust（`~/.cargo/bin`）と Node が要る。**ブリッジは 2026-11-21 に Go から Rust に置き換えた（ADR 0018、`rust-port.md`）**。結合テストは `zip`・`unzip` も使う。**開発機のブリッジは古い版**（上の「実行中」）。
```bash
export PATH=$HOME/.cargo/bin:$PATH
# ブリッジ: テスト → ビルド → 入れ替え → 再起動 → 疎通
cd ~/GitWorkspaces/Spellink/bridge && cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked
cargo build --release --locked && cp target/release/spellink-bridge /tmp/spellink-bridge-dev && pkill -f '^/home/daisuke/.local/bin/spellink-bridge serve'; sleep 1
install -m 755 /tmp/spellink-bridge-dev ~/.local/bin/spellink-bridge   # 設定・台帳・履歴はそのまま使える（上書き更新のテストあり）
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
| 調査・比較 | `docs/notes/research/`（11-20: `relay-decisions`〔中継の運営と Anthropic の規約〕、`direct-connection-options`〔直結〕、`bridge-app-draft`〕 |
| 公開する文書の草案（プライバシーポリシー・利用規約） | `docs/legal/` |
| 中継の運営の手順・systemd 単位 | `docs/guides/relay-operations.md`、`deploy/relay/` |
| 実機テストの手順 | `docs/notes/device-test-guide.md` |
| 知財のルール | `docs/notes/ip/README.md`（分析本体は private/ で git 管理外） |
| プロジェクトの規約 | `CLAUDE.md` |
