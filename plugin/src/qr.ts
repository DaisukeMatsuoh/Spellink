// QR code as a boolean matrix (true = dark module). Used on the pairing screen, where the
// device shows a one-time code that the computer reads with its camera.
import qrcode from 'qrcode-generator';

/** Longest text we accept; a pairing code is far shorter, and a bigger code is hard to scan on e-ink. */
export const MAX_QR_TEXT = 200;

export type QrMatrix = boolean[][];

/** The QR matrix for `text` (error correction M). Throws on empty or too long text. */
export function qrMatrix(text: string): QrMatrix {
  if (text.length === 0 || text.length > MAX_QR_TEXT) {
    throw new Error(`qr: text length ${text.length} not in 1..${MAX_QR_TEXT}`);
  }
  const qr = qrcode(0, 'M');
  qr.addData(text);
  qr.make();
  const n = qr.getModuleCount();
  if (!Number.isInteger(n) || n < 21 || n > 177) {
    throw new Error(`qr: module count ${n} out of range`);
  }
  const rows: QrMatrix = [];
  for (let r = 0; r < n; r++) {
    const row: boolean[] = [];
    for (let c = 0; c < n; c++) {
      row.push(qr.isDark(r, c));
    }
    rows.push(row);
  }
  return rows;
}

/** Runs of dark modules per row, as [start, length] pairs; fewer views to draw than one per module. */
export function darkRuns(row: boolean[]): Array<[number, number]> {
  const runs: Array<[number, number]> = [];
  let start = -1;
  for (let i = 0; i <= row.length; i++) {
    const dark = i < row.length && row[i];
    if (dark && start < 0) {
      start = i;
    } else if (!dark && start >= 0) {
      runs.push([start, i - start]);
      start = -1;
    }
  }
  return runs;
}

/** A block of dark modules, in module units. */
export interface QrRect {
  col: number;
  row: number;
  w: number;
  h: number;
}

/**
 * The dark modules as few rectangles as possible without gaps between neighbours: a run of dark
 * modules in a row, repeated in the rows below it, is one rectangle (a solid corner of a finder
 * pattern is one view, so there is no seam to see inside it).
 */
export function qrRects(matrix: QrMatrix): QrRect[] {
  const out: QrRect[] = [];
  let open = new Map<string, QrRect>(); // runs of the row above that may still grow downward
  matrix.forEach((row, r) => {
    const next = new Map<string, QrRect>();
    for (const [col, w] of darkRuns(row)) {
      const key = `${col}:${w}`;
      const rect = open.get(key);
      if (rect) {
        rect.h += 1;
        next.set(key, rect);
      } else {
        const fresh = {col, row: r, w, h: 1};
        out.push(fresh);
        next.set(key, fresh);
      }
    }
    open = next;
  });
  return out;
}

/**
 * Where two rectangles that touch end to end share a boundary, drawn from a screen that may place
 * each edge on a different pixel: `row` is the module row whose top edge is the boundary, and the
 * patch spans the columns that are dark on both sides of it. A patch lies wholly on dark modules,
 * so it never changes the picture; it only covers a hairline that would otherwise show.
 */
export function qrPatches(
  matrix: QrMatrix,
): Array<{col: number; w: number; row: number}> {
  const out: Array<{col: number; w: number; row: number}> = [];
  for (let r = 1; r < matrix.length; r++) {
    const above = darkRuns(matrix[r - 1]);
    for (const [bc, bl] of darkRuns(matrix[r])) {
      for (const [ac, al] of above) {
        const from = Math.max(ac, bc);
        const to = Math.min(ac + al, bc + bl);
        // the same run above and below is one rectangle (see qrRects): no boundary there
        if (to > from && !(ac === bc && al === bl)) {
          out.push({col: from, w: to - from, row: r});
        }
      }
    }
  }
  return out;
}

/**
 * A module of whole physical pixels: `size` is in layout units and the screen has `ratio` pixels
 * to each, so a module that is a whole number of layout units can still fall between pixels.
 */
export function qrScale(
  size: number,
  modules: number,
  ratio: number,
): {modulePx: number; moduleDp: number; pixelDp: number} {
  const r = Number.isFinite(ratio) && ratio > 0 ? ratio : 1;
  const modulePx = Math.max(1, Math.floor((size * r) / modules));
  return {modulePx, moduleDp: modulePx / r, pixelDp: 1 / r};
}
