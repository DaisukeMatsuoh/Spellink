import {PluginCommAPI} from 'sn-plugin-lib';
import {connectFirst, type BridgeConnection} from './bridgeClient';
import {freeLimit, type Box} from './freeSpace';
import {flushTrace, setTraceSink, trace} from './trace';
import {InkDrops} from './inkDrops';
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
/** Long answers continue on up to this many freshly inserted pages after the current one. */
const MAX_EXTRA_PAGES = 2;

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
      await PluginCommAPI.setLassoBoxState(LASSO_REMOVE);

      await this.answer(question, answerLayout(ctx.size, rect), ctx, update);
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
        ? await readPage(ctx.filePath, ctx.page).catch(() => null)
        : null;
    trace('ask.start', {d: {page: ctx.page, y: layout.y, full: obviouslyFull}});
    let wave = this.startWave(ctx, layout, pen);
    const scan = snapshot
      ? boxesOf(snapshot, SCAN_BUDGET_MS).catch(() => [] as Box[])
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
    return new Promise<void>((resolve, reject) => {
      abort = reject;
      this.endQuestion = reject;
      conn.onClose = reason => {
        // The bridge already sent everything (`done`): the rest is writing what we hold. A
        // dropped connection (e.g. a tunnel reconnecting) must not cut that short.
        if (!finished) {
          reject(new Error(reason));
        }
      };
      conn.onMessage = (m: BridgeMessage) => {
        if ('id' in m && m.id !== this.id) {
          return;
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
      conn.send({
        type: 'ask',
        id: this.id,
        question,
        lang: questionLang(question),
        layout,
      });
    }).finally(clearDrops);
  }
}
