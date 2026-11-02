import {
  Element,
  PluginCommAPI,
  PluginFileAPI,
  PluginManager,
  PluginNoteAPI,
  type ElementDataAccessor,
  type Point,
  type Rect,
} from 'sn-plugin-lib';
import {WAVE_MARKER_PREFIX, boundsOf, type Box} from './freeSpace';
import type {InkStroke} from './protocol';

export class SdkError extends Error {
  constructor(
    readonly api: string,
    readonly code: number | undefined,
    message: string,
  ) {
    super(`${api}: ${message}${code != null ? ` (${code})` : ''}`);
  }
}

type ApiResponse<T> = {
  success?: boolean;
  result?: T;
  error?: {code?: number; message?: string};
};

/** Unwraps the SDK's `{success, result, error}` envelope, throwing on failure. */
export async function call<T>(
  api: string,
  pending: Promise<unknown>,
): Promise<T> {
  const res = (await pending) as ApiResponse<T> | null | undefined;
  if (!res?.success) {
    throw new SdkError(api, res?.error?.code, res?.error?.message ?? 'failed');
  }
  return res.result as T;
}

export const PERMISSIONS = [
  'plugin.permission.FILE:READ',
  'plugin.permission.FILE:WRITE',
  'plugin.permission.INTERNET',
] as const;

/**
 * Checks every permission and asks for the missing ones ("allow this time" grants expire when the
 * plugin closes, so this runs on every use). Returns the permissions still not granted.
 */
export async function ensurePermissions(): Promise<string[]> {
  const missing: string[] = [];
  for (const p of PERMISSIONS) {
    if ((await PluginManager.hasPermission(p)) === 1) {
      continue;
    }
    const r = await PluginManager.requestPermission(p);
    if (r !== 1 && r !== 2) {
      missing.push(p);
    }
  }
  return missing;
}

export interface Size {
  width: number;
  height: number;
}

export interface NoteContext {
  filePath: string;
  page: number;
  size: Size;
}

export async function noteContext(): Promise<NoteContext> {
  const filePath = await call<string>(
    'getCurrentFilePath',
    PluginCommAPI.getCurrentFilePath(),
  );
  const page = await call<number>(
    'getCurrentPageNum',
    PluginCommAPI.getCurrentPageNum(),
  );
  const size = await call<Size>(
    'getPageSize',
    PluginFileAPI.getPageSize(filePath, page),
  );
  return {filePath, page, size};
}

export function lassoRect(): Promise<Rect> {
  return call<Rect>('getLassoRect', PluginCommAPI.getLassoRect());
}

const RECOGNIZABLE = new Set([0, 500, 501, 502]); // strokes and text boxes

/** Runs the firmware's handwriting recognition over the lassoed strokes / text boxes. */
export async function recognizeLasso(size: Size): Promise<string> {
  const elements =
    (await call<Element[] | null>(
      'getLassoElements',
      PluginCommAPI.getLassoElements(),
    )) ?? [];
  try {
    const targets = elements.filter(e => RECOGNIZABLE.has(e.type));
    if (targets.length === 0) {
      return '';
    }
    // The recogniser returns null when it finds no text (a drawing, a music score…).
    return (
      (await call<string | null>(
        'recognizeElements',
        PluginCommAPI.recognizeElements(targets, size),
      )) ?? ''
    );
  } finally {
    for (const e of elements) {
      PluginCommAPI.recycleElement(e.uuid);
    }
  }
}

export interface PenStyle {
  type: number;
  color: number;
  width: number;
}

const PEN_FINELINER = 10;
const PEN_MARKER = 11;

/** The user's current pen, except a marker (too wide for text) becomes a fineliner. */
export async function currentPen(): Promise<PenStyle> {
  try {
    const pen = await call<PenStyle>('getPenInfo', PluginCommAPI.getPenInfo());
    return {...pen, type: pen.type === PEN_MARKER ? PEN_FINELINER : pen.type};
  } catch {
    return {type: PEN_FINELINER, color: 0, width: 200};
  }
}

/**
 * How point data gets into a new stroke. 'bulk' writes all points with one native call;
 * 'single' needs one call per point. The docs don't say whether `setRange` can grow an empty
 * accessor, so this is probed once on a throwaway element.
 */
export type FillMode = 'bulk' | 'single';
let fillMode: FillMode | null = null;

export async function probeFillMode(): Promise<FillMode> {
  if (fillMode) {
    return fillMode;
  }
  const el = await call<Element>(
    'createElement',
    PluginCommAPI.createElement(Element.TYPE_STROKE),
  );
  try {
    const pts = [
      {x: 1, y: 1},
      {x: 2, y: 2},
      {x: 3, y: 3},
    ];
    const ok = await el.stroke!.points.setRange(0, pts.length, pts);
    fillMode =
      ok && (await el.stroke!.points.size()) === pts.length ? 'bulk' : 'single';
  } catch {
    fillMode = 'single';
  } finally {
    PluginCommAPI.recycleElement(el.uuid);
  }
  return fillMode;
}

async function fill<T extends Point | number>(
  acc: ElementDataAccessor<T>,
  values: T[],
  mode: FillMode,
): Promise<void> {
  if (mode === 'bulk') {
    if (!(await acc.setRange(0, values.length, values))) {
      throw new Error('bulk point write failed');
    }
    return;
  }
  for (let i = 0; i < values.length; i++) {
    if (!(await acc.add(i, values[i]))) {
      throw new Error(`point write failed at ${i}`);
    }
  }
}

export interface WriteTiming {
  create: number;
  fill: number;
  insert: number;
}

/** Creates stroke elements from `strokes` and inserts them into `page` of the open note in one call. */
export async function writeStrokes(
  strokes: InkStroke[],
  page: number,
  pen: PenStyle,
  timing?: WriteTiming,
  /** Stored on every created stroke (Element.userData); lets us recognise our own strokes later. */
  userData?: string,
): Promise<string[]> {
  const mode = await probeFillMode();
  const elements: Element[] = [];
  try {
    for (const s of strokes) {
      let t = Date.now();
      const el = await call<Element>(
        'createElement',
        PluginCommAPI.createElement(Element.TYPE_STROKE),
      );
      elements.push(el);
      el.pageNum = page;
      el.thickness = pen.width;
      if (userData) {
        el.userData = userData;
      }
      const stroke = el.stroke!;
      stroke.penType = pen.type;
      stroke.penColor = pen.color;
      // Not in the Stroke type, but the SDK's element validator accepts it.
      (stroke as typeof stroke & {penWidth?: number}).penWidth = pen.width;
      if (timing) {
        timing.create += Date.now() - t;
      }

      t = Date.now();
      const points = [];
      for (let i = 0; i < s.p.length; i += 2) {
        points.push({x: s.p[i], y: s.p[i + 1]});
      }
      await fill(stroke.points, points, mode);
      await fill(stroke.pressures, s.w, mode);
      if (timing) {
        timing.fill += Date.now() - t;
      }
    }
    const t = Date.now();
    await call<boolean>(
      'insertPageElements',
      PluginCommAPI.insertPageElements(elements, page),
    );
    if (timing) {
      timing.insert += Date.now() - t;
    }
    return elements.map(e => e.uuid);
  } finally {
    for (const e of elements) {
      PluginCommAPI.recycleElement(e.uuid);
    }
  }
}

/**
 * Edits to the open note live in an in-memory cache until saved, and file-level APIs
 * (insertNotePage, getElements, …) work on the file. Calling one without saving first loses
 * the unsaved edits or reads stale data (SDK docs: saveCurrentNote). Save before every one.
 */
export async function saveNote(): Promise<void> {
  await call<boolean>('saveCurrentNote', PluginNoteAPI.saveCurrentNote());
}

const BLANK_TEMPLATE = 'style_blank';

/**
 * Inserts a new page at `page` (later pages move down) using the same template as `likePage`.
 * Custom templates can't be referenced by name, so those fall back to a blank page.
 */
export async function insertPageAfter(
  filePath: string,
  page: number,
  likePage: number,
): Promise<void> {
  await saveNote();
  let template = BLANK_TEMPLATE;
  try {
    const t = await call<{name: string; md5: string}>(
      'getNotePageTemplate',
      PluginFileAPI.getNotePageTemplate(filePath, likePage),
    );
    if (t.name && (!t.md5 || t.md5 === '0')) {
      template = t.name;
    }
  } catch {
    // keep the blank template
  }
  await call<boolean>(
    'insertNotePage',
    PluginFileAPI.insertNotePage({notePath: filePath, page, template}),
  );
}

/** Thrown when the note we are working on is no longer the open one. */
export class NoteMovedError extends Error {
  constructor(readonly filePath: string) {
    super(`note ${filePath} is no longer open`);
  }
}

/**
 * Comm-level APIs (insertPageElements, deletePageElements, jumpToPage…) act on whatever file is
 * open, whatever path we mean. So every one of them goes through here first: if the open file is
 * not ours, nothing is done.
 */
export async function ensureOpen(filePath: string): Promise<void> {
  let open: string | null = null;
  try {
    open = await call<string>(
      'getCurrentFilePath',
      PluginCommAPI.getCurrentFilePath(),
    );
  } catch {
    // The note app is not bound: it is not open.
  }
  if (open !== filePath) {
    throw new NoteMovedError(filePath);
  }
}

/** Where the user is looking, relative to where we mean to write. */
export async function isOnPage(
  filePath: string,
  page: number,
): Promise<boolean> {
  await ensureOpen(filePath);
  return (
    (await call<number>(
      'getCurrentPageNum',
      PluginCommAPI.getCurrentPageNum(),
    )) === page
  );
}

export async function jumpToPageIn(
  filePath: string,
  page: number,
): Promise<void> {
  await ensureOpen(filePath);
  await call<boolean>('jumpToPage', PluginCommAPI.jumpToPage(page));
}

/**
 * Writes strokes to `page` of `filePath`, only while that note is open, and only after bringing
 * the user to that page (strokes written while another page is on screen have been lost).
 */
export async function writeStrokesAt(
  filePath: string,
  page: number,
  strokes: InkStroke[],
  pen: PenStyle,
  userData?: string,
): Promise<void> {
  if (!(await isOnPage(filePath, page))) {
    await jumpToPageIn(filePath, page);
  }
  await writeStrokes(strokes, page, pen, undefined, userData);
}

/** Deletes elements (highest number first) from `page` of `filePath`, only while it is open. */
export async function deleteElementsAt(
  filePath: string,
  page: number,
  nums: number[],
): Promise<void> {
  if (nums.length === 0) {
    return;
  }
  if (!(await isOnPage(filePath, page))) {
    await jumpToPageIn(filePath, page);
  }
  await call(
    'deletePageElements',
    PluginCommAPI.deletePageElements(
      [...nums].sort((a, b) => b - a),
      page,
    ),
  );
}

const SCAN_CONCURRENCY = 16;

/** The page's elements as saved. Saves first so writing the user just did is included. */
export async function readPage(
  filePath: string,
  page: number,
): Promise<Element[]> {
  await saveNote();
  return (
    (await call<Element[] | null>(
      'getElements',
      PluginFileAPI.getElements(page, filePath),
    )) ?? []
  );
}

/**
 * Bounding boxes of the strokes, titles, pictures and text boxes in `elements` (as returned by
 * readPage; released here). The thinking animation's own strokes are skipped by their marker.
 * Stops after `budgetMs` — pages can hold thousands of strokes — and returns what it found.
 */
export async function boxesOf(
  elements: Element[],
  budgetMs: number,
): Promise<Box[]> {
  const deadline = Date.now() + budgetMs;
  const boxes: Box[] = [];
  try {
    const strokes: Element[] = [];
    for (const e of elements) {
      if (e.userData?.startsWith(WAVE_MARKER_PREFIX)) {
        continue;
      }
      if (e.type === 100 && e.title) {
        boxes.push({
          left: e.title.X,
          top: e.title.Y,
          right: e.title.X + e.title.width,
          bottom: e.title.Y + e.title.height,
        });
      } else if (e.type === 200 && e.picture?.rect) {
        boxes.push(e.picture.rect);
      } else if (
        (e.type === 500 || e.type === 501 || e.type === 502) &&
        e.textBox?.textRect
      ) {
        boxes.push(e.textBox.textRect);
      } else if (e.type === 0 && e.stroke?.points) {
        strokes.push(e);
      }
    }
    for (
      let i = 0;
      i < strokes.length && Date.now() < deadline;
      i += SCAN_CONCURRENCY
    ) {
      const found = await Promise.all(
        strokes.slice(i, i + SCAN_CONCURRENCY).map(async e => {
          const n = await e.stroke!.points.size();
          return boundsOf(n > 0 ? await e.stroke!.points.getRange(0, n) : []);
        }),
      );
      for (const b of found) {
        if (b) {
          boxes.push(b);
        }
      }
    }
  } finally {
    releaseElements(elements);
  }
  return boxes;
}

export function releaseElements(elements: Element[]): void {
  for (const e of elements) {
    PluginCommAPI.recycleElement(e.uuid);
  }
}
