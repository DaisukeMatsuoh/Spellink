// "Thinking" animation shown where the answer will start: a wavy line is drawn slowly, one
// piece per tick, across the answer width; at the end of the line it is wiped and starts again.
// It runs until the first answer line is ready, then everything it drew is removed.
//
// Only elements this module inserted are ever deleted: before each insert the page's element
// numbers are listed, and the numbers that appear afterwards are ours. (Matching by uuid does
// not work — the page reports different uuids after insertion.)
import {PluginCommAPI, PluginFileAPI} from 'sn-plugin-lib';
import {call, writeStrokes, type PenStyle} from './sdk';

const INTERVAL_MS = 450;
/** Length of one drawn piece, in characters' widths. */
const PIECE_CHARS = 1.1;

/** One piece of a sine wave from x0 to x1, centred on `y`. */
function wavePiece(
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

export class InkDrops {
  private nums: number[] = [];
  private cursor: number;
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

  private elementNums(): Promise<number[]> {
    return call<number[]>(
      'getElementNumList',
      PluginFileAPI.getElementNumList(this.filePath, this.page),
    );
  }

  private async tick(): Promise<void> {
    if (this.stopped) {
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
    const before = new Set(await this.elementNums());
    await writeStrokes([piece], this.page, this.pen);
    for (const n of await this.elementNums()) {
      if (!before.has(n)) {
        this.nums.push(n);
      }
    }
    this.cursor = next;
  }

  private async clear(): Promise<void> {
    if (this.nums.length === 0) {
      return;
    }
    const nums = this.nums;
    this.nums = [];
    await call(
      'deletePageElements',
      PluginCommAPI.deletePageElements(nums, this.page),
    );
  }
}
