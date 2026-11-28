# 3. Connect the device

The device can only connect to a **`wss://` address with a public certificate**, so you pick one way to reach the bridge.

## Choose a way to connect
| Way | For whom | What to do |
|---|---|---|
| **The Spellink relay** (standard) | Most people; also use it away from home | Add `"relayUrls": ["wss://jp.relay.spellink.uk", "wss://us.relay.spellink.uk"]` to `config.json`. **Being set up by the project** |
| A direct connection at home | Use only at home | `"lan": {"enabled": true}` in `config.json`. The name service is being set up |
| Your own tunnel | Advanced users only | Prepare your own domain and a Cloudflare tunnel, and add `"extraUrls": ["wss://your-address/v1"]` ([guide](../../guides/cloudflare-tunnel-setup.md), Japanese) |

- **The tunnel is optional.** If you use the relay you do not need it, and no warning appears without it.
- You can list several; the device uses whichever connects.
- After changing `config.json`, restart the bridge ([2. Install the bridge](02-bridge.md#restarting-after-changing-settings)).

## Register the device
On the computer, choose a name for the device (any; for example `my-nomad`) and run:
```
spellink-bridge device add my-nomad
```
"Step 1/4 done" means it worked. A setup file, `Spellink-my-nomad.snplg`, is made in the folder where you ran the command.
- The file works **only for that device and expires (24 hours by default)**. Do not give it to anyone until pairing is done.
- `already exists` means the name is taken: choose another or check with `spellink-bridge device list`.

## Install on the device
1. Put `Spellink-my-nomad.snplg` in the device's `MyStyle` folder (USB, or sending the file to the device).
2. On the device, install it from Settings → Apps → Plugins.
3. Create a **new note** (old notes do not show the button).

## Pair
| Step | Where | What to do |
|---|---|---|
| 3 | Device | Spellink settings → "Pair with the bridge" → "Pair". An **8-digit number and a QR code** appear (for 90 seconds by default) |
| 4 | Computer | Within 90 seconds, run `spellink-bridge device pair` and enter the 8 digits |

- You can give the number first: `spellink-bridge device pair 12345678`.
- If the computer has a camera, `spellink-bridge device pair --web` opens a scanning page in the browser. **Typing the digits is the main way; the QR is a shortcut.**
- "That number was not accepted": check the number on the device. If time ran out, press "Pair" on the device again (five wrong tries close that window).

When the device says it is paired and `spellink-bridge device list` shows `active`, you are done.

Next: [4. Using it](04-use.md)
