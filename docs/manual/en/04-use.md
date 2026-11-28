# 4. Using it

## Asking
1. In a new note, write your question by hand.
2. Lasso it and tap the plugin button (**Ask AI**).
3. A few seconds to a few tens of seconds later the answer is written below. Write a follow-up in the same note and lasso it to continue.

Good to know:
- One question at a time. If you open another note while it is answering, it stops writing.
- If you lasso something that is not text (music, a drawing), it says it could not read it.
- What goes to the AI is the text read from your writing and the earlier conversation of the same note. **No images.**
- Spacing/slant settings and the conversation memory are stored on the computer. "Forget the conversation" deletes it.

## The Bridge App page
On the computer, run this to open the Bridge App page (`/app`) in your browser:
```
spellink-bridge open
```
- **Status:** the bridge's name and version, whether Claude Code was found, whether the relay is on.
- **Devices:** add (same as `device add`), list, pairing state.
- **AI settings:** next section.

The page opens **only from the same computer as the bridge**. A device or the relay cannot change settings.

## AI settings
In "AI settings" on the Bridge App page you decide what the AI is given. All of it is optional; it works without any setting.

| Item | Meaning |
|---|---|
| Tell the date and OS | **On** by default. Tells the AI the current date, time zone and OS kind (such as `linux`) — needed to answer "what time is it?". Host name, user name, paths and IP are not sent |
| Standing instructions | How to address you, assumptions, tone. Example: "I am Daisuke. Answer schedules in Japan time." Up to 2000 characters |
| Allowed commands | Only if you want the AI to run commands on your computer. One per line, like `Bash(date)`. "Allow everything" such as `Bash(*)` cannot be saved. Empty means no commands |

- Saving does not affect the running bridge right away. When it says "restart the bridge", [restart it](02-bridge.md#restarting-after-changing-settings).
- Allowed commands really run on your computer. **Write only ones you understand.**
- You can also skip the page and edit the `ai` section of `config.json` in a text editor (format: [spec](../../spec/ai-tools.md), Japanese).

> The end-to-end check of the date feature ("what time is it?") on a real device is still to be done.

Next: [5. Update, stop, remove](05-maintenance.md)
