> **この手順の位置づけ（2026-11-21）:** 利用者が**自分で**トンネルを用意する経路①（上級者向け）の手順。Spellink の運営の入口は Cloudflare Tunnel を使わない（ADR 0019、`relay-operations.md`）。一般の利用者は、中継（経路②）と、家の中の直結（経路③。ADR 0017）で足りる。

# Cloudflare の名前付きトンネルを設定する手順書

目的: Supernote から、いつでも同じ URL（例 `wss://spellink.example.com/v1`）で、自宅のブリッジにつなげるようにする。
今使っている「クイックトンネル」（`*.trycloudflare.com`）は、数十秒おきに切れ、URL も毎回変わる。名前付きトンネルは、固定の URL で、安定している。

所要時間: 30〜60 分（ネームサーバーの切り替え待ちを除く）。費用: Cloudflare は無料プランのまま。ドメイン代は別。

---

## 0. 先に決めること：どのドメインを使うか

**無料プランでは、ドメイン全体の DNS を Cloudflare に移す方式（Full setup）しか使えない。**
サブドメインだけを Cloudflare に任せる方式（subdomain setup）と、CNAME だけを置く方式（partial setup）は、Enterprise／Business 以上の機能。
（出典: [Cloudflare DNS — Zone setups](https://developers.cloudflare.com/dns/zone-setups/)）

つまり、選択肢は次の 2 つ。

| | A. 持っているドメインを丸ごと移す | B. この用途専用のドメインを別に取る |
|---|---|---|
| 内容 | そのドメインの DNS 管理を Cloudflare に引っ越す | 安いドメインを新しく 1 つ取り、それだけ Cloudflare に置く |
| 向いている場合 | そのドメインで、メール・Web サイトなどを使っていない／使っていても移行できる | 今のドメインで、メールや Web が動いている／触りたくない |
| リスク | **既存のメール・Web の設定を移し間違えると止まる**（§1 の確認が必須） | なし（既存に触れない） |
| 追加費用 | なし | ドメイン代（`.com` で年 1,500 円前後。Cloudflare Registrar なら原価） |

迷ったら **B を勧める**。失敗しても何も壊れず、ドメイン全体を移す心配が要らない。

どちらでも、Cloudflare に移したドメインは、次の 2 つに使える。
- 固定 URL のトンネル（この手順書）
- 将来の「家の中の直結 wss」（ADR 0011。Let's Encrypt の証明書を、Cloudflare の DNS で取る）

内部 DNS（Firewalla／Unbound の内部ドメイン）は、そのまま使える。Cloudflare に移すのは**公開側の DNS**だけ。

---

## 1. ドメインを Cloudflare に追加する（A の場合。B の場合は 1-1 から）

### 1-0. 移す前の確認（A のみ。ここを飛ばさない）
1. 今のドメインの DNS 設定画面（レジストラや DNS サービス）を開き、**全レコードを書き出す**（スクリーンショットで可）。
   特に次の種類が残っているか。
   - `MX`（メール）、`TXT`（SPF・DKIM・DMARC・各種サービスの認証）、`CNAME`、`A`／`AAAA`（Web サイト）
2. **DNSSEC が有効なら、先に無効にする**。有効のままネームサーバーを変えると、そのドメインに繋がらなくなる。
   レジストラの DNSSEC 設定で、DS レコードを削除する（反映に最大 24 時間かかることがある）。
3. 可能なら、今のレコードの TTL を短く（300 秒）しておく。

### 1-1. サイトを追加する
1. [Cloudflare ダッシュボード](https://dash.cloudflare.com/)にログインする。
2. **Add a domain**（ドメインを追加）→ ドメイン名を入力 → **Continue**。
3. プランは **Free** を選ぶ。
4. Cloudflare が、既存の DNS レコードを自動で読み取る。**§1-0 で書き出したレコードと、1 件ずつ見比べる**。足りないものは手で追加する。
   - メール（`MX`・`TXT`）のレコードは、**プロキシ（オレンジの雲）をオフ（灰色の雲、DNS only）**にする。メールのレコードをプロキシすると届かなくなる。
5. 画面に **2 つのネームサーバー**（例 `xxx.ns.cloudflare.com`）が表示される。控える。

### 1-2. ネームサーバーを切り替える
1. ドメインを管理しているレジストラ（お名前.com、Google Domains／Squarespace、Cloudflare Registrar など）の画面を開く。
2. 「ネームサーバー」を、今のものから **Cloudflare の 2 つ**に変更する。
3. Cloudflare の画面に戻り **Done, check nameservers** を押す。
4. ステータスが **Active** になれば完了（数分〜24 時間。メールが届く通知が来る）。
   Active になるまで、既存のメール・Web は今まで通り動く。

---

## 2. トンネルを作る（ダッシュボード方式。コマンド操作が少ない）

Cloudflare は、ダッシュボードから作る「リモート管理のトンネル」を推奨している。
（出典: [Create a remotely-managed tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/get-started/create-remote-tunnel/)）

1. ダッシュボードの **Networking → Tunnels** を開く（見つからなければ、**Zero Trust → Networks → Tunnels**）。
   初めてなら、Zero Trust の初期設定を求められることがある。**Free プラン**を選んで進める。
2. **Create a tunnel** → 種類は **Cloudflared** → 名前を付ける（例 `spellink-rock`）→ **Save tunnel**。
3. 「Install and run a connector」の画面で、**Debian**、**arm64**（このマシンは ROCK 5B+）を選ぶ。
   下に、`cloudflared service install eyJhIjoi…` のようなコマンドが出る。**この長い文字列（トークン）がトンネルの鍵**。
   - **トークンは他人に見せない**（チャットにも貼らない）。漏れたら、誰でもこのトンネルに接続できる。
   - 漏れた・見せてしまった場合は、トンネルのメニューから **Refresh token** で無効にできる。
4. このマシンでの実行は、次のどちらか。

   **(a) 公式の方法（sudo が使える）**: 画面のコマンドをそのまま、ターミナルに貼って実行する。常駐サービスとして登録される。

   **(b) ユーザー権限で動かす方法（sudo 不要。こちらを推奨）**: cloudflared は、すでに `~/.local/bin/cloudflared` に入れてある。
   ```bash
   # 1) トークンを、自分だけが読めるファイルに保存する（eyJhIjoi… は画面の長い文字列）
   install -m 600 /dev/null ~/.config/spellink-bridge/cloudflared.env
   echo 'TUNNEL_TOKEN=eyJhIjoi...' > ~/.config/spellink-bridge/cloudflared.env

   # 2) 常駐させる
   mkdir -p ~/.config/systemd/user
   cp ~/GitWorkspaces/Spellink/bridge/contrib/cloudflared-spellink.service ~/.config/systemd/user/
   systemctl --user daemon-reload
   systemctl --user enable --now cloudflared-spellink
   loginctl enable-linger $USER      # ログアウトしても動かし続ける
   ```
5. ダッシュボードで、コネクターが **Connected**、トンネルが **Healthy** になれば成功。
   つながらないときは、このマシンから外向きの **TCP/UDP 7844** が通るか確認する。

---

## 3. 公開ホスト名を割り当てる

1. 作ったトンネルを開き、**Routes**（または **Public Hostname**）→ **Add route** → **Published application**。
2. 次のように入力する。

   | 項目 | 値 |
   |---|---|
   | Subdomain | `spellink`（好きな名前でよい） |
   | Domain | 追加したドメイン（ドロップダウンから選ぶ） |
   | Path | 空欄 |
   | Service type | `HTTP` |
   | URL | `localhost:8765`（ブリッジのポート） |

3. 保存する。DNS のレコード（`spellink.<ドメイン>` → トンネル）は、**Cloudflare が自動で作る**。手で DNS を触る必要はない。
4. WebSocket は、追加設定なしで通る。

### 動作確認
ブリッジが動いている状態で、ブラウザかターミナルで開く。
```bash
curl https://spellink.<あなたのドメイン>/health
# → {"ok":true,"protocol":1}
```
- `502`／`Bad gateway`: ブリッジが動いていない、またはポートが違う。
- `1033`: コネクターが接続していない（§2 の確認）。
- 名前が引けない: ネームサーバーの切り替えが完了していない（§1-2）。

---

## 4. Spellink に教える（エージェントがやる作業）

動作確認ができたら、ドメイン名（例 `spellink.example.com`）を伝える。次の作業は、こちらでやる。
1. `~/.config/spellink-bridge/config.json` の `extraUrls` に `wss://spellink.<ドメイン>/v1` を追加。
2. ブリッジを再起動し、`spellink-bridge pair` でペアリングファイルを作り直す。
3. プラグインを、新しい URL で作り直す（`Spellink.snplg`）。
4. このマシンで、トンネル経由の質問が通ることを確認する。
5. クイックトンネルを止める。
6. ブリッジ本体も、systemd のユーザーサービスにして、再起動後も自動で立ち上がるようにする（`bridge/contrib/spellink-bridge.service`）。

プラグインは、LAN・名前付きトンネルの URL に、同時に接続を試す。つながった方が使われる。

---

## 5. 安全面の注意

- トンネルを作ると、ブリッジが**インターネットから届く状態**になる。守りは次の 3 つ。
  - **トークン**（192 ビット。推測は現実的でない）
  - **端末とブリッジの間の暗号化**（Noise。Cloudflare にも中身は読めない）
  - **暗号化なしの接続の拒否**（トンネル経由は、暗号化の握手ができない接続を受け付けない）
- ペアリングファイルとトンネルのトークンは、人に渡さない。
- トークンが漏れたら:
  - トンネル → ダッシュボードの **Refresh token**
  - ブリッジのトークン → `config.json` の `token` を消してブリッジを再起動（新しいトークンができる）→ ペアリングをやり直す
- 迷惑なアクセスが気になるときは、Cloudflare の **Security → WAF → Rate limiting** で、`spellink.<ドメイン>` への連続アクセスを制限できる（無料プランでも基本的なルールは使える）。

## 6. 困ったとき

| 症状 | 確認すること |
|---|---|
| ダッシュボードのトンネルが Down | `systemctl --user status cloudflared-spellink`。トークンの貼り間違い、7844 の外向き通信 |
| `curl` が 502 | `curl http://localhost:8765/health` が通るか（ブリッジの起動） |
| 端末で `closed (1006)` | 回答中に切れた。ブリッジが送り終えていれば、残りは書かれる。頻発するなら、トンネルのログ（`journalctl --user -u cloudflared-spellink`） |
| ネームサーバーが Active にならない | レジストラの設定が反映されたか。DNSSEC が残っていないか |
| メールが届かなくなった（A の場合） | §1-1 の手順 4。`MX` を DNS only にしたか。レコードが漏れていないか |

## 7. 将来: 家の中の直結（任意）

ドメインが Cloudflare にあれば、ADR 0011 の「家の中は直結の wss」も使える。
- 家の中では、トンネルを経由せず、直接つなげる（毎回 Cloudflare を通らないので速い）。
- 必要なもの: Cloudflare の **API トークン**（権限は **Zone → DNS → 編集**、対象は**このドメイン 1 つだけ**）。
- 作り方: ダッシュボード右上のプロフィール → **My Profile → API Tokens → Create Token → Edit zone DNS** のテンプレート → Zone Resources で対象ドメインを選ぶ → 作成。
- 必要になったら、教えてほしい。設定はこちらでやる。
