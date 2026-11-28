# Spellink

**English** · [日本語](README.ja.md)

Ask an AI about your handwriting on a Supernote, and get the answer written back onto the page **as handwriting**.

1. Write a question in a note.
2. Lasso it and tap **Ask AI**.
3. The answer appears below your writing, line by line, as real pen strokes. A follow-up written in the same note continues the conversation.

> **Status: early alpha.** It works end to end on a Supernote Nomad with a bridge on a Linux computer; many pieces
> are still being verified on the device. Not yet packaged for other people — see the roadmap in
> [docs/notes/progress/release-plan.md](docs/notes/progress/release-plan.md) (Japanese).

## How it works

```
Supernote (plugin)  ── wss ──▶  spellink-bridge  (your Mac / Linux / Windows computer)
  lasso → on-device OCR           └─ asks the AI — today: the official Claude Code CLI, signed in with your own account
  ◀─ answer as pen strokes ──     └─ turns the answer into handwriting strokes
```

- `plugin/` — the Supernote plugin (React Native, [sn-plugin-lib](https://docs.supernote.com/en)). Recognition happens on the device.
- `bridge/` — a small Rust program, one binary per OS (about 4 MB, 10 MB of RAM). **You run it on your own computer.** It never sees or
  stores your AI credentials: it only starts the unmodified CLI you have already signed in to
  ([ADR 0005](docs/decisions/0005-claude-code-bridge-on-user-machine.md)). Which AI it asks is a setting
  ([ADR 0015](docs/decisions/0015-ai-provider-abstraction.md)); Claude Code is the only one so far.
- The device only speaks `wss://` with a public certificate. There are three ways to reach the bridge
  ([architecture](docs/guides/architecture.md)): the **Spellink relay** (operated by the project, funded by donations; being set up —
  [ADR 0020](docs/decisions/0020-operating-model.md)), a **direct connection at home** (the bridge gets its own certificate for a name
  under `lan.spellink.uk`; [ADR 0017](docs/decisions/0017-lan-name-service.md)), or **your own tunnel**
  ([guide](docs/guides/cloudflare-tunnel-setup.md)). Everything is end-to-end encrypted, so the relay sees only ciphertext.
- `docs/` — decisions (ADRs), the specification (state transition tables), guides and progress notes.

## Use it

Start with the **[user manual](docs/manual/en/README.md)** ([日本語](docs/manual/ja/README.md)): install the bridge, put the plugin on your
Supernote, pair, and ask. About 30 minutes for a first-time user.

## Requirements

- A Supernote Nomad (tested) or Manta.
- A computer that stays on, with [Claude Code](https://code.claude.com) installed and signed in (your own subscription; no API key).
- A way for the device to reach it over `wss://` (see above; the relay is the standard way once it is open).

## Build it yourself

```bash
cd bridge
cargo build --release --locked             # Rust (rustup.rs); target/release/spellink-bridge
./spellink-bridge serve                    # first run downloads the glyph data and creates the config
./spellink-bridge service install          # start by itself with the computer (add --dry-run to look first)
./spellink-bridge check                    # reach the bridge the way the plugin does, encrypted handshake included
./spellink-bridge device add my-nomad      # makes a setup file for one device; the device shows a QR and an 8-digit code to pair
```

The bridge that makes setup files has the plugin built into it: build it with `bridge/scripts/build-with-plugin.sh`
(a plain `cargo build` has no plugin inside). The plugin carries no address or token of its own; the device gets them by pairing.

Step by step for a user: the [manual](docs/manual/en/README.md). Device test notes: [docs/notes/device-test-guide.md](docs/notes/device-test-guide.md) (Japanese).
What a user needs to do and know: [docs/guides/bridge-and-relay-for-users.md](docs/guides/bridge-and-relay-for-users.md) (Japanese).

## Security and privacy

- Between the device and the bridge everything is encrypted end to end ([Noise](https://noiseprotocol.org), keyed from a pairing
  token only the two hold), so a tunnel or relay in the middle sees only ciphertext.
- **The pairing token is the key to your bridge.** Keep it private; see [SECURITY.md](SECURITY.md).
- What is sent to the AI: the text recognized from the lasso, and the earlier questions and answers of the same note. **No images.**
- What is stored, on your computer only: settings, and each note's conversation (`~/.config/spellink-bridge/`, readable by you only,
  **not encrypted**). The device stores nothing.

## Contributing

Spellink is run by one person in their spare time, so replies can take a while (best effort). It is an early alpha: things will change and may break.
Spellink は個人が余暇に運営しています。返事は遅れることがあります(ベストエフォート)。初期のアルファ版で、変更や不具合があります。

[CONTRIBUTING.md](CONTRIBUTING.md). The project works from specification tables
([docs/spec/README.md](docs/spec/README.md)) and a development policy ([ADR 0014](docs/decisions/0014-engineering-policy.md)).

## Credits

- Handwriting glyphs: [KanjiVG](https://kanjivg.tagaini.net) © Ulrich Apel, CC BY-SA 3.0 — **downloaded on your computer at setup, not
  redistributed here**. Latin letters and symbols: Hershey fonts via [hersheytext](https://github.com/techninja/hersheytextjs).
- Other software this uses: [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

Not affiliated with Anthropic or Ratta. Claude is a trademark of Anthropic; Supernote is a trademark of Ratta.

## License

Apache-2.0 — see [LICENSE](LICENSE).
