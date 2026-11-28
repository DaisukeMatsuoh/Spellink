# 6. Troubleshooting

First run `spellink-bridge check`. It reaches the bridge the way the device does, down to the encrypted handshake. OK means the path works.

| Symptom | What to do |
|---|---|
| `device add` fails with a `plugin package` error | This build has no plugin inside. Use a Releases build, or get `Spellink.snplg` and set `"pairing": {"pluginPackage": "…path"}` in `config.json`, then restart |
| `the bridge did not accept the admin token` | The bridge is running with another settings folder. Run the command as the same user |
| No button on the device | It is an old note. Try a new note |
| "Cannot connect to the bridge" | The bridge is stopped, the computer is asleep, or the internet is down |
| Pairing never finishes | Look at `spellink-bridge device list`. If it stays `issued`, check that you pressed "Pair" on the device and check the connection setting ([3. Connect the device](03-device.md#choose-a-way-to-connect)) |
| Lost the setup file | `spellink-bridge device package my-nomad` (it stops if a file with that name exists; delete it first) |
| Expired, or want to pair again | `spellink-bridge device reissue my-nomad`, then `spellink-bridge device package my-nomad` |
| Bridge App says Claude Code is not found | Install `claude` on the computer and sign in. If the bridge runs as a service, redo `service install` (the `PATH` at that time is recorded) |
| No answer comes back | Check you have not used up your Claude allowance, and that `claude` answers a question directly |
| It does not answer "what time is it?" | The bridge may be an old version. Check `spellink-bridge version` and restart ([restart](02-bridge.md#restarting-after-changing-settings)) |
| Bridge App: "No way for a device to reach this bridge" | The relay is off and there is no home or tunnel address. Pick one way ([connect](03-device.md#choose-a-way-to-connect)) |

If it is still not fixed, open an Issue with `spellink-bridge version` and what happened. **Do not post the token or the setup file.**
