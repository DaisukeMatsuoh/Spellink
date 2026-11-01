import {fakeNote} from '../__mocks__/sn-plugin-lib';
import {InkDrops, wavePiece} from '../src/inkDrops';

describe('wavePiece', () => {
  it('starts and ends where asked and has matching pressure data', () => {
    const {p, w} = wavePiece(100, 150, 500, 10, 60);
    expect(p[0]).toBe(100);
    expect(p[p.length - 2]).toBe(150);
    expect(p.length / 2).toBe(w.length);
  });
  it('stays within the amplitude around the baseline', () => {
    const {p} = wavePiece(0, 200, 500, 10, 60);
    for (let i = 1; i < p.length; i += 2) {
      expect(Math.abs(p[i] - 500)).toBeLessThanOrEqual(10);
    }
  });
  it('uses integer points and pressures within the digitizer range', () => {
    const {p, w} = wavePiece(0, 77, 123, 9, 61);
    expect(p.every(Number.isInteger)).toBe(true);
    expect(w.every(v => v > 0 && v <= 4096)).toBe(true);
  });
});

// The thinking animation must remove its own strokes and nothing else.
describe('InkDrops', () => {
  const pen = {type: 10, color: 0, width: 200};
  const userPoints = (x: number, y: number) =>
    Array.from({length: 17}, (_, i) => ({x: x + i, y})); // same point count as a wave piece
  const make = (base: number) =>
    new InkDrops('/note.note', 0, 100, 600, 500, 40, pen, base);
  const tick = (d: InkDrops) =>
    (d as unknown as {tick(): Promise<void>}).tick();

  beforeEach(() => fakeNote.reset());

  it('draws pieces and removes them all when stopped, leaving the user strokes', async () => {
    const user = [
      fakeNote.addUserStroke(0, userPoints(150, 80)),
      fakeNote.addUserStroke(0, userPoints(150, 400)),
    ];
    const d = make(2);
    for (let i = 0; i < 4; i++) {
      await tick(d);
    }
    expect(fakeNote.strokes(0)).toHaveLength(6);
    await d.stop();
    expect(fakeNote.strokes(0)).toEqual(user);
    expect(d.drawn).toBe(4);
    expect(d.removed).toBe(4);
  });

  it('never deletes a user stroke written after the animation started, even in the wave area', async () => {
    const d = make(0);
    await tick(d);
    const late = fakeNote.addUserStroke(0, userPoints(100, 500)); // starts exactly where the wave starts
    late.stroke!.points.data.push({x: 999, y: 999}); // …but has a different point count
    await tick(d);
    await d.stop();
    expect(fakeNote.strokes(0)).toEqual([late]);
  });

  it('never deletes strokes that were on the page before it started', async () => {
    // Even if numbering were wrong, pre-existing strokes are protected by the baseline.
    const existing = fakeNote.addUserStroke(
      0,
      wavePiece(100, 144, 500, 8.8, 64).p.reduce<{x: number; y: number}[]>(
        (acc, v, i, a) => (i % 2 === 0 ? [...acc, {x: v, y: a[i + 1]}] : acc),
        [],
      ),
    );
    const d = make(1);
    await tick(d);
    await d.stop();
    expect(fakeNote.strokes(0)).toEqual([existing]);
  });

  it('wipes and restarts when the wave reaches the end of the line', async () => {
    const d = make(0);
    for (let i = 0; i < 40; i++) {
      await tick(d);
    }
    expect(d.removed).toBeGreaterThan(0);
    await d.stop();
    expect(fakeNote.strokes(0)).toHaveLength(0);
  });

  it('removes everything after the wave wrapped around the line, even if deletion renumbers as it goes', async () => {
    const user = [
      fakeNote.addUserStroke(0, userPoints(150, 80)),
      fakeNote.addUserStroke(0, userPoints(150, 900)),
    ];
    const d = make(2);
    for (let i = 0; i < 40; i++) {
      await tick(d); // far more than one pass of the line
    }
    await d.stop();
    expect(fakeNote.strokes(0)).toEqual(user);
  });

  it('the second pass is distinguishable from the first (different signatures)', () => {
    const first = wavePiece(100, 144, 500, 9, 64, 0);
    const second = wavePiece(100, 144, 500, 9, 64, 1);
    expect([second.p[0], second.p[1]]).not.toEqual([first.p[0], first.p[1]]);
  });

  it('keeps going until nothing of ours is left when a delete does not take effect', async () => {
    const d = make(0);
    for (let i = 0; i < 3; i++) {
      await tick(d);
    }
    fakeNote.ignoreNextDelete = true; // the first delete reports success but removes nothing
    await d.stop();
    expect(fakeNote.strokes(0)).toHaveLength(0);
  });

  describe('marker in Element.userData', () => {
    const marked = () =>
      fakeNote.strokes(0).filter(e => e.userData.startsWith('spellink-wave:'));

    it('tags every stroke it draws', async () => {
      const d = make(0);
      await tick(d);
      await tick(d);
      expect(marked()).toHaveLength(2);
      expect(new Set(marked().map(e => e.userData)).size).toBe(1); // one marker per animation
      await d.stop();
    });

    it('finds its strokes by the marker without reading any points', async () => {
      const d = make(0);
      for (let i = 0; i < 3; i++) {
        await tick(d);
      }
      const reads = jest.spyOn(fakeNote.strokes(0)[0].stroke!.points, 'size');
      await d.stop();
      expect(fakeNote.strokes(0)).toHaveLength(0);
      expect(reads).not.toHaveBeenCalled();
    });

    it('falls back to the point signature when the device does not keep userData', async () => {
      fakeNote.dropUserData = true;
      const user = fakeNote.addUserStroke(0, userPoints(150, 80));
      const d = make(1);
      for (let i = 0; i < 3; i++) {
        await tick(d);
      }
      expect(marked()).toHaveLength(0);
      await d.stop();
      expect(fakeNote.strokes(0)).toEqual([user]);
    });

    it("ignores another animation's strokes and anything the user tagged themselves", async () => {
      const other = make(0);
      await tick(other); // a different run, different marker
      const mine = make(0);
      const theirs = fakeNote.addUserStroke(0, [
        {x: 5, y: 5},
        {x: 6, y: 6},
      ]);
      theirs.userData = 'user-note';
      const before = fakeNote.strokes(0).length;
      await mine.stop(); // nothing drawn → nothing to remove
      expect(fakeNote.strokes(0)).toHaveLength(before);
      await other.stop();
      expect(fakeNote.strokes(0)).toEqual([theirs]);
    });
  });
});
