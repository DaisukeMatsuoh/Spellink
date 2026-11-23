// Device checks for the questions the docs leave open (see docs/notes/progress/README.md).
// Every result is shown on screen and also sent to the bridge's report log when one is reachable.
import {PluginCommAPI, PluginManager, PluginNoteAPI} from 'sn-plugin-lib';
import {answerLayout, AskFlow} from './askFlow';
import {connectFirst, type ConnectAttempt} from './bridgeClient';
import {loadBridges} from './bridgeConfig';
import {randomSource} from './noise';
import {spread, spreadText, timeRequests} from './netProbe';
import {dumpTrace} from './trace';
import type {InkStroke} from './protocol';
import {
  callChecked,
  call,
  currentPen,
  deleteElementsAt,
  ensurePermissions,
  insertPageAfter,
  jumpToPageIn,
  noteContext,
  PERMISSIONS,
  readPage,
  releaseElements,
  probeFillMode,
  writeStrokes,
  type WriteTiming,
} from './sdk';

export type Log = (line: string) => void;

const safe = async (
  label: string,
  f: () => Promise<unknown>,
  log: Log,
): Promise<unknown> => {
  try {
    const v = await f();
    log(`${label}: ${JSON.stringify(v)}`);
    return v;
  } catch (e) {
    log(`${label}: ERROR ${(e as Error).message}`);
    return undefined;
  }
};

export async function environment(log: Log): Promise<void> {
  // What the end-to-end encryption can rely on in this JS engine.
  log(
    `js: BigInt=${typeof BigInt} crypto.getRandomValues=${randomSource()} TextEncoder=${typeof TextEncoder} TextDecoder=${typeof TextDecoder}`,
  );
  await safe('deviceType', () => PluginManager.getDeviceType(), log);
  await safe('pluginDir', () => PluginManager.getPluginDirPath(), log);
  await safe(
    'locale',
    async () => new Intl.Collator().resolvedOptions().locale,
    log,
  );
  for (const p of PERMISSIONS) {
    await safe(p, () => PluginManager.hasPermission(p), log);
  }
}

/**
 * What the note app says about the open note, the pen and handwriting. These ask the note app
 * about the note itself, and some situations (no note in front, a screen opened over it) leave
 * them unanswered; a request that gets no answer holds up everything behind it in the line. So
 * they run last, after the checks that matter most (permissions, the network).
 */
export async function noteProbe(log: Log): Promise<void> {
  await safe('note', () => noteContext(), log);
  await safe(
    'pen',
    () => call('getPenInfo', () => PluginCommAPI.getPenInfo()),
    log,
  );
  await safe(
    'canHandwrite',
    () => call('canHandwrite', () => PluginCommAPI.canHandwrite()),
    log,
  );
}

export async function requestPermissions(log: Log): Promise<void> {
  for (const p of PERMISSIONS) {
    await safe(`request ${p}`, () => PluginManager.requestPermission(p), log);
  }
}

/** Cleartext HTTP and WebSocket reachability of every paired bridge URL. */
/**
 * The plugin may not touch the network (or read the note) until the user grants it; without
 * this, connections fail on the device before ever reaching the bridge.
 */
async function withPermissions(log: Log): Promise<boolean> {
  let missing: string[];
  try {
    missing = await ensurePermissions();
  } catch (e) {
    // the note app did not answer: say so and go on (the checks below do not need its answer)
    log(`permissions: ERROR ${(e as Error).message}`);
    return false;
  }
  if (missing.length > 0) {
    log(`permissions missing: ${missing.join(', ')}`);
  }
  return missing.length === 0;
}

const HEALTH_REPEATS = 6;

export async function network(log: Log): Promise<void> {
  await withPermissions(log);
  const loaded = await loadBridges();
  log(
    `bridges: ${
      loaded.endpoints.map(e => e.name).join(', ') || 'none'
    } (source=${loaded.source}${
      loaded.fileError ? `, file: ${loaded.fileError}` : ''
    })`,
  );
  for (const url of loaded.endpoints.flatMap(e => e.urls)) {
    const health = url.replace(/^ws/, 'http').replace(/\/v1$/, '/health');
    const t = Date.now();
    try {
      const res = await fetch(health);
      log(
        `http ${health}: ${res.status} ${await res.text()} ${Date.now() - t}ms`,
      );
    } catch (e) {
      log(`http ${health}: ERROR ${(e as Error).message} ${Date.now() - t}ms`);
      continue;
    }
    // Round trips to the same address, one after another: the first pays for the connection.
    const timed = await timeRequests(async () => {
      const r = await fetch(health);
      await r.text();
    }, HEALTH_REPEATS);
    log(
      `  rtt ${health}: first ${timed.first ?? 'failed'}ms, then ${spreadText(
        spread(timed.rest),
      )}${timed.failures.length ? ` (${timed.failures.length} failed)` : ''}`,
    );
  }
  const attempts: ConnectAttempt[] = [];
  try {
    const conn = await connectFirst(loaded.endpoints, 5000, attempts);
    log(`ws winner: ${conn.name} ${conn.url} [${conn.mode}]`);
    conn.close();
  } catch (e) {
    log(`ws: ERROR ${(e as Error).message}`);
  }
  for (const a of attempts) {
    log(`  ws ${a.url}: ${a.error ?? 'ok'} ${a.ms}ms`);
  }
}

/** A zig-zag of `count` strokes × `pointsPerStroke` points near the bottom of the page. */
function testStrokes(
  width: number,
  height: number,
  count: number,
  pointsPerStroke: number,
): InkStroke[] {
  const strokes: InkStroke[] = [];
  const cols = Math.ceil(Math.sqrt(count * 4));
  const cell = Math.floor((width * 0.8) / cols);
  for (let i = 0; i < count; i++) {
    const x0 = Math.round(width * 0.1 + (i % cols) * cell);
    const y0 = Math.round(height * 0.7 + Math.floor(i / cols) * cell);
    const p: number[] = [];
    const w: number[] = [];
    for (let k = 0; k < pointsPerStroke; k++) {
      p.push(
        x0 + Math.round((k / pointsPerStroke) * cell * 0.8),
        y0 + (k % 2 ? Math.round(cell * 0.6) : 0),
      );
      w.push(2500);
    }
    strokes.push({p, w});
  }
  return strokes;
}

/** Inserts synthetic strokes into the open page and times each phase. */
export async function strokeBenchmark(count: number, log: Log): Promise<void> {
  try {
    log(`fill mode: ${await probeFillMode()}`);
    const ctx = await noteContext();
    const pen = await currentPen();
    const timing: WriteTiming = {create: 0, fill: 0, insert: 0};
    const t = Date.now();
    await writeStrokes(
      testStrokes(ctx.size.width, ctx.size.height, count, 30),
      ctx.page,
      pen,
      timing,
    );
    log(
      `strokes ${count}×30pt: total ${Date.now() - t}ms ${JSON.stringify(
        timing,
      )} pen=${JSON.stringify(pen)}`,
    );
    log(
      '→ Close this screen: are the zig-zags visible on the page without reopening it?',
    );
  } catch (e) {
    log(`strokes: ERROR ${(e as Error).message}`);
  }
}

export async function textBoxTest(log: Log): Promise<void> {
  const ctx = await noteContext().catch(() => null);
  if (!ctx) {
    return log('textbox: no note open');
  }
  const {width, height} = ctx.size;
  await safe(
    'insertText',
    () =>
      call('insertText', () =>
        PluginNoteAPI.insertText({
          textContentFull: 'Spellink text box test / テキストボックスのテスト',
          textRect: {
            left: Math.round(width * 0.1),
            top: Math.round(height * 0.6),
            right: Math.round(width * 0.9),
            bottom: Math.round(height * 0.6) + 80,
          },
          fontSize: 32,
        }),
      ),
    log,
  );
}

/** Inserts a page after the current one, writes test strokes on it and turns to it. */
export async function newPageTest(log: Log): Promise<void> {
  try {
    const ctx = await noteContext();
    const target = ctx.page + 1;
    let t = Date.now();
    await insertPageAfter(ctx.filePath, target, ctx.page);
    log(`insertNotePage(${target}): ${Date.now() - t}ms`);
    t = Date.now();
    await writeStrokes(
      testStrokes(ctx.size.width, ctx.size.height / 3, 10, 30),
      target,
      await currentPen(),
    );
    log(`write on new page: ${Date.now() - t}ms`);
    await jumpToPageIn(ctx.filePath, target);
    log(
      '→ Did the view turn to a new page with zig-zags on it? (template copied from this page)',
    );
  } catch (e) {
    log(`new page: ERROR ${(e as Error).message}`);
  }
}

/**
 * Does the device keep Element.userData? Writes one marked stroke, reads it back and deletes it
 * (only if the marker came back, or by its position otherwise). Reports what it found.
 */
export async function markerTest(log: Log): Promise<void> {
  try {
    const ctx = await noteContext();
    const pen = await currentPen();
    const marker = `spellink-test:${Date.now().toString(36)}-${Math.random()
      .toString(36)
      .slice(2, 10)}`;
    const x = Math.round(ctx.size.width * 0.2);
    const y = Math.round(ctx.size.height * 0.5);
    const points = Array.from({length: 7}, (_, i) => ({x: x + i * 6, y}));
    await writeStrokes(
      [{p: points.flatMap(p => [p.x, p.y]), w: points.map(() => 2500)}],
      ctx.page,
      pen,
      undefined,
      marker,
    );
    const els = await readPage(ctx.filePath, ctx.page);
    const ours = els.filter(e => e.userData === marker);
    log(
      `strokes carrying the marker after saving: ${ours.length} (${
        ours.length > 0 ? 'MARKER KEPT' : 'marker lost'
      })`,
    );
    const nums = ours.map(e => e.numInPage);
    releaseElements(els);
    if (nums.length > 0) {
      await callChecked('deletePageElements', () =>
        PluginCommAPI.deletePageElements(nums, ctx.page),
      );
      log(`removed ${nums.length} test stroke(s) by marker`);
    } else {
      log('→ the test stroke stays on the page: erase it by hand');
    }
  } catch (e) {
    log(`marker test: ERROR ${(e as Error).message}`);
  }
}

/**
 * Can an element's page number be read straight from the in-memory cache after inserting it,
 * without saving the note and listing the page (which took 2-3 s on a big note)? Writes one
 * marked stroke, asks the cache for it, compares with the saved page, then deletes it.
 */
export async function cacheNumberTest(log: Log): Promise<void> {
  try {
    const ctx = await noteContext();
    const pen = await currentPen();
    const marker = `spellink-test:${Date.now().toString(36)}-${Math.random()
      .toString(36)
      .slice(2, 10)}`;
    const x = Math.round(ctx.size.width * 0.2);
    const y = Math.round(ctx.size.height * 0.5);
    const points = Array.from({length: 7}, (_, i) => ({x: x + i * 6, y}));
    const [uuid] = await writeStrokes(
      [{p: points.flatMap(p => [p.x, p.y]), w: points.map(() => 2500)}],
      ctx.page,
      pen,
      undefined,
      marker,
      true, // keep it in the cache so it can be read back
    );
    const cached = await call<{numInPage?: number; userData?: string} | null>(
      'getCacheElement',
      () => PluginCommAPI.getCacheElement(uuid),
    ).catch(() => null);
    log(
      `cache: numInPage=${cached?.numInPage} userData=${JSON.stringify(
        cached?.userData,
      )} (cache entry ${cached ? 'found' : 'missing'})`,
    );
    const t0 = Date.now();
    const els = await readPage(ctx.filePath, ctx.page);
    const saved = els.filter(e => e.userData === marker).map(e => e.numInPage);
    log(
      `saved page: numInPage=${JSON.stringify(saved)} (save+read took ${
        Date.now() - t0
      }ms of ${els.length} elements)`,
    );
    log(
      cached?.numInPage !== undefined &&
        saved.length === 1 &&
        cached.numInPage === saved[0]
        ? '→ the cache number matches: the wave could be removed without saving or listing the page'
        : '→ the cache number cannot be used',
    );
    releaseElements(els);
    PluginCommAPI.recycleElement(uuid);
    await deleteElementsAt(ctx.filePath, ctx.page, saved);
    log(`removed ${saved.length} test stroke(s)`);
  } catch (e) {
    log(`cache test: ERROR ${(e as Error).message}`);
  }
}

export async function reloadFile(log: Log): Promise<void> {
  await safe('reloadFile', () => PluginCommAPI.reloadFile(), log);
}

/** Full pipeline minus OCR: a fixed question answered into the middle of the page. */
export async function askWithoutLasso(log: Log): Promise<void> {
  await withPermissions(log);
  const ctx = await noteContext().catch(() => null);
  if (!ctx) {
    return log('ask: no note open');
  }
  const layout = answerLayout(ctx.size, {
    left: 0,
    bottom: ctx.size.height * 0.35,
  });
  const flow = new AskFlow(s => log(`ask: ${s.message.replace(/\n/g, ' ')}`));
  const t = Date.now();
  await flow.runWithQuestion(
    '富士山の高さと、日本一高い理由を二文で教えてください。',
    layout,
    ctx,
  );
  log(`ask total ${Date.now() - t}ms`);
}

/** Sends the collected log to the bridge (reports.jsonl on the computer). */
export async function sendReport(lines: string[], log: Log): Promise<void> {
  await withPermissions(log);
  try {
    const conn = await connectFirst((await loadBridges()).endpoints);
    conn.send({type: 'report', data: {log: lines, trace: dumpTrace()}});
    setTimeout(() => conn.close(), 500);
    log(`report sent to ${conn.name}`);
  } catch (e) {
    log(`report: ERROR ${(e as Error).message}`);
  }
}
