// "Thinking" animation shown where the answer will start: a wavy line drawn slowly, one piece
// per tick, across the answer width; at the end of the line it is wiped and starts again. It
// runs until the first answer line is ready, then everything it drew is removed.
//
// Every stroke carries the same unique marker in Element.userData. Removal is exactly "find the
// elements with this marker, delete them" — nothing is inferred from positions or numbers, so
// nothing else can be matched.
import {PluginCommAPI} from 'sn-plugin-lib';
import {WAVE_MARKER_PREFIX} from './freeSpace';
import {
  call,
  readPage,
  releaseElements,
  writeStrokes,
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
    await writeStrokes([piece], this.page, this.pen, undefined, this.marker);
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
    this.drawnPieces = 0;
    if (nums.length > 0) {
      await call(
        'deletePageElements',
        PluginCommAPI.deletePageElements(nums, this.page),
      );
    }
  }
}
