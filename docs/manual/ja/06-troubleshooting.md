# 6. うまくいかないとき

まず、`spellink-bridge check` を実行します。端末と同じ道で、暗号化の握手まで確かめます。OK なら、道は通っています。

| 症状 | こうする |
|---|---|
| `device add` が `plugin package` のエラー | プラグインが入っていないビルドです。Releases の版を使うか、`Spellink.snplg` を取り、`config.json` の `"pairing": {"pluginPackage": "…のパス"}` に書いて再起動します |
| `the bridge did not accept the admin token` | ブリッジが別の設定フォルダで動いています。同じユーザーで実行してください |
| 端末にボタンが出ない | 古いノートです。新しいノートで試します |
| 「ブリッジに接続できません」 | ブリッジが止まっている、パソコンがスリープ、またはインターネットが切れています |
| ペアリングが終わらない | `spellink-bridge device list` を見ます。`issued` のままなら、端末で「ペアリング」を押したか、つなぐ道の設定([3. 端末をつなぐ](03-device.md#つなぐ道を選ぶ))を確かめます |
| 設置ファイルをなくした | `spellink-bridge device package my-nomad`(同じ名前のファイルがあると止まるので、先に消す) |
| 期限切れ・やり直したい | `spellink-bridge device reissue my-nomad` のあと `spellink-bridge device package my-nomad` |
| Bridge App に「Claude Code が見つからない」 | `claude` をパソコンに入れ、ログインします。サービスで動かしているなら、`service install` をやり直します(実行時の `PATH` が記録されます) |
| 答えが返らない | Claude の利用枠を使い切っていないか、`claude` で直接質問できるか、確かめます |
| 「今は?」に答えない | ブリッジが古い版かもしれません。`spellink-bridge version` と、再起動を確かめます([再起動](02-bridge.md#設定を変えたあとの再起動)) |
| Bridge App の「つながる道がありません」 | 中継が無効で、家の直結・トンネルもありません。[つなぐ道](03-device.md#つなぐ道を選ぶ)を 1 つ選びます |

それでも直らないときは、Issue に、`spellink-bridge version` と、起きたことを書いてください。**トークンや設置ファイルは、載せないでください。**
