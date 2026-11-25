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
import {runProcedure, type Outcome} from './procedure';
import {trace} from './trace';
import {INSERT_TABLE} from './insertTable.generated';
import {WRITE_TABLE} from './writeTable.generated';
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

/** The host understood the call but refused it (`result: false`). */
export class RefusedError extends SdkError {}

/** The host answered with something we cannot use (a wrong type, a page count that did not grow). */
export class UnexpectedError extends SdkError {}

/** Which outcome of the spec's "結果" table an error is. */
export function classify(e: unknown): Outcome {
  if (e instanceof NoteMovedError) {
    return 'NoteChanged';
  }
  if (e instanceof CallTimeoutError) {
    return 'NoAnswer';
  }
  if (e instanceof RefusedError) {
    return 'Refused';
  }
  if (e instanceof UnexpectedError) {
    return 'Unexpected';
  }
  return 'Failed';
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
 * What a request is, which decides how it waits and what happens to its place when the caller
 * gives up (docs/spec/host-requests.md). `manager`: asks the plugin host, not the note (permissions,
 * opening a screen): outside the line of note requests. `read`: reads the state of the note app:
 * never beside a `mutate`, one at a time; its place is given back when the caller gives up (a read
 * that runs late cannot harm the note). `mutate`: changes the note: one at a time, and its place
 * is given back only when the host has answered (it may still be running; sending another on top
 * of it could corrupt the note). `shared`: reads data of an element we already hold (a stroke's
 * points): up to MAX_SHARED side by side, never beside a `mutate` or a `read`.
 */
export type Kind = 'manager' | 'read' | 'mutate' | 'shared';

const MANAGER_CALLS = new Set([
  'hasPermission',
  'requestPermission',
  'getDeviceType',
  'getPluginDirPath',
  'showPluginView',
]);
const READ_CALLS = new Set([
  'getCurrentFilePath',
  'getPenInfo',
  'canHandwrite',
  'getCurrentPageNum',
  'getNoteTotalPageNum',
  'getPageSize',
  'getLassoRect',
  'getLassoElements',
  'getElements',
  'getNotePageTemplate',
  'recognizeElements',
]);

/** The kind of a request: from its name, unless the caller says (`shared`). A name we do not know is a `mutate` (the safe one). */
export function kindOf(api: string): Kind {
  if (MANAGER_CALLS.has(api)) {
    return 'manager';
  }
  return READ_CALLS.has(api) ? 'read' : 'mutate';
}

/** At most this many shared reads are in flight at once. */
const MAX_SHARED = 8;
/** At most this many manager requests are in flight at once. */
const MAX_MANAGER = 4;

interface Job {
  api: string;
  kind: Kind;
  start: () => Promise<unknown>;
  started: boolean;
  /** Its place in the line has been given back (once). */
  released: boolean;
  /** The caller stopped waiting: if it has not started yet, it is never sent. */
  abandoned: boolean;
  run: (p: Promise<unknown>) => void;
}

const waiting: Job[] = []; // the line of note requests (read, mutate, shared), in the order asked
const managerWaiting: Job[] = [];
let sharedRunning = 0;
let readRunning = false;
let mutateRunning = false; // from the start until the host answers (also after the caller gave up)
let managerRunning = 0;

/** Tests that abandon a call which never settles must not block the next test. */
export function resetHostQueueForTests(): void {
  waiting.length = 0;
  managerWaiting.length = 0;
  sharedRunning = 0;
  readRunning = false;
  mutateRunning = false;
  managerRunning = 0;
}

/** Gives back a started job's place, once. Then whatever waits may start. */
function release(job: Job): void {
  if (job.released) {
    return;
  }
  job.released = true;
  switch (job.kind) {
    case 'manager':
      managerRunning--;
      break;
    case 'read':
      readRunning = false;
      break;
    case 'mutate':
      mutateRunning = false;
      break;
    case 'shared':
      sharedRunning--;
      break;
  }
  pump();
}

function launch(job: Job): void {
  job.started = true;
  if (HEAVY_CALLS.has(job.api)) {
    trace(`${job.api}.begin`);
  }
  let pending: Promise<unknown>;
  try {
    pending = Promise.resolve(job.start());
  } catch (e) {
    pending = Promise.reject(e);
  }
  // The host has settled: the place is always given back then (a late answer after the caller
  // gave up included; for a `read` it was given back already).
  const free = () => release(job);
  pending.then(free, free);
  job.run(pending);
}

/** Starts whatever may start now, in the order requests were made (docs/spec/host-requests.md). */
function pump(): void {
  while (managerWaiting.length > 0 && managerRunning < MAX_MANAGER) {
    const job = managerWaiting.shift()!;
    if (job.abandoned) {
      continue; // its caller gave up while it waited: never send it
    }
    managerRunning++;
    launch(job);
  }
  while (waiting.length > 0) {
    const job = waiting[0];
    if (job.abandoned) {
      waiting.shift();
      continue;
    }
    if (job.kind === 'shared') {
      if (mutateRunning || readRunning || sharedRunning >= MAX_SHARED) {
        return;
      }
      sharedRunning++;
    } else {
      // read and mutate run alone; nothing behind them may jump ahead
      if (mutateRunning || readRunning || sharedRunning > 0) {
        return;
      }
      if (job.kind === 'read') {
        readRunning = true;
      } else {
        mutateRunning = true;
      }
    }
    waiting.shift();
    launch(job);
  }
}

/**
 * Every request to the host goes through here. Note requests wait in one line (see `Kind`); a
 * request starts only once what must come before it has finished. Waiting in line counts towards
 * the timeout, so while the note app is stuck requests fail with CallTimeoutError instead of
 * piling up, and a request whose caller already gave up is never sent late. `shared` is the only
 * kind a caller names; every other kind comes from the request's name.
 */
export function native<T>(
  api: string,
  start: () => Promise<T>,
  share?: 'shared',
): Promise<T> {
  let job!: Job;
  const run = new Promise<T>((resolve, reject) => {
    job = {
      api,
      kind: share ?? kindOf(api),
      start,
      started: false,
      released: false,
      abandoned: false,
      run: p => (p as Promise<T>).then(resolve, reject),
    };
  });
  run.catch(() => {}); // the caller has stopped listening after a timeout
  (job.kind === 'manager' ? managerWaiting : waiting).push(job);
  pump();
  return withTimeout(api, run, () => {
    job.abandoned = true;
    // A started read, shared or manager request gives its place back now (it cannot harm the
    // note if it runs late); a started mutate keeps it until the host answers.
    if (job.started && job.kind !== 'mutate') {
      release(job);
    }
    pump(); // a request that was only waiting for this one can now be dropped or started
  });
}

/** A request whose answer is a plain value (accessors on elements), not an SDK envelope. */
export async function raw<T>(
  api: string,
  start: () => Promise<T>,
  share?: 'shared',
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
    throw new RefusedError(
      api,
      undefined,
      'the note app refused (result: false)',
    );
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

/**
 * Closes the plugin's own screen. It does NOT wait in the line of requests to the note app: it
 * touches no note, and a person who cannot leave a screen is stuck (the only way out would be to
 * restart the device). When an earlier request never gets an answer, everything behind it in the
 * line waits for ever — this one must still go out. Returns whether the host said it closed.
 */
export async function closeView(): Promise<boolean> {
  const started = Date.now();
  try {
    const ok = await withTimeout(
      'closePluginView',
      Promise.resolve().then(() => PluginManager.closePluginView()),
      () => {},
    );
    if (ok === false) {
      trace('closePluginView.refused');
    }
    return ok !== false;
  } catch (e) {
    trace('closePluginView', {
      ms: Date.now() - started,
      err: (e as Error).message,
    });
    return false;
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
  throw new UnexpectedError(api, undefined, `unexpected answer (${what})`);
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
 * Inserts a new page at `page` (later pages move down) using the same template as `likePage`,
 * and turns the view to it. The steps and what each outcome means are the "挿入" table of
 * docs/spec/page-turn.md (src/insertTable.generated.ts): the steps below only do the work and
 * report Ok / Failed / Refused / NoAnswer / NoteChanged / Unexpected; the table decides whether to go on, to fall back
 * (blank template, a view that would not turn) or to stop — and whether the note may have been
 * changed when it stops. On a stop the error that caused it is thrown.
 */
export async function insertPageAfter(
  filePath: string,
  page: number,
  likePage: number,
): Promise<void> {
  let before = 0;
  let template = BLANK_TEMPLATE;
  let failure: unknown = null;
  const step = (work: () => Promise<void>) => async (): Promise<Outcome> => {
    try {
      await work();
      return 'Ok';
    } catch (e) {
      failure = e;
      return classify(e);
    }
  };
  const end = await runProcedure({
    name: 'insert',
    table: INSERT_TABLE,
    start: 'CheckNote',
    ends: ['Done', 'AbortClean', 'AbortDirty'],
    steps: {
      // saving acts on whatever note is open: it must be ours
      CheckNote: step(() => ensureOpen(filePath)),
      // a save that was refused stops everything: never insert with unsaved edits
      Save: step(() => saveNote()),
      CountBefore: step(async () => {
        before = await totalPages(filePath);
      }),
      GetTemplate: step(async () => {
        const t = await call<{name: string; md5: string}>(
          'getNotePageTemplate',
          () => PluginFileAPI.getNotePageTemplate(filePath, likePage),
        );
        // Custom templates can't be referenced by name: those fall back to a blank page.
        if (!t?.name || (t.md5 && t.md5 !== '0')) {
          unexpected('getNotePageTemplate', 'no template that can be named');
        }
        template = t.name;
      }),
      InsertPage: step(() =>
        callChecked('insertNotePage', () =>
          PluginFileAPI.insertNotePage({notePath: filePath, page, template}),
        ),
      ),
      // "Inserted" is only believed once the note really has one more page.
      CountAfter: step(async () => {
        const after = await totalPages(filePath);
        if (after <= before) {
          trace('insertNotePage.notGrown', {d: {before, after}});
          throw new UnexpectedError(
            'insertNotePage',
            undefined,
            `the page count did not grow (${before} → ${after})`,
          );
        }
      }),
      Jump: step(() => jumpToPageIn(filePath, page)),
    },
    handlers: {
      X0: () => {},
      X1: arg => trace('pageTurn.dirty', {d: arg}),
      X2: () => {},
      X3: arg => {
        trace('pageTurn.fallback', {d: arg}); // a blank page, or a view that would not turn
        failure = null;
      },
      X4: () => {},
    },
  });
  if (end !== 'Done') {
    throw failure ?? new Error(`inserting a page ended at ${end}`);
  }
}

async function totalPages(filePath: string): Promise<number> {
  const n = await call<number>('getNoteTotalPageNum', () =>
    PluginFileAPI.getNoteTotalPageNum(filePath),
  );
  if (!Number.isInteger(n)) {
    unexpected('getNoteTotalPageNum', `not a page count: ${String(n)}`);
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
  // Turning the user's view is not needed to write (strokes name their page): the insert table
  // treats a refusal as a recorded fallback, a failure or silence as a stop.
  await callChecked('jumpToPage', () => PluginCommAPI.jumpToPage(page));
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
  let failure: unknown = null;
  const step = (work: () => Promise<void>) => async (): Promise<Outcome> => {
    try {
      await work();
      return 'Ok';
    } catch (e) {
      failure = e;
      return classify(e);
    }
  };
  // The "書き込み" table of docs/spec/page-turn.md: only into the open note; once the insert has
  // been asked for, any failure leaves the page possibly changed.
  const end = await runProcedure({
    name: 'write',
    table: WRITE_TABLE,
    start: 'CheckNote',
    ends: ['Written', 'AbortClean', 'AbortDirty'],
    steps: {
      CheckNote: step(() => ensureOpen(filePath)),
      WriteStrokes: step(async () => {
        await writeStrokes(strokes, page, pen, undefined, userData);
      }),
    },
    handlers: {
      X0: () => {},
      X1: arg => trace('write.dirty', {d: arg}),
      X2: () => {},
      X3: () => {},
      X4: () => {},
    },
  });
  if (end !== 'Written') {
    throw failure ?? new Error(`writing ended at ${end}`);
  }
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
