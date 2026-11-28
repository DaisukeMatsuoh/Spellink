# 1. Getting started

## What it does
1. Write a question by hand in a note.
2. Lasso it and tap the plugin button (**Ask AI**).
3. A few seconds to a few tens of seconds later, the answer is written below your question as pen strokes. Write more in the same note and the conversation continues.

## Requirements
| What | Details |
|---|---|
| Supernote | Nomad (verified) or Manta. A USB cable or another way to send files to the device, to install the plugin |
| A computer | Windows / macOS / Linux. **Keep it on** (if it sleeps, no answer comes back) and online. Only Linux is verified so far |
| A Claude subscription | Pro / Max or similar — **your own**, not shared between people. No API key needed |
| Claude Code | The official one, installed on that computer and **already signed in** (run `claude` once). Spellink uses that sign-in as it is |

## Cost
- Spellink itself is free (Apache-2.0).
- Each question uses **your Claude subscription's usage allowance**.
- The **relay** (for using it away from home) is funded by donations. Using it is optional, and features do not depend on donating.
- Your own domain and tunnel is an advanced route that can cost money. **You do not have to use it.**

## The short path
1. [Install the bridge](02-bridge.md): download → `spellink-bridge serve` → `spellink-bridge service install`
2. [Register the device](03-device.md#register-the-device): `spellink-bridge device add my-nomad` (makes a setup file)
3. [Install on the device](03-device.md#install-on-the-device): put the setup file in `MyStyle`, install from the device
4. [Pair](03-device.md#pair): type the device's 8 digits into `spellink-bridge device pair` on the computer
5. In a new note: write → lasso → **Ask AI**
