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
import {trace} from './trace';
import type {InkStroke} from './protocol';

const isInt = (v: unknown): v is number =>
  typeof v === 'number' && Number.isInteger(v);
const isFiniteNumber = (v: unknown): v is number =>
  typeof v === 'number' && Number.isFinite(v);

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
/** Thrown when the host does not answer a call in time (the note app may be stuck). */
export class CallTimeoutError extends Error {
  constructor(readonly api: string) {
    super(`${api} did not respond`);
  }
}

// How long to wait for each kind of call. Saving or reading a big note is slow; the rest is quick.
const SLOW_CALLS = new Set([
  'saveCurrentNote',
  'getElements',
  'insertNotePage',
  'recognizeElements',
]);
/** Calls that wait for the person (a permission dialog): far longer, but not forever. */
const USER_CALLS = new Set(['requestPermission']);
const USER_TIMEOUT_MS = 300000;
const QUICK_TIMEOUT_MS = 15000;
const SLOW_TIMEOUT_MS = 60000;
let timeoutScale = 1;
/** Tests shorten the waits. */
export function setTimeoutScaleForTests(scale: number): void {
  timeoutScale = scale;
}

/** How long a call may take before it is given up on, in milliseconds. */
export function timeoutFor(api: string): number {
  if (USER_CALLS.has(api)) {
    return USER_TIMEOUT_MS * timeoutScale;
  }
  return (
    (SLOW_CALLS.has(api) ? SLOW_TIMEOUT_MS : QUICK_TIMEOUT_MS) * timeoutScale
  );
}

/** Resolves with `pending`, or rejects once the host has been silent for too long. */
function withTimeout<T>(
  api: string,
  pending: Promise<T>,
  onTimeout: () => void,
): Promise<T> {
  const ms = timeoutFor(api);
  let timer: ReturnType<typeof setTimeout>;
  const silence = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      onTimeout();
      reject(new CallTimeoutError(api));
    }, ms);
  });
  return Promise.race([pending, silence]).finally(() => clearTimeout(timer));
}

/** Calls that can take long or stall the note app: also recorded when they start, so a stall shows. */
const HEAVY_CALLS = new Set([
  'saveCurrentNote',
  'getNotePageTemplate',
  'getNoteTotalPageNum',
  'insertNotePage',
  'jumpToPage',
  'getElements',
]);

/**
 * How a request may share the note app with others. `exclusive`: it changes the note or depends
 * on its state (save, insert, delete, jump, create…), so nothing else runs beside it. `shared`:
 * it only reads data of an element we already hold (a stroke's points), so such reads may run
 * side by side — but never beside an exclusive request.
 */
export type Share = 'exclusive' | 'shared';

/** At most this many shared reads are in flight at once. */
const MAX_SHARED = 8;

interface Job {
  api: string;
  share: Share;
  start: () => Promise<unknown>;
  /** The caller stopped waiting: if it has not started yet, it is never sent. */
  abandoned: boolean;
  run: (p: Promise<unknown>) => void;
}

const waiting: Job[] = [];
let sharedRunning = 0;
let exclusiveRunning = false;

/** Tests that abandon a call which never settles must not block the next test. */
export function resetHostQueueForTests(): void {
  waiting.length = 0;
  sharedRunning = 0;
  exclusiveRunning = false;
}

/** Starts whatever may start now, in the order requests were made. */
function pump(): void {
  while (waiting.length > 0) {
    const job = waiting[0];
    if (job.abandoned) {
      waiting.shift(); // its caller gave up while it waited: never send it
      continue;
    }
    if (job.share === 'exclusive') {
      if (sharedRunning > 0 || exclusiveRunning) {
        return; // wait for everything running to settle; nothing behind it may jump ahead
      }
      exclusiveRunning = true;
    } else {
      if (exclusiveRunning || sharedRunning >= MAX_SHARED) {
        return;
      }
      sharedRunning++;
    }
    waiting.shift();
    if (HEAVY_CALLS.has(job.api)) {
      trace(`${job.api}.begin`);
    }
    let pending: Promise<unknown>;
    try {
      pending = Promise.resolve(job.start());
    } catch (e) {
      pending = Promise.reject(e);
    }
    // The slot is given back when the host has settled — including a request whose caller gave up.
    const free = () => {
      if (job.share === 'exclusive') {
        exclusiveRunning = false;
      } else {
        sharedRunning--;
      }
      pump();
    };
    pending.then(free, free);
    job.run(pending);
  }
}

/**
 * Every request to the note app goes through here. Requests that change the note or depend on its
 * state run one at a time, in order; independent reads of elements we hold may run together
 * (bounded), but never beside one of the others. A request starts only once what must come
 * before it has *settled* — including one we gave up waiting for, which may still be running
 * inside the note app. Waiting in line counts towards the timeout, so while the host is stuck
 * requests fail with CallTimeoutError instead of piling up, and a request whose caller already
 * gave up is never sent late.
 */
export function native<T>(
  api: string,
  start: () => Promise<T>,
  share: Share = 'exclusive',
): Promise<T> {
  let job!: Job;
  const run = new Promise<T>((resolve, reject) => {
    job = {
      api,
      share,
      start,
      abandoned: false,
      run: p => (p as Promise<T>).then(resolve, reject),
    };
  });
  run.catch(() => {}); // the caller has stopped listening after a timeout
  waiting.push(job);
  pump();
  return withTimeout(api, run, () => {
    job.abandoned = true;
    pump(); // a request that was only waiting for this one can now be dropped or started
  });
}

/** A request whose answer is a plain value (accessors on elements), not an SDK envelope. */
export async function raw<T>(
  api: string,
  start: () => Promise<T>,
  share: Share = 'exclusive',
): Promise<T> {
  const started = Date.now();
  try {
    return await native(api, start, share);
  } catch (e) {
    trace(api, {ms: Date.now() - started, err: (e as Error).message});
    throw e;
  }
}

/**
 * A request answered with the SDK's `{success, result, error}` envelope. Throws unless the host
 * says it succeeded; returns `result`.
 */
export async function call<T>(
  api: string,
  start: () => Promise<unknown>,
): Promise<T> {
  const started = Date.now();
  let res: ApiResponse<T> | null | undefined;
  try {
    res = (await native(api, start)) as ApiResponse<T> | null | undefined;
  } catch (e) {
    trace(api, {ms: Date.now() - started, err: (e as Error).message});
    throw e;
  }
  if (!res?.success) {
    const error = new SdkError(
      api,
      res?.error?.code,
      res?.error?.message ?? 'failed',
    );
    trace(api, {ms: Date.now() - started, err: error.message});
    throw error;
  }
  // Booleans are recorded: they say whether the host really did it (SDK docs: "false = failed").
  trace(api, {
    ms: Date.now() - started,
    ...(typeof res.result === 'boolean' ? {d: {result: res.result}} : {}),
  });
  return res.result as T;
}

/**
 * For calls whose `result` says whether the host really did it (save, insert page, insert or
 * delete elements). `success: true` only means the call was understood; `result: false` is a
 * refusal and must stop the work, not be ignored.
 */
export async function callChecked(
  api: string,
  start: () => Promise<unknown>,
): Promise<void> {
  if ((await call<boolean | undefined>(api, start)) === false) {
    trace(`${api}.refused`);
    throw new SdkError(api, undefined, 'the note app refused (result: false)');
  }
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
    const has = await raw('hasPermission', () =>
      PluginManager.hasPermission(p),
    );
    if (has === 1) {
      continue;
    }
    const r = await raw('requestPermission', () =>
      PluginManager.requestPermission(p),
    );
    // 1 = granted, 2 = granted this time; anything else (including an unexpected value) is a no.
    if (r !== 1 && r !== 2) {
      missing.push(p);
    }
  }
  return missing;
}

/** Opens the plugin's own screen. A refusal or failure is recorded; nothing else depends on it. */
export async function showView(): Promise<boolean> {
  try {
    const ok = await raw('showPluginView', () =>
      PluginManager.showPluginView(),
    );
    if (ok === false) {
      trace('showPluginView.refused');
    }
    return ok !== false;
  } catch {
    return false; // already recorded by raw()
  }
}

/** Closes the plugin's own screen. */
export async function closeView(): Promise<void> {
  try {
    const ok = await raw('closePluginView', () =>
      PluginManager.closePluginView(),
    );
    if (ok === false) {
      trace('closePluginView.refused');
    }
  } catch {
    // already recorded by raw()
  }
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

/** The host answered, but with something we cannot use. Stop rather than guess. */
function unexpected(api: string, what: string): never {
  trace(`${api}.unexpected`, {err: what});
  throw new SdkError(api, undefined, `unexpected answer (${what})`);
}

export async function noteContext(): Promise<NoteContext> {
  const filePath = await call<string>('getCurrentFilePath', () =>
    PluginCommAPI.getCurrentFilePath(),
  );
  if (typeof filePath !== 'string' || filePath === '') {
    unexpected('getCurrentFilePath', 'not a path');
  }
  const page = await call<number>('getCurrentPageNum', () =>
    PluginCommAPI.getCurrentPageNum(),
  );
  if (!isInt(page) || page < 0) {
    unexpected('getCurrentPageNum', 'not a page number');
  }
  const size = await call<Size>('getPageSize', () =>
    PluginFileAPI.getPageSize(filePath, page),
  );
  if (
    !isFiniteNumber(size?.width) ||
    !isFiniteNumber(size?.height) ||
    size.width <= 0 ||
    size.height <= 0
  ) {
    unexpected('getPageSize', 'not a size');
  }
  return {filePath, page, size};
}

export async function lassoRect(): Promise<Rect> {
  const r = await call<Rect>('getLassoRect', () =>
    PluginCommAPI.getLassoRect(),
  );
  if (
    !isFiniteNumber(r?.left) ||
    !isFiniteNumber(r?.top) ||
    !isFiniteNumber(r?.right) ||
    !isFiniteNumber(r?.bottom) ||
    r.right < r.left ||
    r.bottom < r.top
  ) {
    unexpected('getLassoRect', 'not a rectangle');
  }
  return r;
}

const RECOGNIZABLE = new Set([0, 500, 501, 502]); // strokes and text boxes

/** Runs the firmware's handwriting recognition over the lassoed strokes / text boxes. */
export async function recognizeLasso(size: Size): Promise<string> {
  const found = await call<Element[] | null>('getLassoElements', () =>
    PluginCommAPI.getLassoElements(),
  );
  if (found != null && !Array.isArray(found)) {
    unexpected('getLassoElements', 'not a list');
  }
  const elements = found ?? [];
  try {
    const targets = elements.filter(e => RECOGNIZABLE.has(e.type));
    if (targets.length === 0) {
      return '';
    }
    // The recogniser returns null when it finds no text (a drawing, a music score…).
    const text = await call<string | null>('recognizeElements', () =>
      PluginCommAPI.recognizeElements(targets, size),
    );
    if (text != null && typeof text !== 'string') {
      unexpected('recognizeElements', 'not text');
    }
    return text ?? '';
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
    const pen = await call<PenStyle>('getPenInfo', () =>
      PluginCommAPI.getPenInfo(),
    );
    if (
      !isInt(pen?.type) ||
      !isInt(pen?.color) ||
      !isFiniteNumber(pen?.width) ||
      pen.width <= 0
    ) {
      unexpected('getPenInfo', 'not a pen'); // caught below: the default pen is used
    }
    return {...pen, type: pen.type === PEN_MARKER ? PEN_FINELINER : pen.type};
  } catch (e) {
    if (e instanceof CallTimeoutError) {
      throw e; // a stuck note app is not "no pen info"
    }
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
  const el = await call<Element>('createElement', () =>
    PluginCommAPI.createElement(Element.TYPE_STROKE),
  );
  try {
    const pts = [
      {x: 1, y: 1},
      {x: 2, y: 2},
      {x: 3, y: 3},
    ];
    const ok = await raw('points.setRange', () =>
      el.stroke!.points.setRange(0, pts.length, pts),
    );
    fillMode =
      ok &&
      (await raw('points.size', () => el.stroke!.points.size())) === pts.length
        ? 'bulk'
        : 'single';
  } catch (e) {
    if (e instanceof CallTimeoutError) {
      throw e; // not an answer to "does bulk work?"
    }
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
    if (
      !(await raw('points.setRange', () =>
        acc.setRange(0, values.length, values),
      ))
    ) {
      throw new Error('bulk point write failed');
    }
    return;
  }
  for (let i = 0; i < values.length; i++) {
    if (!(await raw('points.add', () => acc.add(i, values[i])))) {
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
  /** Diagnostics only: leave the elements in the native cache (the caller recycles them). */
  keepInCache = false,
): Promise<string[]> {
  const mode = await probeFillMode();
  const elements: Element[] = [];
  try {
    for (const s of strokes) {
      let t = Date.now();
      const el = await call<Element>('createElement', () =>
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
    await callChecked('insertPageElements', () =>
      PluginCommAPI.insertPageElements(elements, page),
    );
    if (timing) {
      timing.insert += Date.now() - t;
    }
    return elements.map(e => e.uuid);
  } finally {
    if (!keepInCache) {
      for (const e of elements) {
        PluginCommAPI.recycleElement(e.uuid);
      }
    }
  }
}

/**
 * Edits to the open note live in an in-memory cache until saved, and file-level APIs
 * (insertNotePage, getElements, …) work on the file. Calling one without saving first loses
 * the unsaved edits or reads stale data (SDK docs: saveCurrentNote). Save before every one.
 */
export async function saveNote(): Promise<void> {
  await callChecked('saveCurrentNote', () => PluginNoteAPI.saveCurrentNote());
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
  await ensureOpen(filePath); // saving acts on whatever note is open: it must be ours
  await saveNote(); // throws when the save was refused: never insert with unsaved edits
  const before = await totalPages(filePath);
  let template = BLANK_TEMPLATE;
  try {
    const t = await call<{name: string; md5: string}>(
      'getNotePageTemplate',
      () => PluginFileAPI.getNotePageTemplate(filePath, likePage),
    );
    if (t.name && (!t.md5 || t.md5 === '0')) {
      template = t.name;
    }
  } catch (e) {
    if (e instanceof CallTimeoutError) {
      throw e;
    }
    // any other failure: the blank template (already recorded by call())
  }
  await callChecked('insertNotePage', () =>
    PluginFileAPI.insertNotePage({notePath: filePath, page, template}),
  );
  // "Inserted" is only believed once the note really has one more page.
  const after = await totalPages(filePath);
  if (after <= before) {
    trace('insertNotePage.notGrown', {d: {before, after}});
    throw new SdkError(
      'insertNotePage',
      undefined,
      `the page count did not grow (${before} → ${after})`,
    );
  }
}

async function totalPages(filePath: string): Promise<number> {
  const n = await call<number>('getNoteTotalPageNum', () =>
    PluginFileAPI.getNoteTotalPageNum(filePath),
  );
  if (!Number.isInteger(n)) {
    throw new SdkError(
      'getNoteTotalPageNum',
      undefined,
      `unexpected answer: ${String(n)}`,
    );
  }
  return n;
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
    open = await call<string>('getCurrentFilePath', () =>
      PluginCommAPI.getCurrentFilePath(),
    );
  } catch (e) {
    if (e instanceof CallTimeoutError) {
      throw e; // a stuck note app is not "the note was closed"
    }
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
  const current = await call<number>('getCurrentPageNum', () =>
    PluginCommAPI.getCurrentPageNum(),
  );
  if (!isInt(current)) {
    unexpected('getCurrentPageNum', 'not a page number');
  }
  return current === page;
}

export async function jumpToPageIn(
  filePath: string,
  page: number,
): Promise<void> {
  await ensureOpen(filePath);
  // Turning the user's view is not needed to write (strokes name their page), so a refusal is
  // recorded but does not stop the answer; a failure or silence still does.
  if (
    (await call<boolean | undefined>('jumpToPage', () =>
      PluginCommAPI.jumpToPage(page),
    )) === false
  ) {
    trace('jumpToPage.refused', {d: {page}});
  }
}

/**
 * Writes strokes to `page` of `filePath`, only while that note is open. The page does not have
 * to be the one on screen (the diagnostics showed that writing to another page works), so the
 * user is never moved around.
 */
export async function writeStrokesAt(
  filePath: string,
  page: number,
  strokes: InkStroke[],
  pen: PenStyle,
  userData?: string,
): Promise<void> {
  await ensureOpen(filePath);
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
  await ensureOpen(filePath);
  await callChecked('deletePageElements', () =>
    PluginCommAPI.deletePageElements(
      [...nums].sort((a, b) => b - a),
      page,
    ),
  );
}

/** The page's elements as saved. Saves first so writing the user just did is included. */
export async function readPage(
  filePath: string,
  page: number,
): Promise<Element[]> {
  await saveNote();
  const elements = await call<Element[] | null>('getElements', () =>
    PluginFileAPI.getElements(page, filePath),
  );
  if (elements != null && !Array.isArray(elements)) {
    unexpected('getElements', 'not a list');
  }
  if (!elements) {
    trace('getElements.null', {d: {page}}); // an empty page, or an answer we cannot read
  }
  return elements ?? [];
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
    // Reading the points of strokes we already hold is independent per stroke: these reads run
    // together (up to MAX_SHARED at a time, see `native`), in chunks so the time limit is checked.
    let scanned = 0;
    for (
      let i = 0;
      i < strokes.length && Date.now() < deadline;
      i += SCAN_CHUNK
    ) {
      const chunk = strokes.slice(i, i + SCAN_CHUNK);
      const found = await Promise.all(
        chunk.map(async e => {
          const n = await raw(
            'points.size',
            () => e.stroke!.points.size(),
            'shared',
          );
          return boundsOf(
            n > 0
              ? await raw(
                  'points.getRange',
                  () => e.stroke!.points.getRange(0, n),
                  'shared',
                )
              : [],
          );
        }),
      );
      for (const b of found) {
        if (b) {
          boxes.push(b);
        }
      }
      scanned += chunk.length;
    }
    trace('scan', {d: {strokes: strokes.length, scanned}});
  } finally {
    releaseElements(elements);
  }
  return boxes;
}

const SCAN_CHUNK = 32;

export function releaseElements(elements: Element[]): void {
  for (const e of elements) {
    PluginCommAPI.recycleElement(e.uuid);
  }
}
