import {fakeNote} from '../__mocks__/sn-plugin-lib';
import {
  call,
  CallTimeoutError,
  deleteElementsAt,
  boxesOf,
  ensureOpen,
  insertPageAfter,
  jumpToPageIn,
  readPage,
  resetHostQueueForTests,
  SdkError,
  setTimeoutScaleForTests,
  writeStrokesAt,
} from '../src/sdk';
import {dumpTrace, resetTraceForTests} from '../src/trace';

const PEN = {type: 10, color: 0, width: 200};
const stroke = {p: [1, 1, 2, 2, 3, 3], w: [1, 1, 1]};
const FILE = '/note.note';
const pause = (ms: number) => new Promise(r => setTimeout(r, ms));

beforeEach(() => {
  fakeNote.reset();
  resetTraceForTests();
  resetHostQueueForTests();
});
afterEach(() => {
  setTimeoutScaleForTests(1);
  resetHostQueueForTests();
});

describe('a refusal (`success: true, result: false`) is not a success', () => {
  it('a refused save stops the page insertion: unsaved writing is never put at risk', async () => {
    fakeNote.addUserStroke(0, [{x: 1, y: 1}]);
    fakeNote.refuse.add('saveCurrentNote');
    await expect(insertPageAfter(FILE, 1, 0)).rejects.toThrow(/refused/);
    expect(fakeNote.inserted).toEqual([]); // insertNotePage was never even called
    expect(fakeNote.strokes(0)).toHaveLength(1); // the user's stroke is still there
  });

  it('a refused page insertion is an error', async () => {
    fakeNote.refuse.add('insertNotePage');
    await expect(insertPageAfter(FILE, 1, 0)).rejects.toThrow(/refused/);
  });

  it('an insertion that says yes but adds no page is caught by counting the pages', async () => {
    fakeNote.silentInsertPage = true;
    await expect(insertPageAfter(FILE, 1, 0)).rejects.toThrow(
      /page count did not grow \(1 → 1\)/,
    );
    expect(dumpTrace().some(e => e.k === 'insertNotePage.notGrown')).toBe(true);
  });

  it('a real insertion is believed only after the count grew', async () => {
    await insertPageAfter(FILE, 1, 0);
    expect(fakeNote.totalPages).toBe(2);
  });

  it('refused element insertion and deletion are errors', async () => {
    fakeNote.refuse.add('insertPageElements');
    await expect(writeStrokesAt(FILE, 0, [stroke], PEN)).rejects.toThrow(
      /refused/,
    );
    fakeNote.refuse.clear();
    fakeNote.refuse.add('deletePageElements');
    await expect(deleteElementsAt(FILE, 0, [1])).rejects.toBeInstanceOf(
      SdkError,
    );
  });

  it('a refused jump is recorded but does not stop the answer (strokes name their page)', async () => {
    fakeNote.refuse.add('jumpToPage');
    await jumpToPageIn(FILE, 1);
    expect(dumpTrace().some(e => e.k === 'jumpToPage.refused')).toBe(true);
  });

  it('the host’s boolean answers are recorded, to learn what a real device says', async () => {
    await readPage(FILE, 0);
    const save = dumpTrace().find(e => e.k === 'saveCurrentNote');
    expect(save?.d).toEqual({result: true});
  });
});

describe('requests to the note app never overlap', () => {
  it('the next one starts only after the previous one settled', async () => {
    const log: string[] = [];
    const slow = (name: string, ms: number) => async () => {
      log.push(`${name}:start`);
      await pause(ms);
      log.push(`${name}:end`);
      return {success: true, result: name};
    };
    await Promise.all([
      call('a', slow('A', 30)),
      call('b', slow('B', 1)),
      call('c', slow('C', 1)),
    ]);
    expect(log).toEqual([
      'A:start',
      'A:end',
      'B:start',
      'B:end',
      'C:start',
      'C:end',
    ]);
  });

  it('a call given up on is waited for; a request that gave up in line is never sent', async () => {
    setTimeoutScaleForTests(0.004); // quick calls: 60 ms
    let finishStuck!: () => void;
    const started: string[] = [];
    const stuck = call('createElement', () => {
      started.push('stuck');
      return new Promise(r => (finishStuck = () => r({success: true})));
    });
    const queued = call('createElement', async () => {
      started.push('queued'); // must never run: its caller gives up first
      return {success: true};
    });
    await expect(stuck).rejects.toBeInstanceOf(CallTimeoutError);
    await expect(queued).rejects.toBeInstanceOf(CallTimeoutError);
    expect(started).toEqual(['stuck']);

    finishStuck(); // the note app finally answers the old request…
    await pause(5);
    setTimeoutScaleForTests(1);
    // …and the line moves on, without ever having sent the abandoned request
    await expect(
      call('createElement', async () => ({success: true, result: 7})),
    ).resolves.toBe(7);
    expect(started).toEqual(['stuck']);
  });

  it('element scanning reads one stroke at a time', async () => {
    let running = 0;
    let peak = 0;
    const tracked = (n: number) => ({
      async size() {
        peak = Math.max(peak, ++running);
        await pause(1);
        running--;
        return n;
      },
      async getRange() {
        peak = Math.max(peak, ++running);
        await pause(1);
        running--;
        return Array.from({length: n}, (_, i) => ({x: i, y: i}));
      },
    });
    const strokes = Array.from({length: 8}, (_, i) => ({
      uuid: `s${i}`,
      type: 0,
      userData: '',
      stroke: {points: tracked(3)},
    }));
    const boxes = await boxesOf(strokes as never, 5000);
    expect(boxes).toHaveLength(8);
    expect(peak).toBe(1);
  });
});

describe('a stuck note app is not mistaken for something else', () => {
  it('ensureOpen reports the silence, not "the note was closed"', async () => {
    setTimeoutScaleForTests(0.004);
    const api = require('../__mocks__/sn-plugin-lib').PluginCommAPI;
    jest
      .spyOn(api, 'getCurrentFilePath')
      .mockImplementationOnce(() => new Promise(() => {}));
    await expect(ensureOpen(FILE)).rejects.toBeInstanceOf(CallTimeoutError);
  });
});
