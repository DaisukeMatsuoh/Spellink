# 0011. 家の中は Let's Encrypt 証明書つきの wss:// で直接つなぐ

- 状態: Proposed（実装済み・実証明書での確認待ち）
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
