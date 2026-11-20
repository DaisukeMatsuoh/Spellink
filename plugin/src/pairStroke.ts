// Handwriting on the pairing screen: the strokes the bridge wrote are drawn as short straight
// pieces (a view cannot draw a curve). To keep the number of views small on a slow screen, points
// closer than `minStep` are skipped.
import type {InkStroke} from './protocol';

export interface Segment {
  /** Middle of the piece. */
  cx: number;
  cy: number;
  length: number;
  /** Radians, as a view's rotation wants it. */
  angle: number;
}

/** Most pieces a banner may have; a bigger one is not drawn (the phrase is shown as text instead). */
export const MAX_SEGMENTS = 1500;

/** The pieces of the strokes, or null when there would be too many. */
export function strokeSegments(
  strokes: InkStroke[],
  minStep: number,
): Segment[] | null {
  const out: Segment[] = [];
  for (const s of strokes) {
    let px = s.p[0];
    let py = s.p[1];
    for (let i = 2; i < s.p.length; i += 2) {
      const x = s.p[i];
      const y = s.p[i + 1];
      const last = i === s.p.length - 2;
      const d = Math.hypot(x - px, y - py);
      if (d < minStep && !last) {
        continue; // too close to the last drawn point
      }
      if (d > 0) {
        out.push({
          cx: (px + x) / 2,
          cy: (py + y) / 2,
          length: d,
          angle: Math.atan2(y - py, x - px),
        });
        if (out.length > MAX_SEGMENTS) {
          return null;
        }
      }
      px = x;
      py = y;
    }
  }
  return out;
}

/** The size of the box that holds the strokes (with a little room for the pen's width). */
export function strokeBounds(
  strokes: InkStroke[],
  margin = 6,
): {width: number; height: number} {
  let w = 0;
  let h = 0;
  for (const s of strokes) {
    for (let i = 0; i < s.p.length; i += 2) {
      w = Math.max(w, s.p[i]);
      h = Math.max(h, s.p[i + 1]);
    }
  }
  return {width: Math.ceil(w) + margin, height: Math.ceil(h) + margin};
}
