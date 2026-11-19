#!/usr/bin/env node
// Writes a copy of a .snplg with one line added at the top of Spellink.bundle:
//   globalThis.__SPELLINK_SEED__="<seed>";
// Pairing experiment 1 (docs/notes/research/2026-11-08-pairing.md): does a device install a package
// that was changed after it was built, and does reinstalling with another seed replace the first?
// Usage: node scripts/patch-snplg.js <in.snplg> <out.snplg> <seed>
const fs = require('fs');
const os = require('os');
const path = require('path');
const {execFileSync} = require('child_process');

const BUNDLE = 'Spellink.bundle';
const SEED_RE = /^[A-Za-z0-9_-]{1,256}$/; // safe inside a JS string: no quote, backslash or newline

function patchBundle(text, seed) {
  if (!SEED_RE.test(seed)) {
    throw new Error('seed must be 1-256 chars of A-Z a-z 0-9 _ -');
  }
  if (text.includes('__SPELLINK_SEED__=')) {
    throw new Error(
      'bundle already carries a seed; patch the original package',
    );
  }
  return `globalThis.__SPELLINK_SEED__="${seed}";\n${text}`;
}

function patchPackage(inFile, outFile, seed) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'snplg-'));
  try {
    execFileSync('unzip', ['-q', inFile, '-d', dir]);
    const bundle = path.join(dir, BUNDLE);
    if (!fs.existsSync(bundle)) {
      throw new Error(`${BUNDLE} not found in ${inFile}`);
    }
    fs.writeFileSync(
      bundle,
      patchBundle(fs.readFileSync(bundle, 'utf8'), seed),
    );
    fs.rmSync(outFile, {force: true});
    execFileSync('zip', ['-q', '-X', '-r', path.resolve(outFile), '.'], {
      cwd: dir,
    });
    // Success is checked by the result: read the bundle back out of the new package.
    const back = execFileSync('unzip', ['-p', outFile, BUNDLE], {
      maxBuffer: 1 << 28,
    }).toString('utf8');
    if (!back.startsWith(`globalThis.__SPELLINK_SEED__="${seed}";\n`)) {
      throw new Error('written package does not carry the seed');
    }
  } finally {
    fs.rmSync(dir, {recursive: true, force: true});
  }
}

module.exports = {patchBundle, patchPackage};

if (require.main === module) {
  const [inFile, outFile, seed] = process.argv.slice(2);
  if (!inFile || !outFile || !seed) {
    console.error('usage: patch-snplg.js <in.snplg> <out.snplg> <seed>');
    process.exit(2);
  }
  try {
    patchPackage(inFile, outFile, seed);
    console.log(`wrote ${outFile} (seed ${seed})`);
  } catch (e) {
    console.error(String(e.message || e));
    process.exit(1);
  }
}
