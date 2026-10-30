# 実機テスト手順（Nomad / Manta）

スパイク版（v0.0.1）の確認手順。結果はブリッジ側の `~/.config/cloute-bridge/reports.jsonl`（macOS は `~/Library/Application Support/cloute-bridge/`）に送れるので、
エージェントはそれを読んで次の判断をする。

## 0. ブリッジを起動（PC 側）
```bash
cloute-bridge serve    # ポート 8765。初回だけ KanjiVG / Hershey をダウンロードして glyphs.bin を作る（数秒）
# バイナリの作り方: cd bridge && scripts/build.sh 0.1.0 linux/arm64   （Go 1.24 以上）
# 開発機（ROCK 5B+）では ~/.local/bin/cloute-bridge に入っている
```
- Claude Code にログイン済みであること（`claude` が普段どおり動く状態）。
- 設定: `~/.config/cloute-bridge/config.json`（`model` に `"haiku"` などを書くと速いモデルを使う）

## 1. プラグインを入れる
- `plugin/build/outputs/Cloute.snplg` を Nomad の `MyStyle/` にコピー（USB / Browse & Access / クラウド）。
- 設定 > アプリ > プラグイン > プラグインを追加 → Cloute をインストール。
- 開発中はブリッジの接続先（IP とトークン）をビルド時に埋め込んでいる（`plugin/src/bridges.local.json`、git 管理外）。
  配布版では `MyStyle/Cloute/bridges.json`（`cloute-bridge pair <出力先>` で作れる）を読む予定。

## 2. 診断画面（ノートを開いてサイドバーの「Cloute 診断」）
上から順に押す。
1. **Environment** — 機種、ページサイズ、ペン情報、権限状態
2. **Permissions** — 権限ダイアログ。「常に許可」を選ぶ
3. **Network** — `http://…/health` と `ws://…` に届くか（平文通信が許可されているか＝ADR 0007 の分岐点）
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
