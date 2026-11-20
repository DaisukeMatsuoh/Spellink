import fs from 'fs';
import path from 'path';
import {buttonName, toLocale} from '../src/i18n';
import {LOCALES, STRINGS} from '../src/strings.generated';

// eslint-disable-next-line @typescript-eslint/no-var-requires
const {parse, generate} = require('../scripts/gen-strings.js');
const tsvPath = path.join(__dirname, '../locales/strings.tsv');
const tsv = fs.readFileSync(tsvPath, 'utf8');
const H = 'key\ten\tja\n';

describe('the words of the plugin (locales/strings.tsv)', () => {
  it('is what the generated code says (run `npm run strings` after editing the TSV)', () => {
    const {languages, table} = parse(tsv);
    expect([...LOCALES]).toEqual(languages);
    expect(STRINGS).toEqual(table);
  });

  it('has every message in every language, with no gap that falls back to English silently', () => {
    const {languages, keys} = parse(tsv);
    const cells = new Map<string, string[]>();
    for (const line of tsv.split('\n')) {
      if (line === '' || line.startsWith('#') || line.startsWith('key\t')) {
        continue;
      }
      cells.set(line.split('\t')[0], line.split('\t').slice(1));
    }
    for (const k of keys) {
      languages.forEach((l: string, i: number) => {
        expect({key: k, lang: l, empty: cells.get(k)![i] === ''}).toEqual({
          key: k,
          lang: l,
          empty: false,
        });
      });
    }
  });

  it('offers the languages it lists, with the same messages in each', () => {
    expect([...LOCALES]).toEqual(['en', 'ja']);
    const en = Object.keys(STRINGS.en);
    for (const l of LOCALES) {
      expect(Object.keys(STRINGS[l])).toEqual(en);
    }
  });

  it('keeps the placeholders of every language the same as English', () => {
    const ph = (s: string) =>
      [...s.matchAll(/\{(\w+)\}/g)]
        .map(m => m[1])
        .sort()
        .join();
    for (const k of Object.keys(STRINGS.en) as (keyof typeof STRINGS.en)[]) {
      for (const l of LOCALES) {
        expect({k, l, ph: ph(STRINGS[l][k])}).toEqual({
          k,
          l,
          ph: ph(STRINGS.en[k]),
        });
      }
    }
  });
});

describe('reading a TSV', () => {
  it('reads languages as columns and unescapes cells', () => {
    const r = parse(
      `# note\n\n${H}hello\tHi {name}\\nthere\\there\tこんにちは {name}\nbye\tBye\t\n`,
    );
    expect(r.languages).toEqual(['en', 'ja']);
    expect(r.keys).toEqual(['hello', 'bye']);
    expect(r.table.en.hello).toBe('Hi {name}\nthere\there');
    expect(r.table.ja.bye).toBe('Bye'); // an empty cell uses the English text
  });

  it('accepts more languages and Windows line ends', () => {
    const r = parse('key\ten\tja\tzh_CN\r\nx\tA\tB\tC\r\n');
    expect(r.languages).toEqual(['en', 'ja', 'zh_CN']);
    expect(r.table.zh_CN.x).toBe('C');
  });

  it.each([
    ['no header', 'x\tA\tB\n', /header/],
    ['English not second', 'key\tja\ten\n', /header/],
    ['a bad language code', 'key\ten\tJapanese\n', /not a language code/],
    ['a language twice', 'key\ten\tja\tja\n', /twice/],
    ['a missing cell', `${H}x\tA\n`, /columns/],
    ['an extra cell', `${H}x\tA\tB\tC\n`, /columns/],
    ['a bad key', `${H}1x\tA\tB\n`, /valid key/],
    ['a key twice', `${H}x\tA\tB\nx\tA\tB\n`, /twice/],
    ['an unknown escape', `${H}x\tA\\q\tB\n`, /escape/],
    ['a trailing backslash', `${H}x\tA\\\tB\n`, /escape/],
    ['an empty English text', `${H}x\t\tB\n`, /English text is required/],
    [
      'a placeholder missing in a translation',
      `${H}x\tHi {n}\tこんにちは\n`,
      /placeholders/,
    ],
    [
      'a placeholder that English lacks',
      `${H}x\tHi\tこんにちは {n}\n`,
      /placeholders/,
    ],
    ['nothing at all', '', /header/],
  ])('refuses %s, naming the line or the key', (_n, text, pattern) => {
    expect(() => parse(text)).toThrow(pattern);
  });

  it('generates a module the plugin can import', () => {
    const code = generate(`${H}hello\tHi\t"こんにちは"\n`);
    expect(code).toContain('export const LOCALES = ["en", "ja"] as const;');
    expect(code).toContain('hello: "\\"こんにちは\\""');
  });
});

describe('choosing the language', () => {
  it.each([
    ['en', 'en'],
    ['ja', 'ja'],
    ['ja_JP', 'ja'],
    ['ja-JP', 'ja'],
    ['JA', 'ja'],
    ['zh_CN', 'en'], // no Chinese column yet
    ['fr', 'en'],
    ['', 'en'],
    [undefined, 'en'],
  ])('%s → %s', (code, want) => {
    expect(toLocale(code)).toBe(want);
  });

  it('offers a button name for every language, and the Chinese codes the firmware may ask for', () => {
    const names = JSON.parse(buttonName('askButton'));
    expect(names.en).toBe('Ask AI');
    expect(names.ja).toBe('AIに聞く');
    expect(names.zh_CN).toBe('Ask AI');
    expect(names.zh_TW).toBe('Ask AI');
  });
});
