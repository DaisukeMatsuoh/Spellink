# Spellink の構成: 誰が何を持ち、どこで TLS を張るか

運営する人と、レビューする人のための**構成図**。利用者向けの説明は `bridge-and-relay-for-users.md`、運営の手順は `relay-operations.md`、決定の理由は ADR（0005・0007・0010・0016・0017・0018・0019）。

最終更新: 2026-11-21。**実装と自動テストは済み。VPS・本物の Let's Encrypt・Supernote の実機での直結は、未確認**（ADR 0017 の「未確認」）。

## 1. 登場人物

| 登場人物 | 何者か | 持っているもの |
|---|---|---|
| **端末**（Supernote） | 手書きを読み取る。プラグインが動く | 自分用の秘密 K（ペアリングで受け取る）、接続先の一覧 |
| **ブリッジ**（`spellink-bridge serve`） | 利用者の**自分のパソコン**で動く。AI に聞く | 端末の台帳（B・K）、会話の履歴、AI ツール（Claude Code）のログイン、**自分の証明書の秘密鍵** |
| **AI**（Claude Code） | ブリッジのパソコンに入っている公式 CLI | 利用者本人の契約・ログイン（ブリッジも運営も触れない） |
| **中継**（`spellink-bridge relay`） | **運営**の VPS で動く。外出先の端末とブリッジをつなぐ | 何も持たない（状態はメモリだけ）。自分の TLS の証明書 |
| **名前サービス**（`spellink-bridge names`） | **運営**の VPS で動く。家の中の直結のための名前（DNS）を持つ | ID と LAN の IP、証明書の確認用の一時的な TXT |
| **Let's Encrypt** | 公的な認証局。証明書を出す | — |
| **Cloudflare** | ドメイン `spellink.uk` の DNS の入れ物（通信は通らない） | DNS のレコード数件 |

**利用者は自分のパソコンでブリッジを動かす。** 運営は、中継と名前サービスだけを動かす。AI への質問は、利用者のパソコンから AI に直接行き、運営を通らない。

## 2. 全体図

```mermaid
flowchart LR
  subgraph User["利用者の側"]
    D["端末 Supernote<br/>プラグイン"]
    subgraph PC["利用者のパソコン"]
      B["ブリッジ<br/>(spellink-bridge serve)"]
      C["Claude Code<br/>(本人のログイン)"]
      B --> C
    end
  end
  subgraph Ops["運営の VPS (日本: Kagoya、米国: OVHcloud。固定 IP)"]
    R["中継 (relay)<br/>443"]
    N["名前サービス (names)<br/>DNS 53 / API 8443"]
  end
  LE["Let's Encrypt"]
  AI["Anthropic"]

  D -- "① 自前のトンネル wss<br/>(利用者のドメイン)" --> B
  D -- "② 中継 wss" --> R
  B -- "②' 外向きに常時接続 wss" --> R
  D -. "③ 家の中の直結 wss (8766)" .-> B
  B -- "登録・TXT (署名つき)" --> N
  D -- "DNS: <id>.lan.spellink.uk" --> N
  B -- "証明書の注文 (ACME DNS-01)" --> LE
  LE -- "TXT を引く" --> N
  C -- "本人の契約で" --> AI
```

端末は、設定された経路を**同時に**試し、つながったものを使う（家なら③、外なら②か①）。

## 3. TLS はなぜあり、どこからどこまでか

- 端末の PluginHost は、**公的な CA の `wss://` しか接続させない**（`ws://` や `.local` は拒否。ADR 0007）。TLS は、この制約を満たすためにある。
- **中身の秘密は TLS ではなく Noise が守る。** 端末とブリッジの間は、端末とブリッジだけが持つ鍵で暗号化している（`Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s`）。TLS を終端する中継にも、質問と回答は見えない。
- つまり TLS は「区間ごとの入口の鍵」、Noise は「端から端までの鍵」。

| 経路 | TLS を張る区間（誰が証明書を持つか） | 端から端までの暗号 |
|---|---|---|
| **① 自前のトンネル**（開発者本人が使用中。利用者が自分で用意） | 端末 ⇄ **利用者の Cloudflare Tunnel の入口**（Cloudflare の証明書）。その先は利用者のパソコンの内側で平文の http | Noise |
| **② 中継**（運営） | 端末 ⇄ **中継**、ブリッジ ⇄ **中継**。証明書は `jp.relay.spellink.uk`・`us.relay.spellink.uk`（Let's Encrypt。**中継が自分で取る**。地域ごとに別の名前。ADR 0019 の追記） | Noise（中継は暗号文を流すだけ） |
| **③ 家の直結**（運営は名前を貸すだけ） | 端末 ⇄ **ブリッジ**。証明書は `<id>.lan.spellink.uk`（Let's Encrypt。**ブリッジが自分で取り、秘密鍵はブリッジの外に出ない**） | Noise |

## 4. ペアリングと接続の流れ（ADR 0016）

1. 利用者が、ブリッジで「端末を追加」。端末専用の `.snplg` ができる（初期の秘密 B を入れる。期限 24 時間、ペアリングにだけ使える）。
2. 端末に `.snplg` を入れ、「ペアリング」を押す。端末が 8 桁の数字と QR を出す。
3. 利用者がブリッジに数字を入力（または QR を読む）。一致したら、ブリッジが本番の秘密 K を、B から作った Noise の通信路で端末に渡す。**B はここで破棄される。**
4. 以後は K だけで話す。接続先の一覧（①②③）も、このとき端末に渡る。

質問のとき: 端末が OCR した文字を Noise で送る → ブリッジが Claude Code に聞く → 答えをストロークにして Noise で返す。**中継・名前サービスは、この中身を見ない。**

## 5. 誰が何を見られるか

| | 質問・回答 | 秘密 K・B | 接続元の IP | 端末・ブリッジの ID | 通信量・時刻 |
|---|---|---|---|---|---|
| 端末・ブリッジ | ○ | ○ | – | ○ | ○ |
| 中継（運営） | **見えない** | **見えない** | ○（TLS の本物の接続。ログには書かない） | 部屋 ID（K のハッシュ）だけ | ○ |
| 名前サービス（運営） | **見えない** | **見えない** | 登録のときの接続元（ログには書かない）、DNS の問い合わせ元 | ブリッジの ID と LAN の IP | 登録の時刻 |
| Let's Encrypt | 見えない | 見えない | ブリッジの注文元の IP | ブリッジの名前（`<id>.lan.spellink.uk`）。証明書は公開の記録（CT）に載る | – |
| 利用者のトンネル（①） | 見えない（暗号文） | 見えない | ○ | ホスト名 | ○ |

**注意:** 証明書透明性（CT）のログには、`<id>.lan.spellink.uk` と `jp.relay.spellink.uk`・`us.relay.spellink.uk` が公開される。ID は公開鍵のハッシュで、個人を示さないが、「その ID のブリッジが存在する」ことは誰でも見える。

## 6. 運営と利用者の役割

| | 運営（Spellink） | 利用者 |
|---|---|---|
| 動かすもの | 中継、名前サービス、ドメイン `spellink.uk` | ブリッジ、Claude Code、端末 |
| 持つ秘密 | 中継・名前サービスそれぞれの identity（確認名の持ち主を示す鍵）。**DNS 用のトークンは存在しない** | 自分の AI のログイン、端末ごとの K、ブリッジの台帳と identity、**自分の証明書の秘密鍵** |
| 持たないもの | 利用者の質問・回答・AI の認証・K・証明書の秘密鍵 | 運営の資格情報 |
| 止まったとき | 外出先から使えない。家の中（③）は取得済みの証明書の期間（90 日）使える | ブリッジの `relayUrl`・`extraUrls` を変えれば、自前の中継やトンネルへ移れる |
| 費用 | VPS（定額）、ドメイン | AI の契約（本人の枠）、パソコンの電気 |

**境界の原則:** 運営の資格情報は、運営の VPS の外に出さない。利用者のリポジトリ・バイナリ・`.snplg` にも入れない。利用者の秘密（K・AI のログイン・証明書の秘密鍵）は、運営の機械に送らない。

## 7. 運営する部品の配置（ADR 0019）

```mermaid
flowchart TB
  INET(("インターネット"))
  subgraph VPS["運営の VPS (Linux, Docker, 固定 IP)。ファイアウォール: 443/tcp・8443/tcp・53/udp+tcp だけ"]
    subgraph RC["relay コンテナ (scratch, 非 root, 読み取り専用)"]
      REL["中継 :8443 → ホストの 443<br/>自分で TLS (relay.spellink.uk)"]
    end
    subgraph NC["names コンテナ (scratch, 非 root, 読み取り専用)"]
      NS["DNS :5353 → ホストの 53 (UDP/TCP)<br/>API :8443 → ホストの 8443 (TLS: names.spellink.uk)"]
    end
    REL -- "TXT を置く (署名つき API)" --> NS
  end
  INET --> REL
  INET --> NS
  CF["Cloudflare: spellink.uk の DNS の入れ物<br/>ns1 の A、lan の NS、_acme-challenge.{relay,names} の CNAME"]
  CF -. "lan.spellink.uk 以下を委任" .-> NS
```

- **2 つは別のコンテナ**（同じバイナリ。一方が破られても、もう一方へ広がらない）。それぞれ、自分の volume にだけ書く。
- 外から**ポートを開ける**ので、守りは自前: scratch、非 root、読み取り専用、権限なし、`no-new-privileges`、メモリ・プロセス数の上限、接続元ごとの回数制限、握手と HTTP ヘッダの時間切れ、接続数の上限。

## 8. 公開前に確かめること

- 利用中のトークンが、git の履歴と追跡ファイルに**ない**こと（2026-11-21 に確認。下の §9）。
- セキュリティレビューとコードレビュー（ユーザー指定。実施前は push しない）。
- VPS での初回の運用（`relay-operations.md` の手順。まず Let's Encrypt の staging で）。
- （済）docker でのビルドと、権限なしコンテナでの名前サービスの起動（2026-11-22）。
- 実機で、家の中の直結（③）。
- 利用規約・プライバシーの文書（`docs/notes/research/2026-11-20-relay-decisions.md`）。名前サービスが ID と LAN の IP を見ること、CT に名前が載ることを書く。

## 9. トークンが出ていないことの確認（2026-11-21、push 前）

- 方法: 開発機のブリッジの設定（`config.json`・`devices.json`）にある秘密の文字列を取り出し、**全ブランチ・全コミットの全ファイル**（`git grep -F` を `git rev-list --all` の各コミットに）と、作業ツリー（ignore された物を含む）を照合した。値は画面にも文書にも出していない。
- 結果: 履歴・追跡ファイルに**一致なし**。作業ツリーで一致したのは `plugin/src/bridges.local.json` だけで、これは `plugin/.gitignore` で除外され、一度もコミットされていない（**2026-11-22 に、このファイルとビルドへの埋め込みの道そのものを削除した**。接続先は、ペアリングで端末に渡す）。
- 形のパターン（`sk-ant`、`Bearer …`、`CF_API_TOKEN`、秘密鍵の PEM）も履歴を検索し、該当なし。
- 追跡されている `plugin/android/app/debug.keystore` は React Native の標準のデバッグ用（公開された既定のパスワード）で、秘密ではない。配布版の署名には使わない。
- **限界:** 照合できるのは、開発機に現にある値だけ。過去に作り直して捨てた古いトークンは照合できない。push 前に、`gitleaks` などの専用の道具でも全履歴を検索する（この開発機には未導入）。

## 10. 証明書と CA はどこに立つか（ADR 0017・0019）

**CA は Let's Encrypt（公的な CA）だけ。自前の CA は立てない。** 端末が信頼するのは公的な CA だけで、自前の CA は端末に入れられないため。

### 10.1 中継と名前サービスは別の仕事（置き換えではない）
| | 中継 | 名前サービス |
|---|---|---|
| 役目 | **外出先**の端末とブリッジをつなぐ（ブリッジは家のルーターの内側で、外から入れない） | **家の中**で、端末がブリッジに**直接**つなげるよう、名前を用意する |
| なくなると | 外出先から使えない | 家でも中継経由になる（遅くなるだけ） |

### 10.2 利用者ごとの証明書を取る流れ
```mermaid
sequenceDiagram
  participant B as ブリッジ(利用者のPC)
  participant LE as Let's Encrypt (CA)
  participant NS as 名前サービス (VPS。lan.spellink.uk の DNS サーバ)
  participant D as 端末
  Note over B: ① 自分の秘密鍵を作る（この鍵はPCの外に出ない）
  B->>NS: ② 登録 {ID, 公開鍵, LAN の IP, 署名}
  B->>LE: ③ <ID>.lan.spellink.uk の証明書がほしい
  LE-->>B: ④ _acme-challenge.<ID>.lan… に、この値の TXT を置いて
  B->>NS: ⑤ その TXT を置いて（署名つき）
  LE->>NS: ⑥ DNS で TXT を問い合わせる
  NS-->>LE: TXT の値
  LE-->>B: ⑦ 証明書を発行（利用者ごとに 1 枚。90 日、自動更新）
  D->>NS: ⑧ <ID>.lan.spellink.uk の IP は？
  NS-->>D: ブリッジの LAN の IP
  D->>B: ⑨ wss（⑦の証明書で TLS。中は Noise）
```

### 10.3 DNS の委任（ドメインの側の設定は、これだけ）
`spellink.uk` のゾーンに次を置く。**トークンは作らない。**

| レコード | 意味 |
|---|---|
| `ns1.spellink.uk  A  <VPS の IP>`（プロキシなし） | 名前サービスの場所 |
| `lan.spellink.uk  NS  ns1.spellink.uk` | `lan` 以下の問い合わせを名前サービスに任せる |
| `_acme-challenge.names.spellink.uk  CNAME  _acme-challenge.<名前サービスの ID>.lan.spellink.uk` | 名前サービス自身の証明書の確認を、自分の DNS に任せる |
| `_acme-challenge.jp.relay.spellink.uk  CNAME  _acme-challenge.<jp の中継の ID>.lan.spellink.uk`（`us.relay` も同じ） | 中継の証明書の確認を、名前サービスの DNS に任せる |
| `jp.relay.spellink.uk`・`us.relay.spellink.uk`  A  `<各 VPS の IP>`、`names.spellink.uk`  A  `<名前サービスの VPS の IP>` | 中継と名前サービスの入口 |

CNAME の宛先の ID は、各コンテナの最初の起動で表示される（`relay-operations.md` §3）。名前サービスが触れるのは、委任された `lan.spellink.uk` 以下だけ。

### 10.4 経路ごとの CA と、秘密鍵の持ち主
| 経路 | 名前 | 証明書を出す CA | 秘密鍵の持ち主 |
|---|---|---|---|
| ① 自前のトンネル | 利用者のドメイン | Cloudflare（入口で TLS を終端） | Cloudflare |
| ② 中継 | `jp.relay.spellink.uk`・`us.relay.spellink.uk` | **Let's Encrypt** | 中継（volume の中。外に出ない） |
| ③ 家の直結 | `<ID>.lan.spellink.uk`（利用者ごとに別の名前・別の証明書） | **Let's Encrypt** | **各利用者のブリッジ（外に出ない）** |
| 名前サービスの API | `names.spellink.uk` | **Let's Encrypt** | 名前サービス |

### 10.5 全員で 1 枚の `*.lan.spellink.uk` を配らない理由
- ワイルドカードの秘密鍵を全員のブリッジに配ることになる。**誰か 1 人の PC から鍵が漏れると、全員の名前を騙れる。** 漏れたら失効が必要で、全員が止まる。
- 認証局の規約は、証明書の秘密鍵を他人に配ることを許していない。違反すると失効されうる。
- 利用者ごとの名前と証明書にすれば、漏れても**その 1 人だけ**。
- 通信の中身は、証明書が漏れても Noise が守る（TLS は入口の鍵、Noise は端から端までの鍵）。

### 10.6 Let's Encrypt の上限との関係
- 新規 50 枚/週/登録ドメイン = **新しく直結を使い始める人が週 50 人まで**。更新は同じ名前なら数えない。総数の上限ではない。
- 超えても、直結が遅れるだけで、中継で使える（取得はブリッジが裏で、CA の示した時刻まで待って、やり直す。利用者には「準備中」と出るだけ）。
- 足りなくなったら: 登録ドメインを増やす、例外申請、予備の CA（Google Trust Services。EAB を名前サービスが利用者ごとに出す仕組みが要る）。
