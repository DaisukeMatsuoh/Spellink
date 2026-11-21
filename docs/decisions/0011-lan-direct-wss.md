# 0011. 家の中は Let's Encrypt 証明書つきの wss:// で直接つなぐ

- 状態: Superseded by 0017（2026-11-21。利用者に Cloudflare のトークンとドメインを用意させる方式は、一般の利用者に難しい。Rust への移植でも移さなかった）
- 日付: 2026-11-01

## 背景
- Supernote のプラグインは平文（http/ws）と信頼されない証明書を拒否する（ADR 0007 追記）。
- トンネル経由は毎回 Cloudflare を通り、接続に 0.5〜2 秒かかる。家の中なら直接つなげたい。

## 決定
ブリッジに Let's Encrypt（DNS-01）を組み込み、家用の名前（例 `rock.lan.<ドメイン>`）で wss:// を待ち受ける。
- 名前は Cloudflare DNS に **A レコードで LAN の IP** を公開する（ブリッジが API で作成・更新）。
- 証明書は DNS-01 で取得・自動更新。**ポート開放は不要**。実装は `certmagic` + `libdns/cloudflare`（`bridge/internal/lantls`）。
- ペアリングの URL 一覧の先頭に `wss://<host>:8766/v1` を足す。プラグインは全 URL に同時接続し、家では直接が勝つ。
- トンネル／中継経由も残す（外出先用）。

## 設定（config.json）
```json
"lan": { "host": "rock.lan.example.com", "cloudflareToken": "<Zone:DNS:Edit のトークン>", "email": "任意" }
```
`zone`（`co.jp` など複数ラベルのとき）、`ip`（自動検出を上書き）も指定可。

## 注意
- API トークンの権限は **Zone → DNS → 編集、対象ドメイン 1 つだけ**。`config.json` は 0600。
- ルーターの DNS リバインディング対策が「公開名 → プライベート IP」を遮断する場合がある（例外設定が必要）。
  Unbound/Firewalla に内部ドメインがあるなら、同じ名前を内部 DNS にも置ける。
- 公開 DNS に LAN の IP が載る（秘密ではないが、見える）。
- バイナリが約 7MB → 約 14MB に増えた（certmagic・zap・libdns）。
- 実証明書での発行は未確認（ドメインとトークンが必要）。確認後に Accepted にする。

## 追記（2026-11-07）: 状態の整理
- 実証明書での確認に必要なもの: ドメイン（`spellink.uk` を Cloudflare Registrar で取得済み）と、Cloudflare の API トークン（Zone → DNS → 編集、このドメインのみ）。トークンを `config.json` の `lan` に入れたら確認できる。
- 優先度は低い（トンネル経由で動いている。家の中の遅延を減らすための経路）。


## 追記（2026-11-21）: 置き換え
ADR 0017（名前サービスが DNS を持ち、各ブリッジが自分で証明書を取る）に置き換えた。利用者の Cloudflare トークンは不要になり、`lan.host`・`lan.cloudflareToken` は使われない（読めるが、ログに「使わない」と出る）。この ADR の Go 実装（`certmagic`）は git の履歴にある。
