import {loadPairing} from './pairingState';

export interface BridgeEndpoint {
  /** The device's id on a bridge it was paired with one by one (ADR 0016); older endpoints have none. */
  id?: string;
  name: string;
  token: string;
  urls: string[];
}

/** Written by `spellink-bridge pair` and copied to the device. */
export const CONFIG_PATH = '/storage/emulated/0/MyStyle/Spellink/bridges.json';

const MAX_BRIDGES = 8;
const MAX_URLS = 8;

const isEndpoint = (b: unknown): b is BridgeEndpoint => {
  const e = b as Partial<BridgeEndpoint> | null;
  return (
    typeof e?.name === 'string' &&
    e.name.length > 0 &&
    e.name.length <= 128 &&
    typeof e.token === 'string' &&
    e.token.length > 0 &&
    e.token.length <= 256 &&
    Array.isArray(e.urls) &&
    e.urls.length > 0 &&
    e.urls.length <= MAX_URLS &&
    e.urls.every(
      (u: unknown) =>
        typeof u === 'string' && u.length <= 512 && /^wss?:\/\//.test(u),
    )
  );
};

/** The bridges in a pairing file; an entry that is not usable is left out, not guessed at. */
export function parse(json: unknown): BridgeEndpoint[] {
  const list = (json as {bridges?: unknown})?.bridges;
  if (!Array.isArray(list)) {
    return [];
  }
  return list.filter(isEndpoint).slice(0, MAX_BRIDGES);
}

export interface LoadedBridges {
  endpoints: BridgeEndpoint[];
  source: 'paired' | 'file' | 'none';
  fileError?: string;
}

/** The bridges this device has paired with (ADR 0016): the key it stored, and where to reach each. */
async function pairedEndpoints(): Promise<BridgeEndpoint[]> {
  try {
    return (await loadPairing()).plan.bridges.flatMap(b =>
      b.state === 'Paired' && b.stored
        ? [
            {
              id: b.id,
              name: b.stored.name,
              token: b.stored.key,
              urls: b.stored.urls,
            },
          ]
        : [],
    );
  } catch {
    return []; // a store that cannot be read must not stop the older ways of finding a bridge
  }
}

/**
 * The bridges to connect to: those paired one by one (ADR 0016); only when there are none, the older
 * way — the pairing file in MyStyle (needs FILE:READ). Nothing is baked into the build: a package carries no
 * bridge address or token of its own, so a build made on any machine is safe to hand out.
 */
export async function loadBridges(): Promise<LoadedBridges> {
  const paired = await pairedEndpoints();
  if (paired.length > 0) {
    // Once a device is paired, the shared file in MyStyle is not read: anything that can write
    // MyStyle could add an address there, and the paired key is never offered to it.
    return {endpoints: paired.slice(0, MAX_BRIDGES), source: 'paired'};
  }
  return loadOlderBridges();
}

async function loadOlderBridges(): Promise<LoadedBridges> {
  try {
    const res = await fetch(`file://${CONFIG_PATH}`);
    const endpoints = parse(await res.json());
    if (endpoints.length > 0) {
      return {endpoints, source: 'file'};
    }
    return {endpoints: [], source: 'none', fileError: 'no bridges in file'};
  } catch (e) {
    return {endpoints: [], source: 'none', fileError: String(e)};
  }
}
