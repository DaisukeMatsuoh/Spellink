# 2. ブリッジを入れる

## 1. 入れる
1. Spellink の Releases のページから、お使いの OS 用の `spellink-bridge-…` をダウンロードします。
2. 置き場所を決めます。
   - Linux・macOS: `~/.local/bin/spellink-bridge`。実行できるようにします: `chmod +x ~/.local/bin/spellink-bridge`
   - Windows: `C:\Users\あなた\bin\spellink-bridge.exe`
3. 確かめます: `spellink-bridge version`(番号が出れば成功)。
   - macOS で「開発元を確認できない」と出たら、「システム設定 → プライバシーとセキュリティ」で許可します。

> Releases がまだ無いあいだは、自分でビルドします([README](../../../README.md) の "Build it yourself")。
> 端末に入れる設置ファイルを作るには、プラグイン入りのビルド(`bridge/scripts/build-with-plugin.sh`)が要ります。

## 2. Claude Code にログインしておく
`claude` と打ってログインを済ませます。あとで、Bridge App の画面(`spellink-bridge open`)の先頭に
「Claude Code is ready」と出れば、ブリッジが見つけています。

## 3. 起動する
```
spellink-bridge serve
```
初めての起動では、設定ファイルと、手書きの文字データが作られます(数分。インターネットが要ります)。

設定ファイル `config.json` の場所:
- Linux: `~/.config/spellink-bridge/`
- macOS: `~/Library/Application Support/spellink-bridge/`
- Windows: `%AppData%\spellink-bridge\`

動いたら `Ctrl+C` で止めます。

## 4. パソコンと一緒に動くようにする
```
spellink-bridge service install
```
管理者権限は要りません。先に何をするかだけ見たいときは `--dry-run` を付けます。状態は `spellink-bridge service status`。
(Linux は systemd のユーザーサービス、macOS は launchd、Windows はタスク スケジューラです。)

## 5. 確かめる
```
spellink-bridge check
```
端末と同じ道で、暗号化の握手まで確かめます。OK なら、ブリッジは受け付けています。
画面で見たいときは `spellink-bridge open` です([4. 使う](04-use.md#bridge-app))。

## 設定を変えたあとの再起動
`config.json` を変えたら、ブリッジを再起動します。
- Linux: `systemctl --user restart spellink-bridge`
- macOS・Windows: `spellink-bridge service uninstall` のあと `spellink-bridge service install`
- 手で動かしているとき: `Ctrl+C` のあと `spellink-bridge serve`

次: [3. 端末をつなぐ](03-device.md)
