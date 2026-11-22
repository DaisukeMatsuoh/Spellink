// Builds docs/usage/usage-dashboard.md from what the relays publish at GET /v1/stats (ADR 0021).
// Usage: node scripts/usage-dashboard.mjs [--offline]   (--offline: only rebuild the page from docs/usage/data)
// The relays to ask are listed in docs/usage/relays.json: ["https://us.relay.spellink.uk", ...].
// Histograms are added together, never averaged; a percentile is the upper edge of the bucket it falls in.
import {readFileSync, writeFileSync, readdirSync, mkdirSync, existsSync, renameSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import path from 'node:path';

export const BOUNDS = {
  ping_client_ms: [10, 20, 50, 100, 200, 500, 1000, 2000, 5000],
  ping_bridge_ms: [10, 20, 50, 100, 200, 500, 1000, 2000, 5000],
  forward_ms: [10, 20, 50, 100, 200, 500, 1000, 2000, 5000],
  setup_ms: [10, 20, 50, 100, 200, 500, 1000, 2000, 5000],
  session_s: [10, 30, 60, 120, 300, 600, 1800, 3600],
  session_bytes: [1024, 10240, 102400, 1048576, 10485760],
};
const KEEP_DAYS = 8;
const MAX_COUNT = Number.MAX_SAFE_INTEGER;

const isCount = v => Number.isSafeInteger(v) && v >= 0 && v <= MAX_COUNT;
const isDay = s => typeof s === 'string' && /^\d{4}-\d{2}-\d{2}$/.test(s);

/** The days of a published document, or an Error: the network is a boundary, nothing is trusted. */
export function parseStats(text) {
  let v;
  try {
    v = JSON.parse(text);
  } catch (e) {
    return new Error(`not json: ${e.message}`);
  }
  if (!v || v.v !== 1 || !Array.isArray(v.days) || v.days.length > KEEP_DAYS) {
    return new Error('not a stats document');
  }
  const sameBounds = (got, want) => Array.isArray(got) && got.length === want.length && got.every((x, i) => x === want[i]);
  if (!sameBounds(v.bounds_ms, BOUNDS.ping_client_ms) || !sameBounds(v.bounds_s, BOUNDS.session_s) || !sameBounds(v.bounds_bytes, BOUNDS.session_bytes)) {
    return new Error('other bounds');
  }
  for (const d of v.days) {
    if (!d || !isDay(d.day) || !isCount(d.sessions) || !isCount(d.peak_concurrent)) {
      return new Error('bad day');
    }
    for (const [k, b] of Object.entries(BOUNDS)) {
      if (!Array.isArray(d[k]) || d[k].length !== b.length + 1 || !d[k].every(isCount)) {
        return new Error(`bad histogram ${k}`);
      }
    }
    if (!d.refused || typeof d.refused !== 'object' || !Object.values(d.refused).every(isCount)) {
      return new Error('bad refused');
    }
  }
  return v.days;
}

const addInto = (a, b) => b.forEach((x, i) => (a[i] += x));

/** Adds days together: counts and histograms by sum, the peak by maximum. */
export function merge(days) {
  const out = {sessions: 0, peak_concurrent: 0, refused: {}};
  for (const [k, b] of Object.entries(BOUNDS)) out[k] = new Array(b.length + 1).fill(0);
  for (const d of days) {
    out.sessions += d.sessions;
    out.peak_concurrent = Math.max(out.peak_concurrent, d.peak_concurrent);
    for (const [r, n] of Object.entries(d.refused)) out.refused[r] = (out.refused[r] ?? 0) + n;
    for (const k of Object.keys(BOUNDS)) addInto(out[k], d[k]);
  }
  return out;
}

/** The bucket a quantile (0..1) falls in, as text: "≤ 200", or "> 5000" for the last bucket; "-" if empty. */
export function quantile(counts, bounds, q) {
  const total = counts.reduce((a, b) => a + b, 0);
  if (total === 0) return '-';
  const need = Math.ceil(total * q);
  let seen = 0;
  for (let i = 0; i < counts.length; i++) {
    seen += counts[i];
    if (seen >= need) return i < bounds.length ? `≤ ${bounds[i]}` : `> ${bounds[bounds.length - 1]}`;
  }
  return '-';
}

const UNITS = {ping_client_ms: 'ms', ping_bridge_ms: 'ms', forward_ms: 'ms', setup_ms: 'ms', session_s: 's', session_bytes: 'B'};
const LABEL = {
  ping_client_ms: '端末⇄中継の往復',
  ping_bridge_ms: 'ブリッジ⇄中継の往復',
  forward_ms: '中継の転送',
  setup_ms: 'つながるまで',
  session_s: 'セッションの長さ',
  session_bytes: 'セッションの通信量',
};

export function render(byRelay, minSessions = 1) {
  const lines = ['# 使われ方(中継の集計)', '', '<!-- scripts/usage-dashboard.mjs が作る。手で直さない。ADR 0021 -->', ''];
  lines.push('中継が数えた**時間と件数だけ**の集計です(誰が・いつ・何を、は記録していません)。値は、その幅の上限(「≤ 200」は 200 以下)。', '');
  const relays = Object.keys(byRelay).sort();
  if (relays.length === 0) {
    lines.push('まだ集計がありません(中継が動いていない、または `docs/usage/relays.json` が空です)。', '');
    return lines.join('\n');
  }
  for (const name of [...relays, null]) {
    const days = name === null ? relays.flatMap(r => Object.values(byRelay[r])) : Object.values(byRelay[name]);
    if (name === null && relays.length < 2) continue;
    const m = merge(days);
    const range = days.map(d => d.day).sort();
    lines.push(`## ${name ?? '全体'}`, '');
    lines.push(`期間: ${range[0] ?? '-'} 〜 ${range[range.length - 1] ?? '-'}(${new Set(range).size} 日分)。セッション ${m.sessions}、同時接続の最大 ${m.peak_concurrent}、断った数 ${JSON.stringify(m.refused)}`, '');
    if (m.sessions < minSessions) {
      lines.push('件数が少なすぎるため、分布は出しません。', '');
      continue;
    }
    lines.push('| 項目 | 単位 | p50 | p90 | p99 |', '|---|---|---|---|---|');
    for (const k of Object.keys(BOUNDS)) {
      lines.push(`| ${LABEL[k]} | ${UNITS[k]} | ${[0.5, 0.9, 0.99].map(q => quantile(m[k], BOUNDS[k], q)).join(' | ')} |`);
    }
    lines.push('');
  }
  return lines.join('\n');
}

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.join(here, '..');
const dataDir = path.join(root, 'docs/usage/data');

const nameOf = url => new URL(url).hostname;

function loadStored() {
  const out = {};
  if (!existsSync(dataDir)) return out;
  for (const f of readdirSync(dataDir).filter(f => f.endsWith('.json'))) {
    const days = parseStats(JSON.stringify({v: 1, bounds_ms: BOUNDS.ping_client_ms, bounds_s: BOUNDS.session_s, bounds_bytes: BOUNDS.session_bytes, days: Object.values(JSON.parse(readFileSync(path.join(dataDir, f), 'utf8')))}));
    if (days instanceof Error) throw new Error(`${f}: ${days.message}`);
    out[f.slice(0, -5)] = Object.fromEntries(days.map(d => [d.day, d]));
  }
  return out;
}

async function main() {
  const offline = process.argv.includes('--offline');
  const stored = loadStored();
  if (!offline) {
    const listPath = path.join(root, 'docs/usage/relays.json');
    const urls = existsSync(listPath) ? JSON.parse(readFileSync(listPath, 'utf8')) : [];
    for (const url of urls) {
      const name = nameOf(url);
      try {
        const res = await fetch(new URL('/v1/stats', url), {signal: AbortSignal.timeout(20000)});
        if (!res.ok) throw new Error(`status ${res.status}`);
        const days = parseStats(await res.text());
        if (days instanceof Error) throw days;
        stored[name] = {...(stored[name] ?? {}), ...Object.fromEntries(days.map(d => [d.day, d]))};
        mkdirSync(dataDir, {recursive: true});
        const tmp = path.join(dataDir, `${name}.json.tmp`);
        writeFileSync(tmp, JSON.stringify(stored[name], null, 1) + '\n');
        renameSync(tmp, path.join(dataDir, `${name}.json`));
      } catch (e) {
        console.error(`${url}: ${e.message} (this relay is left out this time)`);
      }
    }
  }
  writeFileSync(path.join(root, 'docs/usage/usage-dashboard.md'), render(stored) + '\n');
}

if (process.argv[1] === fileURLToPath(import.meta.url)) await main();
