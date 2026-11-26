import {decodeBase64Url, encodeBase64Url, isSecret} from '../src/base64url';
import {
  embeddedSetup,
  MAX_NAME_CHARS,
  MAX_URL_CHARS,
  parsePairSetup,
} from '../src/pairConfig';
import {parsePairFrame} from '../src/pairFrames';
import {planBridges} from '../src/pairPlan';
import {
  encodeWhere,
  keyName,
  MAX_NAME_BYTES,
  MAX_PIECE,
  MAX_PIECES,
  MAX_STORED_URLS,
  readEntries,
  readStore,
  saveToStore,
  STORE_DIR,
  type FileOps,
  type StoredBridge,
  whereNames,
} from '../src/pairStore';

const secretOf = (n: number) => encodeBase64Url(new Uint8Array(32).fill(n));
const ID = '0123456789abcdef';
const URL = 'wss://bridge.example.com/v1?d=' + ID;

describe('base64url', () => {
  it.each([
    [[], ''],
    [[0], 'AA'],
    [[255], '_w'],
    [[0, 0], 'AAA'],
    [[251, 255], '-_8'],
    [[1, 2, 3], 'AQID'],
    [[104, 101, 108, 108, 111], 'aGVsbG8'],
  ])('round-trips %j as %s', (bytes, text) => {
    expect(encodeBase64Url(new Uint8Array(bytes))).toBe(text);
    expect(Array.from(decodeBase64Url(text)!)).toEqual(bytes);
  });

  it.each([
    ['a character outside the alphabet', 'AQI+'],
    ['padding', 'AA=='],
    ['a length no byte string has', 'AAAAA'],
    ['leftover bits that are not zero (a second spelling)', 'AB'],
    ['whitespace', 'AQ I'],
  ])('refuses %s', (_n, text) => {
    expect(decodeBase64Url(text)).toBeNull();
  });

  it('knows what a secret is', () => {
    expect(isSecret(secretOf(7))).toBe(true);
    for (const bad of [
      undefined,
      null,
      5,
      '',
      'short',
      secretOf(7) + 'A',
      'A'.repeat(42) + 'B',
      '!'.repeat(43),
    ]) {
      expect(isSecret(bad)).toBe(false);
    }
  });
});

describe('the pairing setup of a package', () => {
  const good = {
    v: 1,
    id: ID,
    gen: 2,
    b: secretOf(1),
    name: 'Nomad',
    urls: [URL],
    windowSec: 60,
  };

  it('is accepted as it is', () => {
    const setup: Record<string, unknown> = {...good};
    delete setup.v;
    expect(parsePairSetup(good)).toEqual({setup});
  });

  it('may lack the first secret and the addresses when the device has paired', () => {
    const paired = {v: 1, id: ID, gen: 2, name: 'Nomad', urls: []};
    expect(parsePairSetup(paired)).toEqual({
      setup: {id: ID, gen: 2, name: 'Nomad', urls: [], windowSec: 90},
    });
  });

  it.each([
    ['null', null],
    ['an array', []],
    ['a string', 'x'],
    ['another version', {...good, v: 2}],
    ['a bad id', {...good, id: 'ABC'}],
    ['gen 0', {...good, gen: 0}],
    ['a fractional gen', {...good, gen: 1.5}],
    ['a huge gen', {...good, gen: 2 ** 31}],
    ['a bad first secret', {...good, b: 'nope'}],
    ['no name', {...good, name: ''}],
    ['a name with a control character', {...good, name: 'a\nb'}],
    ['a name of 65 characters', {...good, name: 'x'.repeat(65)}],
    ['addresses that are not a list', {...good, urls: 'wss://x'}],
    ['a plain ws address', {...good, urls: ['ws://x/v1']}],
    ['an address with a quote', {...good, urls: ['wss://x/"']}],
    ['too many addresses', {...good, urls: Array(9).fill(URL)}],
    ['an address too long', {...good, urls: ['wss://' + 'a'.repeat(512)]}],
    ['no address although it has to pair', {...good, urls: []}],
  ])('is refused with %s', (_n, raw) => {
    expect(parsePairSetup(raw)).toHaveProperty('problem');
  });

  it.each([[5], [601], [1.5], ['60'], [undefined]])(
    'takes the default window for %j',
    w => {
      const r = parsePairSetup({...good, windowSec: w}) as {
        setup: {windowSec: number};
      };
      expect(r.setup.windowSec).toBe(90);
    },
  );

  it('keeps the limits of the window', () => {
    for (const w of [10, 600]) {
      const r = parsePairSetup({...good, windowSec: w}) as {
        setup: {windowSec: number};
      };
      expect(r.setup.windowSec).toBe(w);
    }
  });

  it('is read from what the package put in the global scope', () => {
    const g = globalThis as {__SPELLINK_PAIR__?: unknown};
    try {
      delete g.__SPELLINK_PAIR__;
      expect(embeddedSetup()).toEqual({setup: null});
      g.__SPELLINK_PAIR__ = good;
      expect(embeddedSetup().setup?.id).toBe(ID);
      g.__SPELLINK_PAIR__ = {...good, id: 'bad'};
      const r = embeddedSetup();
      expect(r.setup).toBeNull();
      expect(r.problem).toMatch(/id/);
    } finally {
      delete g.__SPELLINK_PAIR__;
    }
  });
});

describe('the frames of the pairing channel', () => {
  it('accepts a grant with a key and addresses', () => {
    const f = {type: 'pairGrant', key: secretOf(3), endpoints: [URL]};
    expect(parsePairFrame(f)).toEqual({kind: 'frame', frame: f});
  });

  it.each([
    ['a grant with a bad key', {type: 'pairGrant', key: 'x', endpoints: [URL]}],
    [
      'a grant without addresses',
      {type: 'pairGrant', key: secretOf(3), endpoints: []},
    ],
    [
      'a grant with too many addresses',
      {
        type: 'pairGrant',
        key: secretOf(3),
        endpoints: Array(MAX_STORED_URLS + 1).fill(URL),
      },
    ],
    [
      'a grant with a plain address',
      {type: 'pairGrant', key: secretOf(3), endpoints: ['ws://x']},
    ],
    [
      'a grant whose addresses are not a list',
      {type: 'pairGrant', key: secretOf(3), endpoints: URL},
    ],
    ['an unknown type', {type: 'welcome'}],
    ['something that is not an object', 'x'],
    ['an array', []],
    ['null', null],
  ])('refuses %s', (_n, raw) => {
    expect(parsePairFrame(raw).kind).toBe('bad');
  });

  it('knows the reasons of a refusal and says "other" for the rest', () => {
    for (const reason of [
      'code_in_use',
      'pair_busy',
      'grant_pending',
      'window_closed',
      'expired',
      'superseded',
      'unknown_device',
    ]) {
      expect(parsePairFrame({type: 'pairDenied', reason})).toEqual({
        kind: 'frame',
        frame: {type: 'pairDenied', reason},
      });
    }
    expect(
      parsePairFrame({type: 'pairDenied', reason: 'something new'}),
    ).toEqual({kind: 'frame', frame: {type: 'pairDenied', reason: 'other'}});
    expect(parsePairFrame({type: 'pairDenied'})).toEqual({
      kind: 'frame',
      frame: {type: 'pairDenied', reason: 'other'},
    });
  });

  it('passes on the code of an error, bounded', () => {
    expect(parsePairFrame({type: 'error', code: 'bad_request'})).toEqual({
      kind: 'frame',
      frame: {type: 'error', code: 'bad_request'},
    });
    expect(parsePairFrame({type: 'error'})).toEqual({
      kind: 'frame',
      frame: {type: 'error', code: 'unknown'},
    });
    const long = parsePairFrame({type: 'error', code: 'x'.repeat(500)});
    expect(
      long.kind === 'frame' &&
        long.frame.type === 'error' &&
        long.frame.code.length,
    ).toBe(64);
  });
});

describe('frames that are not part of pairing', () => {
  it.each([
    {type: 'pairBanner', strokes: []},
    {type: 'line'},
    {},
    null,
    'x',
    [],
  ])('%j is refused', raw => {
    expect(parsePairFrame(raw)).toMatchObject({kind: 'bad'});
  });
});

describe('the store in directory names', () => {
  const key = secretOf(5);
  const b: StoredBridge = {id: ID, gen: 3, key, name: 'Nomad', urls: [URL]};
  const names = (x: StoredBridge = b) => [
    keyName(x.id, x.gen, x.key),
    ...whereNames(x.id, x.gen, x.name, x.urls),
  ];
  const piece = (gen: number, i: number, n: number, text: string) =>
    `e-${ID}-${gen}-${i}-${n}-${text}`;
  const whereText = (n: string[]) =>
    n
      .slice(1)
      .map(e => e.split('-').slice(5).join('-'))
      .join('');

  it('reads back what it names', () => {
    expect(readEntries(names())).toEqual({bridges: [b], problems: []});
  });

  it('ignores names that are not its own', () => {
    expect(
      readEntries([
        'Spellink.snplg',
        'k-short',
        'x-' + ID + '-1-abc',
        ...names(),
      ]),
    ).toEqual({bridges: [b], problems: []});
  });

  it('uses the highest generation of each device, leaving older ones where they are', () => {
    const newer = {...b, gen: 4, key: secretOf(6), name: 'Renamed'};
    expect(readEntries([...names(), ...names(newer)]).bridges).toEqual([newer]);
    const other = {...b, id: 'fedcba9876543210', name: 'Other'};
    expect(readEntries([...names(), ...names(other)]).bridges).toHaveLength(2);
  });

  it.each([
    ['two keys', () => [...names(), keyName(ID, 3, secretOf(9))]],
    [
      'two sets of address pieces',
      () => [...names(), ...whereNames(ID, 3, 'X', [URL])],
    ],
    ['a key without addresses', () => [names()[0]]],
    ['addresses without a key', () => names().slice(1)],
    [
      'a key that is not a secret',
      () => [`k-${ID}-3-abc`, ...names().slice(1)],
    ],
    ['a piece that is missing', () => [names()[0], piece(3, 1, 2, 'AAAA')]],
    [
      'a piece that is there twice',
      () => [names()[0], piece(3, 0, 2, 'A'), piece(3, 0, 2, 'B')],
    ],
    [
      'pieces that disagree on how many there are',
      () => [names()[0], piece(3, 0, 2, 'A'), piece(3, 1, 3, 'B')],
    ],
    ['a piece number past the end', () => [names()[0], piece(3, 1, 1, 'A')]],
    [
      'more pieces than allowed',
      () => [names()[0], piece(3, 0, MAX_PIECES + 1, 'A')],
    ],
    [
      'addresses that are not base64url JSON',
      () => [names()[0], piece(3, 0, 1, 'AAAA')],
    ],
    [
      'addresses that are not base64url at all',
      () => [names()[0], piece(3, 0, 1, 'A')],
    ],
    [
      'addresses that are not JSON',
      () => [
        names()[0],
        piece(3, 0, 1, encodeBase64Url(new TextEncoder().encode('nope'))),
      ],
    ],
    [
      'addresses of the wrong shape',
      () => [
        names()[0],
        piece(
          3,
          0,
          1,
          encodeBase64Url(new TextEncoder().encode('{"n":"x","u":["ws://x"]}')),
        ),
      ],
    ],
    [
      'addresses that are null',
      () => [
        names()[0],
        piece(3, 0, 1, encodeBase64Url(new TextEncoder().encode('null'))),
      ],
    ],
    [
      'a generation that is too large',
      () => [`k-${ID}-9999999999-${key}`, ...names().slice(1)],
    ],
  ])('does not use a device with %s, and says so', (_n, make) => {
    const r = readEntries(make());
    expect(r.bridges).toEqual([]);
    expect(r.problems.length).toBeGreaterThanOrEqual(1);
    expect(r.problems.join()).not.toContain(key); // a key never goes into a message
  });

  it('does not fall back to an older generation when the newest is damaged', () => {
    const older = {...b, gen: 2};
    const r = readEntries([...names(older), keyName(ID, 3, key)]);
    expect(r.bridges).toEqual([]);
  });

  it('refuses to name what does not fit', () => {
    expect(() => keyName('bad', 1, key)).toThrow();
    expect(() => keyName(ID, 0, key)).toThrow();
    expect(() => keyName(ID, 1, 'has space')).toThrow();
    expect(() => whereNames('bad', 1, 'x', [URL])).toThrow();
    expect(() => whereNames(ID, 0, 'x', [URL])).toThrow();
    expect(() => encodeWhere('', [URL])).toThrow();
    expect(() => encodeWhere('x', [])).toThrow();
    expect(() =>
      encodeWhere('x', Array(MAX_STORED_URLS + 1).fill(URL)),
    ).toThrow();
    expect(() => encodeWhere('x', ['ws://x'])).toThrow();
  });

  const relay = (host: string) =>
    `wss://${host}.relay.spellink.uk/v1/relay/client?room=${'a'.repeat(32)}`;

  it('keeps every name within 255 bytes, whatever the addresses', () => {
    const many = {
      ...b,
      name: 'a'.repeat(30),
      urls: [
        'wss://bridge.spellink.uk/v1?d=' + ID,
        relay('jp'),
        relay('us'),
        relay('eu'),
        relay('au'),
        relay('br'),
        relay('in'),
        relay('za'),
      ],
    };
    const n = names(many);
    expect(n.length).toBeGreaterThan(2); // it needed more than one piece
    for (const name of n) {
      expect(new TextEncoder().encode(name).length).toBeLessThanOrEqual(
        MAX_NAME_BYTES,
      );
    }
    expect(readEntries(n)).toEqual({bridges: [many], problems: []});
  });

  it('puts the case that failed on a device (two relays and the tunnel) in pieces and reads it back', () => {
    const real = {
      ...b,
      name: 'rock-5b-plus',
      urls: ['wss://bridge.spellink.uk/v1?d=' + ID, relay('jp'), relay('us')],
    };
    expect(encodeWhere(real.name, real.urls).length).toBeGreaterThan(MAX_PIECE);
    expect(readEntries(names(real))).toEqual({bridges: [real], problems: []});
  });

  it('stores anything the validators accept: the most addresses, as long as allowed, with the longest name', () => {
    const worst = {
      ...b,
      name: 'n'.repeat(MAX_NAME_CHARS),
      urls: Array.from(
        {length: MAX_STORED_URLS},
        (_, i) => `wss://h${i}.example/` + 'a'.repeat(MAX_URL_CHARS - 20),
      ),
    };
    const n = names(worst);
    expect(n.length).toBeLessThanOrEqual(1 + MAX_PIECES);
    expect(readEntries(n)).toEqual({bridges: [worst], problems: []});
  });

  it('reads the pieces in any order of the listing', () => {
    const real = {...b, urls: [URL, relay('jp'), relay('us')]};
    const n = names(real);
    expect(readEntries([...n].reverse()).bridges).toEqual([real]);
  });

  it('cuts at MAX_PIECE characters and never beyond MAX_PIECES pieces', () => {
    const text = whereText(names());
    expect(text).toBe(encodeWhere(b.name, b.urls));
    expect(() =>
      whereNames(ID, 1, 'x'.repeat(64), Array(MAX_STORED_URLS).fill(URL)),
    ).not.toThrow();
    expect(MAX_PIECE * MAX_PIECES).toBeGreaterThan(6000);
  });

  // ---- a store on a fake folder ----
  function folder(
    opts: {fail?: 'list' | 'make' | 'refuse'; hide?: boolean} = {},
  ) {
    const have: string[] = [];
    const ops: FileOps = {
      async list(dir) {
        expect(dir).toBe(STORE_DIR);
        if (opts.fail === 'list') {
          throw new Error('listFiles failed');
        }
        return opts.hide ? [] : have.length ? [...have] : null;
      },
      async makeDir(path) {
        expect(path.startsWith(STORE_DIR + '/')).toBe(true);
        if (opts.fail === 'make') {
          throw new Error('makeDir failed');
        }
        if (opts.fail === 'refuse') {
          return false;
        }
        have.push(path.slice(STORE_DIR.length + 1));
        return true;
      },
    };
    return {have, ops};
  }

  it('saves, reads back and checks', async () => {
    const f = folder();
    expect(await readStore(f.ops)).toEqual({bridges: [], problems: []}); // no folder yet
    expect(await saveToStore(f.ops, b)).toBe(true);
    expect(f.have).toHaveLength(names().length);
    expect((await readStore(f.ops)).bridges).toEqual([b]);
    expect(await saveToStore(f.ops, b)).toBe(true); // already there: nothing is made twice
    expect(f.have).toHaveLength(names().length);
  });

  it('adds a newer generation next to the old one and never removes anything', async () => {
    const f = folder();
    await saveToStore(f.ops, b);
    const newer = {...b, gen: 4, key: secretOf(8)};
    expect(await saveToStore(f.ops, newer)).toBe(true);
    expect(f.have).toHaveLength(names().length + names(newer).length);
    expect((await readStore(f.ops)).bridges).toEqual([newer]);
  });

  it.each([
    ['a folder that cannot be listed', {fail: 'list' as const}],
    ['a directory that cannot be made', {fail: 'make' as const}],
    ['a directory the system refuses', {fail: 'refuse' as const}],
    ['names that are not there when read back', {hide: true}],
  ])('reports failure for %s', async (_n, opts) => {
    expect(await saveToStore(folder(opts).ops, b)).toBe(false);
  });

  it('reports failure when the read-back is a different pairing', async () => {
    const f = folder();
    const clash = {...b, key: secretOf(1)};
    f.have.push(...names(clash)); // someone else's key for the same id and gen
    expect(await saveToStore(f.ops, b)).toBe(false);
  });

  it.each([
    ['a key that is not a secret', {...b, key: 'nope'}],
    ['a bad id', {...b, id: 'x'}],
    ['no addresses', {...b, urls: []}],
  ])('refuses to save %s', async (_n, bad) => {
    expect(await saveToStore(folder().ops, bad)).toBe(false);
  });

  it('turns a failing listing into a problem, not a crash', async () => {
    const r = await readStore(folder({fail: 'list'}).ops);
    expect(r.bridges).toEqual([]);
    expect(r.problems[0]).toMatch(/cannot read/);
  });
});

describe('which bridges a device knows', () => {
  const setup = {
    id: ID,
    gen: 3,
    b: secretOf(1),
    name: 'Pkg',
    urls: [URL],
    windowSec: 90,
  };
  const stored = (gen: number) => ({
    id: ID,
    gen,
    key: secretOf(2),
    name: 'Stored',
    urls: [URL],
  });
  const none = {bridges: [], problems: []};

  it('nothing known: no bridges', () => {
    expect(planBridges(null, none).bridges).toEqual([]);
  });

  it('a package alone has to pair', () => {
    expect(planBridges(setup, none).bridges).toEqual([
      {id: ID, name: 'Pkg', state: 'Unpaired', setup},
    ]);
  });

  it('a store alone is paired: an update with a plain package keeps working', () => {
    const p = planBridges(null, {bridges: [stored(3)], problems: []});
    expect(p.bridges).toEqual([
      {id: ID, name: 'Stored', state: 'Paired', stored: stored(3)},
    ]);
  });

  it('a package and a store of the same issue: paired, the package is not needed', () => {
    expect(
      planBridges(setup, {bridges: [stored(3)], problems: []}).bridges[0].state,
    ).toBe('Paired');
  });

  it('a package of a newer issue (a reissue) makes the stored key useless', () => {
    const b = planBridges(setup, {bridges: [stored(2)], problems: []}).bridges;
    expect(b).toHaveLength(1);
    expect(b[0]).toMatchObject({state: 'Unpaired', name: 'Pkg'});
  });

  it('an older package does not undo a newer pairing', () => {
    expect(
      planBridges(setup, {bridges: [stored(5)], problems: []}).bridges[0].state,
    ).toBe('Paired');
  });

  it('bridges are independent: several can be paired and one more can be waiting', () => {
    const other = {
      id: 'fedcba9876543210',
      gen: 1,
      key: secretOf(9),
      name: 'Other',
      urls: [URL],
    };
    const p = planBridges(setup, {bridges: [other], problems: ['x']});
    expect(p.bridges.map(b => [b.id, b.state])).toEqual([
      [other.id, 'Paired'],
      [ID, 'Unpaired'],
    ]);
    expect(p.problems).toEqual(['x']);
  });
});
