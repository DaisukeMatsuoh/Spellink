// "Thinking" animation shown where the answer will start: a wavy line drawn slowly, one piece
// per tick, across the answer width; at the end of the line it is wiped and starts again. It
// runs until the first answer line is ready, then everything it drew is removed.
//
// Safety: a stroke is deleted only if it is provably ours — it sits after the elements that were
// on the page when we started, AND its first point and point count match a piece we drew.
// Anything else (the user's writing) is never touched, even if element numbers are not what
// we expect.
import {PluginCommAPI, PluginFileAPI, type Element} from 'sn-plugin-lib';
import {matchesSignature, type Signature} from './freeSpace';
import {call, writeStrokes, type PenStyle} from './sdk';

const INTERVAL_MS = 450;
/** Length of one drawn piece, in character widths. */
const PIECE_CHARS = 1.1;
const MAX_CLEAR_ROUNDS = 4;

/** One piece of a sine wave from x0 to x1, centred on `y`. */
export function wavePiece(
  x0: number,
  x1: number,
  y: number,
  amp: number,
  wavelength: number,
  /** Shifts the wave's phase; each pass over the line uses a new one so its strokes differ. */
  pass = 0,
) {
  const p: number[] = [];
  const w: number[] = [];
  const n = 16;
  for (let i = 0; i <= n; i++) {
    const x = x0 + ((x1 - x0) * i) / n;
    p.push(
      Math.round(x),
      Math.round(
        y +
          amp * Math.sin((2 * Math.PI * x) / wavelength + pass * (Math.PI / 2)),
      ),
    );
    // Pressure swells in the middle of a piece, like a pen drawn at an even speed.
    w.push(Math.round(2600 + 1200 * Math.sin((Math.PI * i) / n)));
  }
  return {p, w};
}

export class InkDrops {
  private sigs: Signature[] = [];
  private cursor: number;
  private pass = 0;
  private timer: ReturnType<typeof setInterval> | null = null;
  private chain: Promise<void> = Promise.resolve();
  private stopped = false;
  /** Strokes drawn / removed, for the debug panel. */
  drawn = 0;
  removed = 0;

  constructor(
    private readonly filePath: string,
    private readonly page: number,
    private readonly x: number,
    private readonly width: number,
    private readonly baseline: number,
    private readonly size: number,
    private readonly pen: PenStyle,
    /** Number of elements on the page before we drew anything. */
    private readonly baseCount: number,
  ) {
    this.cursor = x;
  }

  start(): void {
    this.timer = setInterval(() => {
      this.chain = this.chain.then(() => this.tick()).catch(() => {});
    }, INTERVAL_MS);
  }

  /** Stops and removes everything drawn; resolves once the page is clean. */
  async stop(): Promise<void> {
    this.stopped = true;
    if (this.timer) {
      clearInterval(this.timer);
    }
    await this.chain;
    await this.clear().catch(() => {});
  }

  private async tick(): Promise<void> {
    if (this.stopped) {
      return;
    }
    const end = this.x + this.width;
    if (this.cursor >= end) {
      await this.clear();
      this.cursor = this.x;
      this.pass++;
      return;
    }
    const next = Math.min(end, this.cursor + this.size * PIECE_CHARS);
    const piece = wavePiece(
      this.cursor,
      next,
      this.baseline,
      this.size * 0.22,
      this.size * 1.6,
      this.pass,
    );
    this.sigs.push({x: piece.p[0], y: piece.p[1], n: piece.p.length / 2});
    await writeStrokes([piece], this.page, this.pen);
    this.drawn++;
    this.cursor = next;
  }

  /**
   * Removes every stroke of ours. Each round lists the page, picks the strokes that are provably
   * ours and deletes them (highest number first, so deleting one cannot shift the numbers of
   * the rest); it repeats until a round finds none, because a delete may be partial.
   */
  private async clear(): Promise<void> {
    if (this.sigs.length === 0) {
      return;
    }
    const sigs = this.sigs;
    for (let round = 0; round < MAX_CLEAR_ROUNDS; round++) {
      const nums = await this.findOurs(sigs);
      if (nums.length === 0) {
        break;
      }
      await call(
        'deletePageElements',
        PluginCommAPI.deletePageElements(
          nums.sort((a, b) => b - a),
          this.page,
        ),
      );
      this.removed += nums.length;
    }
    this.sigs = this.sigs.filter(s => !sigs.includes(s));
  }

  private async findOurs(sigs: Signature[]): Promise<number[]> {
    const elements =
      (await call<Element[] | null>(
        'getElements',
        PluginFileAPI.getElements(this.page, this.filePath),
      )) ?? [];
    const nums: number[] = [];
    try {
      for (const e of elements) {
        if (
          e.type !== 0 ||
          !e.stroke?.points ||
          e.numInPage <= this.baseCount
        ) {
          continue;
        }
        const n = await e.stroke.points.size();
        const first = n > 0 ? await e.stroke.points.get(0) : null;
        if (sigs.some(s => matchesSignature(s, first, n))) {
          nums.push(e.numInPage);
        }
      }
    } finally {
      for (const e of elements) {
        PluginCommAPI.recycleElement(e.uuid);
      }
    }
    return nums;
  }
}
