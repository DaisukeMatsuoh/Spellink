# Spellink インストールガイド（利用者向け）

Supernote で手書きを囲むと、あなたのパソコンの AI（Claude）が答え、答えが手書きの筆跡で書き戻されます。
このガイドは、パソコンに**ブリッジ**を入れて、Supernote と**ペアリング**するまでの手順です。所要 20 分ほど。

> **状態（2026-11-22）**: 開発中のプレリリースです。Windows と macOS は、実機で確認していません（Linux で確認）。
> 手順のうち、**中継（外出先から使う道）は、運営の準備中**です。それまでは、§3 の「自分の道」が要ります。

## 0. 用意するもの
| もの | 内容 |
|---|---|
| Supernote | Nomad または Manta。プラグインの導入に、パソコンとつなぐ USB ケーブルか、端末にファイルを送る方法 |
| パソコン | **Windows / macOS / Linux**。**常時つけておく**（スリープすると答えが返らない）。インターネットにつながっていること |
| Claude の契約 | Pro / Max など。**本人の契約**（複数人で共有しない）。APIキーは要りません |
| Claude Code | 公式の Claude Code を、そのパソコンに入れて、**ログイン済み**にする（`claude` と打って、ログインを済ませる）。Spellink は、そのログインをそのまま使います。Spellink の運営や端末に、ログイン情報は渡りません |

## 1. ブリッジを入れる
1. Spellink の Releases のページから、あなたの OS 用の `spellink-bridge-…` をダウンロードします。
2. 置き場所を決めます（例: Linux・macOS は `~/.local/bin/spellink-bridge`、Windows は `C:\Users\あなた\bin\spellink-bridge.exe`）。Linux・macOS では実行できるようにします: `chmod +x ~/.local/bin/spellink-bridge`
3. 確認します: `spellink-bridge version`（番号が出れば OK）。
   - macOS で「開発元を確認できない」と出たら、システム設定の「プライバシーとセキュリティ」で許可します（公証は未対応です）。

## 2. 設定して、起動する
1. 初めて起動すると、設定ファイルと、手書きの文字データ（KanjiVG などからあなたのパソコンで作る。数分）ができます。
   ```
   spellink-bridge serve
   ```
   設定ファイルは、次の場所にできます（`config.json`）。
   - Linux: `~/.config/spellink-bridge/`
   - macOS: `~/Library/Application Support/spellink-bridge/`
   - Windows: `%AppData%\spellink-bridge\`
2. 動いたら `Ctrl+C` で止め、設定ファイルに**端末がつながる住所**を足します（§3）。
3. パソコンの起動と一緒に動くようにします（管理者権限は要りません）:
   ```
   spellink-bridge service install
   ```
   先に何をするかだけ見たいときは `--dry-run` を付けます。状態は `spellink-bridge service status`。

## 3. 端末がつながる道
端末は、**公的な証明書つきの `wss://`** にしかつながれません。次のどちらかを、設定ファイルに書きます。

- **Spellink の中継（標準）**: `"relayUrl": "wss://relay.spellink.uk"` を足します。ドメインもトンネルも要りません。**運営の準備中です。開始したら、ここに書きます。**
- **自分の道（トンネル）**: 自分のドメインと Cloudflare のトンネルで、`wss://…` の住所を作ります（`docs/guides/cloudflare-tunnel-setup.md`）。`"extraUrls": ["wss://あなたの住所/v1"]` を足します。

設定を変えたら、ブリッジを再起動します（`systemctl --user restart spellink-bridge`、macOS・Windows は `service uninstall` のあと `service install`）。

## 4. 端末を登録する（ペアリング用のファイルを作る）
パソコンで、端末に付ける名前を決めて実行します。
```
spellink-bridge device add my-nomad
```
`Spellink-my-nomad.snplg` ができ、続きの手順が表示されます。
- このファイルは、**その端末だけが使える、期限つき（既定 24 時間）**です。ペアリングが終わるまで、人に渡さないでください（`0600` の権限で作られます）。
- 設定は要りません（プラグインは、ブリッジに入っています）。

## 5. 端末にインストールする
1. `Spellink-my-nomad.snplg` を、Supernote の `MyStyle` フォルダに入れます（USB、または端末へのファイル送信）。
2. 端末の「設定」→「アプリ」→「プラグイン」から、インストールします。
3. **新しいノート**を作って試します（OS を更新する前に作ったノートには、ボタンが出ません）。

## 6. ペアリングする（QR コードと 8 桁）
1. 端末で、Spellink の設定を開き、「ブリッジとペアリング」→「ペアリング」を押します。**8 桁の数字と QR コード**が画面に出ます（既定で 90 秒だけ）。
2. パソコンで、次を実行して、8 桁の数字を入力します。
   ```
   spellink-bridge device pair
   ```
   数字を先に付けて `spellink-bridge device pair 12345678` でも構いません。
   - パソコンにカメラがあるときは、`spellink-bridge device pair --web` で、ブラウザに読み取りのページが開きます。端末の QR にカメラを向けます。**数字の入力が主で、QR は近道です。**
3. 端末に「ペアリングしました」と出ます。パソコンで `spellink-bridge device list` を実行して、`active` になっていれば完了です。

数字を入れないまま時間が過ぎると、端末に「時間内に入力されませんでした」と出ます。もう一度「ペアリング」を押してください（5 回まちがえると、その窓は閉じます）。

## 7. 使う
- 手書きをラッソで囲み、プラグインのボタンを押すと、数秒〜数十秒で答えが書かれます。
- 一度に 1 つの質問です。回答中に別のノートを開くと、書くのを止めます。
- 設定（字間・傾き）と会話の記憶は、パソコンに保存されます。「会話を忘れさせる」で消せます。

## 8. うまくいかないとき
| 症状 | こうする |
|---|---|
| まず確かめる | `spellink-bridge check`。端末と同じ道で、暗号化の握手まで確かめます。OK なら道は通っています |
| `device add` で `plugin package` のエラー | 古い、または自分でビルドしたブリッジです。Releases の版を使うか、Releases から `Spellink.snplg` を取り、設定の `"pairing": {"pluginPackage": "…のパス"}` に書いて、ブリッジを再起動します |
| `the bridge did not accept the admin token` | ブリッジが別の設定ディレクトリで動いています。同じユーザーで実行してください |
| 端末にボタンが出ない | 古いノートです。新しいノートで試します |
| 「ブリッジに接続できません」 | ブリッジが止まっている／パソコンがスリープ／インターネットが切れている |
| 端末のペアリングが終わらない | `spellink-bridge device list` を見ます。`issued` のままなら、端末で「ペアリング」を押したか、§3 の住所が合っているか確認します |
| ファイルをなくした | `spellink-bridge device package my-nomad`（同じファイルをもう一度作る。同じ名前のファイルがあると、上書きせずに止まるので、先に消す） |
| 期限が切れた・ペアリングをやり直したい | `spellink-bridge device reissue my-nomad` のあと、`spellink-bridge device package my-nomad` |
| 端末を手放す・なくした | `spellink-bridge device revoke my-nomad`（その端末だけが使えなくなります） |

## 9. 応援する
Spellink は、個人が運営しています。中継のサーバー代は寄付で支えています（準備中: Buy Me a Coffee ほか）。機能は寄付の有無で変わりません。
