# 2. Install the bridge

## 1. Install
1. From the Spellink Releases page, download `spellink-bridge-…` for your OS.
2. Choose where to put it.
   - Linux / macOS: `~/.local/bin/spellink-bridge`, then make it executable: `chmod +x ~/.local/bin/spellink-bridge`
   - Windows: `C:\Users\you\bin\spellink-bridge.exe`
3. Check: `spellink-bridge version` (a version number means it worked).
   - If macOS says the developer cannot be verified, allow it in System Settings → Privacy & Security.

> While there are no Releases, build it yourself (see "Build it yourself" in the [README](../../../README.md)).
> Making the setup file for the device needs the build with the plugin inside (`bridge/scripts/build-with-plugin.sh`).

## 2. Sign in to Claude Code
Run `claude` and sign in. Later, if the top of the Bridge App page (`spellink-bridge open`) says "Claude Code is ready", the bridge has found it.

## 3. Start it
```
spellink-bridge serve
```
The first start creates the config file and builds the handwriting glyph data (a few minutes; needs the internet).

Where `config.json` lives:
- Linux: `~/.config/spellink-bridge/`
- macOS: `~/Library/Application Support/spellink-bridge/`
- Windows: `%AppData%\spellink-bridge\`

When it is running, stop it with `Ctrl+C`.

## 4. Run it with the computer
```
spellink-bridge service install
```
No administrator rights needed. Add `--dry-run` to see first what it would do. Status: `spellink-bridge service status`.
(Linux uses a systemd user service, macOS launchd, Windows Task Scheduler.)

## 5. Check
```
spellink-bridge check
```
It reaches the bridge the way the device does, down to the encrypted handshake. OK means the bridge accepts connections.
To see it on a page, use `spellink-bridge open` ([4. Using it](04-use.md#the-bridge-app-page)).

## Restarting after changing settings
After you change `config.json`, restart the bridge:
- Linux: `systemctl --user restart spellink-bridge`
- macOS / Windows: `spellink-bridge service uninstall`, then `spellink-bridge service install`
- If you run it by hand: `Ctrl+C`, then `spellink-bridge serve`

Next: [3. Connect the device](03-device.md)
