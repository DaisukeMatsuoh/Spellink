// Pure geometry for placing the answer without writing over existing content.

export interface Box {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/** The point sets of the Supernote elements, reduced to what placement needs. */
export function boundsOf(points: {x: number; y: number}[]): Box | null {
  if (points.length === 0) {
    return null;
  }
  let left = Infinity;
  let top = Infinity;
  let right = -Infinity;
  let bottom = -Infinity;
  for (const p of points) {
    left = Math.min(left, p.x);
    right = Math.max(right, p.x);
    top = Math.min(top, p.y);
    bottom = Math.max(bottom, p.y);
  }
  return {left, top, right, bottom};
}

const overlapsX = (a: Box, left: number, right: number) =>
  a.right > left && a.left < right;

/**
 * The lowest y the answer may reach on the starting page.
 *
 * `boxes` are the page's elements. Anything inside the lasso selection (the question itself) is
 * ignored. An element is an obstacle when it overlaps the answer's horizontal band and reaches
 * below `startY`; the answer must stop `gap` above the nearest one. An element already
 * overlapping `startY` leaves no room at all.
 */
export function freeLimit(
  boxes: Box[],
  selection: Box,
  band: {left: number; right: number},
  startY: number,
  pageBottom: number,
  gap: number,
): number {
  const tolerance = 4;
  let limit = pageBottom;
  for (const b of boxes) {
    const isQuestion =
      b.left >= selection.left - tolerance &&
      b.right <= selection.right + tolerance &&
      b.top >= selection.top - tolerance &&
      b.bottom <= selection.bottom + tolerance;
    if (
      isQuestion ||
      !overlapsX(b, band.left, band.right) ||
      b.bottom <= startY
    ) {
      continue;
    }
    limit = Math.min(limit, Math.max(b.top, startY) - gap);
  }
  return limit;
}

/** A stroke we drew, remembered by its first point and point count. */
export interface Signature {
  x: number;
  y: number;
  n: number;
}

/** True when a stroke read back from the page is the one described by `sig`. */
export function matchesSignature(
  sig: Signature,
  first: {x: number; y: number} | null,
  count: number,
): boolean {
  return (
    first !== null &&
    count === sig.n &&
    Math.abs(first.x - sig.x) <= 2 &&
    Math.abs(first.y - sig.y) <= 2
  );
}
