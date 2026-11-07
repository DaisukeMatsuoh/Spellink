export interface BridgeEndpoint {
  name: string;
  token: string;
  urls: string[];
}

/** Written by `spellink-bridge pair` and copied to the device. */
export const CONFIG_PATH = '/storage/emulated/0/MyStyle/Spellink/bridges.json';

// Fallback baked in at build time (plugin/src/bridges.local.json, git-ignored).

const baked = require('./bridges.local.json') as {bridges?: BridgeEndpoint[]};

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
  source: 'file' | 'baked';
  fileError?: string;
}

/** Reads the paired bridges from MyStyle (needs FILE:READ), falling back to the baked-in list. */
export async function loadBridges(): Promise<LoadedBridges> {
  try {
    const res = await fetch(`file://${CONFIG_PATH}`);
    const endpoints = parse(await res.json());
    if (endpoints.length > 0) {
      return {endpoints, source: 'file'};
    }
    return {
      endpoints: parse(baked),
      source: 'baked',
      fileError: 'no bridges in file',
    };
  } catch (e) {
    return {endpoints: parse(baked), source: 'baked', fileError: String(e)};
  }
}
