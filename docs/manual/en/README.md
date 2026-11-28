# Spellink Manual (for users)

English · [日本語](../ja/README.md) (the original)

> This English version was written with AI help from the Japanese original, so some phrasing may sound unnatural.
> Corrections are very welcome — see [CONTRIBUTING.md](../../../CONTRIBUTING.md).

Lasso your handwriting on a Supernote, and the AI on your own computer answers — **written back onto the page as handwriting**.
This manual takes a first-time user from installation to everyday use.

> **Status: early alpha.** Verified on a Linux computer with a Supernote Nomad. Windows and macOS are not yet verified on real
> hardware. The relay (the way to use it away from home) is being set up by the project.

## The big picture

```
 Supernote (plugin)  ──encrypted──▶  bridge (your computer)  ──▶  Claude Code (your own subscription)
   lasso your writing                  passes the question to the AI,
   ◀── answer as pen strokes ──        turns the answer into handwriting
```

- The **plugin** goes on the device. It turns your handwriting into text on the device and sends the text to the bridge.
- The **bridge** is a small program on your computer. It never touches your AI login: it just calls the Claude Code you are already signed in to.
- Between the device and the bridge, everything is encrypted with a key only those two hold.

## Reading order

| Chapter | Contents | Time |
|---|---|---|
| [1. Getting started](01-start.md) | What it does, requirements, cost, the short path | 5 min |
| [2. Install the bridge](02-bridge.md) | Install, start, and run automatically | 10 min |
| [3. Connect the device](03-device.md) | Pick a way to connect, install the plugin, pair | 10 min |
| [4. Using it](04-use.md) | Asking, the Bridge App page, AI settings | reading |
| [5. Update, stop, remove](05-maintenance.md) | Updating; what to delete when you stop | when needed |
| [6. Troubleshooting](06-troubleshooting.md) | Look up by symptom | when needed |
| [7. What is sent and what stays](07-privacy.md) | Privacy and the pairing token | 5 min |

If something is wrong, first run `spellink-bridge check` (first row of [6. Troubleshooting](06-troubleshooting.md)).
