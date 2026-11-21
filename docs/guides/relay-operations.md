# 運営の手順: 中継と名前サービスを VPS で動かす

対象: Spellink の公式の中継（ADR 0010）と名前サービス（ADR 0017）を、運営の VPS で動かす人。構成と理由は ADR 0019 と `architecture.md`。判断すること（運営するか、費用、規約、プライバシー）は `docs/notes/research/2026-11-20-relay-decisions.md`。

**状態（2026-11-21）: 実装と自動テストは済み。VPS・本物の Let's Encrypt・docker でのビルドは、未確認。** 下の手順は、初回の運用で確かめながら、直す。

## 1. 中継と名前サービスが見るもの・見ないもの
- 見る: 部屋の ID（鍵のハッシュ）、接続元の IP（TLS の本物の接続）、時刻、通信量。名前サービスは、ブリッジの ID と LAN の IP。
- 見ない: 質問・回答・鍵・端末の名前（端末とブリッジの間の Noise で暗号化。中継は、暗号文のフレームを流すだけ）。
- ログ: 中継は**クライアントの IP をログに書かない**（部屋 ID の先頭 8 文字と、接続・切断の出来事だけ）。名前サービスも、IP をログに書かない。

## 2. 用意するもの
- 固定の IP の VPS（Linux）と Docker Engine（Apache-2.0）か Podman。
- ドメイン（`spellink.uk`）の DNS を編集できること（Cloudflare でも、どこでもよい。**トークンは作らない**）。
- ファイアウォール（事業者のものと、ホストのもの）で開けるのは、**443/tcp、8443/tcp、53/tcp、53/udp、自分用の SSH**だけ。

## 3. 初回の手順
1. イメージを作る（VPS と同じ CPU で）: `docker build -f bridge/Dockerfile.edge -t spellink-edge bridge`
2. **DNS（ドメインの側）に、先にこの 2 つを置く**（Cloudflare なら、プロキシなし = 灰色の雲）:
   - `ns1.spellink.uk.  A  <VPS の IP>`
   - `lan.spellink.uk.  NS  ns1.spellink.uk.`（`lan` 以下の問い合わせを、名前サービスに任せる）
3. 名前サービスを一度だけ起動して、自分の確認名を知る: `docker compose -f deploy/vps/compose.yaml up names`。ログに、次のような行が出る。
   ```
   names identity 0123…（32 桁）
     _acme-challenge.names.spellink.uk.  CNAME  _acme-challenge.0123….lan.spellink.uk.
   ```
   この CNAME を、ドメインの DNS に置く（TXT を書くためのトークンの代わり。**これが、DNS に置く最後のもの**）。
4. 中継の確認名を知る: `docker compose -f deploy/vps/compose.yaml run --rm relay` を一度起動し、同じ形の行（`_acme-challenge.relay.spellink.uk. CNAME …`）が出るので、置く。
5. 本番の起動: `docker compose -f deploy/vps/compose.yaml up -d`。
6. 確認:
   - `dig @<VPS の IP> lan.spellink.uk NS`（権威の応答 `aa`）
   - `dig lan.spellink.uk NS`（委任が効いている）
   - `curl -s https://names.spellink.uk:8443/health`（`{"ok":true,"records":N}`）
   - `curl -s https://relay.spellink.uk/health`（`{"ok":true,"bridges":0}`）
   - 最初は Let's Encrypt の **staging** で試す: `compose.yaml` の `command` に `--ca staging` を足す（ブラウザは信頼しない証明書だが、手順の確認になる。本番の上限を使わない）。
7. ブリッジの `config.json`: `"relayUrl": "wss://relay.spellink.uk"`。直結を使うブリッジは `"lan": {"enabled": true}`（名前サービスが動いてから）。

初回に証明書が出るまで、数十秒かかる。出なければ、`docker compose logs` の `tls … preparing …` の行に理由が出る（待ち・再試行の時刻つき）。

## 4. 運用
- 限度（既定）: 接続元ごとの回数（端末 60／ブリッジ 12／受け口 120 回・分）、部屋数 10,000、部屋あたり 4 セッション、**部屋ごとの帯域 4 MiB/s（バースト 16 MiB。超えると遅くなる。切れはしない）**、**無通信 15 分でセッションを閉じる**。名前サービス: API は 60 回/分、DNS は 100 回/秒（接続元ごと）、ID は 10 万まで。
- 監視: 外から `/health` を見る（中継の `bridges` = 登録されている部屋の数、名前サービスの `records` = ID の数）。ログの `tls …: ready` / `preparing …` で証明書の状態。**scratch のコンテナにはシェルも `curl` もないので、コンテナの中の healthcheck は書けない**（外から見る）。
- 証明書: 自動で、残り 30 日で更新を始める。更新に失敗しても、古い証明書が切れるまで使い続ける。期限切れになると、握手を断る（古い証明書は出さない）。
- 更新（バイナリの入れ替え）: イメージを作り直して `docker compose up -d`。中継は状態を持たないので、ブリッジは自動で再接続する（数秒）。名前サービスの記録（`names-data` ボリューム）は残る。
- バックアップ: `names-data`（記録と、名前サービス自身の identity）と `relay-data`（中継の identity と証明書）。identity を失うと確認名が変わるので、CNAME を置き直す。
- 止めるとき: 利用者に先に知らせる（ブリッジの `relayUrl` を変えるだけで、自前の中継へ移れる）。

## 5. 障害のとき
| 症状 | 原因の候補 | 確認 |
|---|---|---|
| 端末が「登録されたブリッジが見つかりません」 | ブリッジが落ちている／パソコンがスリープ／中継が落ちている／ペアリングの解除 | 中継の `/health` の `bridges`。ブリッジのパソコンで `spellink-bridge device list` |
| 中継の TLS が通らない | 証明書が未取得・期限切れ | `docker compose logs relay` の `tls relay…` の行。CNAME が置いてあるか（`dig _acme-challenge.relay.spellink.uk CNAME`） |
| 証明書が取れない（`preparing`） | CNAME がない／委任（NS）が効いていない／Let's Encrypt の上限 | `dig @8.8.8.8 lan.spellink.uk NS`、ログの理由。上限は待ち時刻が出る |
| 家の中で直結にならない | ブリッジの `lan.enabled` が false／登録に失敗／ルーターの DNS リバインディング対策 | ブリッジのログの `lan: …`。`dig <id>.lan.spellink.uk`（プライベートの IP が返るか） |
| 429 | 回数制限（同じ接続元から多すぎる） | 接続元の IP が本物か（中継は直接 TLS なので、前段のヘッダは使わない） |
| 部屋が増え続ける | 乱用 | `/health` の `bridges` が 10,000 に近い。回数制限と、ファイアウォール |

## 6. 前段に TLS を話すものを置く場合（平文のモード）
Cloudflare Tunnel などの後ろで動かすなら、`spellink-bridge relay --listen 127.0.0.1:8780 --trust-proxy`（`deploy/relay/spellink-relay.service`）。`--trust-proxy` は、前段を通ったときだけ付ける（付けると、クライアントの住所をヘッダから取る。直接公開すると、ヘッダの偽装で回数制限を回避される）。自前の TLS（`--tls-domain`）とは併用できない。大きな攻撃を受けたときに、前段を戻す逃げ道。
