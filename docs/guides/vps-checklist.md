# VPS の作業表: Kagoya（日本）で今できること、OVHcloud（米国）を借りたあとにやること

対象: 運営の VPS を 2 台立てる人（ユーザー [U] と、立ち会うエージェント [A]）。詳しい理由と手順は `relay-operations.md`、構成は `architecture.md`、決定は ADR 0017・0019・0020・0021。**このファイルは、作業の順番と持ち物のチェックリスト**。実環境は未確認なので、つまずいたら、この表を直す。

## 0. 全体の順序と、置き場所の決め方
| 段階 | どこで | 何をする | 前提 |
|---|---|---|---|
| A | **Kagoya（日本）** | 名前サービス（`ns1`・`names`）と、日本の中継（`jp.relay`）を動かす | VPS を借りた（済） |
| B | **OVHcloud（米国）** | 米国の中継（`us.relay`）を動かす | 段階 A の名前サービスが動いている |
| C | 両方が動いたあと | ブリッジの設定、集計の登録、トンネルの切り離し、利用者への案内 | A と B |

**名前サービスは 1 台だけ**。置き場所の提案は **Kagoya（日本）**。理由: 今すぐ始められる。名前サービスは軽い（DNS と署名つきの小さな API）ので、米国の中継からの往復が遠くても問題ない。**米国の VPS は「人が少なくなったら落とす」予定なので、落とす側に名前サービスを置かない。** 名前サービスが落ちている間は、新しい直結の証明書が取れず、既存の証明書は更新できる限り動き続ける（中継は影響を受けない）。

## 1. 先に用意するもの（段階 A の前）
- [ ] Kagoya の VPS の **固定の IPv4 アドレス**（`<JP の IP>` とする）と、SSH で入れること。
- [ ] `spellink.uk` の DNS を編集できること（Cloudflare）。**API トークンは作らない**。
- [ ] 手元の開発機から VPS に `rsync` と `ssh` が使えること（push しないので、ソースは rsync で送る）。
- [ ] メモ用: 各サービスが最初の起動で表示する **ID**（32 桁）を控える場所。

## 2. 段階 A: Kagoya（日本）で、今できること

### A1. VPS を整える（Kagoya の管理画面と SSH）
OS は **Debian でも Ubuntu でもよい**（手順は同じ。違いは、Ubuntu だけ A2 の `systemd-resolved` の競合が起きること、`chrony.conf` の既定のプール名が `ubuntu` か `debian` かだけ）。以下は 1 項目ずつ、**意味 → 設定 → 確認**の順に書く。

**1. OS と CPU**
- 意味: イメージは VPS 上で作るので、CPU の種類が合っていること。
- 設定: Debian か Ubuntu の LTS。`uname -m` が `x86_64` であること（開発機は aarch64 なので、イメージは VPS 上で作る。A4）。

**2. SSH と作業用ユーザー**
- 意味: パスワードでの総当たりを断ち、root を使わない（誤操作と乗っ取りの被害を小さくする）。
- 設定: 作業用ユーザーを作り（`adduser <ユーザー>`、`usermod -aG sudo <ユーザー>`）、公開鍵を `~/.ssh/authorized_keys` に置く（権限は `~/.ssh` が 700、`authorized_keys` が 600）。**別の端末から鍵でログインできると確かめてから**、`/etc/ssh/sshd_config.d/10-spellink.conf` に書く:
  ```
  PasswordAuthentication no
  PermitRootLogin no
  KbdInteractiveAuthentication no
  ```
  `sudo sshd -t`（文法の確認）→ `sudo systemctl reload ssh`。**今のログインは閉じず**、新しい窓でログインできるか見る。
- 確認: `sudo sshd -T | grep -Ei 'passwordauth|permitroot'` が `no` になっている。
- Kagoya では: 既定で root が**公開鍵だけ**で入る（作成時に「VPS ログイン用認証キー」を発行）。パスワードの SSH は元から無いので、`PermitRootLogin no` にしてよい。管理画面の**コンソール**は sshd を通らず、`root` と「コンソールログインパスワード」で入れる（鍵をなくしたときの非常口。長く強いものにする）。鍵ユーザーの `sudo` にはパスワードが要る（`passwd <ユーザー>`）。秘密鍵は 2 か所以上に保管する（なくすと、スナップショット → 初期化）。

**3. ファイアウォール**
- 意味: 使わないポートを外から見えなくする。**二重**（Kagoya の管理画面と、ホスト）にして、片方の設定ミスを、もう片方で受ける。
- 設定: **Kagoya の「パケットフィルター」（管理画面のファイアウォール）と、ホストのファイアウォールで、開けるのは次だけ**: `22/tcp`（自分の IP だけに絞れるなら絞る）、`53/tcp`、`53/udp`、`443/tcp`、`8443/tcp`。ホストは `ufw` が簡単（`ufw default deny incoming`、`ufw allow 22/tcp` ほか、**最後に** `ufw enable`。先に 22 を許可しないと、自分が締め出される）。
- 注意: **Docker が公開したポートは、`ufw` を素通りする**（Docker が `iptables` に直接書くため）。だから、公開するポートを `compose.yaml` の `ports` で絞り、**外側の管理画面のフィルターを本命にする**。
- 確認: 別のネットワークから `nmap -Pn <JP の IP>`（開いているのが上の 5 つだけ）。

**4. 自動更新（`unattended-upgrades`）と時刻同期**
- **自動更新**
  - 意味: セキュリティの修正を、人手なしで入れる（動かすのは `apt-daily.timer` と `apt-daily-upgrade.timer`。サービスを start するだけでは足りない。**自動で動かす設定を有効にする**）。
  - 設定:
    ```bash
    sudo apt install unattended-upgrades
    sudo dpkg-reconfigure -plow unattended-upgrades   # 「Yes」
    ```
    `/etc/apt/apt.conf.d/20auto-upgrades` が `APT::Periodic::Update-Package-Lists "1";` と `APT::Periodic::Unattended-Upgrade "1";` で作られる。**対象は既定でセキュリティ更新だけ**（`/etc/apt/apt.conf.d/50unattended-upgrades` の `Origins-Pattern`）。これで足りる。
  - 任意（再起動）: カーネルの更新は、再起動しないと効かない。自動で再起動するなら `50unattended-upgrades` で `Unattended-Upgrade::Automatic-Reboot "true";` と `Unattended-Upgrade::Automatic-Reboot-Time "04:00";` のコメントを外す。名前サービスは再起動の間、数十秒止まる（コンテナは `restart: unless-stopped` で上がる）。避けるなら自動再起動は入れず、`/var/run/reboot-required` があるときに手で再起動する。
  - 注意: Docker の公式リポジトリの Docker は既定の対象外（更新でデーモンが再起動し、コンテナが止まるので、手動のままにする）。
  - 確認: `systemctl list-timers 'apt-daily*'`（2 つ動いている）、`sudo unattended-upgrade --dry-run --debug`（エラーなし）、数日後に `/var/log/unattended-upgrades/` にログ。
- **時刻同期（`chrony`）**
  - 意味: **署名つきの要求は、時刻のずれ 5 分以内**。ずれると、中継・名前サービスへの要求が拒まれ、証明書の取得にも響く。`chrony` は NTP のクライアント。
  - 設定: `sudo apt install chrony`。日本のサーバーに替えるなら `/etc/chrony/chrony.conf` の、既定のプールの行を置き換える:
    ```
    # Use Debian vendor zone.
    # pool 2.debian.pool.ntp.org iburst
    pool ntp.nict.jp iburst
    ```
    `pool`（複数のアドレスを使い分け、1 台落ちても続く）。`iburst` は、起動直後に早く合わせる指定。`/etc/chrony/sources.d/` に別の指定がないか見る。`allow` は書かない（サーバーとして待ち受けないので、`123/udp` は閉じたままでよい）。
  - 反映: `sudo systemctl restart chrony`。
  - 確認: `chronyc sources -v`（`^*` が付く行がある）、`chronyc tracking`（System time が ms 単位）、`timedatectl`（`System clock synchronized: yes`）。**`systemd-timesyncd` と同時に動かさない**（`systemctl is-active systemd-timesyncd` が `inactive`。`systemd-timesyncd` で済ますなら chrony は入れない）。

**5. Docker Engine と compose プラグイン**
- 意味: ブリッジ・中継・名前サービスを、コンテナ（約 11 MB のイメージ）で動かす。
- 設定: Docker 公式の手順（apt のリポジトリを足して `docker-ce`、`docker-compose-plugin` を入れる）。`sudo usermod -aG docker <ユーザー>` のあと、**入り直す**。
- 注意: `docker` グループは、実質 root と同じ権限。作業ユーザー以外は入れない。
- 確認: `docker compose version`、`docker run --rm hello-world`。
- 任意: ログの肥大は、`compose.yaml` の `logging`（A8）で抑える。

### A2. 53 番ポートの競合を先に片づける（Ubuntu でよく起きる）
Ubuntu は `systemd-resolved` が `127.0.0.53:53` で待っている。コンテナが `53` を開こうとして、`address already in use` になることがある。
- 確認: `sudo ss -lntup | grep ':53 '`
- 対処（どちらか）: `/etc/systemd/resolved.conf` に `DNSStubListener=no` を書いて `sudo systemctl restart systemd-resolved`（`/etc/resolv.conf` は `/run/systemd/resolve/resolv.conf` を向かせる）。または、`compose.yaml` の `ports` に、公開の IP を指定する（`"<JP の IP>:53:5353/udp"` と tcp）。

### A3. DNS（Cloudflare）に、まず 3 つを置く（**プロキシなし = 灰色の雲**）
| 種類 | 名前 | 値 |
|---|---|---|
| A | `ns1.spellink.uk` | `<JP の IP>` |
| NS | `lan.spellink.uk` | `ns1.spellink.uk` |
| A | `names.spellink.uk` | `<JP の IP>` |
- `jp.relay` の A は、中継を立てるとき（A7）に置く。`bridge.spellink.uk`（いまのトンネル）は、**消さない**。

### A4. ソースを送って、イメージを作る（VPS 上で）
```bash
# 開発機から（target は送らない）
rsync -a --delete --exclude target --exclude .git bridge/ <ユーザー>@<JP の IP>:~/spellink/bridge/
rsync -a deploy/ <ユーザー>@<JP の IP>:~/spellink/deploy/
# VPS で
cd ~/spellink && docker build -f bridge/Dockerfile.edge -t spellink-edge bridge
```
- **メモリの目安は 2 GB 以上**（Rust のリリースビルドは、LTO で重い。**未確認**）。足りなければ、スワップを足す（`fallocate -l 4G /swapfile` ほか）か、一時的に大きいプランで作る。
- 作ったあと、`docker image ls` で、**約 11 MB** のイメージができたか確認する。

### A5. 設定ファイル（VPS で）
`~/spellink/deploy/vps/.env.example` を `.env` にコピーする。Kagoya では:
```
RELAY_NAME=jp.relay.spellink.uk
NAMES_URL=https://names.spellink.uk:8443
INFRA_IDS=
```

### A6. 名前サービスを起動して、委任（NS）が効くか確かめる（**まず staging**）
1. `deploy/vps/compose.yaml` の `names` の `command` の末尾に `--ca staging` を足す（Let's Encrypt の試験環境。本番の枠を使わない）。
2. `docker compose -f deploy/vps/compose.yaml up -d names`、`docker compose -f deploy/vps/compose.yaml logs names` を見る。次のような行が出る:
   ```
   names identity 0123…（32 桁）
     _acme-challenge.names.spellink.uk.  CNAME  _acme-challenge.0123….lan.spellink.uk.
   ```
3. **この CNAME を Cloudflare に置く**（これが、DNS に置く最後の種類のもの。プロキシなし）。
4. 外から確認:
   - `dig @<JP の IP> lan.spellink.uk NS` → `aa`（権威の応答）が付く。
   - `dig lan.spellink.uk NS`（別の場所から）→ `ns1.spellink.uk` が返る（委任が効いている）。
   - `dig _acme-challenge.names.spellink.uk CNAME` → 上で置いた値。
   - 数十秒〜数分で、ログに `tls names…: ready` が出る（証明書が取れた）。出ないときは `preparing …` の行に理由が出る。
   - `curl -sk https://names.spellink.uk:8443/health` → `{"ok":true,"records":N}`（staging の証明書なので `-k`）。
5. **staging で通ったら、本番に切り替える。** staging の証明書は、**ブリッジや中継の TLS クライアントが信頼しない**ので、このまま中継をつなげない。
   ```bash
   # compose.yaml の --ca staging を消す。保存された staging の証明書を消す（identity は残す）
   docker compose -f deploy/vps/compose.yaml stop names
   docker run --rm -v spellink_names-data:/data alpine rm -f /data/cert.pem
   docker compose -f deploy/vps/compose.yaml up -d names
   ```
   - **`/data` の中の identity は消さない**（消すと ID が変わり、CNAME を置き直す）。消すのは `cert.pem` だけ。
   - 本番で `ready` が出たら、`curl -s https://names.spellink.uk:8443/health`（`-k` なし）が通る。

### A7. 日本の中継を起動する
1. Cloudflare に `A  jp.relay.spellink.uk  <JP の IP>`（プロキシなし）を置く。
2. 中継の ID を知る: `docker compose -f deploy/vps/compose.yaml run --rm relay`（出た行の ID と CNAME を控え、Ctrl+C）。
   ```
   relay identity 4567…
     _acme-challenge.jp.relay.spellink.uk.  CNAME  _acme-challenge.4567….lan.spellink.uk.
   ```
3. この CNAME を Cloudflare に置く。
4. **名前サービスに、この中継の ID を知らせる**（週の枠の予約。ADR 0017 の追記）: `.env` の `INFRA_IDS=4567…` を書いて、`docker compose -f deploy/vps/compose.yaml up -d names`（作り直す）。
5. `docker compose -f deploy/vps/compose.yaml up -d relay`。ログに `tls …: ready`。
6. 確認: `curl -s https://jp.relay.spellink.uk/health` → `{"ok":true,"bridges":0}`、`curl -s https://jp.relay.spellink.uk/v1/stats` → `{"v":1,…"days":[…]}`。

### A8. 運用の最小限（A の最後）
- [ ] ログが肥大しない（`compose.yaml` の `logging` で 10 MB × 3）。
- [ ] 再起動後に自動で上がる（`restart: unless-stopped`。VPS を再起動して、確認する）。
- [ ] バックアップ: `spellink_names-data` と `spellink_relay-data` を、手元に取っておく（identity を失うと、ID が変わる）。例: `docker run --rm -v spellink_names-data:/d -v "$PWD":/b alpine tar czf /b/names-data.tgz -C /d .`。**手元に置いたら、権限を絞る（0600）。他人に見せない。**
- [ ] 外から `53/udp` が届くこと（`dig @<JP の IP> …` を別のネットワークから）。

## 3. 段階 B: OVHcloud（米国）を借りたあと

### B1. 借りるとき
- 場所は米国（またはカナダ）。**通信量が無制限のプラン**。固定の IPv4。Linux（Ubuntu の LTS）。メモリは 2 GB 以上（イメージのビルド。A4 と同じ）。
- OVH の管理画面で、ファイアウォールを確認する。**開けるのは `22/tcp` と `443/tcp` だけ**（米国の VPS は中継だけを動かす。名前サービスの `53`・`8443` は開けない）。

### B2. VPS を整える
A1 と同じ（SSH の鍵、一般ユーザー、ファイアウォール、自動更新、時刻同期、Docker）。ファイアウォールで開けるのは B1 の 2 つだけ。**A2（53 番の競合）は要らない。**

### B3. ソースを送って、イメージを作る
A4 と同じ（`<US の IP>` に送る）。

### B4. 設定ファイル
`.env` を作る（米国には `INFRA_IDS` は要らない）:
```
RELAY_NAME=us.relay.spellink.uk
NAMES_URL=https://names.spellink.uk:8443
```

### B5. DNS（Cloudflare）
`A  us.relay.spellink.uk  <US の IP>`（プロキシなし）。

### B6. 米国の中継の ID を知る → 登録する
1. `docker compose -f deploy/vps/compose.yaml run --rm relay`（ID と CNAME が出る。Ctrl+C）。
2. CNAME を Cloudflare に置く: `_acme-challenge.us.relay.spellink.uk  CNAME  _acme-challenge.<US の ID>.lan.spellink.uk`。
3. **日本の VPS で**、`.env` の `INFRA_IDS` に米国の ID を足す（カンマ区切り: `INFRA_IDS=<jp の ID>,<us の ID>`）。`docker compose -f deploy/vps/compose.yaml up -d names`。
4. 米国の VPS で、**中継だけ**を起動: `docker compose -f deploy/vps/compose.yaml up -d relay`（`names` は起動しない。compose の `depends_on` は外してある）。

### B7. 確認
- `curl -s https://us.relay.spellink.uk/health` と `/v1/stats`。
- 日本の VPS のログに、米国の中継の登録（名前サービスへの `register`）が出る。
- 米国の中継のログに `tls …: ready`。名前サービスへの往復（米国 ⇄ 日本）が通っていること。

### B8. 米国の VPS を落とすとき（人が少なくなったら）
- 先に、利用者に知らせる。ブリッジは、`relayUrls` の残りの中継（日本）に登録済みなので、端末は競争で日本に流れる。
- `docs/usage/relays.json` から `us.relay` を外し、DNS の `us.relay` の A と CNAME を消す。`INFRA_IDS` から米国の ID を外す。

## 4. 段階 C: 両方が動いたあと
1. **ブリッジの設定（利用者全員）**: `config.json` に
   ```json
   "relayUrls": ["wss://jp.relay.spellink.uk", "wss://us.relay.spellink.uk"]
   ```
   ブリッジは両方に登録し、端末は両方を同時に試して、近いほうを使う。**ブリッジを再起動**し、`device add`（または `device package`）で作る設置用ファイルから、新しい URL が端末に入る。
2. **既定にする**: 動くと確認できたら、ブリッジの既定の `relayUrls` に、この 2 つを入れる（[A]。`config.rs`）。`install-guide.md` の「準備中」を消す（[A]）。
3. **集計**: `docs/usage/relays.json` に `["https://jp.relay.spellink.uk","https://us.relay.spellink.uk"]` を書く（[A]）。Actions は、push したあとで動く。
4. **トンネルの切り離し**（`bridge.spellink.uk`）: 中継経由で、実機から問題なく使えると確認したあとに、[U] `sudo systemctl stop cloudflared`、`sudo systemctl disable cloudflared`、Cloudflare の DNS の `bridge.spellink.uk` を消す。ブリッジの `extraUrls` から `wss://bridge.spellink.uk/v1` を外す。
5. **実機で確かめる**: 診断の「3. Network」で、`jp.relay` と `us.relay` の往復時間を見る。家の中の直結（`<id>.lan.spellink.uk`）は、`lan.enabled` を true にして、ブリッジのログの `lan: …` を見る。
6. **公開前**: プライバシーポリシーと利用規約に、名前サービスと中継が見るもの・数えるものを書く（`docs/legal/`）。

## 5. つまずきやすい所（まとめ）
| 症状 | 原因の候補 | 確認 |
|---|---|---|
| `address already in use`（53） | `systemd-resolved` | A2 |
| 証明書が出ない（`preparing`） | CNAME がない、NS の委任が効いていない、Let's Encrypt の失敗の制限（同じ名前の検証の失敗は 1 時間に 5 回まで） | `dig` で CNAME・NS、ログの理由と待ち時刻。**待つ。繰り返し再起動しない** |
| staging から本番に切り替えても staging の証明書のまま | `cert.pem` が残っている | A6 の 5 |
| 中継が名前サービスにつながらない | 名前サービスの証明書が staging のまま、`8443` が閉じている | `curl https://names.spellink.uk:8443/health`（`-k` なし）、ファイアウォール |
| 直結の注文が `lan_quota` | 週の枠（45）が埋まった | ブリッジのログに空く時刻。急ぐなら `--new-per-week` を上げる（50 を超えない範囲で） |
| 中継の新規の証明書が取れない | `INFRA_IDS` に中継の ID を入れ忘れ、かつ週の枠が埋まっている | `.env` の `INFRA_IDS`（A7 の 4・B6 の 3） |
| 時刻のずれ | 署名つきの要求は、5 分以内 | `timedatectl status` |

## 6. 役割
- **[U] しかできないこと**: VPS の契約、Kagoya・OVH の管理画面（パケットフィルター）、Cloudflare の DNS の編集、SSH の鍵、`sudo`（トンネルの切り離し）。
- **[A] ができること**: 手順の確認（ログ・`dig` の出力を見て、原因を切り分ける）、設定の追記、ブリッジの既定値の変更、集計の登録、文書の更新。**VPS の中身は、[U] が貼ってくれたログだけを見る。トークンや秘密鍵（`identity` のファイル、`.env` の他の値）は、貼らない。**
