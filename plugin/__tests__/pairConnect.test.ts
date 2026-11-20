import {CipherState, SecureChannel} from '../src/noise';
import {connectPairing, type PairChannel} from '../src/pairConnect';
import type {PairFrame} from '../src/pairFrames';
import {utf8Decode, utf8Encode} from '../src/utf8';

// The real handshake is checked against the real bridge (integration.test); here the handshake is
// replaced by a fake that yields a channel whose other end the test holds.
const mockState = {
  throwOnStart: false,
  failHandshake: false,
  psks: [] as Uint8Array[],
  plugin: null as unknown,
};
jest.mock('../src/noise', () => ({
  ...jest.requireActual('../src/noise'),
  NoiseInitiator: class {
    constructor(psk: Uint8Array) {
      mockState.psks.push(psk);
    }
    writeMessage1() {
      if (mockState.throwOnStart) {
        throw new Error('no crypto');
      }
      return new Uint8Array([1, 2, 3]);
    }
    readMessage2() {
      if (mockState.failHandshake) {
        throw new Error('handshake: wrong key');
      }
      return mockState.plugin;
    }
  },
}));

const k1 = new Uint8Array(32).fill(1);
const k2 = new Uint8Array(32).fill(2);
const newPair = () => ({
  plugin: new SecureChannel(new CipherState(k1), new CipherState(k2)),
  bridge: new SecureChannel(new CipherState(k2), new CipherState(k1)),
});

class FakeSocket {
  static all: FakeSocket[] = [];
  binaryType = '';
  sent: unknown[] = [];
  closed = 0;
  onopen: () => void = () => {};
  onmessage: (e: {data: unknown}) => void = () => {};
  onerror: () => void = () => {};
  onclose: (e: {reason: string; code: number}) => void = () => {};
  constructor(readonly url: string) {
    FakeSocket.all.push(this);
  }
  send(d: unknown) {
    this.sent.push(d);
  }
  close() {
    this.closed++;
  }
}
const Socket = FakeSocket as unknown as new (u: string) => WebSocket;
const buf = (u: Uint8Array) =>
  u.buffer.slice(u.byteOffset, u.byteOffset + u.byteLength);

let bridge: SecureChannel;
beforeEach(() => {
  FakeSocket.all = [];
  Object.assign(mockState, {
    throwOnStart: false,
    failHandshake: false,
    psks: [],
  });
  const p = newPair();
  mockState.plugin = p.plugin;
  bridge = p.bridge;
});

const SECRET = 'a-first-secret';
const sealed = (o: unknown) => bridge.seal(utf8Encode(JSON.stringify(o)))[0];

describe('opening the pairing channel', () => {
  it('uses the first address that completes the handshake and drops the others', async () => {
    const p = connectPairing(['wss://a', 'wss://b'], SECRET, 1000, Socket);
    const [a, b] = FakeSocket.all;
    expect(a.binaryType).toBe('arraybuffer');
    a.onopen();
    expect(a.sent).toEqual([new Uint8Array([1, 2, 3])]);
    b.onmessage({data: buf(new Uint8Array([9]))}); // b answers first
    const ch = await p;
    expect(b.closed).toBe(0);
    expect(a.closed).toBe(1);
    expect(mockState.psks).toHaveLength(2);
    ch.close();
    expect(b.closed).toBe(1);
  });

  it('a second answer after the first is closed, not kept', async () => {
    const p = connectPairing(['wss://a', 'wss://b'], SECRET, 1000, Socket);
    const [a, b] = FakeSocket.all;
    a.onmessage({data: buf(new Uint8Array([9]))});
    await p;
    b.onmessage({data: buf(new Uint8Array([9]))});
    expect(b.closed).toBeGreaterThanOrEqual(1);
  });

  it('refuses a call with no address', async () => {
    await expect(connectPairing([], SECRET, 1000, Socket)).rejects.toThrow(
      /no address/,
    );
  });

  it('says why when every address fails', async () => {
    const p = connectPairing(
      ['wss://a', 'wss://b', 'wss://c'],
      SECRET,
      1000,
      Socket,
    );
    const [a, b, c] = FakeSocket.all;
    a.onerror();
    b.onclose({reason: 'unauthorized', code: 1008});
    c.onclose({reason: '', code: 1006});
    await expect(p).rejects.toThrow(
      /connection error; unauthorized; closed \(1006\)/,
    );
  });

  it('counts an address once even if it errors and then closes', async () => {
    const p = connectPairing(['wss://a', 'wss://b'], SECRET, 1000, Socket);
    const [a, b] = FakeSocket.all;
    a.onerror();
    a.onclose({reason: 'x', code: 1});
    b.onclose({reason: 'y', code: 1});
    await expect(p).rejects.toThrow(/connection error; y/);
  });

  it('gives up after the time limit, with what it knows', async () => {
    jest.useFakeTimers();
    try {
      const p = connectPairing(['wss://a'], SECRET, 500, Socket);
      const assertion = expect(p).rejects.toThrow(/in time/);
      jest.advanceTimersByTime(500);
      await assertion;
      expect(FakeSocket.all[0].closed).toBe(1);
    } finally {
      jest.useRealTimers();
    }
  });

  it('a failure after the time limit changes nothing', async () => {
    jest.useFakeTimers();
    try {
      const p = connectPairing(['wss://a'], SECRET, 100, Socket);
      const assertion = expect(p).rejects.toThrow(/in time/);
      jest.advanceTimersByTime(100);
      await assertion;
      FakeSocket.all[0].onclose({reason: 'late', code: 1});
    } finally {
      jest.useRealTimers();
    }
  });

  it('treats a plaintext answer as a refusal of the handshake', async () => {
    const p = connectPairing(['wss://a'], SECRET, 1000, Socket);
    FakeSocket.all[0].onmessage({data: '{"type":"error"}'});
    await expect(p).rejects.toThrow(/did not answer the handshake/);
  });

  it('treats a handshake that does not verify as a failure of that address', async () => {
    mockState.failHandshake = true;
    const p = connectPairing(['wss://a'], SECRET, 1000, Socket);
    FakeSocket.all[0].onmessage({data: buf(new Uint8Array([9]))});
    await expect(p).rejects.toThrow(/wrong key/);
  });

  it('cannot start without the crypto code, and says so', async () => {
    mockState.throwOnStart = true;
    await expect(
      connectPairing(['wss://a'], SECRET, 1000, Socket),
    ).rejects.toThrow(/encryption unavailable: no crypto/);
    expect(FakeSocket.all).toHaveLength(0);
  });

  it('ignores a message that arrives after an address already failed', async () => {
    const p = connectPairing(['wss://a', 'wss://b'], SECRET, 1000, Socket);
    const [a, b] = FakeSocket.all;
    a.onclose({reason: 'x', code: 1});
    a.onmessage({data: buf(new Uint8Array([9]))}); // too late: a is already counted as lost
    b.onclose({reason: 'y', code: 1});
    await expect(p).rejects.toThrow(/x; y/);
  });
});

describe('the open channel', () => {
  async function open() {
    const p = connectPairing(['wss://a'], SECRET, 1000, Socket);
    const ws = FakeSocket.all[0];
    ws.onmessage({data: buf(new Uint8Array([9]))});
    const ch: PairChannel = await p;
    const frames: PairFrame[] = [];
    const closes: string[] = [];
    ch.onFrame = f => frames.push(f);
    ch.onClose = r => closes.push(r);
    return {ws, ch, frames, closes};
  }

  it('seals what it sends so that only the bridge can read it', async () => {
    const {ws, ch} = await open();
    ch.send({type: 'pairRequest', id: 'x', gen: 1, code: '12345678'});
    expect(
      JSON.parse(utf8Decode(bridge.open(ws.sent[0] as Uint8Array)!)),
    ).toEqual({type: 'pairRequest', id: 'x', gen: 1, code: '12345678'});
    expect(
      Buffer.from(ws.sent[0] as Uint8Array).includes(Buffer.from('12345678')),
    ).toBe(false);
  });

  it('delivers frames that check out', async () => {
    const {ws, frames} = await open();
    ws.onmessage({data: buf(sealed({type: 'pairDenied', reason: 'expired'}))});
    expect(frames).toEqual([{type: 'pairDenied', reason: 'expired'}]);
  });

  it.each([
    ['a text frame', () => '{"type":"pairDenied"}'],
    ['a frame that does not decrypt', () => buf(new Uint8Array(40).fill(7))],
    [
      'a frame that is not a pairing frame',
      () => buf(sealed({type: 'welcome'})),
    ],
    ['a frame that is not JSON', () => buf(bridge.seal(utf8Encode('nope'))[0])],
  ])('closes the channel on %s', async (_n, make) => {
    const {ws, frames} = await open();
    ws.onmessage({data: make()});
    expect(frames).toEqual([]);
    expect(ws.closed).toBeGreaterThanOrEqual(1);
  });

  it('waits for the rest of a long message', async () => {
    const {ws, frames} = await open();
    const big = {type: 'error', code: 'x', pad: 'p'.repeat(70000)};
    const parts = bridge.seal(utf8Encode(JSON.stringify(big)));
    expect(parts.length).toBeGreaterThan(1);
    parts.slice(0, -1).forEach(f => ws.onmessage({data: buf(f)}));
    expect(frames).toEqual([]);
    ws.onmessage({data: buf(parts[parts.length - 1])});
    expect(frames).toEqual([{type: 'error', code: 'x'}]);
  });

  it('reports the end of the channel with its reason', async () => {
    const {ws, closes} = await open();
    ws.onclose({reason: 'pair_denied', code: 1008});
    ws.onclose({reason: '', code: 1006});
    ws.onerror();
    expect(closes).toEqual(['pair_denied', 'closed (1006)']);
  });
});
