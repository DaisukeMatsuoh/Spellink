import {
  CipherState,
  MAX_PLAIN_CHUNK,
  SecureChannel,
  pskFromToken,
  randomBytes,
} from '../src/noise';

const key = new Uint8Array(32).fill(7);
const pair = () => ({
  tx: new SecureChannel(new CipherState(key), new CipherState(key)),
  rx: new SecureChannel(new CipherState(key), new CipherState(key)),
});

describe('SecureChannel', () => {
  it('round-trips a small message', () => {
    const {tx, rx} = pair();
    const [frame] = tx.seal(new Uint8Array([1, 2, 3]));
    expect(Array.from(rx.open(frame)!)).toEqual([1, 2, 3]);
  });

  it('splits large messages into chunks and reassembles them', () => {
    const {tx, rx} = pair();
    const big = new Uint8Array(MAX_PLAIN_CHUNK * 2 + 10).map((_, i) => i % 251);
    const frames = tx.seal(big);
    expect(frames.length).toBe(3);
    let out: Uint8Array | null = null;
    for (const f of frames) {
      out = rx.open(f) ?? out;
    }
    expect(Buffer.compare(Buffer.from(out!), Buffer.from(big))).toBe(0);
  });

  it('detects tampering', () => {
    const {tx, rx} = pair();
    const [frame] = tx.seal(new Uint8Array([9, 9, 9]));
    frame[2] ^= 1;
    expect(() => rx.open(frame)).toThrow();
  });
});

describe('keys', () => {
  it('derives a 32-byte key from the token, differently per token', () => {
    expect(pskFromToken('a')).toHaveLength(32);
    expect(
      Buffer.from(pskFromToken('a')).equals(Buffer.from(pskFromToken('b'))),
    ).toBe(false);
  });
  it('produces distinct random bytes even without crypto.getRandomValues', () => {
    const a = randomBytes(32);
    const b = randomBytes(32);
    expect(a).toHaveLength(32);
    expect(Buffer.from(a).equals(Buffer.from(b))).toBe(false);
  });
});
