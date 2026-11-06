import {call} from '../src/sdk';
import {
  dumpTrace,
  flushTrace,
  resetTraceForTests,
  setTraceSink,
  trace,
} from '../src/trace';

describe('flight recorder', () => {
  beforeEach(resetTraceForTests);

  it('records every SDK call with its duration, and failures with their error', async () => {
    await call('okCall', () => Promise.resolve({success: true, result: 1}));
    await expect(
      call('badCall', () =>
        Promise.resolve({
          success: false,
          error: {code: 105, message: 'No note-related app is bound!'},
        }),
      ),
    ).rejects.toThrow();
    const [ok, bad] = dumpTrace();
    expect(ok).toMatchObject({k: 'okCall'});
    expect(ok.err).toBeUndefined();
    expect(bad).toMatchObject({k: 'badCall'});
    expect(bad.err).toContain('105');
  });

  it('keeps only the most recent events', () => {
    for (let i = 0; i < 500; i++) {
      trace(`e${i}`);
    }
    const events = dumpTrace();
    expect(events).toHaveLength(400);
    expect(events[0].k).toBe('e100');
  });

  it('sends events once to the sink, in chunks, and survives a dead connection', () => {
    const batches: number[] = [];
    setTraceSink(events => batches.push(events.length));
    for (let i = 0; i < 250; i++) {
      trace('x');
    }
    flushTrace();
    flushTrace(); // nothing new → nothing sent
    expect(batches).toEqual([100, 100, 50]);
    setTraceSink(() => {
      throw new Error('socket closed');
    });
    trace('y');
    expect(() => flushTrace()).not.toThrow();
    expect(dumpTrace().some(e => e.k === 'y')).toBe(true);
  });
});

import {
  CallTimeoutError,
  resetHostQueueForTests,
  setTimeoutScaleForTests,
  timeoutFor,
} from '../src/sdk';

describe('a call the host never answers', () => {
  afterEach(() => {
    setTimeoutScaleForTests(1);
    resetHostQueueForTests();
  });

  it('is given up on, recorded, and reported as a timeout', async () => {
    resetTraceForTests();
    setTimeoutScaleForTests(0.001); // 15 s → 15 ms
    const never = () => new Promise(() => {});
    await expect(call('createElement', never)).rejects.toBeInstanceOf(
      CallTimeoutError,
    );
    const [e] = dumpTrace();
    expect(e.k).toBe('createElement');
    expect(e.err).toContain('did not respond');
  });

  it('allows saving and reading a big note far longer than a quick call', () => {
    for (const api of [
      'saveCurrentNote',
      'getElements',
      'insertNotePage',
      'recognizeElements',
    ]) {
      expect(timeoutFor(api)).toBeGreaterThan(timeoutFor('createElement') * 3);
    }
    expect(timeoutFor('insertPageElements')).toBe(timeoutFor('createElement'));
  });
});
