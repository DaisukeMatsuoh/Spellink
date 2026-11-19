// Device checks for the pairing design (docs/notes/research/2026-11-08-pairing.md, section 5).
// Each probe only reads and reports; nothing here changes the device.
import {NativeModules, TurboModuleRegistry} from 'react-native';
import {
  PluginCommAPI,
  PluginDocAPI,
  PluginFileAPI,
  PluginManager,
  PluginNoteAPI,
} from 'sn-plugin-lib';

type Log = (line: string) => void;

const PROBE_TIMEOUT_MS = 5000;
const MAX_SEED_LEN = 256;

const withTimeout = <T>(p: Promise<T>, label: string): Promise<T> =>
  new Promise<T>((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error(`${label}: no answer in ${PROBE_TIMEOUT_MS}ms`)),
      PROBE_TIMEOUT_MS,
    );
    p.then(
      v => {
        clearTimeout(timer);
        resolve(v);
      },
      e => {
        clearTimeout(timer);
        reject(e);
      },
    );
  });

/** The text a seed-patched package carries (scripts/patch-snplg.js), or null. Validated, not trusted. */
export function embeddedSeed(): string | null {
  const v = (globalThis as {__SPELLINK_SEED__?: unknown}).__SPELLINK_SEED__;
  return typeof v === 'string' && v.length > 0 && v.length <= MAX_SEED_LEN
    ? v
    : null;
}

/** Experiment 1: did the text that the host added to the bundle arrive? */
export async function probeSeed(log: Log): Promise<void> {
  const raw = (globalThis as {__SPELLINK_SEED__?: unknown}).__SPELLINK_SEED__;
  const seed = embeddedSeed();
  log(
    `seed: ${
      seed !== null
        ? `present, "${seed}" (${seed.length} chars)`
        : `absent (typeof ${typeof raw})`
    }`,
  );
}

const ownNames = (o: unknown): string[] => {
  if (o == null || (typeof o !== 'object' && typeof o !== 'function')) {
    return [];
  }
  const names = new Set<string>();
  let p: object | null = o as object;
  for (let depth = 0; p && p !== Object.prototype && depth < 4; depth++) {
    Object.getOwnPropertyNames(p).forEach(n => names.add(n));
    p = Object.getPrototypeOf(p);
  }
  return [...names].filter(n => !['length', 'name', 'prototype'].includes(n));
};

/** Experiment 3a: every name the SDK and the native modules expose, to look for an id or a file writer. */
export async function probeApis(log: Log): Promise<void> {
  const groups: Array<[string, unknown]> = [
    ['PluginManager', PluginManager],
    ['PluginCommAPI', PluginCommAPI],
    ['PluginNoteAPI', PluginNoteAPI],
    ['PluginFileAPI', PluginFileAPI],
    ['PluginDocAPI', PluginDocAPI],
  ];
  for (const [label, o] of groups) {
    const names = ownNames(o);
    log(`${label} (${names.length}): ${names.join(' ')}`);
  }
  const mods = ownNames(NativeModules);
  log(`NativeModules (${mods.length}): ${mods.join(' ')}`);
  for (const m of ['RTNFileUtils', 'FileSelector', 'NativeUIUtils']) {
    const mod = TurboModuleRegistry.get(m);
    log(`TurboModule ${m}: ${mod ? ownNames(mod).join(' ') : 'absent'}`);
  }
  const hint = /serial|device|uuid|imei|mac|android|id$/i;
  const all = groups.flatMap(([, o]) => ownNames(o)).concat(mods);
  log(
    `names that look like an id: ${
      all.filter(n => hint.test(n)).join(' ') || 'none'
    }`,
  );
}

const FILES = [
  '/proc/cpuinfo',
  '/sys/devices/soc0/serial_number',
  '/sys/class/android_usb/android0/iSerial',
  '/sys/class/net/wlan0/address',
  '/proc/sys/kernel/random/boot_id',
  '/storage/emulated/0/MyStyle/Spellink/bridges.json',
];

/** Experiment 3b: which files the plugin can read through file:// (sizes and a short head only). */
export async function probeFiles(log: Log): Promise<void> {
  for (const path of FILES) {
    try {
      const res = await withTimeout(fetch(`file://${path}`), path);
      const text = await withTimeout(res.text(), path);
      const head = text.replace(/\s+/g, ' ').slice(0, 48);
      log(`read ${path}: ok ${text.length} chars, head "${head}"`);
    } catch (e) {
      log(`read ${path}: ${(e as Error).message}`);
    }
  }
  const fu = TurboModuleRegistry.get('RTNFileUtils') as unknown as {
    exists(p: string): Promise<boolean>;
    listFiles(p: string): Promise<string[] | null | undefined>;
  } | null;
  if (!fu) {
    return log('RTNFileUtils: absent');
  }
  for (const dir of [
    '/storage/emulated/0/MyStyle',
    '/storage/emulated/0/MyStyle/Spellink',
  ]) {
    try {
      const exists = await withTimeout(fu.exists(dir), dir);
      const list = exists ? await withTimeout(fu.listFiles(dir), dir) : null;
      log(
        `exists ${dir}: ${exists}${
          Array.isArray(list)
            ? ` files=${JSON.stringify(list.slice(0, 20))}`
            : ''
        }`,
      );
    } catch (e) {
      log(`exists ${dir}: ${(e as Error).message}`);
    }
  }
}

type FileUtilsModule = {
  exists(p: string): Promise<boolean>;
  makeDir(p: string): Promise<boolean>;
  listFiles(
    p: string,
  ): Promise<Array<{type: number; path: string}> | null | undefined>;
};

const MYSTYLE = '/storage/emulated/0/MyStyle';
const STORE = `${MYSTYLE}/Spellink-probe-store`;
const ENTRY_PREFIX = 'seed-';

/**
 * Experiment 5: can a short secret be kept in a directory NAME? The SDK has no call that writes a
 * file's contents, but makeDir / listFiles exist. This probe never deletes anything (a delete in an
 * earlier version hung the plugin): it lists the entry, and creates it only if it is not there.
 * First press: "created" (and read back). After the device has been restarted, press again:
 * "survived" means the name is still there.
 */
export async function probeNameStore(log: Log): Promise<void> {
  const fu = TurboModuleRegistry.get(
    'RTNFileUtils',
  ) as unknown as FileUtilsModule | null;
  if (!fu) {
    return log('name store: RTNFileUtils absent');
  }
  const name = `${ENTRY_PREFIX}Zk3_Qw-0123456789abcdefghijklmnopqrstuvwxyzABCDEFGH`; // 62 chars, url-safe
  const list = async (): Promise<string[]> => {
    log('name store: listFiles…');
    const raw = await withTimeout(fu.listFiles(STORE), 'listFiles');
    if (!Array.isArray(raw)) {
      return [];
    }
    return raw
      .map(e =>
        typeof e?.path === 'string' ? e.path.split('/').pop() ?? '' : '',
      )
      .filter(n => n.startsWith(ENTRY_PREFIX));
  };
  try {
    log('name store: exists…');
    const there = await withTimeout(fu.exists(STORE), 'exists');
    const found = there ? await list() : [];
    if (found.length > 0) {
      return log(
        `name store: survived — ${JSON.stringify(found)} (${
          found.includes(name) ? 'EXACT' : 'DIFFERENT'
        })`,
      );
    }
    log('name store: makeDir…');
    const made = await withTimeout(fu.makeDir(`${STORE}/${name}`), 'makeDir');
    const back = await list();
    log(
      `name store: created makeDir=${made}, listed ${JSON.stringify(
        back,
      )}, read back ${
        back.includes(name) ? 'EXACT' : 'DIFFERENT'
      }; restart the device and press again`,
    );
  } catch (e) {
    log(`name store: ERROR ${(e as Error).message}`);
  }
}

/** Experiment 3c: can file:// read a file in MyStyle that really exists (the first .snplg listed)? */
export async function probeReadExisting(log: Log): Promise<void> {
  const fu = TurboModuleRegistry.get(
    'RTNFileUtils',
  ) as unknown as FileUtilsModule | null;
  if (!fu) {
    return log('read existing: RTNFileUtils absent');
  }
  try {
    const raw = await withTimeout(fu.listFiles(MYSTYLE), 'listFiles');
    const first = Array.isArray(raw)
      ? raw.find(e => e?.type === 1 && typeof e.path === 'string')
      : undefined;
    if (!first) {
      return log('read existing: no file listed');
    }
    const res = await withTimeout(fetch(`file://${first.path}`), first.path);
    const buf = await withTimeout(res.arrayBuffer(), first.path);
    log(`read existing ${first.path}: ok ${buf.byteLength} bytes`);
  } catch (e) {
    log(`read existing: ${(e as Error).message}`);
  }
}
