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

// The animation must remove exactly the strokes it drew, however the note behaves.
describe('InkDrops', () => {
  const pen = {type: 10, color: 0, width: 200};
  const pts = (x: number, y: number, n = 17) =>
    Array.from({length: n}, (_, i) => ({x: x + i, y}));
  const make = () => new InkDrops('/note.note', 0, 100, 600, 500, 40, pen);
  const tick = (d: InkDrops) =>
    (d as unknown as {tick(): Promise<void>}).tick();
  const waveOnPage = () =>
    fakeNote.strokes(0).filter(e => e.userData.startsWith('spellink-wave:'));

  beforeEach(() => fakeNote.reset());

  it('tags every piece with one shared marker', async () => {
    const d = make();
    await tick(d);
    await tick(d);
    expect(waveOnPage()).toHaveLength(2);
    expect(new Set(waveOnPage().map(e => e.userData)).size).toBe(1);
    await d.stop();
  });

  it('removes all of its pieces and leaves everything else', async () => {
    const user = [
      fakeNote.addUserStroke(0, pts(150, 80)),
      fakeNote.addUserStroke(0, pts(150, 400)),
    ];
    const d = make();
    for (let i = 0; i < 4; i++) {
      await tick(d);
    }
    await d.stop();
    expect(fakeNote.strokes(0)).toEqual(user);
  });

  it('removes everything after wrapping around the line many times', async () => {
    const user = fakeNote.addUserStroke(0, pts(150, 900));
    const d = make();
    for (let i = 0; i < 40; i++) {
      await tick(d);
    }
    await d.stop();
    expect(fakeNote.strokes(0)).toEqual([user]);
  });

  it('never touches strokes it did not draw — same place, same shape, even same point count', async () => {
    const lookalike = fakeNote.addUserStroke(
      0,
      wavePiece(100, 144, 500, 8.8, 64).p.reduce<{x: number; y: number}[]>(
        (acc, v, i, a) => (i % 2 === 0 ? [...acc, {x: v, y: a[i + 1]}] : acc),
        [],
      ),
    );
    const tagged = fakeNote.addUserStroke(0, pts(10, 10));
    tagged.userData = 'my-own-tag';
    const d = make();
    await tick(d);
    await d.stop();
    expect(fakeNote.strokes(0)).toEqual([lookalike, tagged]);
  });

  it("does not remove another animation's strokes", async () => {
    const other = make();
    await tick(other);
    const mine = make();
    await tick(mine);
    await mine.stop();
    expect(waveOnPage()).toHaveLength(1);
    await other.stop();
    expect(waveOnPage()).toHaveLength(0);
  });

  it('does nothing when it drew nothing', async () => {
    const user = fakeNote.addUserStroke(0, pts(150, 80));
    await make().stop();
    expect(fakeNote.strokes(0)).toEqual([user]);
    expect(fakeNote.deleted).toHaveLength(0);
  });

  it('saves before reading the page, so strokes still in the cache are found', async () => {
    const d = make();
    await tick(d); // only in the cache, not yet in the file
    expect(fakeNote.saved(0)).toHaveLength(0);
    await d.stop();
    expect(fakeNote.saves).toBeGreaterThan(0);
    expect(waveOnPage()).toHaveLength(0);
  });

  it('deletes highest numbers first, so a sequential delete cannot shift the rest', async () => {
    const d = make();
    for (let i = 0; i < 3; i++) {
      await tick(d);
    }
    await d.stop();
    expect(fakeNote.deleted).toEqual(
      [...fakeNote.deleted].sort((a, b) => b - a),
    );
  });
});
