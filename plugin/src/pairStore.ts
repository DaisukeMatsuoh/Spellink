// Where a paired device keeps what it needs to reach its bridge (ADR 0016, pairing.md): in the
// NAMES of directories under MyStyle. The SDK cannot write a file's contents, but it can make a
// directory and list a folder, and that survived a restart on a real device. Nothing is ever
// deleted (a delete hung the plugin); a newer pairing simply adds names with a higher `gen`.
//
//   k-<id>-<gen>-<K>                 the key (43 characters of base64url)
//   e-<id>-<gen>-<i>-<n>-<piece>     piece i of n of {"n": name, "u": [wss urls]} in base64url
//
// A directory name holds at most 255 bytes (the file name limit of the Android file systems), so
// the addresses are cut into pieces of MAX_PIECE characters, as many as they need (at most
// MAX_PIECES): a new relay does not need a new format. A set is used only when it is complete.
import {decodeBase64Url, encodeBase64Url, isSecret} from './base64url';
import {isGen, isId, isName, isWssUrl, MAX_URLS} from './pairConfig';
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
/** As many addresses as a setup file may carry. */
export const MAX_STORED_URLS = MAX_URLS;
/** Characters of the addresses in one directory name (the rest of the 255 is the prefix). */
export const MAX_PIECE = 200;
/**
 * Pieces of one set: 6400 characters. MAX_STORED_URLS addresses of MAX_URL_CHARS each, with the
 * longest name, come to about 6000 once encoded (the name in UTF-8 included), so whatever the validators accept can be stored.
 */
export const MAX_PIECES = 32;

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

const KEY_RE = /^k-([0-9a-f]{16})-(\d{1,10})-([A-Za-z0-9_-]+)$/;
const PIECE_RE =
  /^e-([0-9a-f]{16})-(\d{1,10})-(\d{1,2})-(\d{1,2})-([A-Za-z0-9_-]+)$/;

/** `name` if it is a well formed entry that fits in a directory name; throws otherwise. */
function fitted(name: string, re: RegExp): string {
  if (!re.test(name) || utf8Encode(name).length > MAX_NAME_BYTES) {
    throw new Error('pairing entry does not fit in a directory name');
  }
  return name;
}

/** The directory name of the key; throws if it would not fit or its parts are invalid. */
export function keyName(id: string, gen: number, key: string): string {
  if (!isId(id) || !isGen(gen)) {
    throw new Error('pairing entry does not fit in a directory name');
  }
  return fitted(`k-${id}-${gen}-${key}`, KEY_RE);
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

/** The directory names that carry a bridge's name and addresses, in order; throws if they cannot. */
export function whereNames(
  id: string,
  gen: number,
  name: string,
  urls: string[],
): string[] {
  const payload = encodeWhere(name, urls);
  const n = Math.ceil(payload.length / MAX_PIECE);
  if (!isId(id) || !isGen(gen) || n < 1 || n > MAX_PIECES) {
    throw new Error('pairing entry does not fit in directory names');
  }
  return Array.from({length: n}, (_, i) =>
    fitted(
      `e-${id}-${gen}-${i}-${n}-${payload.slice(i * MAX_PIECE, (i + 1) * MAX_PIECE)}`,
      PIECE_RE,
    ),
  );
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

interface Piece {
  i: number;
  n: number;
  text: string;
}

interface Slot {
  keys: string[];
  pieces: Piece[];
}

/** The addresses' text of a complete set (every piece 0..n-1 exactly once), or null. */
function joinPieces(ps: Piece[]): string | null {
  const n = ps[0]?.n ?? 0;
  if (n < 1 || n > MAX_PIECES || ps.length !== n || ps.some(p => p.n !== n)) {
    return null;
  }
  const by: string[] = [];
  for (const p of ps) {
    if (p.i >= n || by[p.i] !== undefined) {
      return null;
    }
    by[p.i] = p.text;
  }
  return by.join('');
}

/**
 * What the names say, per device: for each id the highest gen that has any entry is the one in
 * use; it is valid only if it has exactly one key and one complete set of address pieces, both
 * well formed.
 */
export function readEntries(names: string[]): Reading {
  const byId = new Map<string, Map<number, Slot>>();
  const problems: string[] = [];
  for (const n of names) {
    const k = KEY_RE.exec(n);
    const e = k ? null : PIECE_RE.exec(n);
    const m = k ?? e;
    if (!m) {
      continue; // not ours
    }
    const gen = Number(m[2]);
    if (!isGen(gen)) {
      problems.push(`${n[0]}-${m[1]}: bad generation`);
      continue;
    }
    const gens = byId.get(m[1]) ?? new Map<number, Slot>();
    byId.set(m[1], gens);
    const slot = gens.get(gen) ?? {keys: [], pieces: []};
    gens.set(gen, slot);
    if (k) {
      slot.keys.push(k[3]);
    } else if (e) {
      slot.pieces.push({i: Number(e[3]), n: Number(e[4]), text: e[5]});
    }
  }
  const bridges: StoredBridge[] = [];
  for (const [id, gens] of byId) {
    const gen = Math.max(...gens.keys());
    const slot = gens.get(gen)!;
    const text = joinPieces(slot.pieces);
    if (slot.keys.length !== 1 || text === null) {
      problems.push(
        `${id}: gen ${gen} has ${slot.keys.length} key(s) and ${slot.pieces.length} address piece(s) that do not make one set`,
      );
      continue;
    }
    const where = decodeWhere(text);
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
    if (!isSecret(b.key)) {
      return false;
    }
    const wanted = [
      keyName(b.id, b.gen, b.key),
      ...whereNames(b.id, b.gen, b.name, b.urls),
    ];
    const have = (await ops.list(STORE_DIR)) ?? [];
    for (const n of wanted) {
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
