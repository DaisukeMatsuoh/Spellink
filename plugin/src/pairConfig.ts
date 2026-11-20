// The pairing setup the bridge writes at the top of a device's own .snplg (ADR 0016):
//   globalThis.__SPELLINK_PAIR__ = {v, id, gen, b?, name, urls, windowSec}
// It is read once and checked here; a package that carries nothing, or something wrong, is
// treated as carrying nothing (the screen then explains how to get a proper one).
import {isSecret} from './base64url';

export interface PairSetup {
  /** The device's id on the bridge: 16 lowercase hex digits. */
  id: string;
  /** Which issue of the device's secret this package belongs to; a reissue raises it. */
  gen: number;
  /** The first secret B: only present until the device is paired. */
  b?: string;
  /** The bridge's name, shown to the user. */
  name: string;
  /** Where to connect with B (wss:// only). Empty for a package of an already paired device. */
  urls: string[];
  /** How long the pairing mode lasts, in seconds. */
  windowSec: number;
}

export const DEFAULT_WINDOW_SEC = 90;
export const MIN_WINDOW_SEC = 10;
export const MAX_WINDOW_SEC = 600;
export const MAX_NAME_CHARS = 64;
export const MAX_URLS = 8;
export const MAX_URL_CHARS = 512;
export const MAX_GEN = 2 ** 30;

const ID = /^[0-9a-f]{16}$/;
// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/;

export const isId = (v: unknown): v is string =>
  typeof v === 'string' && ID.test(v);

/** A name the bridge may have given: 1 to 64 characters, no control characters. */
export const isName = (v: unknown): v is string =>
  typeof v === 'string' &&
  [...v].length >= 1 &&
  [...v].length <= MAX_NAME_CHARS &&
  !CONTROL.test(v);

export const isWssUrl = (v: unknown): v is string =>
  typeof v === 'string' &&
  v.length <= MAX_URL_CHARS &&
  /^wss:\/\/[^\s"\\]+$/.test(v);

export const isGen = (v: unknown): v is number =>
  typeof v === 'number' && Number.isInteger(v) && v >= 1 && v <= MAX_GEN;

export type ParsedSetup = {setup: PairSetup} | {problem: string};

/** Checks an untrusted value (from the package) and returns the setup, or says what is wrong. */
export function parsePairSetup(raw: unknown): ParsedSetup {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) {
    return {problem: 'the pairing setup is not an object'};
  }
  const r = raw as Record<string, unknown>;
  if (r.v !== 1) {
    return {problem: `pairing setup version ${String(r.v)} is not understood`};
  }
  if (!isId(r.id)) {
    return {problem: 'the device id is not 16 hex digits'};
  }
  if (!isGen(r.gen)) {
    return {problem: 'the generation is not a positive integer'};
  }
  if (r.b !== undefined && !isSecret(r.b)) {
    return {problem: 'the first secret is not a 32-byte secret'};
  }
  if (!isName(r.name)) {
    return {problem: 'the bridge name is not 1 to 64 printable characters'};
  }
  if (
    !Array.isArray(r.urls) ||
    r.urls.length > MAX_URLS ||
    !r.urls.every(isWssUrl)
  ) {
    return {problem: `urls must be at most ${MAX_URLS} wss:// addresses`};
  }
  if (r.urls.length === 0 && r.b !== undefined) {
    return {problem: 'a package that still has to pair needs an address'};
  }
  const w = r.windowSec;
  const windowSec =
    typeof w === 'number' &&
    Number.isInteger(w) &&
    w >= MIN_WINDOW_SEC &&
    w <= MAX_WINDOW_SEC
      ? w
      : DEFAULT_WINDOW_SEC;
  return {
    setup: {
      id: r.id,
      gen: r.gen,
      ...(r.b !== undefined ? {b: r.b} : {}),
      name: r.name,
      urls: r.urls as string[],
      windowSec,
    },
  };
}

/** What this package carries, or null when it carries nothing; `problem` says why it is unusable. */
export function embeddedSetup(): {
  setup: PairSetup | null;
  problem?: string;
} {
  const raw = (globalThis as {__SPELLINK_PAIR__?: unknown}).__SPELLINK_PAIR__;
  if (raw === undefined) {
    return {setup: null};
  }
  const parsed = parsePairSetup(raw);
  return 'setup' in parsed
    ? {setup: parsed.setup}
    : {setup: null, problem: parsed.problem};
}
