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

## 2. 診断画面（ノートを開いてサイドバーの「Spellink 診断」）
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
2. ラッソで囲む → ラッソツールバーの「Claudeに聞く」
3. 小さなパネルに進捗が出て、選択範囲の下に回答が 1 行ずつ書かれる
4. 下端に達したら、直後に新しいページが挿入されて表示が切り替わり、続きが書かれる（最大 2 ページ）

## 4. 見てほしいこと（目視）
- 診断 4 / 7 の後、パネルを閉じずに・閉じた後に、それぞれページに線が見えるか（「Reload file」が必要だったか）
- 書かれた線の太さ・濃さ（ペン設定がそのまま使われているか）
- 書かれた回答を消しゴム・ラッソで普通に扱えるか
- 「Claudeに聞く」のパネルの大きさ・位置
