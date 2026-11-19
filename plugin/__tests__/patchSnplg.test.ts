const {patchBundle, patchPackage} = require('../scripts/patch-snplg');
import {execFileSync} from 'child_process';
import fs from 'fs';
import os from 'os';
import path from 'path';

describe('patch-snplg', () => {
  it('prepends the seed line and keeps the rest', () => {
    expect(patchBundle('var a=1;', 'abc-1_X')).toBe(
      'globalThis.__SPELLINK_SEED__="abc-1_X";\nvar a=1;',
    );
  });
  it.each(['', 'a"b', 'a\\b', 'a\nb', 'x'.repeat(257), 'あ'])(
    'refuses the seed %j',
    s => expect(() => patchBundle('x', s)).toThrow(),
  );
  it('refuses a bundle that already has a seed', () => {
    expect(() => patchBundle(patchBundle('x', 'a'), 'b')).toThrow();
  });
  it('writes a package whose bundle carries the seed and whose other files are unchanged', () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'pt-'));
    try {
      const src = path.join(dir, 'src');
      fs.mkdirSync(src);
      fs.writeFileSync(path.join(src, 'Spellink.bundle'), 'var x=1;');
      fs.writeFileSync(path.join(src, 'PluginConfig.json'), '{"a":1}');
      const a = path.join(dir, 'a.snplg');
      execFileSync('zip', ['-q', '-X', '-r', a, '.'], {cwd: src});
      const b = path.join(dir, 'b.snplg');
      patchPackage(a, b, 's1');
      const get = (f: string, n: string) =>
        execFileSync('unzip', ['-p', f, n]).toString();
      expect(get(b, 'Spellink.bundle')).toBe(
        'globalThis.__SPELLINK_SEED__="s1";\nvar x=1;',
      );
      expect(get(b, 'PluginConfig.json')).toBe('{"a":1}');
      expect(() => patchPackage(b, path.join(dir, 'c.snplg'), 's2')).toThrow();
    } finally {
      fs.rmSync(dir, {recursive: true, force: true});
    }
  });
});
