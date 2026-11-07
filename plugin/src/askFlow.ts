import {PluginCommAPI} from 'sn-plugin-lib';
import {connectFirst, type BridgeConnection} from './bridgeClient';
import {freeLimit, type Box} from './freeSpace';
import {MAX_EXTRA_PAGES} from './frames';
import {flushTrace, setTraceSink, trace} from './trace';
import {InkDrops, sweepLeftovers, waveNumsOn} from './inkDrops';
import {loadBridges} from './bridgeConfig';
import {t} from './i18n';
import type {
  BridgeMessage,
  InkStroke,
  Lang,
  LayoutParams,
  Settings,
} from './protocol';
import {
  call,
  currentPen,
  ensurePermissions,
  insertPageAfter,
  jumpToPageIn,
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
/**
 * For work that is nice to have (reading the page for free space): a failure is recorded and the
 * answer goes on without it, but a note app that has stopped answering ends the answer, because
 * everything after would stall too.
 */
function unlessStalled<T>(what: string, fallback: T) {
  return (e: unknown): T => {
    if (e instanceof CallTimeoutError) {
      throw e;
    }
    trace(`${what}.failed`, {err: (e as Error).message});
    return fallback;
  };
}

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

/**
 * The whole "Ask Claude" gesture: read the lasso, recognise it on-device, ask the bridge, and
 * write each answer line into the note as soon as it arrives.
 */
export class AskFlow {
  private conn: BridgeConnection | null = null;
  private id = '';
  private traceTimer: ReturnType<typeof setInterval> | null = null;
  private cancelled = false;
  /** Set when the connection drops after it was made but before the question was sent. */
  private earlyClose: string | null = null;

  constructor(private readonly report: Report) {}

  /** Stops the answer. Always ends the running question, whatever state the connection is in. */
  cancel(): void {
    this.cancelled = true;
    if (this.conn && this.id) {
      try {
        this.conn.send({type: 'cancel', id: this.id});
      } catch {
        // The connection is already gone; nothing to tell the bridge.
      }
    }
    this.conn?.close();
    this.endQuestion?.(new Error('cancelled'));
  }

  /** Settles the running question (set while one is waiting for the bridge). */
  private endQuestion: ((e: Error) => void) | null = null;

  async run(): Promise<void> {
    const state: AskState = {message: t('permissions'), finished: false};
    const update = (patch: Partial<AskState>) => {
      Object.assign(state, patch);
      this.report({...state});
    };
    try {
      update({});
      const missing = await ensurePermissions();
      if (missing.length > 0) {
        return update({
          message: t('permissionDenied', {list: missing.join(', ')}),
          finished: true,
          failed: true,
        });
      }

      update({message: t('recognizing')});
      const ctx = await noteContext();
      const rect = await lassoRect();
      const question = (await recognizeLasso(ctx.size)).trim();
      if (!question) {
        return update({message: t('noText'), finished: true, failed: true});
      }
      update({question});
      await call('setLassoBoxState', () =>
        PluginCommAPI.setLassoBoxState(LASSO_REMOVE),
      );

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
      await this.answer(
        question,
        answerLayout(ctx.size, rect),
        ctx,
        update,
        selection,
      );
    } catch (e) {
      update({
        message: describeFailure(e),
        finished: true,
        failed: true,
      });
    }
  }

  /** Same as `run` but with a known question and position (used by diagnostics). */
  async runWithQuestion(
    question: string,
    layout: LayoutParams,
    ctx: NoteContext,
    selection?: Box,
  ): Promise<void> {
    const state: AskState = {message: '', question, finished: false};
    const update = (patch: Partial<AskState>) => {
      Object.assign(state, patch);
      this.report({...state});
    };
    try {
      await this.answer(question, layout, ctx, update, selection);
    } catch (e) {
      update({
        message: describeFailure(e),
        finished: true,
        failed: true,
      });
    }
  }

  private async answer(
    question: string,
    layout: LayoutParams,
    ctx: NoteContext,
    update: (patch: Partial<AskState>) => void,
    selection?: Box,
  ): Promise<void> {
    const pen = await currentPen();
    const pageBottom = layout.pageHeight - layout.bottomMargin;
    const newPage = async () => {
      await insertPageAfter(ctx.filePath, ctx.page + 1, ctx.page);
      await jumpToPageIn(ctx.filePath, ctx.page + 1);
      ctx = {...ctx, page: ctx.page + 1};
      layout = {...layout, y: layout.continueTop ?? layout.bottomMargin};
    };

    // If there is plainly no room under the selection, start on a fresh page right away.
    const obviouslyFull = layout.y + layout.charSize > pageBottom;
    if (obviouslyFull) {
      await newPage();
    }
    // Read the page (saved first, so the user's latest writing is in it) before anything of ours
    // is drawn; the boxes are worked out while we connect.
    const snapshot =
      selection && !obviouslyFull
        ? await readPage(ctx.filePath, ctx.page).catch(
            unlessStalled('readPage', null),
          )
        : null;
    // Wave strokes an earlier question left on this page (docs/spec/wave.md): taken from the same
    // read, before the scan below releases the elements.
    const stale = snapshot ? waveNumsOn(snapshot) : null;
    trace('ask.start', {d: {page: ctx.page, y: layout.y, full: obviouslyFull}});
    if (stale) {
      await sweepLeftovers(ctx.filePath, ctx.page, stale);
    }
    let wave = this.startWave(ctx, layout, pen);
    const scan = snapshot
      ? boxesOf(snapshot, SCAN_BUDGET_MS).catch(
          unlessStalled('boxesOf', [] as Box[]),
        )
      : Promise.resolve([] as Box[]);

    try {
      update({message: t('connecting')});
      const {endpoints} = await loadBridges();
      try {
        this.conn = await connectFirst(endpoints);
      } catch {
        return update({message: t('noBridge'), finished: true, failed: true});
      }
      update({showProgress: this.conn.settings.showProgress});
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
      if (this.cancelled) {
        return;
      }
      if (selection && !obviouslyFull) {
        const limit = freeLimit(
          await scan,
          selection,
          {left: layout.x, right: layout.x + layout.width},
          layout.y,
          pageBottom,
          layout.charSize * 0.4,
        );
        if (limit - layout.y < layout.charSize) {
          // Existing writing right below the selection: move the wave and the answer to a new page.
          await wave.stop();
          await newPage();
          wave = this.startWave(ctx, layout, pen);
        } else {
          layout = {...layout, startLimit: Math.round(limit)};
        }
      }
      await this.converse(question, layout, ctx, pen, update, wave);
    } catch (e) {
      trace('ask.failed', {err: (e as Error).message});
      throw e;
    } finally {
      await wave.stop();
      trace('ask.end');
      flushTrace();
      if (this.traceTimer) {
        clearInterval(this.traceTimer);
      }
      setTraceSink(null);
      this.conn?.close();
    }
  }

  /** A wavy line drawn where the answer will appear until its first line arrives. */
  private startWave(
    ctx: NoteContext,
    layout: LayoutParams,
    pen: Awaited<ReturnType<typeof currentPen>>,
  ): {stop: () => Promise<void>} {
    // The settings come from the bridge, which we are not connected to yet: use the ones seen last.
    if (this.cancelled || !knownSettings().thinkingAnimation) {
      return {stop: async () => {}};
    }
    const drops = new InkDrops(
      ctx.filePath,
      ctx.page,
      layout.x,
      layout.width,
      Math.round(layout.y + layout.charSize * 0.8),
      layout.charSize,
      pen,
    );
    drops.start();
    let stopped: Promise<void> | null = null;
    return {stop: () => (stopped ??= drops.stop())};
  }

  /**
   * Sends the question and writes lines strictly in order while more keep streaming in.
   * When the answer reaches the bottom of the page, a new page is inserted right after and
   * the view turns to it, so the writing carries on without scrolling.
   */
  private converse(
    question: string,
    layout: LayoutParams,
    ctx: NoteContext,
    pen: Awaited<ReturnType<typeof currentPen>>,
    update: (patch: Partial<AskState>) => void,
    wave: {stop: () => Promise<void>},
  ): Promise<void> {
    const conn = this.conn!;
    this.id = `${Date.now().toString(36)}-${Math.random()
      .toString(36)
      .slice(2, 8)}`;
    let writes = Promise.resolve();
    let written = 0;
    let answer = '';
    let pagesReady = 1;
    const clearDrops = () => wave.stop();
    const writeLine = (
      strokesIn: InkStroke[] | null,
      text: string,
      pageOffset: number,
    ) => {
      const strokes = strokesIn ?? []; // a fragment may have no glyphs at all
      writes = writes
        .then(async () => {
          await clearDrops();
          if (this.cancelled) {
            return;
          }
          // The first line past the bottom inserts the next page and turns to it.
          while (pagesReady <= pageOffset) {
            await insertPageAfter(
              ctx.filePath,
              ctx.page + pagesReady,
              ctx.page,
            );
            await jumpToPageIn(ctx.filePath, ctx.page + pagesReady);
            pagesReady++;
          }
          // A few strokes at a time, so the answer is seen being written, not appearing whole.
          for (let i = 0; i < strokes.length; i += STROKES_PER_STEP) {
            if (this.cancelled) {
              return;
            }
            await writeStrokesAt(
              ctx.filePath,
              ctx.page + pageOffset,
              strokes.slice(i, i + STROKES_PER_STEP),
              pen,
            );
          }
          written++;
          answer += text;
          update({message: t('writing', {lines: written}), answer});
        })
        .catch(e => {
          // Could not write (e.g. the note is no longer open): stop here, do not carry on half-way.
          abort(e as Error);
          this.cancel();
        });
    };

    let abort: (e: Error) => void = () => {};
    let finished = false;
    let nextSeq = 0;
    let askSent = false;
    // Reconnection attempts in a row without hearing anything (docs/spec/ask-flow.md, S5).
    let streak = 0;
    let recovering = false;
    let quiet: ReturnType<typeof setTimeout> | null = null;
    const stopQuiet = () => {
      if (quiet) {
        clearTimeout(quiet);
        quiet = null;
      }
    };
    return new Promise<void>((resolve, reject) => {
      abort = reject;
      this.endQuestion = reject;
      const fail = (e: Error) => {
        // Could not go on (bad data, the note moved…): stop, do not carry on half-way.
        abort(e);
        this.cancel();
      };
      const sendAsk = (c: BridgeConnection) => {
        c.send({
          type: 'ask',
          id: this.id,
          question,
          note: ctx.filePath,
          lang: questionLang(question),
          layout,
        });
        askSent = true;
      };
      // The connection carries traffic (frames or keep-alives) as long as it is alive. Silence
      // means it is dead although the socket looks open: treat it like a drop.
      const armQuiet = () => {
        stopQuiet();
        if (!finished && !this.cancelled) {
          quiet = setTimeout(() => {
            trace('ask.quiet', {d: {ms: quietMs}});
            const dead = this.conn;
            dead?.close();
            if (dead === this.conn) {
              void recover('no traffic');
            }
          }, quietMs);
        }
      };
      const attach = (c: BridgeConnection) => {
        c.onClose = reason => {
          // The bridge already sent everything (`done`): the rest is writing what we hold. A
          // dropped connection (e.g. a tunnel reconnecting) must not cut that short. A close
          // of a connection we already replaced is not news.
          if (c === this.conn && !finished && !this.cancelled) {
            void recover(reason);
          }
        };
        c.onBad = reason => fail(new Error(`${t('badFrame')} (${reason})`));
        c.onMessage = handle;
      };
      // The connection dropped before the answer was complete (S5). Up to MAX_RECONNECTS
      // attempts in a row, each waiting at most for the connection to be made; hearing from the
      // bridge again resets the count. The bridge keeps numbered frames, so we ask only for the
      // ones after the last we handled (ADR 0013) — or, if it never got the question, ask again.
      const recover = async (reason: string) => {
        if (recovering || finished || this.cancelled) {
          return;
        }
        recovering = true;
        stopQuiet();
        try {
          while (streak < MAX_RECONNECTS) {
            streak++;
            update({
              message: t('reconnecting', {n: streak, max: MAX_RECONNECTS}),
            });
            trace('ask.resume', {
              d: {n: streak, next: nextSeq, reason, askSent},
            });
            try {
              const {endpoints} = await loadBridges();
              const c = await connectFirst(endpoints);
              if (finished || this.cancelled) {
                c.close();
                return;
              }
              const old = this.conn;
              this.conn = c;
              old?.close();
              setTraceSink(events =>
                c.send({type: 'report', data: {trace: events}}),
              );
              attach(c);
              if (askSent) {
                c.send({type: 'resume', id: this.id, next: nextSeq});
              } else {
                sendAsk(c);
              }
              armQuiet();
              return;
            } catch {
              // this attempt failed; the loop decides whether there is another
            }
          }
          reject(new Error(reason));
        } finally {
          recovering = false;
        }
      };
      const handle = (m: BridgeMessage) => {
        // Any frame, a keep-alive included, proves the connection works.
        streak = 0;
        armQuiet();
        if ('id' in m && m.id !== this.id) {
          return;
        }
        // A resumed connection replays from the number we asked for; never handle a frame twice.
        if ('seq' in m && typeof m.seq === 'number') {
          if (m.seq < nextSeq) {
            return;
          }
          nextSeq = m.seq + 1;
        }
        switch (m.type) {
          case 'status':
            if (m.stage === 'thinking') {
              update({message: t('thinking')});
            }
            break;
          case 'line':
            writeLine(m.strokes, m.text, m.page);
            break;
          case 'done':
            finished = true;
            stopQuiet();
            writes.then(() => {
              const notes = [t('done')];
              if (m.truncated) {
                notes.push(t('truncated'));
              }
              update({message: notes.join('\n'), finished: true});
              resolve();
            }, reject);
            break;
          case 'error':
            reject(new Error(m.message));
            break;
        }
      };
      attach(conn);
      if (this.earlyClose) {
        // It dropped between connecting and asking: connect again and ask (nothing was sent yet).
        void recover(this.earlyClose);
      } else {
        try {
          sendAsk(conn);
          armQuiet();
        } catch (e) {
          void recover((e as Error).message);
        }
      }
    }).finally(() => {
      stopQuiet();
      return clearDrops();
    });
  }
}
