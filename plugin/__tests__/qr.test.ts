import jsQR from 'jsqr';
import {darkRuns, MAX_QR_TEXT, qrMatrix} from '../src/qr';

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
