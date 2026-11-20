import {MAX_SEGMENTS, strokeBounds, strokeSegments} from '../src/pairStroke';

const stroke = (p: number[]) => ({
  p,
  w: p.map(() => 100).slice(0, p.length / 2),
});

describe('drawing handwriting as short pieces', () => {
  it('makes one piece per step, with its middle, length and direction', () => {
    const s = strokeSegments([stroke([0, 0, 10, 0, 10, 10])], 1)!;
    expect(s).toHaveLength(2);
    expect(s[0]).toEqual({cx: 5, cy: 0, length: 10, angle: 0});
    expect(s[1].cx).toBe(10);
    expect(s[1].cy).toBe(5);
    expect(s[1].length).toBe(10);
    expect(s[1].angle).toBeCloseTo(Math.PI / 2);
  });

  it('skips points that are too close, but always reaches the end of the stroke', () => {
    const s = strokeSegments(
      [stroke([0, 0, 2, 0, 4, 0, 6, 0, 8, 0, 10, 0])],
      5,
    )!;
    expect(s.map(x => x.length)).toEqual([6, 4]); // 0→6 (the first point at least 5 away), 6→10 (the end)
    const last = s[s.length - 1];
    expect(last.cx + last.length / 2).toBe(10);
  });

  it('draws nothing for a point that does not move, and for no strokes', () => {
    expect(strokeSegments([stroke([3, 3, 3, 3])], 1)).toEqual([]);
    expect(strokeSegments([], 8)).toEqual([]);
    expect(strokeSegments([stroke([3, 3])], 8)).toEqual([]);
  });

  it('keeps strokes apart: nothing joins the end of one to the start of the next', () => {
    const s = strokeSegments(
      [stroke([0, 0, 10, 0]), stroke([100, 100, 110, 100])],
      1,
    )!;
    expect(s).toHaveLength(2);
    expect(s[1].cx).toBe(105);
  });

  it('refuses a drawing that would need too many pieces', () => {
    const long: number[] = [];
    for (let i = 0; i <= MAX_SEGMENTS + 1; i++) {
      long.push(i * 10, 0);
    }
    expect(strokeSegments([stroke(long)], 1)).toBeNull();
    expect(
      strokeSegments([stroke(long.slice(0, MAX_SEGMENTS * 2))], 1),
    ).toHaveLength(MAX_SEGMENTS - 1);
  });

  it('measures the box that holds the writing, with room for the pen', () => {
    expect(strokeBounds([stroke([0, 0, 30, 40, 60, 0])])).toEqual({
      width: 66,
      height: 46,
    });
    expect(strokeBounds([stroke([0, 0, 30.2, 10.1])], 0)).toEqual({
      width: 31,
      height: 11,
    });
    expect(strokeBounds([])).toEqual({width: 6, height: 6});
  });
});
