# Supernote 拡張方式・SDK 調査（2026-10-30）

> 状態: **決着** → ADR 0001。結論: 公式プラグイン SDK（sn-plugin-lib）を使う。サイドロードや root 化は採らない。

目的: 「ラッソで囲んだ手書き → Claude に質問 → 回答を手書きの筆跡としてページに書き戻す」を、
もっとも軽く・キビキビ・省電力に実現できる拡張方式を見極める。

一次情報: https://docs.supernote.com/en （索引: https://docs.supernote.com/llms.txt、API 索引: https://docs.supernote.com/_llms/en/api-reference.txt）
SDK: npm `sn-plugin-lib` 0.2.3（2026-10-12 更新、MIT）、テンプレート `@supernote-plugin/sn-plugin-template` 1.0.13

---

## 1. 拡張方式の比較

| 方式 | NOTE への介入 | 速さ・電力 | 安定性 | 判定 |
|---|---|---|---|---|
| **A. 公式プラグイン (sn-plugin-lib)** | ラッソツールバーにボタン、ラッソ要素の取得、オンデバイス手書き認識、ストローク/画像/テキストボックスをページへ直接挿入 | 常駐の PluginHost 上で動く。待機中は何もしなければほぼゼロコスト | 公式・後方互換を意識した API | **採用** |
| B. サイドロード APK | NOTE の内部（ラッソ・ストローク）に触れない。自前の描画アプリを作るしかない | 純正ペン描画パイプライン（低遅延）を失う | 設定 > セキュリティとプライバシーの Sideloading スイッチ（Chauvet 3.16.27 追加、3.17 で一度削除→3.18 で復活）。いつ消えるか不明 | 不採用 |
| C. ファイル同期型（.note を PC/クラウドで解析し書き戻し） | 保存後のファイル単位。対話的でない | 端末側は軽いが、往復が同期待ち | .note は非公開フォーマット | 補助用途のみ |
| D. root / システム改変 | 何でもできる | ― | 保証・アップデートで破綻、文鎮化リスク | 不採用 |

**結論:** 「アプリレベルでシステムに介入」したい要求のうち、本ユースケースに必要なもの（ラッソ・OCR・ストローク書き込み）は
すべて公式プラグイン API で揃っている。しかもプラグインは Java/Kotlin の TurboModule を同梱でき（JS→Java→C/C++ まで降りられる）、
重い処理はネイティブ側に逃がせる。これが現状もっとも軽く確実な方法。

## 2. プラグインの実行モデル（要点）

- 3者構成: **Plugin**（RN 0.79.2 の JS/TS）／**PluginHost**（`com.ratta.supernote.pluginhost`。RN ランタイムを提供し常駐）／**NOTE・DOC**（ホストアプリ）。
- NOTE/DOC → PluginHost のイベントは Android AIDL。ボタン押下で PluginHost が対象プラグインのリスナーを呼ぶ。
- ボタン種別: `1` サイドツールバー、`2` **ラッソツールバー**（`editDataTypes` で表示条件。`0`=手書きストローク）、`3` DOC のテキスト選択。
- `showType: 0` にすると **UI を開かずに** ボタンイベントだけ受けてバックグラウンド処理できる → 本件はこれが基本。
- RN バージョンは 0.79.2 固定（ホストと一致必須）。
- パッケージ: `buildPlugin.sh` が `react-native bundle` → zip → `*.snplg`。端末の `MyStyle/` に置いて 設定 > アプリ > プラグイン からインストール。
  - **JS のみなら Gradle 不要**（Node だけでビルド可。この ARM64 開発機でもビルドできる見込み）。
  - ネイティブモジュール（ReactPackage）を含む場合のみ `gradlew buildCustomApkDebug` → `app.npk` として同梱。
    ARM64 Linux には公式 aapt2 が無いので、ネイティブ部分は x86_64 機か CI でビルドするのが無難。
- 権限: `PluginConfig.json` の `uses-permissions` に宣言 → 実行時 `hasPermission`/`requestPermission`。
  - `plugin.permission.INTERNET`（Claude API 呼び出しに必須）
  - `plugin.permission.FILE:READ` / `FILE:WRITE`（`getLassoElements` は READ、`insertPageElements` は WRITE に依存）
  - 「今回のみ許可」はプラグイン終了で失効。毎回 check→request が推奨フロー。「常に許可」なら以後ダイアログ無し。
  - プラグイン専用ディレクトリ `/data/data/com.ratta.supernote.pluginhost/files/plugins/<pluginID>` は権限不要。

## 3. 本ユースケースに効く API

### 入力（囲んだ手書きを取る）
| API | 用途 |
|---|---|
| `PluginManager.registerButton(2, ['NOTE'], {editDataTypes:[0], showType:0, ...})` | ラッソツールバーに「Ask Claude」ボタン |
| `PluginCommAPI.getLassoRect()` | ラッソ矩形（Android px、ページ実寸基準）→ 回答の書き出し位置の基準 |
| `PluginCommAPI.getLassoElements()` | 選択要素 `Element[]`（点列はアクセサ経由で遅延取得） |
| `PluginCommAPI.recognizeElements(elements, pageSize)` | **ファームウェア内蔵の手書き認識**（ストロークとテキストボックスのみ）。`size` は `getPageSize` のページ全体サイズを渡す（ラッソ矩形を渡すと失敗するとの報告あり） |
| `PluginCommAPI.generateLassoPreview(path)` | ラッソ範囲の PNG を生成 → Vision で Claude に送る用 |
| `Stroke.recognPoints` | `X, Y, Flag, timestamp` 付き点列。「MyScript 等の認識ライブラリに渡せる」と明記 → 外部のオンライン手書き認識（ベクトル OCR）にも使える |

### 出力（回答を書き戻す）
| API | 用途 |
|---|---|
| `PluginCommAPI.createElement(ElementType.TYPE_STROKE)` | 新規ストローク要素（ネイティブ側キャッシュに生成） |
| `stroke.points / pressures` の `ElementDataAccessor.add/set/setRange` | 点列・筆圧（0..4096）の書き込み |
| `stroke.penType`（10 テクニカル/16 筆圧/11 マーカー/15 カリグラフィ）, `penColor`（0 黒/157 濃灰/201 薄灰）, `thickness` | 筆跡の見た目 |
| `PluginCommAPI.insertPageElements(elements, page, layer?)` | 現在開いているファイルのページに挿入 |
| `PluginCommAPI.batchUpdatePageElements` | まとめて更新 |
| `recycleElement` / `clearElementCache` | ネイティブキャッシュ解放（ループ内で必須） |
| `PluginNoteAPI.insertText` / `insertImage` | フォールバック（テキストボックス / 画像） |
| `PluginNoteAPI.insertNotePage`（FileAPI） | 回答が溢れたら次ページ追加 |
| `PluginCommAPI.reloadFile` | 挿入後に画面へ反映されない場合の保険 |

### その他
- 座標系: sn-plugin-lib > 0.1.65 は全 API が **Android ピクセル・ページ実寸基準**で統一（A5X 1404×1872、Manta 1920×2560）。`PointUtils` 変換は使わない。
- `registerEventListener('event_pen_up', ...)` / `registerMotionListener` もあるが、**ペンを上げるたび JS が起きる**ので省電力目的では使わない（将来の「書き終えたら自動送信」モードでのみ検討）。
- `getPenInfo()` で現在のペン種・太さを取得 → 回答をユーザーのペン設定に合わせられる。

## 4. 「何が一番キビキビ動くか」の整理

1. **待機コスト 0**: ボタン押下時以外はコードを走らせない（pen_up/motion リスナー不使用）。PluginHost は元々常駐。
2. **OCR はオンデバイス優先**: `recognizeElements` はオフライン・無料。テキストだけ送れば通信量は数百バイト。
   画像併送（`generateLassoPreview`）は数十KB。図や数式が絡むときのみ送るか、常に併送して Claude に OCR 誤りを補正させるかは要計測。
3. **ストリーミング**: Claude の streaming 応答を受けながら、1行分たまるごとにストロークを挿入 → 最初の行が早く浮かび上がる（体感速度と「魔法の紙」演出を両立）。
4. **ストローク生成の重さ**: 日本語 300 字の回答 ≒ 1,500〜2,500 ストローク × 各 20〜50 点。JS↔ネイティブ境界の往復がボトルネックになり得る。
   まず JS で計測し、遅ければ「グリフ→点列→Element 生成」を Java TurboModule に移す。
5. **e-ink 再描画**: 挿入 1 回ごとに部分リフレッシュが起きる想定。行単位でまとめて挿入するのが落とし所（要実機確認）。

## 5. 回答を「筆跡」にする方法

- **単線（センターライン）フォント**のグリフ = そのままペンの軌跡。
  - 日本語: **KanjiVG**（かな・漢字約 6,700 字、筆順付き SVG パス、CC BY-SA 3.0 → 帰属表示と継承の義務あり。商用 SaaS 化時は要注意）
  - 英数字: **Hershey fonts**（単線ベクタ、ほぼ自由に利用可）
- パス → ポリライン化 → ゆらぎ（位置・傾き・筆圧の微小ノイズ）で手書き感 → ラッソ矩形の下に行送り・折り返しレイアウト。
- 将来（SaaS 候補）: ユーザー自身の筆跡サンプル（`getElements` で取得できる）から筆跡スタイルを学習して「本人の字」で書く。

## 6. OCR の選択肢（SaaS 化の余地）

| 選択肢 | 長所 | 短所 |
|---|---|---|
| 端末内 `recognizeElements` | 無料・オフライン・速い | 精度はファーム依存。日本語精度は要実機確認 |
| ラッソ PNG → Claude Vision | 図・数式・崩し字に強い。OCR とQ&Aを1リクエストで | 通信量・トークン費 |
| ストローク時系列 → 自前/外部のオンライン手書き認識（MCP サーバ化も可） | 時系列情報を使えるので画像より高精度になり得る。**SaaS 化の核になり得る** | サーバ運用が必要 |

当面は「端末 OCR テキスト + （必要時）PNG」を Claude に直接送る構成で始め、精度不満が出たらベクタ OCR サービスを検討。

## 7. 先行事例

- **j-raghavan/sn-copilot**（MIT）: Claude/OpenAI/Gemini 対応の AI サイドバー。BYOK（キーは `MyStyle/SnCopilot/` のテキストファイル）、バックエンド無し。
  「PilotChat」は全画面の専用ページに手書きで質問→画像で LLM に送信→**回答はテキストボックス**として挿入。
  → **ラッソ起点 + 回答を筆跡ストロークで書く** 点は未開拓で、本プロジェクトの差別化ポイント。
- **j-raghavan/sn-dictionary**: ラッソ→`recognizeElements`→辞書引き。再入防止や `closePluginView` のタイミング等、実運用上の知見が README にまとまっている。
- **ofples/supernote-plugin-research**: SDK ソース解析と SDK-REFERENCE.md。SuperTask（ラッソ→OCR→Todoist）。
- 非公式ノウハウ（コミュニティの Claude skill「supernote-plugin-dev」より、要検証）:
  - `recognizeElements` の size はラッソ矩形ではなくページ全体サイズ。
  - プラグイン UI 上でのペン入力がノートに漏れることがある。UI を出さない `showType:0` 構成なら回避できる。
  - `closePluginView` を成功パスで早く呼びすぎると入力チャネルが不安定になる報告あり。
  - カスタム NativeModule は `PluginConfig.json` の `reactPackages` に載っていないと黙って失敗（buildPlugin.sh が自動検出して書き込む）。

## 8. 実機で確かめないと分からないこと

1. `createElement` + アクセサ書き込みで作った**合成ストロークが正しく挿入・表示されるか**、その速度（1,000 ストローク規模）。
2. `showType:0`（UI 無し）で挿入したとき、NOTE の画面に即反映されるか（`reloadFile` が要るか）。
3. `recognizeElements` の日本語精度と所要時間。
4. INTERNET 権限ダイアログの体験（「常に許可」を選んでもらう導線）。
5. ユーザーの端末機種・ファームウェア（Manta / Nomad / A5X …、Chauvet バージョン）。

## 参考リンク

- 公式: [Plugin Principles](https://docs.supernote.com/en/principle.md) / [Your First Plugin](https://docs.supernote.com/en/first-plugin.md) / [Lasso](https://docs.supernote.com/en/plugin-base/lasso.md) / [Permissions](https://docs.supernote.com/en/plugin-base/permission.md) / [Element Operations](https://docs.supernote.com/en/plugin-base/file-op/element-op.md) / [recognizeElements](https://docs.supernote.com/en/api-reference/supernote-plugin/plugin-comm-api/recognize-elements.md)
- [Manta & Nomad Changelog](https://support.supernote.com/change-log/changelog-for-manta-and-nomad)（Sideloading スイッチの経緯）
- [sugoi-supernote サイドロードガイド](https://github.com/dwongdev/sugoi-supernote/blob/main/Guides/how%20to%20sideload%20supernote.md)
- [sn-copilot](https://github.com/j-raghavan/sn-copilot) / [sn-dictionary](https://github.com/j-raghavan/sn-dictionary) / [supernote-plugin-research](https://github.com/ofples/supernote-plugin-research)
- [KanjiVG](https://kanjivg.tagaini.net/)
