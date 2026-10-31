# Spellink

Ask Claude about your handwriting on a Supernote, and get the answer written back onto the page **as handwriting**.

1. Write a question in a note.
2. Lasso it and tap **Ask Claude**.
3. The answer appears below your writing, line by line, as real pen strokes.

> Status: early spike (v0.0.1). Tested on a Radxa ROCK 5B+ bridge; on-device behaviour is being verified on a Supernote Nomad.

## How it works

```
Supernote (plugin)  ──LAN──▶  spellink-bridge (your Mac / Linux / Windows PC)
  lasso → on-device OCR          └─ runs the official Claude Code CLI, signed in with your own account
  ◀── answer as pen strokes ───  └─ turns the answer into handwriting strokes
```

- `plugin/` — Supernote plugin (React Native, [sn-plugin-lib](https://docs.supernote.com/en)).
- `bridge/` — small Go relay, shipped as a single binary per OS/arch (~7 MB, ~18 MB RAM). It never sees or stores
  your Claude credentials; it only launches the
  unmodified `claude` CLI you have already signed in to. See [ADR 0005](docs/decisions/0005-claude-code-bridge-on-user-machine.md).
- `docs/` — design decisions (ADRs), research notes and progress.

Requirements: a Mac / Linux / Windows computer on the same network with [Claude Code](https://code.claude.com)
installed and signed in.

```bash
cd bridge && scripts/build.sh dev linux/arm64   # or darwin/arm64 …; needs Go 1.24+
./dist/spellink-bridge-linux-arm64 serve           # first run downloads the glyph data
./dist/spellink-bridge-linux-arm64 pair            # prints the connection file for the plugin
```

Release binaries will be published by CI (`bridge-v*` tags).

See [docs/notes/device-test-guide.md](docs/notes/device-test-guide.md) for installing the plugin (Japanese).

## Security note

The spike talks to the bridge over **unencrypted WebSocket** with a shared token. Use it only on a network you trust
until encryption lands ([ADR 0007](docs/decisions/0007-relay-transport-and-security.md)).

## Credits

- Handwriting glyphs: [KanjiVG](https://kanjivg.tagaini.net) © Ulrich Apel, CC BY-SA 3.0 — downloaded at setup, not redistributed here.
- Latin symbols: Hershey fonts via [hersheytext](https://github.com/techninja/hersheytextjs) (MIT).

Not affiliated with Anthropic or Ratta. Claude is a trademark of Anthropic.

## License

Apache-2.0
