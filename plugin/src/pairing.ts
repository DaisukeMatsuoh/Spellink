// One pairing attempt of this device with one bridge (pairing.md, B). The machine decides what
// happens; this class does it: connect with B, show the code, wait for the owner at the bridge,
// store K, say so. Everything it needs from the outside is in `PairDeps`.
import {isSecret} from './base64url';
import type {PairSetup} from './pairConfig';
import type {PairChannel} from './pairConnect';
import type {PairFrame} from './pairFrames';
import {strokeBounds} from './pairStroke';
import type {InkStroke} from './protocol';
import {PairMachine, type PairState} from './pairingMachine';
import type {StoredBridge} from './pairStore';
import type {Log} from './stm';

export const CODE_DIGITS = 8;
/** How long a connection to the bridge may take before it counts as failed. */
export const CONNECT_TIMEOUT_MS = 8000;

export interface PairDeps {
  connect(urls: string[], b: string, timeoutMs: number): Promise<PairChannel>;
  /** Stores the pairing and returns true only if it was read back correctly. */
  save(b: StoredBridge): Promise<boolean>;
  /** `n` decimal digits, uniformly random. */
  digits(n: number): string;
  after(ms: number, f: () => void): () => void;
  every(ms: number, f: () => void): () => void;
  now(): number;
  /** The phrase to be shown in the bridge's handwriting next to the QR code, or null for none. */
  banner(): {text: string; width: number; charSize: number} | null;
  onView(v: PairView): void;
  log: Log;
}

/** What the screen shows. `note` is a key for the screen's text (with `detail` filled in). */
export interface PairView {
  state: PairState;
  note: PairNote;
  detail?: string;
  /** While pairing mode runs: the code to type at the bridge, its QR text and the seconds left. */
  code?: string;
  qr?: string;
  remainingSec?: number;
  /** The bridge's handwriting of the phrase, when it sent one. */
  banner?: {strokes: InkStroke[]; width: number; height: number};
}

export type PairNote =
  | 'idle'
  | 'connecting'
  | 'showing'
  | 'storing'
  | 'paired'
  | 'timeout'
  | 'cancelled'
  | 'storeFailed'
  | 'disconnected'
  | 'denied'
  | 'refused'
  | 'noSecret';

interface Grant {
  key: string;
  endpoints: string[];
}

export class PairingController {
  private readonly m: PairMachine;
  /** Counts attempts; whatever a finished attempt still delivers is dropped. */
  private attempt = 0;
  private ch: PairChannel | null = null;
  private code = '';
  private deadline = 0;
  private stops: Array<() => void> = [];
  private note: PairNote = 'idle';
  private detail: string | undefined;
  private banner: InkStroke[] = [];

  constructor(
    readonly setup: PairSetup,
    private readonly deps: PairDeps,
    start: PairState,
  ) {
    this.m = new PairMachine(
      {
        S1: () => this.startConnecting(),
        S2: a => this.showCode(a as PairChannel),
        S3: a => this.receiveKey(a as Grant),
        S4: () => this.finish('paired', true),
        S5: () => this.finish('storeFailed'),
        S6: () => this.finish('timeout'),
        S7: a => this.finish('disconnected', false, String(a)),
        S8: a => this.finish('denied', false, String(a)),
        S9: () => this.finish('cancelled'),
        S10: () => this.finish('refused'),
        S11: a => this.showBanner(a as InkStroke[]),
      },
      deps.log,
      start,
    );
  }

  get state(): PairState {
    return this.m.state;
  }

  view(): PairView {
    const v: PairView = {
      state: this.m.state,
      note: this.note,
      detail: this.detail,
    };
    if (this.m.state === 'U3') {
      v.code = this.code;
      v.qr = `spellink://pair?v=1&i=${this.setup.id}&c=${this.code}`;
      v.remainingSec = Math.max(
        0,
        Math.ceil((this.deadline - this.deps.now()) / 1000),
      );
      if (this.banner.length > 0) {
        v.banner = {strokes: this.banner, ...strokeBounds(this.banner)};
      }
    }
    return v;
  }

  /** The user pressed "Pair". */
  tap(): void {
    this.m.dispatch('TAP_PAIR');
  }

  /** The user pressed "Cancel" or left the screen. */
  cancel(): void {
    this.m.dispatch('CANCEL');
  }

  /** The bridge refused this device's key. */
  keyRefused(): void {
    this.m.dispatch('K_REFUSED');
  }

  private emit(): void {
    this.deps.onView(this.view());
  }

  private stopTimers(): void {
    this.stops.forEach(s => s());
    this.stops = [];
  }

  private closeChannel(): void {
    const ch = this.ch;
    this.ch = null;
    ch?.close();
  }

  // S1: connect with the first secret.
  private startConnecting(): void {
    const a = ++this.attempt;
    this.note = 'connecting';
    this.detail = undefined;
    this.emit();
    if (!this.setup.b) {
      // this package belongs to a device that has already paired: it has no first secret left
      this.m.dispatch('DISCONNECT', 'noSecret');
      return;
    }
    this.deps.connect(this.setup.urls, this.setup.b, CONNECT_TIMEOUT_MS).then(
      ch => {
        if (a !== this.attempt) {
          ch.close(); // the user gave up while it connected
          return;
        }
        this.ch = ch;
        ch.onFrame = f => this.onFrame(a, f);
        ch.onClose = r =>
          a === this.attempt && this.m.dispatch('DISCONNECT', r);
        this.m.dispatch('CONNECTED', ch);
      },
      (e: Error) =>
        a === this.attempt && this.m.dispatch('DISCONNECT', e.message),
    );
  }

  // S2: show the code and ask the bridge's owner to confirm it.
  private showCode(ch: PairChannel): void {
    const a = this.attempt;
    this.code = this.deps.digits(CODE_DIGITS);
    try {
      ch.send(this.request());
    } catch (e) {
      this.m.dispatch('DISCONNECT', (e as Error).message);
      return;
    }
    const ms = this.setup.windowSec * 1000;
    this.deadline = this.deps.now() + ms;
    this.note = 'showing';
    this.stops.push(
      this.deps.after(
        ms,
        () => a === this.attempt && this.m.dispatch('WINDOW_END'),
      ),
      this.deps.every(1000, () => a === this.attempt && this.emit()),
    );
    this.emit();
  }

  /** The first frame: who we are, the code, and — if there is a phrase — what to write in handwriting. */
  private request(): Record<string, unknown> {
    const frame: Record<string, unknown> = {
      type: 'pairRequest',
      id: this.setup.id,
      gen: this.setup.gen,
      code: this.code,
    };
    const b = this.deps.banner();
    if (b) {
      frame.banner = b.text;
      frame.layout = {
        x: 0,
        y: 0,
        width: b.width,
        pageHeight: b.charSize * 8,
        bottomMargin: 0,
        charSize: b.charSize,
      };
    }
    return frame;
  }

  // S11: the bridge's handwriting arrived (decoration; an empty one changes nothing).
  private showBanner(strokes: InkStroke[]): void {
    if (strokes.length === 0) {
      this.deps.log('pair.banner.empty');
      return;
    }
    this.banner = strokes;
    this.emit();
  }

  // S3: keep the key, then say so — and only then tell the bridge it has arrived.
  private receiveKey(g: Grant): void {
    this.stopTimers();
    this.note = 'storing';
    this.emit();
    this.deps
      .save({
        id: this.setup.id,
        gen: this.setup.gen,
        key: g.key,
        name: this.setup.name,
        urls: g.endpoints,
      })
      .then(
        ok => this.m.dispatch(ok ? 'STORE_OK' : 'STORE_FAIL'),
        () => this.m.dispatch('STORE_FAIL'),
      );
  }

  // S4–S10: the ends.
  private finish(note: PairNote, ack = false, detail?: string): void {
    this.banner = [];
    this.attempt++; // whatever this attempt still has in flight (a connection, a frame) is now stale
    this.stopTimers();
    if (ack && this.ch) {
      try {
        this.ch.send({type: 'pairAck', id: this.setup.id, gen: this.setup.gen});
      } catch {
        // the key is stored; the bridge also counts the first connection with it as the ack
      }
    }
    this.closeChannel();
    this.note = note;
    this.detail = detail;
    this.emit();
  }

  private onFrame(a: number, f: PairFrame): void {
    if (a !== this.attempt) {
      return;
    }
    switch (f.type) {
      case 'pairGrant':
        if (!isSecret(f.key)) {
          return this.m.dispatch('DISCONNECT', 'bad key');
        }
        return this.m.dispatch('GRANT', {key: f.key, endpoints: f.endpoints});
      case 'pairBanner':
        return this.m.dispatch('BANNER', f.strokes);
      case 'pairDenied':
        return this.m.dispatch('DENIED', f.reason);
      default:
        return this.m.dispatch('DENIED', `error:${f.code}`);
    }
  }
}
