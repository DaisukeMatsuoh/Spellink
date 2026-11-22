# GitHub の運用(公開リポジトリとして、人に使ってもらうために)

対象: 初めて、他人が使うオープンソースを公開する人(運営者)。**公開の直後にやる設定、CI の注意、プルリクエストと問題報告の受け方、事故の防ぎ方。** この文書は、チェックリストとして使う。確認できていない点は、そう書く。

## 1. 公開の順序(最初の 1 回)
1. `main` に push する(この時点で、全履歴が世界に見える。gitleaks は、直前にもう一度流す)。
2. **Actions の最初の実行を見る。** CI は、一度も動いたことがない。落ちるジョブがあれば、直す(下の 6)。緑になるまで、`main` の保護(3)は、まだかけない(自分の修正が通らなくなる)。
3. 設定(2〜4)を入れる。
4. タグ `edge-v0.1.0` を付けて push → イメージが `ghcr.io` に出る。**最初は非公開なので、パッケージの設定で、一度だけ「公開」に変える**(バイナリに秘密は入っていない)。
5. リリースのタグ(`bridge-v…`、`plugin-v…`)は、CI が緑で、実機で確かめてから。**付けたタグは、付け直さない**(取得した人の手元と食い違う)。直すなら、次の版を出す。
6. 告知は、最後。

## 2. リポジトリの設定(Settings)
| 場所 | 設定 | 理由 |
|---|---|---|
| General | Issues: オン。Wikis・Projects: オフ。Discussions: **オン**(質問はこちら、不具合は Issues と分ける) | 質問と不具合が混ざると、どちらにも返せなくなる |
| General › Pull Requests | **Squash merge だけ**を許可。マージ後にブランチを自動で消す | 履歴が読みやすい |
| Code security | **Dependabot alerts**、**Dependabot security updates**、**Secret scanning**、**Push protection**、**Private vulnerability reporting** を、すべてオン。(公開リポジトリは、すべて無料) | 依存の脆弱性、秘密の誤 push、非公開の報告の窓口 |
| Code security | Code scanning(CodeQL の「既定」の設定)は、オンにしてよい | 対応言語は、JavaScript/TypeScript など。Rust への対応は、最新の状況を確認する |
| Actions › General | **Fork pull request workflows**: 「**Require approval for first-time contributors**」以上(慎重なら「all outside collaborators」)。**Workflow permissions**: 「**Read repository contents**」(読み取りだけ) | 他人の PR が、承認なしに動かない。ワークフローが、勝手に書き込めない |
| Rules(ブランチ) | `main` を保護(下の 3) | 事故の防止 |

## 3. `main` の保護
- 直接 push を禁止し、**プルリクエスト経由**にする。強制 push と削除を禁止。
- 必須のチェック: `bridge`(test)、`plugin`(test)。(**一度、CI が緑になってから**指定する。)
- **承認の必須は、いまはかけない。** 運営者が 1 人だと、自分の PR を自分で承認できず、詰まる。管理者の例外(bypass)は、緊急用に残す。共同作業者が増えたら、承認 1 件を必須にする。
- `CODEOWNERS` を置いてある(全部、運営者)。共同作業者が増えたら、領域ごとに分ける。

## 4. Dependabot
- **公開リポジトリで使うことに、問題はありません。無料で、使うべき機能です。**(以前、「公開では避ける」と言った記録は、私にはありません。そのような一般的な決まりも、ありません。)
- 3 つの機能がある。**alerts**(脆弱性の通知)、**security updates**(脆弱性を直す PR を自動で作る)、**version updates**(新しい版への PR。`.github/dependabot.yml` で設定する)。
- 設定は `.github/dependabot.yml`: Rust(`/bridge`)、npm(`/plugin`)、GitHub Actions、Docker を、**週 1 回**、**グループにまとめて**。プラグインは、Supernote の SDK が前提とする React Native に合わせるので、`react`・`react-native` などは、**パッチ以外を、自動では上げない**。
- Dependabot の PR も、CI が動く。**緑で、中身を読んでからマージする**(依存の更新は、供給網の入口)。**自動マージは、使わない。**
- Dependabot の PR には、秘密が渡らない。いまの CI は、秘密を使っていないので、問題ない。
- `cargo audit` と `cargo deny`(`bridge.yml`)と、Dependabot alerts は、重なる。重なっても害はない。

## 5. CI(GitHub Actions)で気をつけること
- **公開リポジトリの Actions は、無料**(標準の実行環境)。ただし、macOS の実行環境は、実行時間の計算が重い。
- **フォークからの PR は、秘密なし・読み取り専用の権限で動く。** これが安全の土台。次を、**絶対にしない**:
  - `pull_request_target` で、PR のコードを checkout して動かす(他人のコードが、秘密つきで動く)。
  - `${{ github.event.pull_request.title }}` のような、他人が書ける値を、`run:` の中に直接書く(コマンドの注入)。環境変数を経由する。
- **権限は最小に**。各ワークフローの先頭は `permissions: contents: read`。書き込みが要るジョブにだけ、そのジョブの中で足す(`edge-image.yml` の `packages: write`)。
- **第三者の Action は、更新を Dependabot に任せる。** より慎重にするなら、タグではなく**コミットの SHA** で固定する(Dependabot が、SHA 固定も更新する)。GitHub 自身(`actions/*`)と、`docker/*`・`Swatinem/*`・`dtolnay/*` を使っている。
- **自前の実行機(self-hosted runner)を、公開リポジトリにつなげない**(他人の PR が、あなたのマシンで動く)。手元の Intel の Mac を使うなら、**タグを付けたリリースのビルドだけ**に限る。あるいは、手元で作って、手で確かめる。
- **秘密**(トークン、鍵)は、いまは使っていない。将来、リリースの署名や、デプロイの鍵を入れるなら、**Environment**(承認者つき)に入れ、タグかなどの保護された場所からだけ使う。
- **定期実行(cron)**: 公開リポジトリで、60 日、動きがないと、GitHub が定期実行を止める。`usage.yml` は、止まったら、手で再開する。
- `concurrency` で、同じブランチの古い実行を取り消す(`bridge.yml`・`plugin.yml` に入れた)。
- **`usage.yml` は、`main` に push しない**。数字は、`usage-data` という別のブランチに置く(`main` の保護と、衝突しない)。

## 6. 最初の実行で、落ちそうなところ(確認できていない)
- `bridge.yml` の `supply-chain`: `cargo install cargo-audit cargo-deny` に、時間がかかる。`deny.toml` の設定が、ライセンスや助言で、落ちるかもしれない。落ちたら、設定を直す(本質ではない)。
- `release.yml`: macOS Intel の実行環境のラベル `macos-15-intel` は、タグを付けて初めて確かめられる。
- `plugin.yml`: プラグインのテストが、実際のブリッジのバイナリ(`cargo build`)を使う。Rust の導入と、`zip`・`unzip` が、実行環境に要る。
- `edge-image.yml`: `linux/amd64` だけ。VPS が ARM なら、プラットフォームを足す。

## 7. プルリクエストの受け方
- **README に、「個人の運営で、返事は遅れる(ベストエフォート)」「初期のアルファ版」と書く**(README に書いた)。期待を先に決める。
- 受ける変更の方針を、`CONTRIBUTING.md` に書く(書いてある): 小さく・テストつき・状態機械は、表を先に直す・境界の値の検証。
- **ライセンス**: Apache-2.0 は、貢献者が出した変更を、同じライセンスで受けることを、条文(5 項)で決めている。**CLA は、要らない。** 履歴に、貢献者の署名(DCO の `Signed-off-by`)を求めるかは、任意。
- **次の変更は、特に、時間をかけて読む**: `.github/workflows/`(供給網)、`Dockerfile`・`deploy/`(運営の入口)、`Cargo.toml`・`package.json` の依存の追加、認証・暗号・パスの処理。
- 初めての貢献者の CI は、**承認してから**動かす(2 の設定)。コードを読んでから承認する。
- 断るときは、丁寧に、理由を一言。「方針と違う」は、悪いことではない。
- ラベル: `bug`、`question`、`enhancement`、`good first issue`、`help wanted`、`security`(公開しない)。

## 8. 問題報告の受け方
- Issue のテンプレート(`.github/ISSUE_TEMPLATE/`)で、**版、OS、道、`spellink-bridge check` の出力**を、最初から集める。「**トークンやペアリングの数字を貼らない**」を、冒頭に置いた。
- 診断は、端末の「診断」→「確認」→「送る」。記録は、利用者自身のブリッジにだけ行く(開発者には届かない)。**利用者に、`reports.jsonl` の関係部分を貼ってもらう。**
- **脆弱性は、公開の Issue にしない。** 「Report a vulnerability」(非公開)で受ける(`SECURITY.md`)。受け取ったら、数日以内に返事をし、直してから、アドバイザリを公開する。
- よくある質問は、`docs/guides/install-guide.md` の「うまくいかないとき」へ誘導する。

## 9. 公開後のよくある事故
| 事故 | 防ぎ方・直し方 |
|---|---|
| 秘密を push した | **まず、鍵を無効にして作り直す**(履歴からの削除は、その後)。履歴を消しても、すでに誰かが取っているかもしれない。Push protection で、多くは防げる |
| Issue に、トークンや個人情報が貼られた | コメントを編集で消し、トークンを作り直すよう伝える(編集の履歴も消す: 「Delete revision history」)。GitHub サポートに、キャッシュの削除を頼む手もある |
| 通知が多すぎて、重要なものが埋もれる | Watch を「Custom」(Issues・PR・Security alerts)に。メールは、フィルタで分ける |
| 利用者が「Claude の規約」を聞いてくる | 規約の判断は、Anthropic のもの。`docs/legal/` の草案と、公式ページへの案内に留める。個別の判断をしない |
| 中継の負荷・乱用 | `docs/usage/usage-dashboard.md` と、`docs/guides/relay-operations.md` の「障害のとき」 |
| 古い版の利用者 | リリースノートに、互換性を書く(プロトコルの版、設定の変更)。0.x の間は、「壊れうる」と書く |

## 10. 運営者が、定期的にすること
- 週 1 回: Dependabot の PR と、セキュリティのアラートを見る。
- 月 1 回: Issues と PR の整理。Anthropic の規約と、Supernote の SDK の変更を確認(`docs/notes/progress/README.md` の「次にやること」)。
- 版を出すたびに: CI が緑、実機で確認、リリースノート、各配布物の SHA-256 を載せる。
