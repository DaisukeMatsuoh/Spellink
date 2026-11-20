# プライバシーポリシー（草案 / Privacy Policy — DRAFT）

**これは草案です。法律の専門家の確認を受けていません。公開の前に、ユーザー（運営者）が内容を確認し、事実に合わせて直してください。**
*This is a draft. It has not been reviewed by a lawyer. The operator must check it against the facts before publishing it.*

## 日本語

### 1. このポリシーが対象とするもの
Spellink（Supernote 用プラグイン、パソコンで動くブリッジ「Spellink Bridge」、任意の中継サーバー）。

### 2. 私たちが扱わないもの
- あなたの手書き、質問、AI の回答、ペアリングの鍵は、**あなたの端末とあなたのパソコンの間だけ**を通ります。中継サーバーを通るときも暗号化されており、運営者には読めません。
- Spellink は、あなたの Claude のアカウント・パスワード・API キーを扱いません。AI への問い合わせは、あなたのパソコンにある公式の Claude Code が、あなた自身のログインで行います。
- 利用状況の計測、クラッシュ報告、広告の識別子を、**送りません**。

### 3. 中継サーバーが見るもの（公式の中継を使うときだけ）
- 部屋の識別子（あなたの鍵から作った、元に戻せない値）、接続元の IP アドレス、接続した時刻、通信量。
- 質問・回答・鍵・端末の名前は見えません。
- 中継のプログラムは、接続元の IP アドレスを**ログに書きません**。前段（[運営者が使う、たとえば Cloudflare]）が、通信の運用のために IP アドレスを一時的に記録することがあります。保存期間: [運営者が確認して記入]。
- 目的: 中継を動かすこと、乱用（過剰な接続）を防ぐこと。それ以外には使いません。第三者に提供しません。

### 4. あなたのパソコンと端末に保存されるもの
- パソコン: ブリッジの設定、端末の台帳（端末の名前・状態・鍵）、会話の記憶（ノートごとの質問と回答）、診断のログ。すべて、あなたのパソコンの設定フォルダにあります。**「このノートの会話を忘れさせる」「すべての会話を忘れさせる」で消せます。**
- 端末: ペアリングの鍵と接続先（`MyStyle/Spellink-pairing/` のフォルダの名前）。端末を手にした人には、読めます。端末を失くしたら、パソコンで「解除」してください。

### 5. 保証と変更
中継は、ベストエフォートで運営します。保証はありません。このポリシーを変えるときは、[告知の場所] でお知らせします。

### 6. 連絡先
[運営者の連絡先]

## English

### 1. What this covers
Spellink: the Supernote plugin, the "Spellink Bridge" that runs on your computer, and the optional relay server.

### 2. What we do not handle
- Your handwriting, your questions, the AI's answers and the pairing keys travel **only between your device and your computer**. Through the relay they stay encrypted; the operator cannot read them.
- Spellink does not handle your Claude account, password or API key. The AI is asked by the official Claude Code on your computer, with your own login.
- It sends **no** usage analytics, crash reports or advertising identifiers.

### 3. What the relay sees (only if you use the official relay)
- A room identifier (an irreversible value made from your key), the connecting IP address, the time of the connection and the amount of data.
- It cannot see questions, answers, keys or device names.
- The relay program does **not write client IP addresses to its log**. The service in front of it ([the operator's provider, e.g. Cloudflare]) may record IP addresses temporarily to run the connection. Retention: [operator to fill in].
- Purpose: running the relay and preventing abuse (excessive connections). Nothing else. It is not shared with third parties.

### 4. What is stored on your computer and your device
- Computer: the bridge's settings, the ledger of devices (names, states, keys), the conversation memory (questions and answers per note), diagnostic logs. All in the bridge's settings folder on your computer. **"Forget this note's conversation" and "Forget all conversations" delete the memory.**
- Device: the pairing key and the addresses (names of folders under `MyStyle/Spellink-pairing/`). Anyone who holds the device can read them. If you lose the device, remove it on your computer.

### 5. No warranty; changes
The relay is run on a best-effort basis, with no guarantee. Changes to this policy will be announced at [where].

### 6. Contact
[operator's contact]
