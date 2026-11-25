import {PluginCommAPI} from 'sn-plugin-lib';
import {
  connectFirst,
  noBridgeMessage,
  type BridgeConnection,
} from './bridgeClient';
import {AskMachine, type AskEvent, type Handlers} from './askMachine';
import {freeLimit, type Box} from './freeSpace';
import {MAX_EXTRA_PAGES} from './frames';
import {flushTrace, setTraceSink, trace} from './trace';
import {InkDrops, sweepLeftovers, waveNumsOn} from './inkDrops';
import {loadBridges} from './bridgeConfig';
import {t} from './i18n';
import type {BridgeMessage, Lang, LayoutParams, Settings} from './protocol';
import {
  call,
  currentPen,
  ensurePermissions,
  insertPageAfter,
  CallTimeoutError,
  NoteMovedError,
  lassoRect,
  noteContext,
  boxesOf,
  readPage,
  recognizeLasso,
  writeStrokesAt,
  type NoteContext,
} from './sdk';

/** What the user reads when the answer could not be completed. */
export function describeFailure(e: unknown): string {
  if (e instanceof NoteMovedError) {
    return t('noteMoved');
  }
  if (e instanceof CallTimeoutError) {
    return t('hostBusy', {api: e.api});
  }
  return t('error', {message: (e as Error).message});
}

/** The settings last read from a bridge; the wave needs them before the first connection. */
let lastSettings: Settings = {
  showProgress: false,
  lineTilt: true,
  thinkingAnimation: false,
  avoidOverwriting: false,
  busyNotice: true,
};
export function rememberSettings(s: Settings): void {
  lastSettings = s;
}
export function knownSettings(): Settings {
  return lastSettings;
}

export interface AskState {
  message: string;
  question?: string;
  answer?: string;
  finished: boolean;
  /** Ended without an answer (no text, no bridge, error…): the user must be told. */
  failed?: boolean;
  /** The user's "show progress" setting, known once the bridge has answered. */
  showProgress?: boolean;
}

type Report = (s: AskState) => void;

const LASSO_REMOVE = 2;
/** Answers are inserted this many strokes at a time (about two or three characters). */
const STROKES_PER_STEP = 15;
/** Working out where existing content is stops after this long (big pages hold thousands of strokes). */
const SCAN_BUDGET_MS = 2500;
/** Reconnection attempts in a row, without hearing from the bridge, before the answer is given up. */
const MAX_RECONNECTS = 3;

/**
 * No traffic at all from the bridge for this long means the connection is dead. The bridge sends
 * a keep-alive about every 5 s while it works, so a slow AI does not look like a dead connection.
 */
let quietMs = 15000;

/** Tests shorten the wait. */
export function setQuietMsForTests(ms: number): void {
  quietMs = ms;
}

/** Answers follow the language the question was written in. */
function questionLang(q: string): Lang {
  return /[぀-ヿ㐀-鿿]/.test(q) ? 'ja' : 'en';
}

/**
 * Places the answer below the selection, wrapping within the page width (so it never needs
 * horizontal scrolling on the device), and lets it continue on new pages from the top margin.
 */
export function answerLayout(
  size: NoteContext['size'],
  rect: {left: number; bottom: number},
): LayoutParams {
  const margin = Math.round(size.width * 0.06);
  const charSize = Math.max(24, Math.min(96, Math.round(size.width / 30)));
  let x = Math.max(margin, Math.round(rect.left));
  if (size.width - margin - x < size.width * 0.45) {
    x = margin;
  }
  return {
    x,
    y: Math.min(size.height, Math.round(rect.bottom + charSize * 0.6)),
    width: size.width - margin - x,
    pageHeight: size.height,
    bottomMargin: margin,
    charSize,
    maxPages: 1 + MAX_EXTRA_PAGES,
    continueTop: margin,
  };
}

type LineFrame = Extract<BridgeMessage, {type: 'line'}>;
type DoneFrame = Extract<BridgeMessage, {type: 'done'}>;
type StatusFrame = Extract<BridgeMessage, {type: 'status'}>;

/** How a question ended. Set once, by the action that ended it. */
type Outcome =
  | {kind: 'ok'}
  | {kind: 'cancel'}
  | {kind: 'notext'}
  /** `message` is what the user reads, already worded; otherwise the error is described. */
  | {kind: 'fail'; error?: Error; message?: string};

/** Thrown at a checkpoint once the machine has reached an end state: stop quietly. */
const STOP = new Error('stopped');

const toError = (v: unknown): Error =>
  v instanceof Error ? v : new Error(typeof v === 'string' ? v : 'failed');

/**
 * The whole "Ask AI" gesture: read the lasso, recognise it on-device, ask the bridge, and write
 * each answer line into the note as soon as it arrives.
 *
 * What happens when is decided by the table in docs/spec/ask-flow.md (src/askMachine.ts runs
 * it). The code below does the work — each step reports what happened as an event, and each
 * action in the table is one handler here.
 */
export class AskFlow {
  private readonly machine: AskMachine;
  private outcome: Outcome | null = null;
  private update: (patch: Partial<AskState>) => void = () => {};
  private pressedWhileBusy = false;

  private conn: BridgeConnection | null = null;
  private id = '';
  private traceTimer: ReturnType<typeof setInterval> | null = null;
  /** Set when the connection drops after it was made but before the question was sent. */
  private earlyClose: string | null = null;
  private settle: {resolve: () => void; reject: (e: Error) => void} | null =
    null;

  private question = '';
  private note!: NoteContext;
  private layout!: LayoutParams;
  private pen!: Awaited<ReturnType<typeof currentPen>>;
  private wave: {stop: () => Promise<void>} = {stop: async () => {}};

  private writes: Promise<void> = Promise.resolve();
  private stopWrites = false;
  private written = 0;
  private answerText = '';
  private pagesReady = 1;

  private nextSeq = 0;
  private askSent = false;
  /** Reconnection attempts in a row without hearing from the bridge. */
  private streak = 0;
  private recovering = false;
  private quiet: ReturnType<typeof setTimeout> | null = null;
  private doneFrame: DoneFrame | null = null;

  constructor(private readonly report: Report) {
    this.machine = new AskMachine(this.handlers(), (k, d) => trace(k, {d}));
  }

  // ------------------------------------------------------------------ entry points

  /** The user pressed the button while this question is running; true = tell them (A2). */
  press(): boolean {
    this.pressedWhileBusy = false;
    this.machine.dispatch('Pressed');
    return this.pressedWhileBusy;
  }

  /** Stops the answer. Always ends the running question, whatever state it is in. */
  cancel(): void {
    this.machine.dispatch('CancelRequested');
  }

  /** What a panel shows; the last state also says whether the question is over. */
  async run(): Promise<void> {
    this.begin({message: t('permissions'), finished: false});
    try {
      this.machine.dispatch('Pressed'); // Idle → CheckingPermission
      const missing = await ensurePermissions();
      this.checkpoint();
      if (missing.length > 0) {
        this.machine.dispatch('Denied', {
          message: t('permissionDenied', {list: missing.join(', ')}),
        });
        return;
      }
      this.machine.dispatch('Granted'); // CheckingPermission → Recognizing
      this.update({message: t('recognizing')});
      const ctx = await noteContext();
      this.checkpoint();
      const rect = await lassoRect();
      this.checkpoint();
      const question = (await recognizeLasso(ctx.size)).trim();
      this.checkpoint();
      if (!question) {
        this.machine.dispatch('NoText');
        return;
      }
      this.update({question});
      this.machine.dispatch('Recognized'); // Recognizing → Preparing
      await call('setLassoBoxState', () =>
        PluginCommAPI.setLassoBoxState(LASSO_REMOVE),
      );
      this.checkpoint();

      // The selection is where the answer starts and what it must not run over. Handing it over
      // makes the page be read (a save and a read of the whole page); that is the user's choice.
      const selection = knownSettings().avoidOverwriting
        ? {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
          }
        : undefined;
      await this.answer(question, answerLayout(ctx.size, rect), ctx, selection);
    } catch (e) {
      this.fault(e);
    } finally {
      this.conclude();
    }
  }

  /** Same as `run` but with a known question and position (used by diagnostics). Starts at Preparing. */
  async runWithQuestion(
    question: string,
    layout: LayoutParams,
    ctx: NoteContext,
    selection?: Box,
  ): Promise<void> {
    this.begin({message: '', question, finished: false});
    this.machine.state = 'Preparing';
    try {
      await this.answer(question, layout, ctx, selection);
    } catch (e) {
      this.fault(e);
    } finally {
      this.conclude();
    }
  }

  private begin(state: AskState): void {
    this.update = patch => {
      Object.assign(state, patch);
      this.report({...state});
    };
    this.update({});
  }

  // ------------------------------------------------------------------ events from the work

  private terminal(): boolean {
    return this.machine.terminal;
  }

  /** Between steps: once the machine has reached an end state, nothing more is done. */
  private checkpoint(): void {
    if (this.terminal()) {
      throw STOP;
    }
  }

  /** An error from a step becomes the event the table has a cell for. */
  private fault(e: unknown): void {
    if (e === STOP) {
      return;
    }
    const event: AskEvent =
      e instanceof NoteMovedError
        ? 'NoteChanged'
        : e instanceof CallTimeoutError
          ? 'NoAnswer'
          : 'Refused';
    this.machine.dispatch(event, e);
  }

  /** Reports how it ended. By now the machine must be in an end state; if not, that is a bug. */
  private conclude(): void {
    if (!this.terminal()) {
      trace('ask.notTerminal', {d: {state: this.machine.state}});
      this.outcome ??= {kind: 'fail', error: new Error('did not finish')};
    }
    const o = this.outcome ?? {kind: 'ok'};
    switch (o.kind) {
      case 'ok':
        this.update({finished: true});
        break;
      case 'notext':
        this.update({message: t('noText'), finished: true, failed: true});
        break;
      case 'cancel':
        this.update({
          message: describeFailure(new Error('cancelled')),
          finished: true,
          failed: true,
        });
        break;
      default:
        this.update({
          message: o.message ?? describeFailure(o.error),
          finished: true,
          failed: true,
        });
    }
  }

  // ------------------------------------------------------------------ the actions of the table

  private handlers(): Handlers {
    const fail = (arg: unknown): void => {
      // Either an Error, or {message} already worded for the user (e.g. "no bridge reachable").
      const worded =
        !(arg instanceof Error) &&
        typeof (arg as {message?: unknown} | null)?.message === 'string'
          ? (arg as {message: string}).message
          : undefined;
      const error = worded === undefined ? toError(arg) : undefined;
      this.outcome ??= {kind: 'fail', error, message: worded};
      this.stopQuiet();
      this.settle?.reject(error ?? new Error(worded));
    };
    return {
      A1: () => this.update({message: t('permissions')}),
      A2: () => {
        this.pressedWhileBusy = true;
        trace('ask.pressedWhileBusy');
      },
      A3: () => {
        this.outcome ??= {kind: 'cancel'};
        this.stopWrites = true;
        this.cancelBridge();
        this.conn?.close();
        this.stopQuiet();
        this.settle?.reject(new Error('cancelled'));
      },
      A4: arg => fail(arg),
      A5: () => {
        this.outcome ??= {kind: 'notext'};
      },
      A6: arg =>
        trace('ask.softfail', {err: (arg as Error | undefined)?.message}),
      A7: arg => {
        this.stopWrites = true;
        this.cancelBridge();
        fail(new Error(`${t('badFrame')} (${String(arg)})`));
      },
      A8: arg => {
        this.doneFrame = arg as DoneFrame;
        this.stopQuiet();
        this.writes.then(
          () => this.machine.dispatch('Drained'),
          e => this.fault(e),
        );
      },
      A9: arg => void this.recover(String(arg)),
      A9b: arg => void this.recover(String(arg)),
      A10: arg => trace('ask.late', {d: describeEvent(arg)}),
      A11: arg => trace('ask.duplicate', {d: describeEvent(arg)}),
      A12: arg => {
        this.stopWrites = true;
        this.cancelBridge();
        fail(arg);
      },
      A13: () => {},
      A15: arg => this.writeLine(arg as LineFrame),
      A16: () => {},
      A17: () => {
        this.outcome ??= {kind: 'ok'};
        const notes = [t('done')];
        if (this.doneFrame?.truncated) {
          notes.push(t('truncated'));
        }
        this.update({message: notes.join('\n'), finished: true});
        this.settle?.resolve();
      },
      A18: arg => {
        if ((arg as StatusFrame).stage === 'thinking') {
          this.update({message: t('thinking')});
        }
      },
    };
  }

  /** Tells the bridge to stop, if it was ever asked. */
  private cancelBridge(): void {
    if (this.conn && this.id && this.askSent) {
      try {
        this.conn.send({type: 'cancel', id: this.id});
      } catch {
        // The connection is already gone; nothing to tell the bridge.
      }
    }
  }

  // ------------------------------------------------------------------ Preparing: getting ready

  private async answer(
    question: string,
    layoutIn: LayoutParams,
    ctxIn: NoteContext,
    selection?: Box,
  ): Promise<void> {
    this.question = question;
    this.layout = layoutIn;
    this.note = ctxIn;
    this.pen = await currentPen();
    this.checkpoint();
    const pageBottom = this.layout.pageHeight - this.layout.bottomMargin;

    // If there is plainly no room under the selection, start on a fresh page right away.
    const obviouslyFull = this.layout.y + this.layout.charSize > pageBottom;
    if (obviouslyFull) {
      await this.newPage();
      this.checkpoint();
    }
    // Read the page (saved first, so the user's latest writing is in it) before anything of ours
    // is drawn; the boxes are worked out while we connect.
    const snapshot =
      selection && !obviouslyFull
        ? await readPage(this.note.filePath, this.note.page).catch(e =>
            this.softFail(e, null),
          )
        : null;
    this.checkpoint();
    // Wave strokes an earlier question left on this page (docs/spec/wave.md): taken from the same
    // read, before the scan below releases the elements.
    const stale = snapshot ? waveNumsOn(snapshot) : null;
    trace('ask.start', {
      d: {page: this.note.page, y: this.layout.y, full: obviouslyFull},
    });
    if (stale) {
      await sweepLeftovers(this.note.filePath, this.note.page, stale);
      this.checkpoint();
    }
    this.wave = this.startWave();
    const scan = snapshot
      ? boxesOf(snapshot, SCAN_BUDGET_MS).catch(e =>
          this.softFail(e, [] as Box[]),
        )
      : Promise.resolve([] as Box[]);

    try {
      this.update({message: t('connecting')});
      const {endpoints} = await loadBridges();
      this.checkpoint();
      try {
        this.conn = await connectFirst(endpoints);
      } catch (e) {
        this.machine.dispatch('NoBridge', {message: noBridgeMessage(e)});
        return;
      }
      if (this.terminal()) {
        return;
      }
      this.update({showProgress: this.conn.settings.showProgress});
      this.earlyClose = null;
      this.conn.onClose = reason => {
        this.earlyClose = reason; // the question has not been sent yet: converse() starts over
      };
      rememberSettings(this.conn.settings);
      // Everything from here on is also reported to the bridge, so a failure on the device can
      // be read afterwards on the computer.
      const connection = this.conn;
      setTraceSink(events =>
        connection.send({type: 'report', data: {trace: events}}),
      );
      this.traceTimer = setInterval(flushTrace, 1500);
      trace('ask.connected', {d: {url: connection.url, mode: connection.mode}});
      if (selection && !obviouslyFull) {
        const limit = freeLimit(
          await scan,
          selection,
          {left: this.layout.x, right: this.layout.x + this.layout.width},
          this.layout.y,
          pageBottom,
          this.layout.charSize * 0.4,
        );
        this.checkpoint();
        if (limit - this.layout.y < this.layout.charSize) {
          // Existing writing right below the selection: move the wave and the answer to a new page.
          await this.wave.stop();
          await this.newPage();
          this.checkpoint();
          this.wave = this.startWave();
        } else {
          this.layout = {...this.layout, startLimit: Math.round(limit)};
        }
      }
      await this.converse();
    } catch (e) {
      trace('ask.failed', {err: (e as Error).message});
      throw e;
    } finally {
      await this.wave.stop();
      trace('ask.end');
      flushTrace();
      if (this.traceTimer) {
        clearInterval(this.traceTimer);
      }
      setTraceSink(null);
      this.conn?.close();
    }
  }

  /**
   * Reading the page is nice to have: a failure is an event the table handles (SoftFailed: go on
   * without it). A note app that does not answer at all is not soft: it ends the answer.
   */
  private softFail<T>(e: unknown, fallback: T): T {
    if (e instanceof CallTimeoutError) {
      throw e;
    }
    this.machine.dispatch('SoftFailed', e);
    return fallback;
  }

  private async newPage(): Promise<void> {
    await insertPageAfter(
      this.note.filePath,
      this.note.page + 1,
      this.note.page,
    );
    this.note = {...this.note, page: this.note.page + 1};
    this.layout = {
      ...this.layout,
      y: this.layout.continueTop ?? this.layout.bottomMargin,
    };
  }

  /** A wavy line drawn where the answer will appear until its first line arrives. */
  private startWave(): {stop: () => Promise<void>} {
    // The settings come from the bridge, which we are not connected to yet: use the ones seen last.
    if (this.terminal() || !knownSettings().thinkingAnimation) {
      return {stop: async () => {}};
    }
    const drops = new InkDrops(
      this.note.filePath,
      this.note.page,
      this.layout.x,
      this.layout.width,
      Math.round(this.layout.y + this.layout.charSize * 0.8),
      this.layout.charSize,
      this.pen,
    );
    drops.start();
    let stopped: Promise<void> | null = null;
    return {stop: () => (stopped ??= drops.stop())};
  }

  // ------------------------------------------------------------------ Streaming: the answer

  private newId(): string {
    return `${Date.now().toString(36)}-${Math.random()
      .toString(36)
      .slice(2, 8)}`;
  }

  private sendAsk(c: BridgeConnection): void {
    c.send({
      type: 'ask',
      id: this.id,
      question: this.question,
      note: this.note.filePath,
      lang: questionLang(this.question),
      layout: this.layout,
    });
    this.askSent = true;
  }

  /**
   * Sends the question and writes lines strictly in order while more keep streaming in. Resolves
   * when the machine completes (A17), rejects when it fails or is cancelled.
   */
  private converse(): Promise<void> {
    const conn = this.conn!;
    this.id = this.newId();
    this.writes = Promise.resolve();
    this.nextSeq = 0;
    this.askSent = false;
    this.streak = 0;
    return new Promise<void>((resolve, reject) => {
      this.settle = {resolve, reject};
      this.attach(conn);
      if (this.earlyClose) {
        // It dropped between connecting and asking: connect again and ask (nothing was sent yet).
        this.machine.dispatch('Closed', this.earlyClose);
        return;
      }
      try {
        this.sendAsk(conn);
      } catch (e) {
        this.machine.dispatch('Closed', (e as Error).message);
        return;
      }
      this.machine.dispatch('Asked'); // Preparing → Streaming
      this.armQuiet();
    }).finally(() => {
      this.stopQuiet();
      return this.wave.stop();
    });
  }

  private attach(c: BridgeConnection): void {
    c.onClose = reason => {
      // A close of a connection we already replaced is not news.
      if (c === this.conn) {
        this.machine.dispatch('Closed', reason);
      }
    };
    c.onBad = reason => this.machine.dispatch('BadFrame', reason);
    c.onMessage = m => this.receive(m);
  }

  /** Every frame comes in here: shared bookkeeping, then the event the table has a cell for. */
  private receive(m: BridgeMessage): void {
    // Any frame, a keep-alive included, proves the connection works.
    this.streak = 0;
    this.armQuiet();
    if ('id' in m && m.id !== this.id) {
      return;
    }
    // A resumed connection replays from the number we asked for; never handle a frame twice.
    if ('seq' in m && typeof m.seq === 'number') {
      if (m.seq < this.nextSeq) {
        this.machine.dispatch(
          'DONE' === m.type.toUpperCase() ? 'DoneFrame' : 'Late',
          m,
        );
        return;
      }
      this.nextSeq = m.seq + 1;
    }
    switch (m.type) {
      case 'status':
        this.machine.dispatch('Status', m);
        break;
      case 'line':
        this.machine.dispatch('Line', m);
        break;
      case 'done':
        this.machine.dispatch('DoneFrame', m);
        break;
      case 'error':
        this.machine.dispatch('ErrorFrame', new Error(m.message));
        break;
    }
  }

  // No traffic at all from the bridge for quietMs means the connection is dead although the
  // socket looks open (the bridge sends keep-alives while it works). Only watched while Streaming.
  private armQuiet(): void {
    this.stopQuiet();
    if (this.machine.state === 'Streaming') {
      this.quiet = setTimeout(() => {
        trace('ask.quiet', {d: {ms: quietMs}});
        this.machine.dispatch('Quiet', 'no traffic');
      }, quietMs);
    }
  }

  private stopQuiet(): void {
    if (this.quiet) {
      clearTimeout(this.quiet);
      this.quiet = null;
    }
  }

  /**
   * The connection dropped before the answer was complete (state Reconnecting). Up to MAX_RECONNECTS
   * attempts in a row, each waiting at most for the connection to be made; hearing from the bridge
   * again resets the count. The bridge keeps numbered frames, so we ask only for the ones after
   * the last we handled (ADR 0013) — or, if it never got the question, ask again.
   */
  private async recover(reason: string): Promise<void> {
    if (this.recovering) {
      return; // the loop below is already at it
    }
    this.recovering = true;
    this.stopQuiet();
    const dead = this.conn;
    if (dead) {
      dead.onClose = () => {};
      dead.onMessage = () => {};
      dead.onBad = () => {};
      dead.close();
    }
    try {
      while (this.machine.state === 'Reconnecting') {
        this.streak++;
        this.update({
          message: t('reconnecting', {n: this.streak, max: MAX_RECONNECTS}),
        });
        trace('ask.resume', {
          d: {
            n: this.streak,
            next: this.nextSeq,
            reason,
            askSent: this.askSent,
          },
        });
        try {
          const {endpoints} = await loadBridges();
          const c = await connectFirst(endpoints);
          if (this.terminal()) {
            c.close();
            return;
          }
          this.conn = c;
          setTraceSink(events =>
            c.send({type: 'report', data: {trace: events}}),
          );
          this.attach(c);
          if (this.askSent) {
            c.send({type: 'resume', id: this.id, next: this.nextSeq});
          } else {
            this.sendAsk(c);
          }
          this.machine.dispatch('Resumed'); // Reconnecting → Streaming
          this.armQuiet();
          return;
        } catch {
          this.machine.dispatch(
            'ResumeFailed',
            reason,
            this.streak < MAX_RECONNECTS ? 'more' : 'spent',
          );
        }
      }
    } finally {
      this.recovering = false;
    }
  }

  // ------------------------------------------------------------------ writing the lines

  /** A15: queue one line; the first line past the bottom inserts the next page and turns to it. */
  private writeLine(m: LineFrame): void {
    const strokes = m.strokes ?? []; // a line may have no glyphs at all
    this.writes = this.writes
      .then(async () => {
        await this.wave.stop();
        if (this.stopWrites || this.terminal()) {
          return;
        }
        while (this.pagesReady <= m.page) {
          await insertPageAfter(
            this.note.filePath,
            this.note.page + this.pagesReady,
            this.note.page,
          );
          this.pagesReady++;
        }
        // A few strokes at a time, so the answer is seen being written, not appearing whole.
        for (let i = 0; i < strokes.length; i += STROKES_PER_STEP) {
          if (this.stopWrites || this.terminal()) {
            return;
          }
          await writeStrokesAt(
            this.note.filePath,
            this.note.page + m.page,
            strokes.slice(i, i + STROKES_PER_STEP),
            this.pen,
          );
        }
        this.written++;
        this.answerText += m.text;
        this.update({
          message: t('writing', {lines: this.written}),
          answer: this.answerText,
        });
      })
      .catch(e => this.fault(e));
  }
}

/** What goes in the record about an event that was ignored. */
function describeEvent(arg: unknown): unknown {
  const a = arg as {event?: string; state?: string} | undefined;
  return a?.event ? {event: a.event, state: a.state} : undefined;
}
