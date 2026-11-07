// "Thinking" animation shown where the answer will start: a wavy line drawn slowly, one piece
// per tick, across the answer width; at the end of the line it is wiped and starts again. It
// runs until the first answer line is ready, then everything it drew is removed.
//
// Every stroke carries the same unique marker in Element.userData. Removal is exactly "find the
// elements with this marker, delete them" — nothing is inferred from positions or numbers, so
// nothing else can be matched.
import {WAVE_MARKER_PREFIX} from './freeSpace';
import type {Element} from 'sn-plugin-lib';
import {forgetLeftoversOn, recordLeftover} from './leftovers';
import {trace} from './trace';
import {
  CallTimeoutError,
  deleteElementsAt,
  isOnPage,
  readPage,
  releaseElements,
  writeStrokesAt,
  type PenStyle,
} from './sdk';

const INTERVAL_MS = 450;
/** Length of one drawn piece, in character widths. */
const PIECE_CHARS = 1.1;

/** One piece of a sine wave from x0 to x1, centred on `y`. */
export function wavePiece(
  x0: number,
  x1: number,
  y: number,
  amp: number,
  wavelength: number,
) {
  const p: number[] = [];
  const w: number[] = [];
  const n = 16;
  for (let i = 0; i <= n; i++) {
    const x = x0 + ((x1 - x0) * i) / n;
    p.push(
      Math.round(x),
      Math.round(y + amp * Math.sin((2 * Math.PI * x) / wavelength)),
    );
    // Pressure swells in the middle of a piece, like a pen drawn at an even speed.
    w.push(Math.round(2600 + 1200 * Math.sin((Math.PI * i) / n)));
  }
  return {p, w};
}

/** A marker no other run (and no user) will ever have. */
const newMarker = () =>
  `${WAVE_MARKER_PREFIX}${Date.now().toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 12)}`;

export class InkDrops {
  private readonly marker = newMarker();
  private cursor: number;
  private drawnPieces = 0;
  private timer: ReturnType<typeof setInterval> | null = null;
  private chain: Promise<void> = Promise.resolve();
  private stopped = false;

  constructor(
    private readonly filePath: string,
    private readonly page: number,
    private readonly x: number,
    private readonly width: number,
    private readonly baseline: number,
    private readonly size: number,
    private readonly pen: PenStyle,
  ) {
    this.cursor = x;
  }

  start(): void {
    this.timer = setInterval(() => {
      this.chain = this.chain
        .then(() => this.tick())
        .catch(e => this.giveUp(e, false));
    }, INTERVAL_MS);
  }

  /** Stops and removes everything drawn; resolves once the page is clean. */
  async stop(): Promise<void> {
    this.stopped = true;
    if (this.timer) {
      clearInterval(this.timer);
    }
    await this.chain;
    await this.clear().catch(e => this.giveUp(e, true));
  }

  /**
   * Whatever went wrong, the animation just ends; the answer itself does not depend on it. If
   * strokes were drawn, they are removed right now (once) — unless the failure was the removal
   * itself. What cannot be removed is remembered (leftovers.ts) and swept by the next question on
   * that page; it is also recorded, because it is the user's note that stays untidy.
   */
  private async giveUp(error: unknown, duringClear: boolean): Promise<void> {
    trace('wave.giveUp', {
      err: (error as Error)?.message,
      d: {drawnPieces: this.drawnPieces, duringClear},
    });
    this.stopped = true;
    if (this.timer) {
      clearInterval(this.timer);
    }
    if (this.drawnPieces === 0) {
      return;
    }
    if (!duringClear) {
      try {
        await this.clear();
        return;
      } catch (e) {
        error = e;
      }
    }
    recordLeftover({
      filePath: this.filePath,
      page: this.page,
      marker: this.marker,
    });
    trace('wave.leftover', {
      err: (error as Error)?.message,
      d: {page: this.page, pieces: this.drawnPieces},
    });
  }

  private async tick(): Promise<void> {
    if (this.stopped) {
      return;
    }
    // The user looking at another page: draw nothing until they are back.
    if (!(await isOnPage(this.filePath, this.page))) {
      return;
    }
    const end = this.x + this.width;
    if (this.cursor >= end) {
      await this.clear();
      this.cursor = this.x;
      return;
    }
    const next = Math.min(end, this.cursor + this.size * PIECE_CHARS);
    const piece = wavePiece(
      this.cursor,
      next,
      this.baseline,
      this.size * 0.22,
      this.size * 1.6,
    );
    await writeStrokesAt(
      this.filePath,
      this.page,
      [piece],
      this.pen,
      this.marker,
    );
    this.drawnPieces++;
    this.cursor = next;
  }

  /** Deletes every element carrying our marker. Highest number first, so that deleting one
   * cannot renumber the ones still to be deleted. */
  private async clear(): Promise<void> {
    if (this.drawnPieces === 0) {
      return;
    }
    const elements = await readPage(this.filePath, this.page);
    const nums = elements
      .filter(e => e.userData === this.marker)
      .map(e => e.numInPage)
      .sort((a, b) => b - a);
    releaseElements(elements);
    await deleteElementsAt(this.filePath, this.page, nums);
    // Only now is the page clean; a failure above leaves the count, so the leftovers are known.
    this.drawnPieces = 0;
  }
}

/**
 * Numbers of the wave strokes on a page just read, whichever question drew them. The prefix is
 * ours alone (a person's handwriting never carries it), so this is an identity, not a guess.
 */
export function waveNumsOn(elements: Element[]): number[] {
  return elements
    .filter(
      e =>
        typeof e.userData === 'string' &&
        e.userData.startsWith(WAVE_MARKER_PREFIX),
    )
    .map(e => e.numInPage)
    .sort((a, b) => b - a);
}

/**
 * Start-of-question sweep (docs/spec/wave.md): wave strokes left on this page by an earlier
 * question are removed; the memory of them is dropped once the page is clean. Only this page is
 * looked at. A failure is recorded and does not stop the question (a stuck note app does).
 */
export async function sweepLeftovers(
  filePath: string,
  page: number,
  stale: number[],
): Promise<void> {
  if (stale.length > 0) {
    try {
      await deleteElementsAt(filePath, page, stale);
      trace('wave.swept', {d: {page, count: stale.length}});
    } catch (e) {
      if (e instanceof CallTimeoutError) {
        throw e;
      }
      trace('wave.sweepFailed', {err: (e as Error).message});
      return; // still there: keep remembering
    }
  }
  forgetLeftoversOn(filePath, page); // clean now, or already gone
}
