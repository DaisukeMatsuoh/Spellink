# Spellink

[English](README.md) · **日本語**

Supernote のノートで、手書きの質問をラッソで囲んで「AI に聞く」を押すと、**回答が手書きの筆跡として**ページに書かれます。同じノートに続きを書くと、会話が続きます。

> **状態: 初期のアルファ版。** Supernote Nomad と、Linux のパソコンのブリッジで、端末から回答までは動きます。実機での確認が続いています。他の人が使える形には、まだなっていません。計画は [docs/notes/progress/release-plan.md](docs/notes/progress/release-plan.md)。

## 使ってみる
**[利用者マニュアル](docs/manual/ja/README.md)**から始めてください。ブリッジを入れ、端末にプラグインを入れ、ペアリングして、質問するまでを、初めての人向けに順に説明しています(30 分ほど)。

## 仕組み
```
Supernote (プラグイン)  ── wss ──▶  spellink-bridge  (あなたの Mac / Linux / Windows のパソコン)
  ラッソ → 端末内で文字認識          └─ AI に聞く(現状は公式の Claude Code。あなた自身の契約でログイン済みのもの)
  ◀─ 回答を筆跡で ──                └─ 回答を手書きの筆跡に変える
```
- `plugin/` — Supernote のプラグイン(React Native)。文字認識は端末の中で行います。
- `bridge/` — Rust の小さなプログラム(OS ごとに 1 つ。約 4 MB)。**あなたのパソコンで動かします。** AI の認証情報は見ず、ログイン済みの Claude Code をそのまま呼び出します([ADR 0005](docs/decisions/0005-claude-code-bridge-on-user-machine.md))。
- 端末は、公的な証明書つきの `wss://` にだけつながります。道は 3 つ: **Spellink の中継**(運営が準備中・寄付で運営)、**家の直結**、**自分のトンネル**(任意・上級者向け)。通信はすべて端末とブリッジの間で暗号化されます([構成](docs/guides/architecture.md))。
- `docs/` — 決定(ADR)、仕様(状態遷移表)、ガイド、進捗。

## 必要なもの
- Supernote Nomad(確認済み)または Manta。
- 起動したままのパソコンと、ログイン済みの Claude Code(あなた自身の契約。API キーは不要)。
- 端末から `wss://` で届く道([マニュアル 3 章](docs/manual/ja/03-device.md))。

## 自分でビルドする(開発者向け)
```bash
cd bridge
cargo build --release --locked             # Rust (rustup.rs)
```
設置ファイルを作るブリッジには、プラグインが入っている必要があります。`bridge/scripts/build-with-plugin.sh` を使います。

## 安全とプライバシー
- **ペアリングのトークンは、ブリッジの鍵です。** 人に見せないでください([SECURITY.md](SECURITY.md))。
- AI に送るのは、認識した文字と、同じノートの前の会話だけです。**画像は送りません。**
- 保存するのは、あなたのパソコンだけです(設定と会話。**暗号化なし**)。端末は何も保存しません。
- 詳しくは [マニュアル 7 章](docs/manual/ja/07-privacy.md)。

## 開発に参加する
個人が余暇に運営しています。返事は遅れることがあります(ベストエフォート)。[CONTRIBUTING.md](CONTRIBUTING.md)。**翻訳の修正・追加も歓迎します**(英語版のマニュアルは AI の助けで作ったため、直していただけると助かります)。

## ライセンス・クレジット
Apache-2.0([LICENSE](LICENSE))。手書きの字形は [KanjiVG](https://kanjivg.tagaini.net)(CC BY-SA 3.0。**セットアップ時にあなたのパソコンでダウンロードし、ここでは再配布しません**)。そのほか [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
Anthropic、Ratta とは無関係です。
