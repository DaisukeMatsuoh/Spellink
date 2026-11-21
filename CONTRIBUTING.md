# Contributing / 開発への参加

Thank you for helping. This project is small and works from a written specification; the rules below keep changes safe.
プロジェクトは小さく、文書にした仕様に従って作っています。変更を安全にするための決まりです。

## 開発の準備

| 何を | どうやって |
|---|---|
| ブリッジ（Rust、edition 2024） | `cd bridge && cargo clippy --all-targets -- -D warnings && cargo test --locked` |
| プラグイン（Node 22+） | `cd plugin && npm ci && npm run spec && npx tsc --noEmit && npx jest` |
| 端末側の統合テスト | `plugin/__tests__/integration.test.ts`。`cargo` が入っていれば、本物のブリッジをビルドして動かす（なければスキップ） |
| プラグインのビルド | `cd plugin && npm run build` → `build/outputs/Spellink.snplg` |

実機（Supernote）がなくても、ほとんどの変更はテストで確かめられる。SDK の呼び出しは、`plugin/__mocks__/sn-plugin-lib.ts` の偽のノートで再現している。

## 守ること（詳しくは [ADR 0014](docs/decisions/0014-engineering-policy.md)）

1. **表が先。** 状態を持つ処理（質問の流れ、波線、ページ追加と書き込み、ブリッジの run）は、`docs/spec/` の状態遷移表が元データ。
   表を直す → `cd plugin && npm run spec`（生成されたコードの表を作り直す）→ 足りない処理を書く → テスト。
   `*.generated.ts` と `runtable_gen.go` は手で直さない。文書とコードの表が食い違うと、テストが失敗する。
2. **戻り値は必ず確認する。** 成功か失敗かだけでなく、型・範囲・長さ・個数まで。`success` と `result` が別の SDK では `result` も。成功は結果で確かめる。
3. **信頼境界の値は、受け取る側が検証する**（SDK、他プロセス、ネットワーク、ディスク、利用者の入力）。一覧は `docs/spec/interfaces.md`。
4. 場当たり的な再試行や、回数・時間での帳尻合わせをしない。識別は ID と通し番号で。
5. すべての外部呼び出しに時間切れを付ける。無応答を別の失敗と取り違えない。

## テストについて

- 件数だけでなく、**スイートの数**を見る（スイートが落ちると件数が減る）。
- **本番の入口から**通すテストを書く（ラッソなら `AskFlow.run()`）。診断用の入口だけを通すテストは、見逃しを生む。
- 新しいテストは、**壊して落ちることを一度確かめる**。
- プロトコルを変えたら、`cd bridge && UPDATE_GOLDEN=1 cargo test --test contract` で記録を更新し、端末側のテスト（`contract.test.ts`）を通す。

## 変更の出し方

- 設計の判断は ADR（`docs/decisions/`）に、調査は `docs/notes/`、仕様は `docs/spec/` に。ドキュメントは日本語（公開の README は英語と日本語）。
- コミットメッセージは、何を・なぜ。1 つの変更は 1 つのコミット。
- 秘密（トークン、API キー、ペアリング情報、`bridges.local.json`）は、コミットしない。
- 知財の分析は、`docs/notes/ip/private/`（git 管理外）にだけ書く。

## 禁じていること

- AI の認証情報（ログイン、トークン）を、プラグインやブリッジが扱う変更。公式の CLI をそのまま使う、という方針（[ADR 0005](docs/decisions/0005-claude-code-bridge-on-user-machine.md)）は、変えない。
- 画像を外部に送る変更（ストロークから意味を取る方針）。

## ライセンス

Apache-2.0。変更を送ることは、同じライセンスで公開することへの同意とする。
