// Ensures src/bridges.local.json exists (it is git-ignored and baked into the bundle).
// Fill it with `node ../bridge/src/main.ts pair src/bridges.local.json`.
const fs = require('fs');
const path = require('path');
const file = path.join(__dirname, '..', 'src', 'bridges.local.json');
if (!fs.existsSync(file)) fs.writeFileSync(file, '{"bridges": []}\n');
