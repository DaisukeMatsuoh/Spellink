// Device checks for what the plugin could use to talk to the bridge, and how long each route takes.
// Read-only: nothing here changes the device or the note.

export type Log = (line: string) => void;

/** The names a JS engine may offer for talking over a network, other than fetch and WebSocket. */
export const CHANNEL_NAMES = [
  'WebSocket',
  'fetch',
  'XMLHttpRequest',
  'RTCPeerConnection',
  'RTCDataChannel',
  'EventSource',
  'Worker',
  'SharedWorker',
  'BroadcastChannel',
  'MessageChannel',
  'navigator',
  'Blob',
  'URL',
] as const;

/** One line per name: what `typeof` says about it. Anything but "undefined" is worth a closer look. */
export function channelReport(scope: object): string[] {
  const g = scope as Record<string, unknown>;
  return CHANNEL_NAMES.map(n => `${n}: ${typeof g[n]}`);
}

/** The own names of `navigator` (a connection or bluetooth object would show up here). */
export function navigatorNames(scope: object): string[] {
  const nav = (scope as {navigator?: unknown}).navigator;
  if (nav == null || typeof nav !== 'object') {
    return [];
  }
  const names = new Set<string>();
  let p: object | null = nav;
  for (let depth = 0; p && p !== Object.prototype && depth < 3; depth++) {
    Object.getOwnPropertyNames(p).forEach(n => names.add(n));
    p = Object.getPrototypeOf(p);
  }
  return [...names].filter(n => n !== 'constructor').sort();
}

export type Spread = {n: number; min: number; median: number; max: number};

/** min / median / max of the finite, non-negative samples; null if there is none. */
export function spread(samples: number[]): Spread | null {
  const s = samples
    .filter(x => Number.isFinite(x) && x >= 0)
    .sort((a, b) => a - b);
  if (s.length === 0) {
    return null;
  }
  const mid = s.length >> 1;
  const median = s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
  return {n: s.length, min: s[0], median, max: s[s.length - 1]};
}

export function spreadText(sp: Spread | null): string {
  return sp
    ? `n=${sp.n} min ${sp.min}ms median ${sp.median}ms max ${sp.max}ms`
    : 'no sample';
}

/**
 * Times `count` requests one after another. The first one is reported apart: it pays for the
 * connection (TCP, TLS), the rest show what a message costs on a connection that is already open.
 */
export async function timeRequests(
  once: () => Promise<void>,
  count: number,
  now: () => number = Date.now,
): Promise<{first: number | null; rest: number[]; failures: string[]}> {
  const times: Array<number | null> = [];
  const failures: string[] = [];
  for (let i = 0; i < count; i++) {
    const t = now();
    try {
      await once();
      times.push(now() - t);
    } catch (e) {
      times.push(null);
      failures.push((e as Error).message);
    }
  }
  return {
    first: times[0] ?? null,
    rest: times.slice(1).filter((x): x is number => x !== null),
    failures,
  };
}

/** Experiment: which ways out of the plugin exist besides fetch and WebSocket. */
export async function probeChannels(log: Log): Promise<void> {
  channelReport(globalThis).forEach(l => log(l));
  const nav = navigatorNames(globalThis);
  log(`navigator (${nav.length}): ${nav.join(' ') || 'absent'}`);
}
