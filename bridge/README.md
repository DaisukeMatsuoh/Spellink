# spellink-bridge（Rust）

Supernote のプラグインと AI（公式の Claude Code CLI）の間に立つ、利用者のパソコンで動く単一のプログラム。同じバイナリが中継（`relay`）も兼ねる。経緯と決定は ADR 0018（Go から移植）。

| コマンド | 内容 |
|---|---|
| `serve` | ブリッジを動かす（初回はグリフを作る） |
| `pair [out.json]` / `check [url…]` | 接続情報の表示、端末と同じ手順での疎通確認 |
| `device …` / `open` | 端末の追加・ペアリング・取り消し、アプリ画面を開く |
| `service install\|uninstall\|status` | パソコンの起動と一緒に動かす |
| `build-glyphs` | KanjiVG・Hershey から字のデータを作る |
| `relay …` | 家の外から使うための中継（自前の TLS、または TLS を話す前段の後ろ） |
| `names …` | 名前サービス（家の中の直結のための DNS と API。ADR 0017） |
| `preview TEXT OUT.png` | 端末なしで筆跡を確かめる |
| `compose` `ask` `bench` `idle` `testfont` | 計測・試験用 |

```bash
cargo test --locked                     # 単体・結合・上書き更新・Go が記録した筆跡との一致
cargo build --release --locked          # ./target/release/spellink-bridge
docker build -f Dockerfile.edge .       # 中継・名前サービスの、scratch の単一バイナリ
```

## 作り
- `src/` は、Go の `internal/<名前>` と同名のモジュール（`relay`・`securechan`・`devices`・`server`・`config`…）。表（`docs/spec/` の状態遷移表）は `plugin` の `npm run spec` が `src/server/run_table.rs`・`src/devices/table.rs` に出す。`match` が全マスを網羅するので、マスが欠けると通らない。
- `tests/` は、本物の入口（ポートに立てたサーバー、本物のバイナリとファイル）から試す。`tests/contract.rs` は、端末側のテストが検証する `testdata/frames.golden.json` と同じ形のフレームを出すことを確かめる。
- `assets/` は、`/pair` と `/app` の画面（バイナリに埋め込む）。`testdata/glyphs.bin` は、Go 版が作った実際のグリフで、筆跡が Go と1バイトも違わないことの確認に使う。
