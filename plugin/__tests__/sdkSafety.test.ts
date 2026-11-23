import {PluginManager} from 'sn-plugin-lib';
import {fakeNote} from '../__mocks__/sn-plugin-lib';
import {
  call,
  CallTimeoutError,
  closeView,
  deleteElementsAt,
  boxesOf,
  ensureOpen,
  insertPageAfter,
  native,
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

  it('a refused jump is an error by itself; inside a page insert the table makes it a recorded fallback', async () => {
    fakeNote.refuse.add('jumpToPage');
    await expect(jumpToPageIn(FILE, 1)).rejects.toThrow(/refused/);
    await insertPageAfter(FILE, 1, 0); // strokes name their page: the view not turning is no reason to stop
    expect(fakeNote.inserted).toEqual([1]);
    const t = dumpTrace();
    expect(t.some(e => e.k === 'jumpToPage.refused')).toBe(true);
    expect(t.some(e => e.k === 'pageTurn.fallback')).toBe(true);
    expect(t.some(e => e.k === 'pageTurn.dirty')).toBe(false);
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

  it('reads of strokes we already hold run together, but never more than 8 at once', async () => {
    let running = 0;
    let peak = 0;
    const tracked = (n: number) => ({
      async size() {
        peak = Math.max(peak, ++running);
        await pause(2);
        running--;
        return n;
      },
      async getRange() {
        peak = Math.max(peak, ++running);
        await pause(2);
        running--;
        return Array.from({length: n}, (_, i) => ({x: i, y: i}));
      },
    });
    const strokes = Array.from({length: 40}, (_, i) => ({
      uuid: `s${i}`,
      type: 0,
      userData: '',
      stroke: {points: tracked(3)},
    }));
    const boxes = await boxesOf(strokes as never, 5000);
    expect(boxes).toHaveLength(40);
    expect(peak).toBeGreaterThan(1); // in parallel…
    expect(peak).toBeLessThanOrEqual(8); // …within the bound
  });
});

describe('what may run beside what (readers and writers)', () => {
  const job = (
    name: string,
    ms: number,
    log: string[],
    share: 'exclusive' | 'shared',
  ) =>
    native(
      name,
      async () => {
        log.push(`${name}:start`);
        await pause(ms);
        log.push(`${name}:end`);
        return name;
      },
      share,
    );

  it('shared reads overlap each other', async () => {
    const log: string[] = [];
    await Promise.all([
      job('r1', 20, log, 'shared'),
      job('r2', 20, log, 'shared'),
    ]);
    expect(log.slice(0, 2)).toEqual(['r1:start', 'r2:start']); // both began before either ended
  });

  it('an exclusive request waits for the reads already running, and nothing behind it starts first', async () => {
    const log: string[] = [];
    await Promise.all([
      job('r1', 30, log, 'shared'),
      job('w', 5, log, 'exclusive'),
      job('r2', 5, log, 'shared'), // asked after w: must not slip in front of it
    ]);
    expect(log).toEqual([
      'r1:start',
      'r1:end',
      'w:start',
      'w:end',
      'r2:start',
      'r2:end',
    ]);
  });

  it('a read never runs beside an exclusive request', async () => {
    const log: string[] = [];
    await Promise.all([
      job('w', 20, log, 'exclusive'),
      job('r', 5, log, 'shared'),
    ]);
    expect(log).toEqual(['w:start', 'w:end', 'r:start', 'r:end']);
  });

  it('a read whose caller gave up while waiting is never sent; one already sent keeps its place until it settles', async () => {
    setTimeoutScaleForTests(0.004); // 60 ms
    const started: string[] = [];
    let finish!: () => void;
    const stuck = native('createElement', () => {
      started.push('stuck');
      return new Promise<string>(r => (finish = () => r('x')));
    });
    const read = native(
      'points.size',
      async () => {
        started.push('read');
        return 1;
      },
      'shared',
    );
    await expect(stuck).rejects.toBeInstanceOf(CallTimeoutError);
    await expect(read).rejects.toBeInstanceOf(CallTimeoutError);
    expect(started).toEqual(['stuck']);
    finish();
    await pause(5);
    setTimeoutScaleForTests(1);
    expect(await native('points.size', async () => 7, 'shared')).toBe(7);
    expect(started).toEqual(['stuck']);
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

describe('what the note app answers is checked before it is used', () => {
  const comm = () => require('../__mocks__/sn-plugin-lib').PluginCommAPI;
  const file = () => require('../__mocks__/sn-plugin-lib').PluginFileAPI;
  const answer = (result: unknown) => async () => ({success: true, result});
  const sdk = () => require('../src/sdk');
  afterEach(() => jest.restoreAllMocks());

  it.each([
    [
      'a path that is not a string',
      () =>
        jest.spyOn(comm(), 'getCurrentFilePath').mockImplementation(answer(42)),
      /path/,
    ],
    [
      'an empty path',
      () =>
        jest.spyOn(comm(), 'getCurrentFilePath').mockImplementation(answer('')),
      /path/,
    ],
    [
      'a page number that is a string',
      () =>
        jest.spyOn(comm(), 'getCurrentPageNum').mockImplementation(answer('1')),
      /page number/,
    ],
    [
      'a negative page number',
      () =>
        jest.spyOn(comm(), 'getCurrentPageNum').mockImplementation(answer(-1)),
      /page number/,
    ],
    [
      'a size with a zero side',
      () =>
        jest
          .spyOn(file(), 'getPageSize')
          .mockImplementation(answer({width: 0, height: 100})),
      /size/,
    ],
    [
      'a size that is not numbers',
      () =>
        jest
          .spyOn(file(), 'getPageSize')
          .mockImplementation(answer({width: 'a', height: 'b'})),
      /size/,
    ],
    [
      'a missing size',
      () =>
        jest.spyOn(file(), 'getPageSize').mockImplementation(answer(undefined)),
      /size/,
    ],
  ])('noteContext rejects %s', async (_n, arrange, why) => {
    arrange();
    await expect(sdk().noteContext()).rejects.toThrow(why);
  });

  it('lassoRect rejects a rectangle that is not one', async () => {
    jest
      .spyOn(comm(), 'getLassoRect')
      .mockImplementation(answer({left: 5, top: 5, right: 1, bottom: 9}));
    await expect(sdk().lassoRect()).rejects.toThrow(/rectangle/);
    jest
      .spyOn(comm(), 'getLassoRect')
      .mockImplementation(answer({left: NaN, top: 0, right: 1, bottom: 1}));
    await expect(sdk().lassoRect()).rejects.toThrow(/rectangle/);
  });

  it('recognition must give text or nothing; elements must be a list or nothing', async () => {
    fakeNote.lasso = [{type: 0} as never];
    jest
      .spyOn(comm(), 'recognizeElements')
      .mockImplementation(answer({text: 'x'}));
    await expect(sdk().recognizeLasso({width: 10, height: 10})).rejects.toThrow(
      /not text/,
    );
    jest.spyOn(comm(), 'getLassoElements').mockImplementation(answer('nope'));
    await expect(sdk().recognizeLasso({width: 10, height: 10})).rejects.toThrow(
      /not a list/,
    );
  });

  it('a pen that makes no sense falls back to the default pen, and says so in the record', async () => {
    jest
      .spyOn(comm(), 'getPenInfo')
      .mockImplementation(answer({type: 'x', color: 0, width: -3}));
    expect(await sdk().currentPen()).toEqual({type: 10, color: 0, width: 200});
    expect(dumpTrace().some(e => e.k === 'getPenInfo.unexpected')).toBe(true);
  });

  it('a page read that is not a list is an error, not an empty page', async () => {
    jest
      .spyOn(file(), 'getElements')
      .mockImplementation(answer({not: 'a list'}));
    await expect(readPage(FILE, 0)).rejects.toThrow(/not a list/);
  });

  it('isOnPage rejects a page number that is not an integer', async () => {
    jest.spyOn(comm(), 'getCurrentPageNum').mockImplementation(answer(0.5));
    await expect(sdk().isOnPage(FILE, 0)).rejects.toThrow(/page number/);
  });
});

describe('inserting a page checks the open note first', () => {
  it('another note being open means nothing is saved and nothing is inserted', async () => {
    fakeNote.currentFile = '/another.note';
    await expect(insertPageAfter(FILE, 1, 0)).rejects.toThrow(/no longer open/);
    expect(fakeNote.saves).toBe(0); // saving would have saved the other note
    expect(fakeNote.inserted).toEqual([]);
  });
});

describe('permissions and screens use the common path', () => {
  const manager = () => require('../__mocks__/sn-plugin-lib').PluginManager;
  afterEach(() => jest.restoreAllMocks());

  it('a permission asked for and granted (1) or granted this time (2) is fine; anything else is missing', async () => {
    const {ensurePermissions} = require('../src/sdk');
    jest.spyOn(manager(), 'hasPermission').mockResolvedValue(0);
    const request = jest.spyOn(manager(), 'requestPermission');
    request
      .mockResolvedValueOnce(1)
      .mockResolvedValueOnce(2)
      .mockResolvedValueOnce(0);
    expect(await ensurePermissions()).toEqual(['plugin.permission.INTERNET']);
    request.mockResolvedValue('granted' as never); // not a value the SDK documents
    expect((await ensurePermissions()).length).toBe(3);
  });

  it('waiting for the person at a permission dialog gets far longer than a quick call', () => {
    const {timeoutFor} = require('../src/sdk');
    expect(timeoutFor('requestPermission')).toBeGreaterThan(
      timeoutFor('saveCurrentNote') * 3,
    );
  });

  it('a screen that cannot be opened is recorded, not thrown', async () => {
    const {showView} = require('../src/sdk');
    jest.spyOn(manager(), 'showPluginView').mockResolvedValue(false);
    expect(await showView()).toBe(false);
    expect(dumpTrace().some(e => e.k === 'showPluginView.refused')).toBe(true);
    jest
      .spyOn(manager(), 'showPluginView')
      .mockRejectedValue(new Error('no host'));
    expect(await showView()).toBe(false);
  });
});

describe('the page procedures follow their tables (docs/spec/page-turn.md)', () => {
  const file = () => require('../__mocks__/sn-plugin-lib').PluginFileAPI;
  const comm = () => require('../__mocks__/sn-plugin-lib').PluginCommAPI;
  const events = () => dumpTrace().map(e => e.k);
  afterEach(() => jest.restoreAllMocks());

  it('a template that cannot be read or named: carry on with a blank page, and say so', async () => {
    jest
      .spyOn(file(), 'getNotePageTemplate')
      .mockRejectedValue(new Error('no'));
    await insertPageAfter(FILE, 1, 0);
    expect(fakeNote.inserted).toEqual([1]);
    expect(events()).toContain('pageTurn.fallback');
    jest.restoreAllMocks();
    resetTraceForTests();
    jest.spyOn(file(), 'getNotePageTemplate').mockResolvedValue({
      success: true,
      result: {name: 'custom', md5: 'abc123'},
    });
    await insertPageAfter(FILE, 2, 0); // a custom template cannot be referenced by name
    expect(events()).toContain('pageTurn.fallback');
  });

  it('silence from the insert may have changed the note: stop and record it as dirty', async () => {
    setTimeoutScaleForTests(0.004);
    jest
      .spyOn(file(), 'insertNotePage')
      .mockImplementation(() => new Promise(() => {}));
    await expect(insertPageAfter(FILE, 1, 0)).rejects.toBeInstanceOf(
      CallTimeoutError,
    );
    expect(events()).toContain('pageTurn.dirty');
  });

  it('a page count that did not grow is dirty too (the host said yes)', async () => {
    fakeNote.silentInsertPage = true;
    await expect(insertPageAfter(FILE, 1, 0)).rejects.toThrow(/did not grow/);
    expect(events()).toContain('pageTurn.dirty');
  });

  it('a failure before anything was inserted is clean: the note is not touched and nothing is called dirty', async () => {
    jest
      .spyOn(file(), 'getNoteTotalPageNum')
      .mockResolvedValue({success: true, result: 'many'});
    await expect(insertPageAfter(FILE, 1, 0)).rejects.toThrow(
      /not a page count/,
    );
    expect(fakeNote.inserted).toEqual([]);
    expect(events()).not.toContain('pageTurn.dirty');
  });

  it('writing: the wrong note open is clean; a refused insert is dirty', async () => {
    fakeNote.currentFile = '/another.note';
    await expect(writeStrokesAt(FILE, 0, [stroke], PEN)).rejects.toThrow(
      /no longer open/,
    );
    expect(fakeNote.strayWrites).toBe(0);
    expect(events()).not.toContain('write.dirty');
    fakeNote.currentFile = FILE;
    fakeNote.refuse.add('insertPageElements');
    await expect(writeStrokesAt(FILE, 0, [stroke], PEN)).rejects.toThrow(
      /refused/,
    );
    expect(events()).toContain('write.dirty');
  });

  it('a procedure whose table has no move for an outcome stops instead of looping', async () => {
    const {runProcedure} = require('../src/procedure');
    const table = {
      A: {
        OK: {k: 'never', note: 'x'},
        FAIL: {k: 'do', action: 'X0', next: 'END'},
        REF: {k: 'do', action: 'X0', next: 'END'},
        SIL: {k: 'do', action: 'X0', next: 'END'},
        NOP: {k: 'do', action: 'X0', next: 'END'},
        ODD: {k: 'do', action: 'X0', next: 'END'},
      },
      END: {
        OK: {k: 'never', note: 'c'},
        FAIL: {k: 'never', note: 'c'},
        REF: {k: 'never', note: 'c'},
        SIL: {k: 'never', note: 'c'},
        NOP: {k: 'never', note: 'c'},
        ODD: {k: 'never', note: 'c'},
      },
    };
    await expect(
      runProcedure({
        name: 't',
        table,
        start: 'A',
        ends: ['END'],
        steps: {A: async () => 'OK'},
        handlers: {X0: () => {}},
      }),
    ).rejects.toThrow(/stuck at A after OK/);
  });
});

describe('leaving the plugin screen', () => {
  it('goes out even when an earlier request to the note app never gets an answer (the line is stuck)', async () => {
    setTimeoutScaleForTests(0.004);
    const closing = jest.spyOn(PluginManager, 'closePluginView');
    // the host never answers this one, and everything behind it in the line waits for it
    const stuck = native(
      'getCurrentFilePath',
      () => new Promise<never>(() => {}),
    );
    const queued = native('hasPermission', async () => 1);
    await expect(stuck).rejects.toBeInstanceOf(CallTimeoutError);
    await expect(queued).rejects.toBeInstanceOf(CallTimeoutError);
    expect(await closeView()).toBe(true);
    expect(closing).toHaveBeenCalledTimes(1);
    closing.mockRestore();
  });

  it('says so when the host does not answer, a refusal, or an exception — and never throws', async () => {
    setTimeoutScaleForTests(0.004);
    const closing = jest.spyOn(PluginManager, 'closePluginView');
    closing.mockImplementationOnce(() => new Promise<never>(() => {}));
    expect(await closeView()).toBe(false);
    closing.mockImplementationOnce(async () => false);
    expect(await closeView()).toBe(false);
    closing.mockImplementationOnce(() => {
      throw new Error('boom');
    });
    expect(await closeView()).toBe(false);
    closing.mockRestore();
  });
});
