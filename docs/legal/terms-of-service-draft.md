# 利用規約（草案 / Terms of Use — DRAFT）

**これは草案です。法律の専門家の確認を受けていません。公開の前に、運営者（ユーザー）が内容を事実に合わせて確認し、必要なら専門家の助言を受けてください。**
*This is a draft. It has not been reviewed by a lawyer. The operator must check it against the facts, and take legal advice if needed, before publishing it.*

## 日本語

### 1. これは何か
Spellink は、Supernote のノートで手書きを囲み、パソコンで動く Claude Code（Anthropic 社の公式のプログラム）に質問して、回答を筆跡としてページに書き戻す、非公式のオープンソースのソフトウェアです。プラグイン、パソコンで動くブリッジ（Spellink Bridge）、任意の中継サーバーから成ります。

### 2. あなたが約束すること（初回の起動のとき、同意を求めます）
1. このパソコンで Claude Code にログインしているアカウントは、**あなた自身のアカウント**です。
2. Spellink を、**あなた自身のためだけ**に使います。あなたのブリッジを、ほかの人に使わせません。ほかの人に Claude の機能を提供するサービスにしません。
3. Supernote の端末の利用者は、そのブリッジの持ち主と**同じ人**です（あなたが複数の端末を持つのは構いません）。
4. Anthropic 社の利用規約・利用ポリシー（Consumer Terms、Usage Policy、Claude Code の法務・コンプライアンスのページ）を守ります。それらの変更に従う責任は、あなたにあります。
5. Claude Code を改変せず、そのログインの情報を、Spellink に取り出させません（Spellink は、それらに触れません）。

### 3. Spellink がしないこと
- あなたの Claude のアカウント・パスワード・トークンを、読み取らず、保存せず、送りません。Claude への問い合わせは、あなたのパソコンの公式の Claude Code が、あなた自身のログインで行います。
- 質問・回答・手書きを、運営者に送りません。中継を通るときも、暗号化されていて、運営者には読めません。
- 利用状況の計測は、しません。

### 4. 免責事項（重要）
- **現状有姿・無保証。** Spellink は、「現状のまま」提供されます。商品性、特定の目的への適合性、権利を侵害しないこと、正確性、中断のないこと、誤りのないことを含め、明示・黙示を問わず、**いかなる保証もしません**。
- **責任の制限。** 法律が許す最大の範囲で、運営者と貢献者は、Spellink の利用または利用できないことから生じる、直接・間接・偶発・特別・結果的な損害（データの損失、ノートの破損、利益の損失、アカウントの停止・制限を含みます）について、責任を負いません。
- **ノートについて。** Spellink は、ノートのファイルに、筆跡を書き込みます。**大事なノートは、先にバックアップを取ってください。** 書き込みの失敗、ノートアプリの応答停止、データの破損が起こりうることを理解して使ってください。
- **AI の回答。** 回答は、AI が作るもので、誤りを含むことがあります。医療・法律・金融などの判断に、そのまま使わないでください。回答の内容と、それに基づく行動は、あなたの責任です。
- **Anthropic 社との関係。** Spellink は、Anthropic 社、Supernote（Ratta 社）と、**提携も承認もされていません**。Claude、Claude Code、Anthropic は Anthropic 社の、Supernote は Ratta 社の、それぞれ商標または名称です。Anthropic 社の規約・仕様・料金・利用上限（サブスクリプションの利用枠や、`claude -p` の利用に関する取り扱いを含みます）は、予告なく変わることがあり、**それによって Spellink が動かなくなる、または追加の費用や制限が生じても、運営者は責任を負いません。**
- **アカウントのリスク。** Claude の利用が、Anthropic 社の規約に反すると判断された場合に、アカウントに制限や停止が行われる可能性があります。そのリスクは、あなたが負います。
- **中継サーバー。** 公式の中継は、**ベストエフォート**で運営され、いつでも、予告なく、変更・中断・終了することがあります。可用性・速度・継続を保証しません。中継が使えなくなった場合の代わり（自前のトンネル、家の中の直結）は、文書にあります。
- **セキュリティ。** 運営者は、安全に作るよう努めますが、完全な安全を保証しません。鍵（`.snplg`、端末に保存された鍵）の管理は、あなたの責任です。端末を失くしたときは、直ちにパソコンで「解除」してください。
- **第三者のソフトウェアとデータ。** Spellink は、第三者のソフトウェアとデータ（`THIRD_PARTY_NOTICES.md`）を使います。それぞれのライセンスに従います。
- **準拠法・管轄。** [運営者が記入: 例 日本法、東京地方裁判所を専属的合意管轄とする]

### 5. ライセンス
ソースコードは、リポジトリの `LICENSE`（Apache-2.0）に従います。この規約は、公式のバイナリ、公式の中継、公式のサイトの利用に関するものです。

### 6. 変更・連絡先
この規約を変えるときは、[告知の場所] でお知らせします。連絡先: [運営者の連絡先]

## English

### 1. What this is
Spellink is an unofficial open-source program that lets you circle handwriting in a Supernote note, ask Claude Code (Anthropic's official program) on your computer, and write the answer back into the page as handwriting. It consists of the plugin, the bridge that runs on your computer (Spellink Bridge), and an optional relay server.

### 2. What you agree to (asked at first start)
1. The account logged in to Claude Code on this computer is **your own account**.
2. You use Spellink **only for yourself**. You do not let other people use your bridge, and you do not turn it into a service that offers Claude's capabilities to others.
3. The user of the Supernote device is **the same person** as the owner of the bridge (owning several devices is fine).
4. You comply with Anthropic's terms and policies (the Consumer Terms, the Usage Policy, and Claude Code's legal and compliance page). Following their changes is your responsibility.
5. You do not modify Claude Code, and you do not make Spellink extract its login credentials (Spellink does not touch them).

### 3. What Spellink does not do
- It does not read, store or send your Claude account, password or tokens. Claude is asked by the official Claude Code on your computer, with your own login.
- It does not send your questions, answers or handwriting to the operator. Through the relay they stay encrypted; the operator cannot read them.
- It does not collect usage analytics.

### 4. Disclaimers (important)
- **As is, no warranty.** Spellink is provided "as is". To the fullest extent permitted by law, the operator and contributors give **no warranty of any kind**, express or implied, including merchantability, fitness for a particular purpose, non-infringement, accuracy, uninterrupted or error-free operation.
- **Limitation of liability.** To the fullest extent permitted by law, the operator and contributors are not liable for any direct, indirect, incidental, special or consequential damages arising from the use of, or inability to use, Spellink — including loss of data, damage to notes, loss of profit, and suspension or restriction of accounts.
- **Your notes.** Spellink writes handwriting into your note files. **Back up important notes first.** Writes may fail, the note app may stop responding, and data may be damaged; use it with that understanding.
- **AI answers.** Answers are produced by an AI and may be wrong. Do not rely on them for medical, legal, financial or similar decisions. You are responsible for the answers and for what you do with them.
- **Relationship with Anthropic and Supernote.** Spellink is **not affiliated with or endorsed by** Anthropic or Supernote (Ratta). Claude, Claude Code and Anthropic are names or trademarks of Anthropic; Supernote is a name or trademark of Ratta. Anthropic's terms, specifications, prices and usage limits (including how subscription allowances and `claude -p` usage are treated) can change without notice, and **the operator is not liable if Spellink stops working or extra costs or limits arise as a result.**
- **Account risk.** If your use of Claude is judged to violate Anthropic's terms, your account may be restricted or suspended. You bear that risk.
- **Relay server.** The official relay is run on a **best-effort** basis and may be changed, interrupted or shut down at any time without notice. No availability, speed or continuity is guaranteed. Alternatives (your own tunnel, direct connection at home) are documented.
- **Security.** The operator tries to build it safely but does not guarantee perfect security. Keeping your keys (`.snplg`, the key stored on the device) safe is your responsibility. If you lose a device, remove it on your computer at once.
- **Third-party software and data.** Spellink uses third-party software and data (`THIRD_PARTY_NOTICES.md`), under their own licenses.
- **Governing law and jurisdiction.** [operator to fill in]

### 5. License
The source code is under the repository's `LICENSE` (Apache-2.0). These terms cover use of the official binaries, the official relay and the official site.

### 6. Changes and contact
Changes to these terms will be announced at [where]. Contact: [operator's contact]
