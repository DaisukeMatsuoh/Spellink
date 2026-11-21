# 実機テスト手順（Nomad / Manta）

スパイク版（v0.0.1）の確認手順。結果はブリッジ側の `~/.config/spellink-bridge/reports.jsonl`（macOS は `~/Library/Application Support/spellink-bridge/`）に送れるので、
エージェントはそれを読んで次の判断をする。

## 0. ブリッジを起動（PC 側）
```bash
spellink-bridge serve    # ポート 8765。初回だけ KanjiVG / Hershey をダウンロードして glyphs.bin を作る（数秒）
# バイナリの作り方: cd bridge && scripts/build.sh 0.1.0 linux/arm64   （Go 1.24 以上）
# 開発機（ROCK 5B+）では ~/.local/bin/spellink-bridge に入っている
```
- Claude Code にログイン済みであること（`claude` が普段どおり動く状態）。
- 設定: `~/.config/spellink-bridge/config.json`（`model` に `"haiku"` などを書くと速いモデルを使う）

## 1. プラグインを入れる
- `plugin/build/outputs/Spellink.snplg` を Nomad の `MyStyle/` にコピー（USB / Browse & Access / クラウド）。
- 設定 > アプリ > プラグイン > プラグインを追加 → Spellink をインストール。
- 開発中はブリッジの接続先（IP とトークン）をビルド時に埋め込んでいる（`plugin/src/bridges.local.json`、git 管理外）。
  配布版では `MyStyle/Spellink/bridges.json`（`spellink-bridge pair <出力先>` で作れる）を読む予定。

## 2. 診断画面（ノートを開いて、左のツールバーの「プラグイン」（パズルのピース）→「Spellink 診断」）
- ツールバーにパズルのアイコンが無い場合は、一番下の「…」の中にある。
上から順に押す。
1. **Environment** — 機種、ページサイズ、ペン情報、権限状態、**暗号化に必要な JS 機能（BigInt・乱数源）**
2. **Permissions** — 権限ダイアログ。「常に許可」を選ぶ
3. **Network** — `http://…/health` と `ws://…` に届くか、接続方式が `noise`（暗号化）になっているか
4. **Strokes ×10** — 合成ストローク 10 本をページ下部に書く。**閉じた後にジグザグが見えるか**を確認
5. **Strokes ×200** — 速度計測
6. **Text box** — テキストボックス挿入（フォールバック手段の確認）
7. **Ask (no lasso)** — OCR 抜きで「富士山の高さ…」を質問し、回答がページ中央に手書きで出るか
8. **New page** — 今のページの直後に新しいページを挿入し、そこにジグザグを書いて切り替える（長い回答の続き書きの確認）
- **Send log to bridge** — ここまでのログを PC に送る（最後に必ず押す）

## 3. 本番の操作
1. ノートに質問を手書きする
2. ラッソで囲む → 出てきたラッソツールバーの右端の「…」→「Claudeに聞く」
3. 小さなパネルに進捗が出て、選択範囲の下に回答が 1 行ずつ書かれる
4. 下端に達したら、直後に新しいページが挿入されて表示が切り替わり、続きが書かれる（最大 2 ページ）

## 4. 見てほしいこと（目視）
- 診断 4 / 7 の後、パネルを閉じずに・閉じた後に、それぞれページに線が見えるか（「Reload file」が必要だったか）
- 書かれた線の太さ・濃さ（ペン設定がそのまま使われているか）
- 書かれた回答を消しゴム・ラッソで普通に扱えるか
- 「Claudeに聞く」のパネルの大きさ・位置

## 5. ペアリングの実験（診断 11〜14。設計: `docs/notes/research/2026-11-08-pairing.md`）
**実験 1: 書き換えた `.snplg` が入るか、入れ直しで置き換わるか**（最重要）
1. `cd plugin && npm run build` → `node scripts/patch-snplg.js build/outputs/Spellink.snplg build/outputs/Spellink-seedA.snplg seedA-0001`（同様に `seedB-0002` で `Spellink-seedB.snplg`）。
2. `Spellink-seedA.snplg` を端末に入れる → 診断「11. Seed in package」→ `present, "seedA-0001"` が出るか。**入らない・起動しないなら、署名や整合性の検査がある**（エラーの文言を記録）。
3. `Spellink-seedB.snplg` を、**削除せずに**入れる → 診断 11 が `seedB-0002` に変わるか。変わらない・入らないなら、先にアンインストールが要るか（その手順で再試行）。
4. 「Send log to bridge」。

**実験 3: 端末の ID と、ファイルを書く手段**（診断 12・13）
- 「12. SDK names」: SDK とネイティブモジュールの全名前。`names that look like an id` に、シリアルや端末 ID らしきものがあるか。
- 「13. Files」: `file://` で読めるファイル（シリアル、MAC、boot_id、`bridges.json`）と、`MyStyle` の中身。読めた内容の先頭 48 文字だけがログに出る。

**実験 4: QR**（診断 14）
- 「14. QR: show」で、端末に QR（640px）が出る。ログに最初の描画までの時間が出る。
- PC で `cd plugin/scripts && python3 -m http.server 8099` → ブラウザで `http://127.0.0.1:8099/qr-scan.html` → カメラで端末を読む。距離・角度・部屋の明るさを変えて、読めるまでの時間と、読めた文字列（`spellink://pair?...`）を記録。

**実験 5・3c（診断 15・16）**: 「15. Name store」= ディレクトリ名に秘密を保存できるか。**何も削除しない**（以前の削除の手順で、プラグインがハングした）。1 回目は `created`（作って読み戻す）、**端末を再起動**してもう一度押すと `survived` と出れば、再起動をまたいで残る。画面に 1 行ずつ「何をしているか」が出るので、**ハングしたら、画面の最後の行を教えてください**。「16. Read existing」= `MyStyle` に実在するファイルを `file://` で読めるか（確認済み）。

## 6. ペアリング（端末ごとの鍵。ADR 0016、`docs/spec/pairing.md`）
**ブリッジを新しい版に入れ替えてから**行う（`device` コマンドと管理 API は新しい版にだけある）。
1. **基になるパッケージは、ブリッジに組み込む**（`bridge/scripts/build-with-plugin.sh`。クリーンなプラグインを作って埋め込む。ビルド時の環境変数 `SPELLINK_BASE_PACKAGE`）。`config.json` には、端末から届く `wss://` の住所だけ書く: `"extraUrls": ["wss://bridge.spellink.uk/v1"]`。組み込みなしのブリッジ（`cargo build` だけ）では、`"pairing": {"pluginPackage": "/…/Spellink.snplg"}` が要る。**開発機のローカルビルド（`plugin/build/outputs/Spellink.snplg`、`bridges.local.json` の住所とトークンが入る）は、人に配るブリッジに埋め込まない。**利用者向けの手順は `docs/guides/install-guide.md`。ブリッジを再起動。
2. `spellink-bridge device add Nomad` → `Spellink-Nomad.snplg` ができる（**この端末だけが使う、期限つきのファイル**。人に渡さない）。
3. その `.snplg` を端末の `MyStyle/` に入れ、プラグインとしてインストール。
4. 端末で、設定（プラグインの設定ボタン）→「ブリッジとペアリング」→「ペアリング」。8 桁のコードと QR が出る。
5. パソコンで `spellink-bridge device pair` を実行し、コードを入力（または `spellink-bridge device pair 48151623`）。
6. 端末が「ペアリングしました」と出し、パソコンの `spellink-bridge device list` が `active` になる。
7. 普通に質問して、回答が書かれる（以後は、端末に保存された鍵で接続する。`MyStyle/Spellink-pairing/` に `k-…`・`e-…` という名前のフォルダが 2 つできる）。
**確認すること**: ①コードを入力しないまま窓（既定 90 秒）が過ぎると「時間内に入力されませんでした」になるか ②間違ったコードを入れると端末に影響がないか（5 回で窓が閉じる）③ペアリングの後に、新しい版の公開用パッケージ（接続設定なし）を入れても、そのまま使えるか ④`spellink-bridge device revoke Nomad` で、端末が使えなくなるか ⑤`reissue` した後に、古いファイルが使えず、`device package Nomad` の新しいファイルでペアリングし直せるか。
**この版でできないこと**: 鍵を拒否されたときに、端末が「ペアリングし直してください」と出すこと（`keyRefused` を呼ぶ処理が未接続）。

## 端末に行く前の確認（開発機で）
`spellink-bridge check` が、端末と同じ道（LAN とトンネル）で、暗号化の握手まで確かめる。`OK` なら道は通っている。`.local` の行は、端末からは引けないので、`NG` でも気にしない。

