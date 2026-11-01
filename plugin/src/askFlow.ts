import {PluginCommAPI} from 'sn-plugin-lib';
import {connectFirst, type BridgeConnection} from './bridgeClient';
import {InkDrops} from './inkDrops';
import {loadBridges} from './bridgeConfig';
import {t} from './i18n';
import type {BridgeMessage, InkStroke, Lang, LayoutParams} from './protocol';
import {
  currentPen,
  ensurePermissions,
  insertPageAfter,
  jumpToPage,
  lassoRect,
  noteContext,
  recognizeLasso,
  writeStrokes,
  type NoteContext,
} from './sdk';

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
    y: Math.round(rect.bottom + charSize * 0.6),
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
  private cancelled = false;

  constructor(private readonly report: Report) {}

  cancel(): void {
    this.cancelled = true;
    if (this.conn && this.id) {
      this.conn.send({type: 'cancel', id: this.id});
    }
    this.conn?.close();
  }

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
        message: t('error', {message: (e as Error).message}),
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
  ): Promise<void> {
    const state: AskState = {message: '', question, finished: false};
    const update = (patch: Partial<AskState>) => {
      Object.assign(state, patch);
      this.report({...state});
    };
    try {
      await this.answer(question, layout, ctx, update);
    } catch (e) {
      update({
        message: t('error', {message: (e as Error).message}),
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
  ): Promise<void> {
    if (layout.y + layout.charSize > layout.pageHeight - layout.bottomMargin) {
      return update({message: t('noRoom'), finished: true, failed: true});
    }
    const pen = await currentPen();
    update({message: t('connecting')});
    const {endpoints} = await loadBridges();
    try {
      this.conn = await connectFirst(endpoints);
    } catch {
      return update({message: t('noBridge'), finished: true, failed: true});
    }
    update({showProgress: this.conn.settings.showProgress});
    try {
      if (!this.cancelled) {
        await this.converse(question, layout, ctx, pen, update);
      }
    } finally {
      this.conn.close();
    }
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
  ): Promise<void> {
    const conn = this.conn!;
    this.id = `${Date.now().toString(36)}-${Math.random()
      .toString(36)
      .slice(2, 8)}`;
    let writes = Promise.resolve();
    let written = 0;
    let answer = '';
    let pagesReady = 1;
    // Ink drops mark where the answer will appear until its first line arrives.
    const drops = new InkDrops(
      ctx.page,
      layout.x,
      Math.round(layout.y + layout.charSize * 0.8),
      layout.charSize,
      pen,
    );
    let dropsCleared: Promise<void> | null = null;
    const clearDrops = () => (dropsCleared ??= drops.stop());
    drops.start();
    const writeLine = (
      strokes: InkStroke[],
      text: string,
      pageOffset: number,
    ) => {
      writes = writes.then(async () => {
        await clearDrops();
        if (this.cancelled) {
          return;
        }
        // The first line past the bottom inserts the next page and turns to it.
        while (pagesReady <= pageOffset) {
          await insertPageAfter(ctx.filePath, ctx.page + pagesReady, ctx.page);
          await jumpToPage(ctx.page + pagesReady);
          pagesReady++;
        }
        await writeStrokes(strokes, ctx.page + pageOffset, pen);
        written++;
        answer += text;
        update({message: t('writing', {lines: written}), answer});
      });
    };

    return new Promise<void>((resolve, reject) => {
      conn.onClose = reason => reject(new Error(reason));
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
