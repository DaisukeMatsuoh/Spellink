jest.mock('../src/bridges.local.json', () => ({bridges: []}), {virtual: true});
import {encodeBase64Url} from '../src/base64url';
import {setFakeLanguage} from '../__mocks__/sn-plugin-lib';
import {realFileOps} from '../src/fileOps';
import {startLocaleTracking} from '../src/i18n';
import * as store from '../src/pairStore';
import {pairMessage, groupCode} from '../src/pairText';
import type {PairView} from '../src/pairing';

const secretOf = (n: number) => encodeBase64Url(new Uint8Array(32).fill(n));
const ID = '0123456789abcdef';
const URL = 'wss://bridge.example.com/v1?d=' + ID;

describe('the words of the pairing screen', () => {
  beforeAll(() => {
    startLocaleTracking();
    setFakeLanguage('en'); // the machine running the tests may be set to another language
  });
  const v = (note: PairView['note'], detail?: string): PairView => ({
    state: 'Unpaired',
    note,
    detail,
  });
  it.each([
    ['idle', undefined, /Press Pair/],
    ['connecting', undefined, /Connecting/],
    ['showing', undefined, /Scan this QR code/],
    ['storing', undefined, /Saving/],
    ['paired', undefined, /Paired with Nomad/],
    ['timeout', undefined, /not entered in time/],
    ['cancelled', undefined, /Cancelled/],
    ['storeFailed', undefined, /could not be saved/],
    ['refused', undefined, /no longer accepts/],
    ['noSecret', undefined, /already been used/],
    ['denied', 'code_in_use', /refused: code_in_use/],
    ['disconnected', 'closed (1006)', /ended: closed \(1006\)/],
    ['disconnected', 'noSecret', /already been used/],
    ['denied', undefined, /refused: $/],
    ['disconnected', undefined, /ended: $/],
  ] as const)('%s %s', (note, detail, pattern) => {
    expect(pairMessage(v(note, detail), 'Nomad')).toMatch(pattern);
  });

  it('groups a code for reading and typing', () => {
    expect(groupCode('12345678')).toBe('1234 5678');
    expect(groupCode('1234')).toBe('1234');
    expect(groupCode('')).toBe('');
  });
});

describe('the real file module, checked and bounded', () => {
  const module = (over: Record<string, unknown> = {}) => ({
    exists: async () => true,
    listFiles: async () => [
      {type: 0, path: '/a/b/k-x'},
      {type: 1, path: '/a/b/e-y'},
    ],
    makeDir: async () => true,
    ...over,
  });

  it('lists names, not paths', async () => {
    expect(await realFileOps(module()).list('/a/b')).toEqual(['k-x', 'e-y']);
  });

  it('a folder that is not there is null; an empty or odd answer is an empty list', async () => {
    expect(
      await realFileOps(module({exists: async () => false})).list('/x'),
    ).toBeNull();
    expect(
      await realFileOps(module({exists: async () => 'yes'})).list('/x'),
    ).toBeNull();
    expect(
      await realFileOps(module({listFiles: async () => null})).list('/x'),
    ).toEqual([]);
    expect(
      await realFileOps(module({listFiles: async () => 'nope'})).list('/x'),
    ).toEqual([]);
  });

  it('skips entries that are not usable', async () => {
    const files = module({
      listFiles: async () => [
        null,
        {},
        {path: 5},
        {path: '/dir/'},
        {path: '/a/ok'},
      ],
    });
    expect(await realFileOps(files).list('/a')).toEqual(['ok']);
  });

  it('makeDir is true only for true', async () => {
    expect(await realFileOps(module()).makeDir('/x')).toBe(true);
    expect(
      await realFileOps(module({makeDir: async () => false})).makeDir('/x'),
    ).toBe(false);
    expect(
      await realFileOps(module({makeDir: async () => 'ok'})).makeDir('/x'),
    ).toBe(false);
  });

  it('says so when the module is not there', async () => {
    const ops = realFileOps(null);
    await expect(ops.list('/x')).rejects.toThrow(/not available/);
    await expect(ops.makeDir('/x')).rejects.toThrow(/not available/);
  });

  it('does not wait for ever for a module that does not answer', async () => {
    jest.useFakeTimers();
    try {
      const hung = new Promise<boolean>(() => {});
      const p = realFileOps(module({exists: () => hung})).list('/x');
      const assertion = expect(p).rejects.toThrow(/no answer/);
      jest.advanceTimersByTime(5000);
      await assertion;
    } finally {
      jest.useRealTimers();
    }
  });

  it('passes on a failure of the module', async () => {
    await expect(
      realFileOps(
        module({
          makeDir: async () => {
            throw new Error('boom');
          },
        }),
      ).makeDir('/x'),
    ).rejects.toThrow('boom');
  });

  it('the default is the SDK module, which is absent in tests', async () => {
    await expect(realFileOps().list('/x')).rejects.toThrow(/not available/);
  });
});

describe('loading what the device knows', () => {
  const g = globalThis as {__SPELLINK_PAIR__?: unknown};
  afterEach(() => {
    delete g.__SPELLINK_PAIR__;
    jest.resetModules();
    jest.dontMock('../src/fileOps');
  });

  const withStore = (names: string[] | Error) => {
    jest.doMock('../src/fileOps', () => ({
      realFileOps: () => ({
        list: async () => {
          if (names instanceof Error) {
            throw names;
          }
          return names;
        },
        makeDir: async () => true,
      }),
    }));
  };
  const stored = [
    store.entryName('k', ID, 1, secretOf(5)),
    store.entryName('e', ID, 1, store.encodeWhere('Rock', [URL])),
  ];

  it('knows a paired bridge from the store alone, and says nothing about a missing package', async () => {
    withStore(stored);
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const {loadPairing} = require('../src/pairingState');
    const r = await loadPairing();
    expect(r.setupProblem).toBeUndefined();
    expect(r.plan.bridges).toMatchObject([
      {id: ID, state: 'Paired', name: 'Rock'},
    ]);
  });

  it('knows a package that has to pair', async () => {
    withStore([]);
    g.__SPELLINK_PAIR__ = {
      v: 1,
      id: ID,
      gen: 1,
      b: secretOf(1),
      name: 'Rock',
      urls: [URL],
    };
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const {loadPairing} = require('../src/pairingState');
    expect((await loadPairing()).plan.bridges).toMatchObject([
      {id: ID, state: 'Unpaired'},
    ]);
  });

  it('reports a package whose setup is wrong, and a store that cannot be read', async () => {
    withStore(new Error('no store'));
    g.__SPELLINK_PAIR__ = {v: 1, id: 'bad'};
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const {loadPairing} = require('../src/pairingState');
    const r = await loadPairing();
    expect(r.setupProblem).toMatch(/id/);
    expect(r.plan.problems[0]).toMatch(/cannot read/);
  });

  it('gives loadBridges the paired bridges first, then the older ones', async () => {
    withStore(stored);
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const {loadBridges} = require('../src/bridgeConfig');
    const r = await loadBridges();
    expect(r.source).toBe('paired');
    expect(r.endpoints[0]).toEqual({
      id: ID,
      name: 'Rock',
      token: secretOf(5),
      urls: [URL],
    });
  });

  it('gives loadBridges only the older ones when nothing is paired', async () => {
    withStore([]);
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const {loadBridges} = require('../src/bridgeConfig');
    expect((await loadBridges()).source).toBe('baked');
  });

  it('a store that cannot be read does not stop the older ways of finding a bridge', async () => {
    withStore(new Error('no store'));
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const {loadBridges} = require('../src/bridgeConfig');
    expect((await loadBridges()).source).toBe('baked');
  });

  it('a pairing that crashes while loading is the same as no pairing', async () => {
    jest.doMock('../src/pairingState', () => ({
      loadPairing: async () => {
        throw new Error('bug');
      },
    }));
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const {loadBridges} = require('../src/bridgeConfig');
    expect((await loadBridges()).source).toBe('baked');
    jest.dontMock('../src/pairingState');
  });
});

describe('the real parts of a pairing', () => {
  it('makes digits that are digits, of the length asked, and different each time', async () => {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const {randomDigits} = require('../src/pairingState');
    const seen = new Set<string>();
    for (let i = 0; i < 50; i++) {
      const d = randomDigits(8);
      expect(d).toMatch(/^\d{8}$/);
      seen.add(d);
    }
    expect(seen.size).toBeGreaterThan(40);
  });

  it('spreads the digits evenly (no digit is favoured by the way bytes are cut)', async () => {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const {randomDigits} = require('../src/pairingState');
    const count = new Array(10).fill(0);
    for (let i = 0; i < 2000; i++) {
      for (const c of randomDigits(8)) {
        count[Number(c)]++;
      }
    }
    for (const n of count) {
      expect(n / 16000).toBeGreaterThan(0.08);
      expect(n / 16000).toBeLessThan(0.12);
    }
  });

  it('wires timers, the clock and the log to the real ones', async () => {
    jest.useFakeTimers();
    try {
      // eslint-disable-next-line @typescript-eslint/no-var-requires
      const {realPairDeps} = require('../src/pairingState');
      const views: unknown[] = [];
      const d = realPairDeps((v: unknown) => views.push(v));
      let fired = 0;
      const stopAfter = d.after(100, () => fired++);
      const stopEvery = d.every(50, () => fired++);
      jest.advanceTimersByTime(100);
      expect(fired).toBe(3); // every twice, after once
      stopEvery();
      stopAfter();
      jest.advanceTimersByTime(500);
      expect(fired).toBe(3);
      const cancelled = d.after(10, () => fired++);
      cancelled();
      jest.advanceTimersByTime(100);
      expect(fired).toBe(3);
      expect(typeof d.now()).toBe('number');
      d.log('pair.test', {x: 1});
      d.onView({state: 'Unpaired', note: 'idle'});
      expect(views).toHaveLength(1);
      expect(typeof d.connect).toBe('function');
      expect(d.banner()).toEqual({
        text: expect.stringMatching(/QR/),
        width: 1000,
        charSize: 40,
      });
      expect(
        await d.save({id: ID, gen: 1, key: 'x', name: 'n', urls: [URL]}),
      ).toBe(false); // the file module is absent here
    } finally {
      jest.useRealTimers();
    }
  });
});
