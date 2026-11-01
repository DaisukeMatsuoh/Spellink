import {
  boundsOf,
  freeLimit,
  matchesSignature,
  type Box,
} from '../src/freeSpace';

const selection: Box = {left: 400, top: 100, right: 900, bottom: 200};
const band = {left: 400, right: 1300};
const limit = (boxes: Box[], startY = 220) =>
  freeLimit(boxes, selection, band, startY, 1800, 20);

describe('freeLimit', () => {
  it('uses the page bottom when the page is empty', () => {
    expect(limit([])).toBe(1800);
  });

  it('ignores the question itself (inside the selection)', () => {
    expect(limit([{left: 420, top: 110, right: 880, bottom: 190}])).toBe(1800);
  });

  it('stops above the nearest element below, with a gap', () => {
    const boxes = [
      {left: 450, top: 900, right: 700, bottom: 950},
      {left: 450, top: 600, right: 700, bottom: 650},
    ];
    expect(limit(boxes)).toBe(580);
  });

  it('ignores elements outside the answer band (e.g. a column on the left)', () => {
    expect(limit([{left: 100, top: 300, right: 350, bottom: 900}])).toBe(1800);
  });

  it('ignores elements above the start line', () => {
    expect(limit([{left: 450, top: 20, right: 700, bottom: 120}])).toBe(1800);
  });

  it('leaves no room when an element already overlaps the start line', () => {
    const l = limit([{left: 450, top: 150, right: 700, bottom: 500}]);
    expect(l - 220).toBeLessThan(0);
  });
});

describe('boundsOf', () => {
  it('returns null for no points', () => {
    expect(boundsOf([])).toBeNull();
  });
  it('covers all points', () => {
    expect(
      boundsOf([
        {x: 5, y: 9},
        {x: 1, y: 20},
        {x: 7, y: 2},
      ]),
    ).toEqual({
      left: 1,
      top: 2,
      right: 7,
      bottom: 20,
    });
  });
});

describe('matchesSignature (never delete what is not ours)', () => {
  const sig = {x: 100, y: 200, n: 17};
  it('matches the stroke we drew', () => {
    expect(matchesSignature(sig, {x: 100, y: 200}, 17)).toBe(true);
    expect(matchesSignature(sig, {x: 101, y: 199}, 17)).toBe(true);
  });
  it('rejects a different start point or length', () => {
    expect(matchesSignature(sig, {x: 140, y: 200}, 17)).toBe(false);
    expect(matchesSignature(sig, {x: 100, y: 200}, 18)).toBe(false);
    expect(matchesSignature(sig, null, 17)).toBe(false);
  });
});
