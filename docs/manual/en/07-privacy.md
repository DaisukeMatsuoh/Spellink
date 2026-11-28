# 7. What is sent and what stays

| What | Where | Notes |
|---|---|---|
| The **text** read from your lasso, and the earlier conversation of the same note | device → bridge → AI | Encrypted between device and bridge. **No images** |
| Date, time zone, OS kind | bridge → AI | Can be turned off in "AI settings". Host name, user name, paths and IP are not sent |
| Your AI login | nowhere | The bridge only calls the Claude Code you are already signed in to |
| Settings, device registrations, conversation memory | **your computer only** | `~/.config/spellink-bridge/` etc. Readable by you only. **Not encrypted** |
| The device | stores nothing | The pairing key is in the device's storage |

- **The pairing token is the key to your bridge.** Keep it private. If it leaks, disable the device with `device revoke` and pair again.
- Even with the relay, it sees only **encrypted traffic** and which bridge's room it belongs to — never the questions or answers.
- More: [SECURITY.md](../../../SECURITY.md).
