/* eslint-disable no-bitwise -- byte-level crypto framing */
// Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s, initiator side — the plugin half of
// bridge/internal/securechan (ADR 0007). Follows the Noise Protocol Framework spec, rev. 34.
// Only the primitives come from libraries (@noble/*, audited); the state machine below is the
// spec's SymmetricState/CipherState, kept minimal for this one pattern.
import {chacha20poly1305} from '@noble/ciphers/chacha.js';
import {x25519} from '@noble/curves/ed25519.js';
import {blake2s} from '@noble/hashes/blake2.js';
import {hmac} from '@noble/hashes/hmac.js';
import {sha256} from '@noble/hashes/sha2.js';
import {utf8Encode} from './utf8';

const PROTOCOL = 'Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s';
const PROLOGUE = 'spellink/2';
const TAG_LEN = 16;
const DH_LEN = 32;
/** Noise caps a message at 65535 bytes; one byte of each chunk says whether more follow. */
export const MAX_PLAIN_CHUNK = 65535 - TAG_LEN - 1;
const MORE_FOLLOWS = 1;
const LAST_CHUNK = 0;
const MAX_MESSAGE = 4 << 20;

const utf8 = utf8Encode;

function concat(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
}

/** Same derivation as the bridge: SHA-256("spellink-psk-v1:" + token). */
export function pskFromToken(token: string): Uint8Array {
  return sha256(utf8(`spellink-psk-v1:${token}`));
}

// ---------------------------------------------------------------- randomness

let fallbackCounter = 0;

/** Which entropy source ephemeral keys come from (shown in diagnostics). */
export function randomSource(): 'crypto' | 'fallback' {
  const c = (globalThis as {crypto?: {getRandomValues?: unknown}}).crypto;
  return typeof c?.getRandomValues === 'function' ? 'crypto' : 'fallback';
}

/**
 * Prefers crypto.getRandomValues. Hermes may not provide it; then many Math.random() draws and
 * timings are hashed together. That is weaker, but the pre-shared key still keeps the session
 * confidential and authenticated — only forward secrecy relies on this randomness.
 */
export function randomBytes(n: number): Uint8Array {
  const out = new Uint8Array(n);
  if (randomSource() === 'crypto') {
    (
      globalThis as unknown as {
        crypto: {getRandomValues: (a: Uint8Array) => Uint8Array};
      }
    ).crypto.getRandomValues(out);
    return out;
  }
  const seed: number[] = [Date.now(), ++fallbackCounter];
  for (let i = 0; i < 64; i++) {
    seed.push(Math.random() * 2 ** 32, performance.now());
  }
  let pool = blake2s(utf8(seed.join(',')));
  for (let o = 0; o < n; o += pool.length) {
    out.set(pool.subarray(0, Math.min(pool.length, n - o)), o);
    pool = blake2s(concat(pool, utf8(String(Math.random()))));
  }
  return out;
}

// ---------------------------------------------------------------- Noise state

/** HKDF from the Noise spec (HMAC-BLAKE2s), returning `count` 32-byte outputs. */
function hkdf(ck: Uint8Array, ikm: Uint8Array, count: 2 | 3): Uint8Array[] {
  const tempKey = hmac(blake2s, ck, ikm);
  const out1 = hmac(blake2s, tempKey, new Uint8Array([1]));
  const out2 = hmac(blake2s, tempKey, concat(out1, new Uint8Array([2])));
  if (count === 2) {
    return [out1, out2];
  }
  return [
    out1,
    out2,
    hmac(blake2s, tempKey, concat(out2, new Uint8Array([3]))),
  ];
}

export class CipherState {
  private n = 0;

  constructor(private readonly k: Uint8Array) {}

  /** ChaChaPoly nonce: 4 zero bytes then the 64-bit counter, little-endian. */
  private nonce(): Uint8Array {
    const nonce = new Uint8Array(12);
    let v = this.n;
    for (let i = 4; i < 12; i++) {
      nonce[i] = v % 256;
      v = Math.floor(v / 256);
    }
    this.n++;
    return nonce;
  }

  encrypt(ad: Uint8Array, plaintext: Uint8Array): Uint8Array {
    return chacha20poly1305(this.k, this.nonce(), ad).encrypt(plaintext);
  }

  /** Throws if the ciphertext was forged or tampered with. */
  decrypt(ad: Uint8Array, ciphertext: Uint8Array): Uint8Array {
    return chacha20poly1305(this.k, this.nonce(), ad).decrypt(ciphertext);
  }
}

class SymmetricState {
  h: Uint8Array;
  ck: Uint8Array;
  k: CipherState | null = null;

  constructor() {
    const name = utf8(PROTOCOL);
    // Longer than HASHLEN (32), so h = HASH(protocol name).
    this.h =
      name.length <= 32
        ? concat(name, new Uint8Array(32 - name.length))
        : blake2s(name);
    this.ck = this.h;
  }

  mixHash(data: Uint8Array): void {
    this.h = blake2s(concat(this.h, data));
  }

  mixKey(ikm: Uint8Array): void {
    const [ck, k] = hkdf(this.ck, ikm, 2);
    this.ck = ck;
    this.k = new CipherState(k);
  }

  mixKeyAndHash(ikm: Uint8Array): void {
    const [ck, h, k] = hkdf(this.ck, ikm, 3);
    this.ck = ck;
    this.mixHash(h);
    this.k = new CipherState(k);
  }

  encryptAndHash(plaintext: Uint8Array): Uint8Array {
    const out = this.k ? this.k.encrypt(this.h, plaintext) : plaintext;
    this.mixHash(out);
    return out;
  }

  decryptAndHash(ciphertext: Uint8Array): Uint8Array {
    const out = this.k ? this.k.decrypt(this.h, ciphertext) : ciphertext;
    this.mixHash(ciphertext);
    return out;
  }

  /** Returns [initiator→responder, responder→initiator]. */
  split(): [CipherState, CipherState] {
    const [k1, k2] = hkdf(this.ck, new Uint8Array(0), 2);
    return [new CipherState(k1), new CipherState(k2)];
  }
}

/**
 * The plugin's side of the NNpsk0 handshake:
 *   -> psk, e
 *   <- e, ee
 */
export class NoiseInitiator {
  private readonly ss = new SymmetricState();
  private readonly ePriv = randomBytes(32);

  constructor(private readonly psk: Uint8Array) {
    this.ss.mixHash(utf8(PROLOGUE));
  }

  /** Message 1: our ephemeral key plus an (empty) encrypted payload. */
  writeMessage1(): Uint8Array {
    this.ss.mixKeyAndHash(this.psk);
    const ePub = x25519.getPublicKey(this.ePriv);
    this.ss.mixHash(ePub);
    this.ss.mixKey(ePub); // psk handshakes also MixKey the ephemeral key
    return concat(ePub, this.ss.encryptAndHash(new Uint8Array(0)));
  }

  /** Message 2 from the bridge. Throws if the bridge does not hold the same key. */
  readMessage2(msg: Uint8Array): SecureChannel {
    if (msg.length < DH_LEN + TAG_LEN) {
      throw new Error('handshake: short message');
    }
    const re = msg.subarray(0, DH_LEN);
    this.ss.mixHash(re);
    this.ss.mixKey(re);
    this.ss.mixKey(x25519.getSharedSecret(this.ePriv, re));
    this.ss.decryptAndHash(msg.subarray(DH_LEN));
    const [send, recv] = this.ss.split();
    return new SecureChannel(send, recv);
  }
}

/** Encrypted message framing shared with the bridge (chunked, one flag byte per chunk). */
export class SecureChannel {
  private partial: Uint8Array[] = [];
  private partialLen = 0;

  constructor(
    private readonly send: CipherState,
    private readonly recv: CipherState,
  ) {}

  /** Encrypts one message into one or more frames. */
  seal(message: Uint8Array): Uint8Array[] {
    const frames: Uint8Array[] = [];
    let rest = message;
    for (;;) {
      const n = Math.min(rest.length, MAX_PLAIN_CHUNK);
      const flag = n < rest.length ? MORE_FOLLOWS : LAST_CHUNK;
      frames.push(
        this.send.encrypt(
          new Uint8Array(0),
          concat(new Uint8Array([flag]), rest.subarray(0, n)),
        ),
      );
      rest = rest.subarray(n);
      if (flag === LAST_CHUNK) {
        return frames;
      }
    }
  }

  /** Decrypts one frame; returns the whole message once its last chunk arrives, else null. */
  open(frame: Uint8Array): Uint8Array | null {
    const plain = this.recv.decrypt(new Uint8Array(0), frame);
    if (plain.length === 0 || this.partialLen + plain.length > MAX_MESSAGE) {
      throw new Error('bad chunk');
    }
    this.partial.push(plain.subarray(1));
    this.partialLen += plain.length - 1;
    if (plain[0] !== LAST_CHUNK) {
      return null;
    }
    const message = concat(...this.partial);
    this.partial = [];
    this.partialLen = 0;
    return message;
  }
}
