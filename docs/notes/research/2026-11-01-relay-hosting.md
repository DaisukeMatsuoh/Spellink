# 中継サーバ（Go）をどこで動かすか（2026-11-01）

> 状態: **決着（2026-11-21）** → ADR 0019。結論: **Kagoya の VPS（固定 IP、定額）で、scratch のコンテナ**。Cloudflare Tunnel は使わず、中継が自分で TLS を話す。この文書の「Go」「開発機＋Cloudflare Tunnel」「Oracle・Workers の比較」は、歴史として残す（採らなかった）。

中継サーバ（`spellink-bridge relay`）は、暗号化された WebSocket のフレームを右から左へ流すだけなので、とても軽い。
CPU・メモリはほぼ使わず、効いてくるのは**通信量**と**常時接続の数**（ブリッジ 1 台につき 1 本）だけ。

## 候補
| 候補 | 月額 | 長所 | 注意点 |
|---|---|---|---|
| **自宅サーバ / この ROCK 5B+ ＋ Cloudflare Tunnel** | **0 円**（独自ドメイン代のみ。年 1,000〜2,000 円程度） | ポート開放不要、自宅の IP を公開しない、TLS も Cloudflare が用意。Go の中継をそのまま動かせる | 自宅の回線・停電に左右される。Cloudflare の無料プランは「同時 WebSocket 数が少ない範囲」を想定（数百〜数千本なら問題ないとの報告。公式の上限値は非公開）。Cloudflare は TLS を終端するが、Spellink は端末⇔ブリッジ間を Noise で暗号化しているので中身は読めない |
| Oracle Cloud Always Free（ARM） | 0 円 | 本物のサーバが無料。Go の ARM バイナリがそのまま動く | 2026 年に無料枠が 2 OCPU / 12GB に半減したと報じられている。**CPU・通信・メモリがすべて 20% 未満の状態が 7 日続くと「アイドル」として回収される**規定があり、軽すぎる中継は回収対象になり得る（有料アカウントに切り替えると回避できるとの報告あり。要確認） |
| Google Cloud e2-micro（無料枠） | 0 円 | 無料で常時稼働 | 米国 3 リージョン限定、**外向き通信が月 1GB まで**。回答 1 回で数十〜数百 KB 流れるので、少人数でもすぐ超える |
| Hetzner CAX11（ARM） | 約 €6 | 安定、EU は通信量が多い | 2026 年に値上げ・在庫不足の報告。有料 |
| Cloudflare Workers ＋ Durable Objects | 無料〜$5 | 運用不要、世界中の拠点 | Go の中継はそのまま動かない（JS/TS で書き直し）。ADR 0008/0010 の方針から外れる |

## おすすめ
**フェーズ 1（今〜身近な利用者）: この ROCK 5B+（または自宅サーバ）＋ Cloudflare Tunnel。**
- 費用はドメインだけ。手持ちのドメインがあればサブドメイン（例: `relay.<あなたのドメイン>`）で足りる。
- 構成: `spellink-bridge relay -listen 127.0.0.1:8780` を常駐 → `cloudflared` で `relay.<domain>` → `http://127.0.0.1:8780` に転送。
  各ブリッジの `config.json` に `"relayUrl": "wss://relay.<domain>"` を書き、`pair` し直す。
- 運営者（あなた）が一度設定するだけで、利用者は何もしなくてよい。

**フェーズ 2（公開・利用者が増えたら）:** 通信量と同時接続数を見て、Oracle（有料アカウント化して回収を避ける）か安価な VPS に移す。
中継は状態を持たない（再起動してもブリッジが自動で再接続する）ので、移転は DNS の向け先を変えるだけ。

## 運営者がやること（フェーズ 1）
1. Cloudflare にドメインを追加（既存ドメインのネームサーバを Cloudflare に向ける、または Cloudflare で取得）
2. この機械に `cloudflared` を入れ、`cloudflared tunnel login` → `cloudflared tunnel create spellink-relay` → DNS に `relay` を割り当て
3. 中継と `cloudflared` を systemd で常駐させる
（ブラウザでのログインが必要な手順があるので、2 はユーザーが行う。手順書と systemd ユニットはエージェントが用意する）

## 出典
- Oracle の無料枠縮小: https://www.infoq.com/news/2026/07/oracle-cloud-free-tier-limits/ ／ アイドル回収: https://docs.oracle.com/en-us/iaas/Content/FreeTier/freetier_topic-Always_Free_Resources.htm
- Hetzner の価格・在庫: https://vpsdex.com/en/activity/hetzner-cloud-pricing-2026/ ／ https://stackvaluelab.com/hetzner-cx-cax-unavailable/
- GCP 無料枠（e2-micro・1GB）: https://cloudwebschool.com/docs/gcp/fundamentals/free-tier/
- Cloudflare の WebSocket: https://developers.cloudflare.com/network/websockets/ ／ https://community.cloudflare.com/t/websocket-limits/56946
