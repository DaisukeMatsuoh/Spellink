import {
  Element,
  PluginCommAPI,
  PluginFileAPI,
  PluginManager,
  type ElementDataAccessor,
  type Point,
  type Rect,
} from 'sn-plugin-lib';
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
  const elements = await call<Element[]>(
    'getLassoElements',
    PluginCommAPI.getLassoElements(),
  );
  try {
    const targets = elements.filter(e => RECOGNIZABLE.has(e.type));
    if (targets.length === 0) {
      return '';
    }
    return await call<string>(
      'recognizeElements',
      PluginCommAPI.recognizeElements(targets, size),
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
): Promise<void> {
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
  } finally {
    for (const e of elements) {
      PluginCommAPI.recycleElement(e.uuid);
    }
  }
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

export function jumpToPage(page: number): Promise<boolean> {
  return call<boolean>('jumpToPage', PluginCommAPI.jumpToPage(page));
}
