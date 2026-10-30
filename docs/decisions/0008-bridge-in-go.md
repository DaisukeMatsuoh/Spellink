# 0008. ブリッジを Go で実装し、OS・アーキテクチャごとの単一バイナリで配る

- 状態: Accepted
- 日付: 2026-10-31

## 背景
ブリッジは利用者の PC に常駐する。最初の実装（Node.js + TypeScript）は動いたが、常駐プロセスとしては重かった。

| 計測（ROCK 5B+、待機時） | Node 版 | Go 版 |
|---|---|---|
| メモリ（RSS） | 約 129 MB | **約 17〜19 MB** |
| 実行に必要なディスク | Node 本体 268 MB + 依存 27 MB + 文字データ 13 MB | **バイナリ約 7 MB + 文字データ 2.1 MB** |
| 前提ソフト | Node.js 22.18 以上 | なし |

## 決定
Go で書き直す。外部依存は WebSocket の `github.com/coder/websocket` だけで、あとは標準ライブラリを使う。
文字データはデルタ符号化＋gzip の独自バイナリ形式（`glyphs.bin`）にして、初回起動時に自動で作る。

## Rust ではなく Go にした理由
- **クロスコンパイルが容易**: cgo を使わなければ、この ARM Linux 機から macOS（arm64/amd64）・Linux・Windows 向けをそのままビルドできる（確認済み）。
  Rust で macOS 向けを Linux からビルドするには追加の SDK やツールが要る。
- 必要な機能（HTTP、WebSocket、JSON、子プロセス、gzip、暗号）がほぼ標準ライブラリで揃う。
- メモリはランタイム分だけ Rust より数 MB 多いが、常駐プロセスとして十分に小さい。

## 結果
- `bridge/scripts/build.sh` で各ターゲットを `dist/` にビルドし、`SHA256SUMS` を作る。
- CI（`.github/workflows/bridge.yml`）: push/PR でテスト、`bridge-v*` タグで全ターゲットをビルドして GitHub Release に添付。
- 常駐化の雛形: `bridge/contrib/`（systemd ユーザーサービス、launchd エージェント）。
- macOS: Apple Silicon 向けバイナリは Go のリンカがアドホック署名する。ブラウザでダウンロードした場合は Gatekeeper の隔離属性が付くので、
  配布時は公証（notarization）するか、手順に `xattr -d com.apple.quarantine` を書く。
- Windows 向けもビルドはできるが未検証（`claude` が `.cmd` の場合は設定で実行ファイルのパスを指定する必要がある見込み）。
- Node 版は削除した（`git` 履歴にはない。最初のコミット前に置き換えたため）。
