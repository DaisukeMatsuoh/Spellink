export interface BridgeEndpoint {
  name: string;
  token: string;
  urls: string[];
}

/** Written by `spellink-bridge pair` and copied to the device. */
export const CONFIG_PATH = '/storage/emulated/0/MyStyle/Spellink/bridges.json';

// Fallback baked in at build time (plugin/src/bridges.local.json, git-ignored).

const baked = require('./bridges.local.json') as {bridges?: BridgeEndpoint[]};

function parse(json: unknown): BridgeEndpoint[] {
  const list = (json as {bridges?: unknown})?.bridges;
  if (!Array.isArray(list)) {
    return [];
  }
  return list.filter(
    (b): b is BridgeEndpoint =>
      typeof b?.name === 'string' &&
      typeof b?.token === 'string' &&
      Array.isArray(b?.urls) &&
      b.urls.every(
        (u: unknown) => typeof u === 'string' && /^wss?:\/\//.test(u),
      ),
  );
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
