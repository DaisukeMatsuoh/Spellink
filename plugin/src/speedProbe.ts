// Device measurements for "where does the time go". They answer two questions with numbers:
//  - lassoSpeedProbe: reading the strokes of a selection, sending them to the bridge, and the
//    firmware's own handwriting recognition: which is slow, which is not.
//  - writeSpeedProbe: writing the same strokes to the page in batches of different sizes.
// The strokes this writes are marked with the wave marker and removed at the end.
import {Element, PluginCommAPI} from 'sn-plugin-lib';
import {connectFirst, type BridgeConnection} from './bridgeClient';
import {loadBridges} from './bridgeConfig';
import type {Log} from './diagnostics';
import {WAVE_MARKER_PREFIX} from './freeSpace';
import {waveNumsOn} from './inkDrops';
import type {InkStroke} from './protocol';
import {
  call,
  currentPen,
  deleteElementsAt,
  noteContext,
  raw,
  readPage,
  releaseElements,
  writeStrokes,
  type WriteTiming,
} from './sdk';

const READ_CHUNK = 32;
const MAX_STROKES = 80;
const ACK_WAIT_MS = 15000;
/** Strokes and text boxes: what the firmware's recognition accepts (as in `recognizeLasso`). */
const RECOGNIZABLE = new Set([0, 500, 501, 502]);

const ms = (since: number) => Date.now() - since;

/** The points of the stroke elements as [x, y, x, y, …] lists, and how long that took. */
async function pointsOf(elements: Element[]): Promise<number[][]> {
  const strokes = elements
    .filter(e => e.type === 0 && e.stroke?.points)
    .slice(0, MAX_STROKES);
  const out: number[][] = [];
  for (let i = 0; i < strokes.length; i += READ_CHUNK) {
    const part = await Promise.all(
      strokes.slice(i, i + READ_CHUNK).map(async e => {
        const n = await raw(
          'points.size',
          () => e.stroke!.points.size(),
          'shared',
        );
        const pts =
          n > 0
            ? await raw(
                'points.getRange',
                () => e.stroke!.points.getRange(0, n),
                'shared',
              )
            : [];
        const flat: number[] = [];
        for (const p of pts) {
          if (typeof p?.x === 'number' && typeof p?.y === 'number') {
            flat.push(p.x, p.y);
          }
        }
        return flat;
      }),
    );
    out.push(...part);
  }
  return out;
}

/** Sends a probe and waits for its answer; resolves with the milliseconds it took. */
function roundTrip(
  conn: BridgeConnection,
  id: string,
  data: unknown,
): Promise<number> {
  return new Promise((resolve, reject) => {
    const started = Date.now();
    const timer = setTimeout(
      () => reject(new Error(`no answer in ${ACK_WAIT_MS} ms`)),
      ACK_WAIT_MS,
    );
    conn.onMessage = m => {
      if (m.type === 'probeAck' && m.id === id) {
        clearTimeout(timer);
        resolve(ms(started));
      }
    };
    try {
      conn.send({type: 'probe', id, data});
    } catch (e) {
      clearTimeout(timer);
      reject(e);
    }
  });
}

export async function lassoSpeedProbe(log: Log): Promise<void> {
  const ctx = await noteContext();
  let elements: Element[] = [];
  try {
    // 1. The strokes of the selection (or, with none, those of the page: then the page read is
    //    shown too, saving included).
    let t = Date.now();
    elements =
      (await call<Element[] | null>('getLassoElements', () =>
        PluginCommAPI.getLassoElements(),
      )) ?? [];
    let source = 'lasso';
    if (elements.length === 0) {
      source = 'page (no lasso selection)';
      t = Date.now();
      elements = await readPage(ctx.filePath, ctx.page);
    }
    log(`1. ${source}: ${elements.length} elements in ${ms(t)} ms`);

    // 2. Their points.
    t = Date.now();
    const strokes = await pointsOf(elements);
    const pointCount = strokes.reduce((n, s) => n + s.length / 2, 0);
    log(
      `2. points of ${strokes.length} strokes (${pointCount} points): ${ms(t)} ms`,
    );

    // 3. Turned into what would be sent.
    t = Date.now();
    const text = JSON.stringify({strokes});
    log(`3. as JSON: ${text.length} bytes, ${ms(t)} ms`);

    // 4. Sent to the bridge and acknowledged (a tiny probe first: the round trip itself).
    try {
      const {endpoints} = await loadBridges();
      const conn = await connectFirst(endpoints);
      try {
        const rtt = await roundTrip(conn, `rtt-${Date.now()}`, {});
        const upload = await roundTrip(conn, `up-${Date.now()}`, {strokes});
        log(
          `4. bridge ${conn.mode}: round trip ${rtt} ms; the strokes (${text.length} bytes) ${upload} ms`,
        );
      } finally {
        conn.close();
      }
    } catch (e) {
      log(`4. bridge: ERROR ${(e as Error).message}`);
    }

    // 5. The firmware's recognition of the same elements.
    const targets = elements.filter(e => RECOGNIZABLE.has(e.type));
    if (targets.length > 0) {
      t = Date.now();
      try {
        const found = await call<string | null>('recognizeElements', () =>
          PluginCommAPI.recognizeElements(targets, ctx.size),
        );
        log(
          `5. recognition on the device: ${ms(t)} ms (${(found ?? '').length} characters)`,
        );
      } catch (e) {
        log(`5. recognition: ERROR ${(e as Error).message} after ${ms(t)} ms`);
      }
    } else {
      log('5. recognition: nothing to recognise');
    }
    log(
      '→ Compare 5 with 2 + 4: that is the time a bridge-side recognition would have to beat.',
    );
  } finally {
    releaseElements(elements);
  }
}

/** Zig-zags like handwriting strokes: `count` strokes of `points` points each, in a grid. */
function strokesFor(
  width: number,
  height: number,
  count: number,
  points: number,
): InkStroke[] {
  const cols = Math.ceil(Math.sqrt(count * 4));
  const cell = Math.floor((width * 0.8) / cols);
  const out: InkStroke[] = [];
  for (let i = 0; i < count; i++) {
    const x0 = Math.round(width * 0.1 + (i % cols) * cell);
    const y0 = Math.round(height * 0.5 + Math.floor(i / cols) * cell);
    const p: number[] = [];
    const w: number[] = [];
    for (let k = 0; k < points; k++) {
      p.push(
        x0 + Math.round((k / points) * cell * 0.8),
        y0 + (k % 2 ? Math.round(cell * 0.6) : 0),
      );
      w.push(2500);
    }
    out.push({p, w});
  }
  return out;
}

const WRITE_TOTAL = 120;
const WRITE_POINTS = 20;
const BATCHES = [15, 30, 60, 120];

export async function writeSpeedProbe(log: Log): Promise<void> {
  const ctx = await noteContext();
  const pen = await currentPen();
  const marker = `${WAVE_MARKER_PREFIX}speed-${Date.now().toString(36)}`;
  const strokes = strokesFor(
    ctx.size.width,
    ctx.size.height,
    WRITE_TOTAL,
    WRITE_POINTS,
  );
  log(
    `writing ${WRITE_TOTAL} strokes × ${WRITE_POINTS} points, in batches (the answer uses 15)`,
  );
  try {
    for (const batch of BATCHES) {
      const timing: WriteTiming = {create: 0, fill: 0, insert: 0};
      const started = Date.now();
      let calls = 0;
      for (let i = 0; i < strokes.length; i += batch) {
        await writeStrokes(
          strokes.slice(i, i + batch),
          ctx.page,
          pen,
          timing,
          marker,
        );
        calls++;
      }
      const total = ms(started);
      log(
        `batch ${batch}: ${calls} calls, ${total} ms (${(total / WRITE_TOTAL).toFixed(1)} ms per stroke); ` +
          `create ${timing.create} ms, fill ${timing.fill} ms, insert ${timing.insert} ms`,
      );
    }
  } catch (e) {
    log(`write: ERROR ${(e as Error).message}`);
  } finally {
    // Back to a clean page: the strokes are found by their marker and removed.
    try {
      const els = await readPage(ctx.filePath, ctx.page);
      const nums = waveNumsOn(els);
      releaseElements(els);
      await deleteElementsAt(ctx.filePath, ctx.page, nums);
      log(
        `removed ${nums.length} test strokes (expected ${WRITE_TOTAL * BATCHES.length})`,
      );
    } catch (e) {
      log(
        `clean-up: ERROR ${(e as Error).message}: the test strokes may still be on the page (they are removed at the next question)`,
      );
    }
  }
}
