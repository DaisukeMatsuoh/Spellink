# Third-party notices / 第三者のソフトウェアとデータ

Spellink itself is Apache-2.0 ([LICENSE](LICENSE)). It uses the following. The licenses were read from each package's own
license file (Go modules: `go list -m all`, 2026-11-08); check the package itself for the full text and copyright lines.
Spellink 自体は Apache-2.0。次のものを使っている。ライセンスは、各パッケージのライセンスファイルから確認した。

## Data downloaded on the user's computer (not shipped)

| Data | Source | License |
|---|---|---|
| Handwriting strokes for kanji and kana | [KanjiVG](https://kanjivg.tagaini.net), © Ulrich Apel and contributors | **CC BY-SA 3.0** |
| Latin letters and symbols | Hershey fonts, via [hersheytext](https://github.com/techninja/hersheytextjs) | MIT (the fonts: public domain with an attribution request) |

The bridge downloads these when first started and turns them into `glyphs.bin` **on the user's own computer**. Spellink does not
redistribute them. `glyphs.bin` is derived from KanjiVG: if you share it, share it under CC BY-SA 3.0 with the attribution above.
文字データは、利用者のパソコンで取得して作る。このリポジトリでは再配布しない。`glyphs.bin` は KanjiVG の派生物なので、共有するなら CC BY-SA 3.0 と表示が要る。

## Bridge (Go)

| Module | License |
|---|---|
| github.com/coder/websocket | ISC |
| github.com/flynn/noise | BSD-3-Clause |
| github.com/caddyserver/certmagic | Apache-2.0 |
| github.com/caddyserver/zerossl | MIT |
| github.com/libdns/libdns, github.com/libdns/cloudflare | MIT |
| github.com/mholt/acmez/v3 | Apache-2.0 |
| github.com/miekg/dns | BSD-3-Clause |
| github.com/klauspost/cpuid/v2 | MIT |
| github.com/zeebo/blake3 | CC0 / public domain |
| go.uber.org/multierr, go.uber.org/zap, go.uber.org/zap/exp | MIT |
| golang.org/x/crypto, x/net, x/sys, x/text | BSD-3-Clause |

The bridge's pairing page (`bridge/internal/server/pairpage`) carries, inside the binary:

| File | License |
|---|---|
| jsQR 1.4.0 (`jsQR.js`, reads the QR code from the camera image; its license text is `jsQR.LICENSE` next to it) | Apache-2.0 |

(`github.com/kr/pretty` and `gopkg.in/check.v1` appear in the module graph for tests only.)

## Plugin (JavaScript)

| Package | License |
|---|---|
| react, react-native | MIT |
| @noble/ciphers, @noble/curves, @noble/hashes | MIT |
| sn-plugin-lib (Supernote's official SDK) | MIT |
| qrcode-generator (draws the pairing QR code) | MIT |

Development-only packages (jest, eslint, prettier, typescript, ws, …) are not part of the shipped plugin.

## Trademarks

Claude and Claude Code are trademarks of Anthropic. Supernote is a trademark of Ratta Software. Spellink is not affiliated with or endorsed by either.
