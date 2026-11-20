// Everything the bridge sends is checked here before anything acts on it (docs/spec/interfaces.md
// I5). A bridge may be a different version, buggy, or not ours at all; the types in protocol.ts
// say what we expect, not what arrives.
import type {BridgeMessage, InkStroke, Settings} from './protocol';

/** Long answers continue on up to this many freshly inserted pages after the current one. */
export const MAX_EXTRA_PAGES = 2;

/** Limits on what one `line` frame may ask us to write. */
export const MAX_STROKES_PER_LINE = 3000;
export const MAX_POINTS_PER_STROKE = 4000;
export const MAX_COORDINATE = 100000;
export const MAX_TEXT_CHARS = 4000;
export const MAX_FRAME_ID_CHARS = 128;

export type Parsed =
  | {kind: 'frame'; frame: BridgeMessage}
  /** A message type this version does not know: skipped, so a newer bridge can add new ones. */
  | {kind: 'ignore'; type: string}
  | {kind: 'bad'; reason: string};

const bad = (reason: string): Parsed => ({kind: 'bad', reason});

const isObject = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);
const isInt = (v: unknown, min: number, max: number): v is number =>
  typeof v === 'number' && Number.isInteger(v) && v >= min && v <= max;
const isStr = (v: unknown, max: number): v is string =>
  typeof v === 'string' && v.length <= max;

/** Why a stroke cannot be written, or null when it can. */
export function strokeProblem(s: unknown): string | null {
  if (!isObject(s) || !Array.isArray(s.p) || !Array.isArray(s.w)) {
    return 'stroke is not {p, w}';
  }
  const {p, w} = s as {p: unknown[]; w: unknown[]};
  if (p.length < 2 || p.length % 2 !== 0) {
    return 'stroke has no points or an odd number of coordinates';
  }
  if (p.length / 2 > MAX_POINTS_PER_STROKE) {
    return 'stroke has too many points';
  }
  if (w.length !== p.length / 2) {
    return 'stroke pressures do not match its points';
  }
  for (const v of p) {
    if (
      typeof v !== 'number' ||
      !Number.isFinite(v) ||
      Math.abs(v) > MAX_COORDINATE
    ) {
      return 'stroke coordinate is not a sane number';
    }
  }
  for (const v of w) {
    if (
      typeof v !== 'number' ||
      !Number.isFinite(v) ||
      v < 0 ||
      v > MAX_COORDINATE
    ) {
      return 'stroke pressure is not a sane number';
    }
  }
  return null;
}

/**
 * Checks one frame from the bridge. `maxPage` is the furthest page (relative to the page the
 * question was asked on) a line may name; a bridge cannot make us insert more pages than that.
 */
export function parseFrame(raw: unknown, maxPage: number): Parsed {
  if (!isObject(raw) || typeof raw.type !== 'string') {
    return bad('frame is not an object with a type');
  }
  const seq = raw.seq;
  if (seq !== undefined && !isInt(seq, 0, 1e9)) {
    return bad('seq is not a non-negative integer');
  }
  switch (raw.type) {
    case 'status':
      if (!isStr(raw.id, MAX_FRAME_ID_CHARS) || typeof raw.stage !== 'string') {
        return bad('status frame without id or stage');
      }
      return {kind: 'frame', frame: raw as unknown as BridgeMessage};
    case 'settings':
      if (!isObject(raw.settings)) {
        return bad('settings frame without settings');
      }
      return {
        kind: 'frame',
        frame: {type: 'settings', settings: parseSettings(raw.settings)},
      };
    case 'historyCleared':
      if (!isInt(raw.notes, 0, 1e9)) {
        return bad('historyCleared without a count');
      }
      return {kind: 'frame', frame: raw as unknown as BridgeMessage};
    case 'line': {
      if (!isStr(raw.id, MAX_FRAME_ID_CHARS)) {
        return bad('line without id');
      }
      if (!isInt(raw.index, 0, 1e9)) {
        return bad('line index is not a non-negative integer');
      }
      if (!isInt(raw.page, 0, maxPage)) {
        return bad(`line page is outside 0..${maxPage}`);
      }
      if (!isStr(raw.text, MAX_TEXT_CHARS)) {
        return bad('line text is missing or too long');
      }
      const strokes = (raw.strokes ?? []) as unknown; // a line may have no glyphs at all
      if (!Array.isArray(strokes) || strokes.length > MAX_STROKES_PER_LINE) {
        return bad('line strokes are not a reasonable list');
      }
      for (const s of strokes) {
        const problem = strokeProblem(s);
        if (problem) {
          return bad(problem);
        }
      }
      return {
        kind: 'frame',
        frame: {
          ...raw,
          strokes: strokes as InkStroke[],
        } as unknown as BridgeMessage,
      };
    }
    case 'done':
      if (
        !isStr(raw.id, MAX_FRAME_ID_CHARS) ||
        typeof raw.truncated !== 'boolean'
      ) {
        return bad('done without id or truncated');
      }
      return {kind: 'frame', frame: raw as unknown as BridgeMessage};
    case 'error':
      if (typeof raw.message !== 'string' || typeof raw.code !== 'string') {
        return bad('error without code or message');
      }
      if (raw.id !== undefined && !isStr(raw.id, MAX_FRAME_ID_CHARS)) {
        return bad('error id is not a string');
      }
      return {kind: 'frame', frame: raw as unknown as BridgeMessage};
    default:
      return {kind: 'ignore', type: raw.type};
  }
}

const bool = (v: unknown, fallback: boolean): boolean =>
  typeof v === 'boolean' ? v : fallback;
const inRange = (v: unknown, min: number, max: number): number | undefined =>
  typeof v === 'number' && Number.isFinite(v) && v >= min && v <= max
    ? v
    : undefined;

/** Settings from the bridge, with anything out of range or of the wrong type replaced by the default. */
export function parseSettings(raw: unknown): Settings {
  const s = isObject(raw) ? raw : {};
  return {
    showProgress: bool(s.showProgress, false),
    lineTilt: bool(s.lineTilt, true),
    tiltDegrees: inRange(s.tiltDegrees, -10, 10),
    thinkingAnimation: bool(s.thinkingAnimation, false),
    fullWidthSpacing: inRange(s.fullWidthSpacing, 100, 140),
    halfWidthSpacing: inRange(s.halfWidthSpacing, 100, 160),
    avoidOverwriting: bool(s.avoidOverwriting, false),
    busyNotice: bool(s.busyNotice, true),
  };
}
