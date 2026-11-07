import {fakeNote} from '../__mocks__/sn-plugin-lib';
import {AskFlow} from '../src/askFlow';
import {InkDrops, sweepLeftovers, waveNumsOn} from '../src/inkDrops';
import {leftoversOn, resetLeftoversForTests} from '../src/leftovers';
import {readPage, resetHostQueueForTests} from '../src/sdk';
import {dumpTrace, resetTraceForTests} from '../src/trace';

const FILE = '/note.note';
const PEN = {type: 10, color: 0, width: 200};
const pause = (ms: number) => new Promise(r => setTimeout(r, ms));
/** Draws one piece of wave, as a tick does. */
const drawOne = (d: InkDrops) =>
  (d as unknown as {tick: () => Promise<void>}).tick();
const waveCount = (page = 0) =>
  fakeNote.strokes(page).filter(e => e.userData.startsWith('spellink-wave:'))
    .length;

beforeEach(() => {
  fakeNote.reset();
  resetLeftoversForTests();
  resetTraceForTests();
  resetHostQueueForTests();
});

describe('wave strokes that cannot be removed are remembered, then swept', () => {
  it('a refused removal leaves the strokes on the page and records where', async () => {
    const drops = new InkDrops(FILE, 0, 100, 800, 500, 40, PEN);
    await drawOne(drops);
    await drawOne(drops);
    expect(waveCount()).toBe(2);
    fakeNote.refuse.add('deletePageElements');
    await drops.stop();
    expect(waveCount()).toBe(2); // could not be removed
    expect(leftoversOn(FILE, 0)).toHaveLength(1);
    expect(dumpTrace().some(e => e.k === 'wave.leftover')).toBe(true);
  });

  it('the next question on that page removes them, and nothing of the user’s', async () => {
    const drops = new InkDrops(FILE, 0, 100, 800, 500, 40, PEN);
    await drawOne(drops);
    fakeNote.refuse.add('deletePageElements');
    await drops.stop();
    fakeNote.refuse.clear();
    fakeNote.addUserStroke(0, [{x: 200, y: 700}]);

    const stale = waveNumsOn(await readPage(FILE, 0));
    expect(stale).toHaveLength(1);
    await sweepLeftovers(FILE, 0, stale);

    expect(waveCount()).toBe(0);
    expect(fakeNote.strokes(0).some(e => e.uuid.startsWith('user-'))).toBe(
      true,
    );
    expect(leftoversOn(FILE, 0)).toHaveLength(0);
  });

  it('a sweep that fails keeps the memory and is recorded; it does not throw', async () => {
    const drops = new InkDrops(FILE, 0, 100, 800, 500, 40, PEN);
    await drawOne(drops);
    fakeNote.refuse.add('deletePageElements');
    await drops.stop();
    const stale = waveNumsOn(await readPage(FILE, 0));
    await sweepLeftovers(FILE, 0, stale); // still refused
    expect(leftoversOn(FILE, 0)).toHaveLength(1);
    expect(dumpTrace().some(e => e.k === 'wave.sweepFailed')).toBe(true);
  });

  it('a page with nothing left makes the memory of it go away', async () => {
    const drops = new InkDrops(FILE, 0, 100, 800, 500, 40, PEN);
    await drawOne(drops);
    fakeNote.refuse.add('deletePageElements');
    await drops.stop();
    fakeNote.pages.set(0, []); // the person erased it by hand
    fakeNote.refuse.clear();
    await sweepLeftovers(FILE, 0, waveNumsOn(await readPage(FILE, 0)));
    expect(leftoversOn(FILE, 0)).toHaveLength(0);
  });

  it('only the prefix identifies wave strokes: user strokes, and ones with other data, are never swept', async () => {
    const u = fakeNote.addUserStroke(0, [{x: 1, y: 1}]);
    u.userData = 'something-else';
    fakeNote.addUserStroke(0, [{x: 2, y: 2}]);
    expect(waveNumsOn(await readPage(FILE, 0))).toEqual([]);
  });
});

describe('a failure while drawing removes what was drawn, right away', () => {
  it('the removal succeeds: nothing is left to remember', async () => {
    const drops = new InkDrops(FILE, 0, 100, 800, 500, 40, PEN);
    await drawOne(drops);
    await (
      drops as unknown as {giveUp: (e: Error, c: boolean) => Promise<void>}
    ).giveUp(new Error('boom'), false);
    expect(waveCount()).toBe(0);
    expect(leftoversOn(FILE, 0)).toHaveLength(0);
  });

  it('the removal fails too: it is remembered once, not retried', async () => {
    const drops = new InkDrops(FILE, 0, 100, 800, 500, 40, PEN);
    await drawOne(drops);
    fakeNote.refuse.add('deletePageElements');
    const deletes = jest.spyOn(
      require('../__mocks__/sn-plugin-lib').PluginCommAPI,
      'deletePageElements',
    );
    await (
      drops as unknown as {giveUp: (e: Error, c: boolean) => Promise<void>}
    ).giveUp(new Error('boom'), false);
    expect(deletes).toHaveBeenCalledTimes(1);
    expect(leftoversOn(FILE, 0)).toHaveLength(1);
    deletes.mockRestore();
  });

  it('a note that is no longer open is remembered, not written to', async () => {
    const drops = new InkDrops(FILE, 0, 100, 800, 500, 40, PEN);
    await drawOne(drops);
    fakeNote.currentFile = '/another.note';
    await drops.stop();
    expect(fakeNote.strayDeletes).toBe(0);
    expect(leftoversOn(FILE, 0)).toHaveLength(1);
  });
});

describe('the lasso flow gives the page scan the selection', () => {
  it('existing writing below the selection limits where the answer may go', async () => {
    fakeNote.lasso = [{type: 0} as never];
    fakeNote.ocrResult = '質問';
    fakeNote.addUserStroke(
      0,
      Array.from({length: 5}, (_, i) => ({x: 200 + i * 20, y: 700 + i})),
    );
    let sent: {startLimit?: number} | null = null;
    jest
      .spyOn(require('../src/bridgeClient'), 'connectFirst')
      .mockImplementationOnce(
        async () =>
          ({
            name: 'fake',
            settings: {
              showProgress: false,
              lineTilt: true,
              thinkingAnimation: false,
              busyNotice: true,
            },
            onMessage: () => {},
            onClose: () => {},
            onBad: () => {},
            close: () => {},
            send: (m: {type: string; layout: {startLimit?: number}}) => {
              if (m.type === 'ask') {
                sent = m.layout;
              }
            },
          } as never),
      );
    const flow = new AskFlow(() => {});
    void flow.run();
    await pause(400);
    flow.cancel();
    await pause(50);
    expect(sent).not.toBeNull();
    expect(sent!.startLimit).toBeDefined();
    expect(sent!.startLimit!).toBeLessThan(700);
  });
});
