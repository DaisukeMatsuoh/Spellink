jest.mock('../src/bridges.local.json', () => ({bridges: []}), {virtual: true});
import {parse} from '../src/bridgeConfig';

const ok = {name: 'home', token: 'abc', urls: ['wss://bridge.example/v1']};

describe('a pairing file is checked entry by entry', () => {
  it('keeps good entries', () => {
    expect(parse({bridges: [ok]})).toEqual([ok]);
  });

  it.each([
    ['not an object', null],
    ['no bridges key', {}],
    ['bridges that is not a list', {bridges: 'x'}],
    ['an empty token', {bridges: [{...ok, token: ''}]}],
    ['a token that is not a string', {bridges: [{...ok, token: 5}]}],
    ['a nameless bridge', {bridges: [{...ok, name: ''}]}],
    ['no URLs', {bridges: [{...ok, urls: []}]}],
    ['a URL that is not wss or ws', {bridges: [{...ok, urls: ['https://x']}]}],
    ['a URL that is not a string', {bridges: [{...ok, urls: [7]}]}],
    ['a huge token', {bridges: [{...ok, token: 'a'.repeat(300)}]}],
    ['too many URLs', {bridges: [{...ok, urls: Array(9).fill('wss://x')}]}],
  ])('leaves out %s', (_name, json) => {
    expect(parse(json)).toEqual([]);
  });

  it('keeps the good entries next to a bad one, and caps the number of bridges', () => {
    expect(parse({bridges: [{...ok, token: ''}, ok]})).toEqual([ok]);
    expect(parse({bridges: Array(20).fill(ok)})).toHaveLength(8);
  });
});

describe('the older ways of finding a bridge', () => {
  const g = globalThis as {fetch?: unknown};
  const realFetch = g.fetch;
  afterEach(() => {
    g.fetch = realFetch;
    jest.resetModules();
    jest.dontMock('../src/pairingState');
  });
  const load = () => {
    jest.doMock('../src/pairingState', () => ({
      loadPairing: async () => ({plan: {bridges: [], problems: []}}),
    }));
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    return require('../src/bridgeConfig').loadBridges as () => Promise<{
      endpoints: unknown[];
      source: string;
      fileError?: string;
    }>;
  };

  it('reads the pairing file from MyStyle when it is there', async () => {
    g.fetch = async () => ({json: async () => ({bridges: [ok]})});
    const r = await load()();
    expect(r).toMatchObject({source: 'file', endpoints: [ok]});
  });

  it('falls back to the baked list when the file has no usable bridge', async () => {
    g.fetch = async () => ({json: async () => ({bridges: []})});
    expect(await load()()).toMatchObject({
      source: 'baked',
      fileError: 'no bridges in file',
    });
  });

  it('falls back to the baked list, saying why, when the file cannot be read', async () => {
    g.fetch = async () => {
      throw new Error('Network request failed');
    };
    const r = await load()();
    expect(r.source).toBe('baked');
    expect(r.fileError).toMatch(/Network request failed/);
  });
});
