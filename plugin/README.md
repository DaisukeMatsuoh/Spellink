# Cloute plugin (Supernote)

React Native plugin built from the official template (`@supernote-plugin/sn-plugin-template`, RN 0.79.2, `sn-plugin-lib`).

```bash
npm install
cloute-bridge pair src/bridges.local.json   # optional: bake a bridge connection into the build (git-ignored)
npm run build                               # → build/outputs/Cloute.snplg
npm run typecheck && npx eslint index.js App.tsx src
```

Copy the `.snplg` to the device's `MyStyle/` folder and install it from Settings → Apps → Plugins.
See `../docs/notes/device-test-guide.md`.

- `index.js` — registers the lasso-toolbar "Ask Claude" button and the sidebar diagnostics button
- `src/askFlow.ts` — lasso → on-device OCR → bridge → write answer lines (and continue on new pages)
- `src/sdk.ts` — thin wrappers over `sn-plugin-lib`
- `src/bridgeClient.ts` — connects to every paired bridge URL at once and keeps the first that answers
- `src/diagnostics.ts` — device checks; results can be sent to the bridge's report log
