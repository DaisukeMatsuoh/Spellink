import {ratioOf, valueAt} from '../src/slider';

describe('valueAt', () => {
  it('maps the ends of the track to min and max', () => {
    expect(valueAt(0, 200, 100, 160, 5)).toBe(100);
    expect(valueAt(200, 200, 100, 160, 5)).toBe(160);
  });
  it('snaps to the step', () => {
    expect(valueAt(100, 200, 100, 160, 5)).toBe(130);
    expect(valueAt(103, 200, 100, 160, 5)).toBe(130);
  });
  it('clamps touches outside the track', () => {
    expect(valueAt(-50, 200, 100, 160, 5)).toBe(100);
    expect(valueAt(999, 200, 100, 160, 5)).toBe(160);
  });
  it('is safe before the track has been measured', () => {
    expect(valueAt(10, 0, 100, 160, 5)).toBe(100);
  });
});

describe('ratioOf', () => {
  it('places the thumb', () => {
    expect(ratioOf(100, 100, 160)).toBe(0);
    expect(ratioOf(130, 100, 160)).toBe(0.5);
    expect(ratioOf(999, 100, 160)).toBe(1);
  });
  it('handles an empty range', () => {
    expect(ratioOf(5, 5, 5)).toBe(0);
  });
});
