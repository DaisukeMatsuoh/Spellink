# 5. 更新・停止・削除

## 更新する
**ペアリングは消えません。** やり直す必要はありません(鍵は、端末の保存領域にあります)。
1. **ブリッジ(パソコン):** Releases から新しいファイルを取り、置き換えます。置き換えは、**ブリッジを止めてから**行います。
   - Linux: `systemctl --user stop spellink-bridge` → ファイルを置き換え → `systemctl --user start spellink-bridge`
   - macOS・Windows: `spellink-bridge service uninstall` → 置き換え → `spellink-bridge service install`
   - `spellink-bridge version` で、新しい版になったか確かめます。
2. **プラグイン(端末):** Releases の新しい `Spellink.snplg` を、最初と同じ方法で `MyStyle` に入れ、端末から入れ直します。自分用のファイル(`Spellink-my-nomad.snplg`)ではなく、**Releases のもの**を使います。
3. **確かめる:** プラグインの設定に、`Spellink 0.1.0 ・ ブリッジ 0.1.0` のように 2 つの版が出ます。どちらかが古いと、案内が出ます。

## 一時的に止める
- Linux: `systemctl --user stop spellink-bridge`(また動かすときは `start`)
- macOS・Windows: `spellink-bridge service uninstall`

## やめる・消す
1. 端末を手放す・なくしたとき: `spellink-bridge device revoke my-nomad`(その端末だけが使えなくなります)。
2. 自動起動を外す: `spellink-bridge service uninstall`。
3. パソコンのデータを消す: 設定のフォルダ([場所](02-bridge.md#3-起動する))を削除します。設定・端末の登録・会話の記憶・文字データが消えます。
4. 端末のプラグインを、「設定 → アプリ → プラグイン」から削除します。
