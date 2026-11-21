# 中継を scratch コンテナで動かす: TLS をどこに置くか

> 状態: **決着** → ADR 0019。結論: 中継が自分で TLS を話す（案 C）。暗号は `aws-lc-rs`（末尾の追記）。scratch で動くことは、静的バイナリで確認済み。docker でのビルドは未確認。

日付: 2026-11-21。ユーザーの問い: 「Rust で TLS を使える scratch コンテナはできないか。Alpine は sh が入る。OpenSSL の代わりの Rust のライブラリで足りるはず」。

## 結論
**できる。** TLS は Rust 製の `rustls` で、OpenSSL（libssl）は要らない。Alpine は**ビルドの段階でだけ**使い、最終のイメージは `FROM scratch`（sh もライブラリもない）。いまの `bridge/Dockerfile.relay` がその形。

## 確かめたこと（この開発機、aarch64 Linux）
- `cargo build --release --locked --target aarch64-unknown-linux-musl`（ホストの gcc だけ）で、**静的リンクの 4.6 MB**（TLS 込みの全機能）ができた。`ldd` は「動的実行ファイルではありません」。
- そのバイナリを、`/etc` を空の tmpfs に差し替えて（`resolv.conf` だけ置く）、環境変数なしで動かし、公的な CA の `wss://bridge.spellink.uk/v1` に接続した。**TLS のハンドシェイクが通った**（その後、トークンがなく `unauthorized` で閉じられたのは想定どおり）。つまり、CA の束（`/etc/ssl`）も、シェルも、libc の動的ライブラリも要らない。信頼する CA は `webpki-roots` としてバイナリの中にある。
- 未確認: docker での実際のビルド（この機械に docker がない）。

## scratch で気をつけること
| 項目 | 内容 |
|---|---|
| CA の束 | 不要（`webpki-roots` が内蔵）。ただし**内蔵の CA は、バイナリを作り直さないと更新されない**。更新はリリースで追従する |
| 名前解決 | musl は `/etc/resolv.conf` だけ読む。docker が入れてくれる |
| ユーザー | `USER 65534:65534`（数字なら `/etc/passwd` は不要） |
| 書き込み | scratch は読み取り専用にできる（`--read-only`）。証明書やアカウント鍵を保存するなら、ボリュームを 1 つ |
| 低いポート | 非 root では 443 を開けない。`8443` で待って `-p 443:8443` にするのが簡単 |
| ビルドの段 | `rust:alpine`（musl-dev、gcc が要る。`ring` の C/アセンブリのため）。x86 から arm へのクロスは C のクロスコンパイラが要る。VPS と同じ CPU でビルドするのが簡単 |

## 暗号の部品（rustls の下）
| 部品 | 評価 |
|---|---|
| `ring`（いま） | C とアセンブリを含む。musl で静的に作れる。実績が大きい。**維持** |
| `aws-lc-rs` | 大きい。cmake が要る。FIPS が要るときだけ |
| 純粋な Rust の実装（RustCrypto 系の provider） | C なしで作れるが、rustls の公式の標準ではなく若い。**守りの要で使う理由が弱い**ので採らない |

C が混ざるのは**ビルド時だけ**。できあがりは外部のライブラリに依存しない。

## 中継の入口の TLS: 3 案
| 案 | 構成 | 長所 | 短所 |
|---|---|---|---|
| **A（いま）** 中継は平文の http、前段の Cloudflare Tunnel が TLS | `cloudflared` ＋ 中継（`127.0.0.1`）。ポートは一つも開けない | 証明書の管理がない。DDoS の盾になる。中継が 1.4 MB の最小 | Cloudflare に依存。Cloudflare が TLS を終端する（見えるのは暗号文と IP） |
| **B** 中継が rustls で TLS。証明書のファイルは外から渡す（`certbot`、`Caddy` が更新） | 中継に `--tls-cert/--tls-key`。ファイルを読み直す | Cloudflare なし。小さな追加 | 更新の仕組みを別に持つ（運営の手間）。コンテナが 1 つ増える |
| **C** 中継が ACME（Let's Encrypt）を自分で話す | `rustls-acme` 等で TLS-ALPN-01。証明書とアカウント鍵は 1 つのボリューム | **本当の単一バイナリ。外に何もない** | 依存が増える（約 +1 MB）。443 を直接公開するので、盾がない（接続の上限は中継に既にある）。自前の運営の責任が増える |

推奨: **公開の最初は A**（盾と手間の少なさ）。利用者が増えて Cloudflare を外したくなったら C。B は途中案で、単体では勧めない。C にするときは ADR を追加する。
いずれの案でも、**質問と回答は Noise で守られており、TLS を誰が終端しても中身は見えない**（`docs/guides/architecture.md`）。

## docker と Colima、ライセンス
- この開発機は Linux。**Docker Engine（Apache-2.0）をそのまま入れれば足りる**。Podman（Apache-2.0）でも同じ Dockerfile が動く。
- **Colima**（MIT）は、macOS（や Linux）で docker の動く VM を用意する道具。Mac で動かすときに使う。Docker Desktop は、一定規模以上の会社では有料なので、個人・小規模でも避けるなら Colima が無難。
- VPS では Docker Engine か Podman をそのまま使う。

## 追記（同日）: 暗号の部品を ring から aws-lc-rs に変更した
- 理由（ユーザーの指摘）: 保守の活発さ。ring は最後のリリースが 2025-03（0.17.14）、直近 6 か月のコミットは 24。aws-lc-rs は 2026-10 に 1.19.0、直近 6 か月で 100 以上。rustls の現在の既定も aws-lc-rs。
- 変更: `rustls` を `aws-lc-rs`、`ureq` を `rustls-no-provider`、`snow` から `std`（使わない ring を引き込んでいた）を外した。`cargo tree` に ring は残らない。cmake なしでビルドできた。
- 結果: 全機能のバイナリが 4.7 MB → 5.9 MB（musl 静的は 5.8 MB）。`cargo test` 312 件、clippy、fmt が通る。musl 静的を `/etc` 空の環境で動かし、公的 CA の wss のハンドシェイクが通る（ring のときと同じ確認）。
- 次点: graviola（C なしの純 Rust。0.x）。成熟したら Alpine なしのビルドのために再検討する。

## 追記（同日・後）: 決定
ユーザーが、Kagoya の VPS（固定 IP）で、**案 C（中継が自分で TLS を話す）**に決めた（ADR 0019）。上の「推奨: 最初は A」は、この決定で置き換わった。確認の DNS-01 は、`_acme-challenge.<名前>` を名前サービスの確認名への CNAME にする方式（DNS 用のトークンが存在しない）。
