// "Ink drops" shown where the answer will start while Claude is thinking: small dots appear one
// by one, then are wiped and start again, and all of them are removed just before the answer
// is written. Only elements this module inserted are ever deleted (checked by uuid).
import {Element, PluginCommAPI, PluginFileAPI} from 'sn-plugin-lib';
import {call, writeStrokes, type PenStyle} from './sdk';

const INTERVAL_MS = 700;
const DROPS_PER_CYCLE = 4;

/** A small filled blob: a tight spiral of points, heavier in the middle. */
function dropStroke(cx: number, cy: number, r: number) {
  const p: number[] = [];
  const w: number[] = [];
  const turns = 3;
  const n = 18;
  for (let i = 0; i <= n; i++) {
    const t = i / n;
    const a = t * turns * 2 * Math.PI;
    const rr = r * (1 - t);
    p.push(
      Math.round(cx + rr * Math.cos(a)),
      Math.round(cy + rr * Math.sin(a)),
    );
    w.push(Math.round(3000 + 1000 * t));
  }
  return {p, w};
}

export class InkDrops {
  private readonly nums: number[] = [];
  private timer: ReturnType<typeof setInterval> | null = null;
  private chain: Promise<void> = Promise.resolve();
  private stopped = false;

  constructor(
    private readonly page: number,
    private readonly x: number,
    private readonly baseline: number,
    private readonly size: number,
    private readonly pen: PenStyle,
  ) {}

  start(): void {
    this.timer = setInterval(() => {
      this.chain = this.chain.then(() => this.tick()).catch(() => {});
    }, INTERVAL_MS);
  }

  /** Stops and removes every drop; resolves once the page is clean. */
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
    if (this.nums.length >= DROPS_PER_CYCLE) {
      await this.clear();
      return;
    }
    const i = this.nums.length;
    const drop = dropStroke(
      this.x + this.size * (0.3 + 0.6 * i),
      this.baseline,
      Math.max(2, this.size * 0.07),
    );
    const uuid = await writeStrokes([drop], this.page, this.pen);
    // The element just inserted is the page's last one; remember its number only if the uuid
    // proves it is ours, so a user's stroke can never be deleted by mistake.
    const last = await call<Element>(
      'getLastElement',
      PluginFileAPI.getLastElement(),
    );
    PluginCommAPI.recycleElement(last.uuid);
    if (uuid[0] && last.uuid === uuid[0]) {
      this.nums.push(last.numInPage);
    }
  }

  private async clear(): Promise<void> {
    if (this.nums.length === 0) {
      return;
    }
    const nums = this.nums.splice(0);
    await call(
      'deletePageElements',
      PluginCommAPI.deletePageElements(nums, this.page),
    );
  }
}
