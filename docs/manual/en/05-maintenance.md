# 5. Update, stop, remove

## Updating
**Pairing is kept.** You do not need to redo it (the key lives in the device's storage).
1. **Bridge (computer):** get the new file from Releases and replace the old one — **after stopping the bridge**.
   - Linux: `systemctl --user stop spellink-bridge` → replace the file → `systemctl --user start spellink-bridge`
   - macOS / Windows: `spellink-bridge service uninstall` → replace → `spellink-bridge service install`
   - Check with `spellink-bridge version` that it is the new version.
2. **Plugin (device):** put the new `Spellink.snplg` from Releases into `MyStyle` the same way as the first time, and install it again from the device. Use **the Releases one**, not your own file (`Spellink-my-nomad.snplg`).
3. **Check:** the plugin settings show two versions, like `Spellink 0.1.0 · bridge 0.1.0`. If one is old, a hint appears.

## Stopping for a while
- Linux: `systemctl --user stop spellink-bridge` (`start` to run it again)
- macOS / Windows: `spellink-bridge service uninstall`

## Removing everything
1. Giving away or losing a device: `spellink-bridge device revoke my-nomad` (only that device stops working).
2. Remove automatic start: `spellink-bridge service uninstall`.
3. Delete the data on the computer: delete the settings folder ([where](02-bridge.md#3-start-it)). Settings, device registrations, conversation memory and glyph data are removed.
4. On the device, remove the plugin from Settings → Apps → Plugins.
