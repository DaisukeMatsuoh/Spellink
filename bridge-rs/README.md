# bridge-rs — Rust での計測用プロトタイプ

ブリッジの**核**（グリフ・ファイルの読み込み → 行の組み立て → ストロークの生成。Claude Code からの回答を流し込みながら書く部分）を Rust で作り、Go 版と**同じ入力で同じ出力になるか**、**どれだけ軽いか**を測るためのもの。製品の一部ではない（`experiments/`）。

## 何ができるか
| コマンド | 内容 |
|---|---|
| `compose --glyphs F --layout JSON [--seed N --slope S --full F --half H]` | 標準入力（1 行 1 個の JSON 文字列＝流れてくる文章の断片）から、完成した行を JSON で出す |
| `ask --glyphs F --layout JSON [--model M] 質問` | Go 版と同じ引数で公式の `claude` CLI を起動し、同じイベントを読み、回答が流れてくるそばから行にする |
| `bench` / `idle` | 速度とメモリの計測用 |

Go 側の `spellink-bridge compose|bench|idle`（`bridge/compose.go`）は、同じ入出力。

## 作っていないもの（核の外）
WebSocket サーバー、Noise の暗号化、台帳・ペアリング、中継、`/app` と `/pair`、履歴、設定、状態遷移表のコード生成、証明書の取得。移植するなら、ここが本体（下の「判断が要ること」）。

## 試験
```bash
cargo test --release     # Go が出した出力と 1 バイトも違わないことを含む（testdata/）
./measure.sh /path/to/spellink-bridge   # Go と Rust を同じ方法で測る
```
Go の出力の記録（`testdata/expected-go.jsonl`）を作り直すには、`bridge/` で `spellink-bridge testfont testdata/glyphs.bin` のあと、`spellink-bridge compose --glyphs testdata/glyphs.bin --layout "$(cat testdata/layout.json)" --seed 777 --slope -0.005 --full 1.1 --half 1.2 < testdata/input.jsonl`。
