# 0019. 運営の入口: Cloudflare Tunnel をやめ、中継と名前サービスが自分で TLS と DNS を話す

- 状態: Accepted（ユーザーが方針を決定 2026-11-21）。実装済み。**VPS・本物の証明書での確認は未**。
- 日付: 2026-11-21
- 前提: ADR 0010（中継）、0017（名前サービス）、0018（Rust・scratch）

## 決定
1. **運営の VPS（固定の IP がある。Kagoya）の入口は、中継と名前サービスの 2 つのコンテナ。Cloudflare Tunnel は使わない。** 外に開けるのは、443/tcp（中継）、8443/tcp（名前サービスの API）、53/udp+tcp（DNS）だけ。
2. **コンテナは `FROM scratch`**（シェル・パッケージ管理・libc がない。`bridge/Dockerfile.edge`）。実行は、非 root、読み取り専用、権限をすべて落とす、`no-new-privileges`、プロセス数とメモリの上限（`deploy/vps/compose.yaml`）。書くのは、それぞれのボリューム 1 つだけ。**2 つは別のコンテナ**（一方が破られても、もう一方へ広がらない）。同じバイナリ（`relay`・`names` のサブコマンド）。
3. **TLS は、中継と名前サービスが自分で話す。** 証明書は、どちらも Let's Encrypt から取る。確認は DNS-01 で、`_acme-challenge.<名前>` を、**運営が一度だけ、`lan` のゾーンの自分用の確認名への CNAME にしておく**。TXT は、名前サービスが自分の DNS で返す（中継は、名前サービスの API で TXT を置く。ブリッジの証明書と同じ仕組み）。**DNS を書くためのトークンは、どこにも存在しない。**
4. **Cloudflare に残るのは、`spellink.uk` の DNS の入れ物だけ**（A レコード、`lan` の NS、2 つの CNAME）。通信は通らない。
5. 中継の守り（接続元ごとの回数制限、部屋数、無通信の切断、帯域）は、そのまま。前段がなくなるので、`--trust-proxy` は付けない（付けるとエラー）。本物の接続元で回数制限が効く。TLS の握手と HTTP のヘッダの時間切れ（10 秒）、握手の同時数（128）、接続の総数（8192）を、TLS の受け口に持たせた。

## 理由
- 利用者が増えたとき、Cloudflare の無料の範囲（レコード数、トンネル）を気にしなくてよい。定額の VPS の通信量は気にならない。
- DNS 用のトークンが存在しないので、「トークンが漏れる」危険がない。
- 小さな静的バイナリ 1 つと scratch で、攻撃面を小さくする。

## 失うもの
- Cloudflare の DDoS の盾。**自分で守る**: 上の守りと、VPS のファイアウォール（上の 3 つのポートだけ）。大きな攻撃には、VPS の事業者側の対策か、後から前段を戻す（TLS を話さない平文のモード `relay --listen … --trust-proxy` は残してある）。
- 証明書の更新を、自分で（バイナリが）やる。失敗は、状態（`docs/spec/lan-cert.md`）として記録され、期限の 30 日前から更新を始める。

## 運営の初回の手順
`docs/guides/relay-operations.md`。要点: ドメインの DNS に（1）`ns1` の A、（2）`lan` の NS、（3）2 つの CNAME（`_acme-challenge.names`・`_acme-challenge.relay`）を置く。CNAME の宛先は、各コンテナが最初の起動で表示する。

## 結果
- ADR 0010 の「ホスティング」は、この決定で埋まる（Go ・ Cloudflare Tunnel の記述は、歴史として残す）。
- `Dockerfile.relay` は `Dockerfile.edge` に変わった。
