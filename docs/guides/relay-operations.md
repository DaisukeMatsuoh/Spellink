# 中継（リレー）を運営する人のための手順

対象: `spellink-bridge relay`（ADR 0010）を、自分で、または Spellink の公式として動かす人。
判断すべきこと（運営するか、費用、規約、プライバシー）は `docs/notes/research/2026-11-20-relay-decisions.md`。ここは、決めた後の**手順**。

## 1. 中継が見るもの・見ないもの（ポリシーの根拠）
- 見る: 部屋の ID（鍵のハッシュ）、接続元の IP（プロキシ・トンネル経由なら、そのヘッダの値）、時刻、通信量。
- 見ない: 質問・回答・鍵・端末の名前（すべて、端末とブリッジの間の Noise で暗号化されている。中継は、暗号文のフレームを流すだけ）。
- ログ: `spellink-bridge relay` は、**クライアントの IP をログに書かない**（部屋 ID の先頭 8 文字と、接続・切断の出来事だけ）。IP を残すのは、前段（Cloudflare など）のログだけ。前段の設定を確認すること。

## 2. 前提
- 入口は 1 つ: Cloudflare Tunnel（`cloudflared`）か、TLS を終端するリバースプロキシ。中継は `127.0.0.1` だけで待ち受け、外には直接出さない。
- TLS は前段が用意する（端末は、公的な CA の `wss://` しか受け付けない）。
- `-trust-proxy` は、**前段を通ったときだけ**付ける（付けると、クライアントの住所をヘッダから取る。直接公開すると、ヘッダの偽装で回数制限を回避される）。

## 3. 手順（Linux, systemd, Cloudflare Tunnel）
1. バイナリを `/usr/local/bin/spellink-bridge` に置く（リリースの `bridge-v*`。チェックサムを確認する）。
2. 専用ユーザー: `sudo useradd --system --no-create-home --shell /usr/sbin/nologin spellink-relay`
3. 単位ファイル: `deploy/relay/spellink-relay.service` を `/etc/systemd/system/` に置き、`sudo systemctl enable --now spellink-relay`。
4. `curl -s http://127.0.0.1:8780/health` が `{"ok":true,"bridges":0}` を返す。
5. Cloudflare Tunnel: `cloudflared tunnel create spellink-relay` → `~/.cloudflared/config.yml` の `ingress` に `hostname: relay.<ドメイン>` / `service: http://127.0.0.1:8780`（**https ではなく http**。502 になる）→ `cloudflared tunnel route dns spellink-relay relay.<ドメイン>` → `cloudflared service install`。
6. 確認: `curl -s https://relay.<ドメイン>/health`。
7. ブリッジの `config.json`: `"relayUrl": "wss://relay.<ドメイン>"`。

## 4. 運用
- 限度（既定）: 接続元ごとの回数（端末 60／ブリッジ 12／受け口 120 回・分）、部屋数 10,000、部屋あたり 4 セッション、**部屋ごとの帯域 4 MiB/s（バースト 16 MiB。超えると遅くなる。切れはしない）**、**無通信 15 分でセッションを閉じる**。値は `relay.Server` のフィールド（`MaxRooms`、`IdleTimeout`、`RoomBytesPerSec`、`RoomBurst`）。
- 監視: `/health` の `bridges`（いま登録されているブリッジ＝部屋の数）。`journalctl -u spellink-relay` に出るのは、`bridge up/down: room xxxxxxxx…` と `relay full`（部屋が上限）だけ。回数制限で断ったとき（429）は、ログに出ない。
- 更新: バイナリを入れ替えて `systemctl restart spellink-relay`。中継は状態を持たないので、ブリッジは自動で再接続する（数秒）。
- 止めるとき: 利用者に先に知らせる（ブリッジの `relayUrl` を変えるだけで、自前の中継へ移れる）。

## 5. 障害のとき
| 症状 | 原因の候補 | 確認 |
|---|---|---|
| 端末が「登録されたブリッジが見つかりません」 | ブリッジが落ちている／パソコンがスリープ／中継が落ちている／ペアリングの解除 | `/health` の `bridges`。ブリッジのパソコンで `spellink-bridge device list` |
| 502 | トンネルの `service` が https になっている | `config.yml` の `service` を `http://127.0.0.1:8780` に |
| 1033 | `cloudflared` が接続していない | `systemctl status cloudflared` |
| 429 | 回数制限（同じ住所から多すぎる） | 前段のヘッダの住所が正しいか（`-trust-proxy`） |
| 部屋が増え続ける | 乱用 | `/health` の `bridges` が 10,000 に近い。回数制限と、前段の WAF |
