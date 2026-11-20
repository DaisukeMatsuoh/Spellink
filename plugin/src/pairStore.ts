// Where a paired device keeps what it needs to reach its bridge (ADR 0016, pairing.md): in the
// NAMES of directories under MyStyle. The SDK cannot write a file's contents, but it can make a
// directory and list a folder, and that survived a restart on a real device. Nothing is ever
// deleted (a delete hung the plugin); a newer pairing simply adds names with a higher `gen`.
//
//   k-<id>-<gen>-<K>      the key (43 characters of base64url)
//   e-<id>-<gen>-<where>  {"n": name, "u": [wss urls]} in base64url
import {decodeBase64Url, encodeBase64Url, isSecret} from './base64url';
import {isGen, isId, isName, isWssUrl} from './pairConfig';
import {utf8Decode, utf8Encode} from './utf8';

/** The part of the file module this needs; a test replaces it. */
export interface FileOps {
  /** The names (not paths) inside `dir`; null when the folder does not exist. */
  list(dir: string): Promise<string[] | null>;
  makeDir(path: string): Promise<boolean>;
}

export const STORE_DIR = '/storage/emulated/0/MyStyle/Spellink-pairing';
/** A directory name holds at most 255 bytes. */
export const MAX_NAME_BYTES = 255;
export const MAX_STORED_URLS = 4;

export interface StoredBridge {
  id: string;
  gen: number;
  key: string;
  name: string;
  urls: string[];
}

export interface Reading {
  bridges: StoredBridge[];
  /** Names that could not be used and why (never contains a key). */
  problems: string[];
}

const NAME_RE = /^([ke])-([0-9a-f]{16})-(\d{1,10})-([A-Za-z0-9_-]+)$/;

/** The directory name for an entry; throws if it would not fit or its parts are invalid. */
export function entryName(
  kind: 'k' | 'e',
  id: string,
  gen: number,
  payload: string,
): string {
  const name = `${kind}-${id}-${gen}-${payload}`;
  if (
    !isId(id) ||
    !isGen(gen) ||
    !NAME_RE.test(name) ||
    utf8Encode(name).length > MAX_NAME_BYTES
  ) {
    throw new Error('pairing entry does not fit in a directory name');
  }
  return name;
}

/** The `e-` payload for a bridge's name and addresses, or throws if they cannot be stored. */
export function encodeWhere(name: string, urls: string[]): string {
  if (!isName(name) || urls.length === 0 || urls.length > MAX_STORED_URLS) {
    throw new Error('bridge name or addresses cannot be stored');
  }
  if (!urls.every(isWssUrl)) {
    throw new Error('only wss:// addresses can be stored');
  }
  return encodeBase64Url(utf8Encode(JSON.stringify({n: name, u: urls})));
}

function decodeWhere(payload: string): {name: string; urls: string[]} | null {
  const bytes = decodeBase64Url(payload);
  if (!bytes) {
    return null;
  }
  let v: unknown;
  try {
    v = JSON.parse(utf8Decode(bytes));
  } catch {
    return null;
  }
  const o = v as {n?: unknown; u?: unknown} | null;
  if (
    typeof o !== 'object' ||
    o === null ||
    !isName(o.n) ||
    !Array.isArray(o.u) ||
    o.u.length === 0 ||
    o.u.length > MAX_STORED_URLS ||
    !o.u.every(isWssUrl)
  ) {
    return null;
  }
  return {name: o.n, urls: o.u};
}

interface Slot {
  keys: string[];
  wheres: string[];
}

/**
 * What the names say, per device: for each id the highest gen that has any entry is the one in
 * use; it is valid only if it has exactly one key and exactly one `where`, both well formed.
 */
export function readEntries(names: string[]): Reading {
  const byId = new Map<string, Map<number, Slot>>();
  const problems: string[] = [];
  for (const n of names) {
    const m = NAME_RE.exec(n);
    if (!m) {
      continue; // not ours
    }
    const gen = Number(m[3]);
    if (!isGen(gen)) {
      problems.push(`${m[1]}-${m[2]}: bad generation`);
      continue;
    }
    const gens = byId.get(m[2]) ?? new Map<number, Slot>();
    byId.set(m[2], gens);
    const slot = gens.get(gen) ?? {keys: [], wheres: []};
    gens.set(gen, slot);
    (m[1] === 'k' ? slot.keys : slot.wheres).push(m[4]);
  }
  const bridges: StoredBridge[] = [];
  for (const [id, gens] of byId) {
    const gen = Math.max(...gens.keys());
    const slot = gens.get(gen)!;
    if (slot.keys.length !== 1 || slot.wheres.length !== 1) {
      problems.push(
        `${id}: gen ${gen} has ${slot.keys.length} key(s) and ${slot.wheres.length} address entries`,
      );
      continue;
    }
    const where = decodeWhere(slot.wheres[0]);
    if (!isSecret(slot.keys[0]) || !where) {
      problems.push(`${id}: gen ${gen} is damaged`);
      continue;
    }
    bridges.push({id, gen, key: slot.keys[0], ...where});
  }
  return {bridges, problems};
}

/** Reads the store. A missing folder is an empty store; a listing that fails is reported. */
export async function readStore(ops: FileOps): Promise<Reading> {
  try {
    return readEntries((await ops.list(STORE_DIR)) ?? []);
  } catch (e) {
    return {
      bridges: [],
      problems: [`cannot read the store: ${(e as Error).message}`],
    };
  }
}

/**
 * Stores a pairing and checks the result by reading it back: true only if exactly this key and
 * these addresses are what the store now says for the device.
 */
export async function saveToStore(
  ops: FileOps,
  b: StoredBridge,
): Promise<boolean> {
  try {
    const key = entryName('k', b.id, b.gen, b.key);
    if (!isSecret(b.key)) {
      return false;
    }
    const where = entryName('e', b.id, b.gen, encodeWhere(b.name, b.urls));
    const have = (await ops.list(STORE_DIR)) ?? [];
    for (const n of [key, where]) {
      if (!have.includes(n) && !(await ops.makeDir(`${STORE_DIR}/${n}`))) {
        return false;
      }
    }
    const back = (await readStore(ops)).bridges.find(x => x.id === b.id);
    return (
      !!back &&
      back.gen === b.gen &&
      back.key === b.key &&
      back.name === b.name &&
      back.urls.length === b.urls.length &&
      back.urls.every((u, i) => u === b.urls[i])
    );
  } catch {
    return false;
  }
}
