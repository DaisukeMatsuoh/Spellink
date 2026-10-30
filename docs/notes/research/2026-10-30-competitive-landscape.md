# 競合・類似事例（2026-10-30 時点）

質問: 「手書きを AI に読ませて答えてもらう」ものは既に人気があるか？ iPad にはあるか？

## iPad
- **Goodnotes**（利用者 2,400 万人超と発表）: Ask Goodnotes（手書き・PDF を横断して質問）、Math Assist（手書き数式を解く/ヒント）、手書きのスペルチェック。
  **回答はテキスト表示**。「AI の出力を自分の筆跡で書いてほしい」はフィードバック掲示板の要望止まり。
- **Apple メモ（iPadOS 18 以降）**: Math Notes（手書きの式に答えを書き足す）と Smart Script（手書きの清書、ペーストした文字を本人風の筆跡にする）。
  ※後者2つは調査時の検索では裏付けが取れず、記憶ベース。要確認。汎用の質問に筆跡で答える機能ではない。
- まとめ: 「手書きを AI が読む」は iPad では既に一般的。**「汎用 LLM の回答を手書きとしてページに書く」は主流製品には無い**。ただし Apple は部品（筆跡合成）を持っており、いつでも参入しうる。

## e-ink 端末
- **reMarkable — Ghostwriter**（OSS・非公式ハック）: ジェスチャで手書きを LLM に送り、回答を画面上にペンの軌跡として書き戻す。Claude にも対応。**本プロジェクトに最も近い先行事例**。
  Paper Pro 向けに、回答を文単位でストリーミングしてアニメーションするストロークとして描く派生ハックもある。いずれも root 寄りの非公式手段。
- **reMarkable 公式**: 手書き→テキスト変換と要約程度。生成 AI には慎重な姿勢と報じられている。
- **Supernote — sn-copilot**（コミュニティプラグイン）: AI サイドバー + PilotChat（全画面ページで手書き質問→**テキストボックスで回答**）。
- **Penombra**: e-ink 向け AI 手書きアシスタント（先行登録受付中、詳細不明）。
- Boox / Kindle Scribe の AI 機能は今回未調査。

## 示唆
- Supernote 上で「ラッソ起点 + 筆跡での回答」をやっているものは見当たらない → ここで一番乗りできる。
- iPad は市場は大きいが、Goodnotes / Apple と真正面から競合し、他社アプリの中には入り込めない（独立アプリになる）。
- 核になる部品（単線グリフ→筆跡ストローク生成、ブリッジ）は端末非依存に作り、Supernote で磨いてから reMarkable / iPad へ広げるのが筋。

## 出典
- https://www.techradar.com/computing/artificial-intelligence/goodnotes-adds-an-ai-that-can-read-and-explain-even-the-worst-handwriting
- https://www.vktr.com/the-wire/goodnotes-elevates-productivity-for-its-24m-users-with-new-ai-powered-enhancements/
- https://feedback.goodnotes.com/forums/191274-customer-suggestions-for-goodnotes/suggestions/48624011-please-implement-the-smart-script-features-in-ipad
- https://discuss.pytorch.kr/t/ghostwriter-remarkable2/6079
- https://github.com/j-raghavan/sn-copilot
- https://penombra.ai/
