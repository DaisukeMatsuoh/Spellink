import jsQR from 'jsqr';
import {
  darkRuns,
  MAX_QR_TEXT,
  qrMatrix,
  qrPatches,
  qrRects,
  qrScale,
} from '../src/qr';

// Rasterise the matrix the way the screen draws it (module = scale px, quiet zone = 4 modules).
function raster(m: boolean[][], scale: number) {
  const q = 4;
  const size = (m.length + 2 * q) * scale;
  const data = new Uint8ClampedArray(size * size * 4).fill(255);
  m.forEach((row, r) =>
    row.forEach((dark, c) => {
      if (!dark) {
        return;
      }
      for (let y = 0; y < scale; y++) {
        for (let x = 0; x < scale; x++) {
          const i = (((r + q) * scale + y) * size + (c + q) * scale + x) * 4;
          data[i] = data[i + 1] = data[i + 2] = 0;
        }
      }
    }),
  );
  return {data, size};
}

describe('qr', () => {
  it.each([
    'a',
    '123456789012',
    'spellink://pair?v=1&n=abcdef0123456789&h=aaaa:8766',
  ])('is decoded back to the text: %s', text => {
    const {data, size} = raster(qrMatrix(text), 4);
    expect(jsQR(data, size, size)?.data).toBe(text);
  });

  it('is square and has the finder pattern in the corner', () => {
    const m = qrMatrix('x');
    expect(m.every(r => r.length === m.length)).toBe(true);
    expect(m[0].slice(0, 7).every(Boolean)).toBe(true);
  });

  it('refuses empty and too long text', () => {
    expect(() => qrMatrix('')).toThrow();
    expect(() => qrMatrix('x'.repeat(MAX_QR_TEXT + 1))).toThrow();
  });

  it('darkRuns covers exactly the dark modules', () => {
    const row = [true, true, false, true, false, false, true];
    expect(darkRuns(row)).toEqual([
      [0, 2],
      [3, 1],
      [6, 1],
    ]);
    expect(darkRuns([])).toEqual([]);
    expect(darkRuns([false, false])).toEqual([]);
    expect(darkRuns([true, true])).toEqual([[0, 2]]);
  });
});

describe('qr drawing without seams', () => {
  const text = 'spellink://pair?v=1&n=0123456789abcdef&h=192.168.2.180:8766';

  it('the rectangles cover exactly the dark modules, each once', () => {
    const m = qrMatrix(text);
    const seen = m.map(r => r.map(() => 0));
    for (const b of qrRects(m)) {
      for (let r = b.row; r < b.row + b.h; r++) {
        for (let c = b.col; c < b.col + b.w; c++) {
          seen[r][c] += 1;
        }
      }
    }
    m.forEach((row, r) =>
      row.forEach((dark, c) => expect(seen[r][c]).toBe(dark ? 1 : 0)),
    );
    expect(qrRects(m).length).toBeLessThan(m.flat().filter(Boolean).length / 2);
  });

  it('on every screen density: the picture is the QR code, and each boundary between blocks is crossed by a patch', () => {
    const m = qrMatrix(text);
    for (const ratio of [1, 1.2, 1.5, 2, 2.625, 3, NaN, 0, -2]) {
      const n = m.length + 8;
      const {modulePx} = qrScale(560, n, ratio);
      expect(Number.isInteger(modulePx) && modulePx >= 1).toBe(true);
      const size = n * modulePx;
      const px = new Uint8Array(size * size); // 1 = black
      const fill = (x0: number, y0: number, w: number, h: number) => {
        for (let y = y0; y < y0 + h; y++) {
          for (let x = x0; x < x0 + w; x++) {
            px[y * size + x] = 1;
          }
        }
      };
      for (const b of qrRects(m)) {
        fill(
          (4 + b.col) * modulePx,
          (4 + b.row) * modulePx,
          b.w * modulePx,
          b.h * modulePx,
        );
      }
      const blocksOnly = Uint8Array.from(px);
      const patches = qrPatches(m);
      for (const p of patches) {
        // a patch is one physical pixel above and below the boundary
        fill(
          (4 + p.col) * modulePx,
          (4 + p.row) * modulePx - 1,
          p.w * modulePx,
          2,
        );
      }
      // the picture is exactly the QR code: no patch lies on a white module
      m.forEach((row, r) =>
        row.forEach((dark, c) => {
          for (const [x, y] of [
            [0, 0],
            [modulePx - 1, modulePx - 1],
            [0, modulePx - 1],
            [modulePx - 1, 0],
          ]) {
            expect(
              px[((4 + r) * modulePx + y) * size + (4 + c) * modulePx + x],
            ).toBe(dark ? 1 : 0);
          }
        }),
      );
      expect(px.reduce((a, b) => a + b, 0)).toBe(
        blocksOnly.reduce((a, b) => a + b, 0),
      );
      // every dark-over-dark boundary between two different blocks has a patch over it
      let boundaries = 0;
      for (let r = 1; r < m.length; r++) {
        for (let c = 0; c < m.length; c++) {
          if (m[r][c] && m[r - 1][c]) {
            const sameBlock = qrRects(m).some(
              b =>
                b.col <= c &&
                c < b.col + b.w &&
                b.row <= r - 1 &&
                r < b.row + b.h,
            );
            if (!sameBlock) {
              boundaries += 1;
              expect(
                patches.some(p => p.row === r && p.col <= c && c < p.col + p.w),
              ).toBe(true);
            }
          }
        }
      }
      expect(boundaries).toBeGreaterThan(0);
    }
  });

  it('the module is a whole number of physical pixels and fits the size', () => {
    for (const [size, n, ratio] of [
      [560, 45, 1.5],
      [560, 45, 2.625],
      [100, 45, 1],
    ] as Array<[number, number, number]>) {
      const s = qrScale(size, n, ratio);
      expect(Number.isInteger(s.modulePx)).toBe(true);
      expect(s.modulePx * n).toBeLessThanOrEqual(Math.max(n, size * ratio));
      expect(s.moduleDp * ratio).toBeCloseTo(s.modulePx, 9);
    }
  });
});
