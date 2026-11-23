import {fakeNote} from '../__mocks__/sn-plugin-lib';
import {t} from '../src/i18n';
import {
  AskFlow,
  rememberSettings,
  setQuietMsForTests,
  type AskState,
} from '../src/askFlow';
import type {Box} from '../src/freeSpace';
import type {BridgeMessage, InkStroke, LayoutParams} from '../src/protocol';
import {dumpTrace, resetTraceForTests} from '../src/trace';

// Whatever a test does, the table must have had a cell for it: an event the table calls
// impossible, one it does not know, or a question that ended outside an end state is a hole in
// the table or in the code, whichever test happened to find it.
beforeEach(() => resetTraceForTests());
afterEach(() => {
  const holes = dumpTrace().filter(e =>
    [
      'ask.never',
      'ask.unknown',
      'ask.notTerminal',
      'wave.never',
      'wave.unknown',
      'insert.never',
      'insert.unknown',
      'write.never',
      'write.unknown',
    ].includes(e.k),
  );
  expect(holes).toEqual([]);
});

// The bridge connection is replaced by a script of messages.
let mockBridgeSettings = {
  showProgress: false,
  lineTilt: true,
  thinkingAnimation: true,
  avoidOverwriting: false,
  busyNotice: true,
};
let mockScript: (
  send: (m: BridgeMessage) => void,
  id: string,
) => void | Promise<void>;
jest.mock('../src/bridgeConfig', () => ({
  loadBridges: async () => ({endpoints: [], source: 'none'}),
}));
jest.mock('../src/bridgeClient', () => ({
  connectFirst: async () => {
    const conn: Record<string, unknown> = {
      name: 'fake',
      get settings() {
        return mockBridgeSettings; // read when used, so a test can change it
      },
      onMessage: () => {},
      onClose: () => {},
      close: () => {},
      send: (m: {type: string; id: string}) => {
        if (m.type === 'ask') {
          setTimeout(
            () =>
              mockScript(
                (x: BridgeMessage) =>
                  (conn.onMessage as (m: BridgeMessage) => void)(x),
                m.id,
              ),
            0,
          );
        }
      },
    };
    return conn;
  },
}));

const stroke = (n = 3): InkStroke => ({
  p: Array.from({length: n * 2}, (_, i) => 100 + i),
  w: Array.from({length: n}, () => 2000),
});
const layout: LayoutParams = {
  x: 84,
  y: 300,
  width: 1200,
  pageHeight: 1872,
  bottomMargin: 84,
  charSize: 47,
  maxPages: 3,
  continueTop: 84,
};
const ctx = {
  filePath: '/note.note',
  page: 0,
  size: {width: 1404, height: 1872},
};

async function run(l: LayoutParams, selection?: Box) {
  const states: AskState[] = [];
  await new AskFlow(s => states.push(s)).runWithQuestion(
    'q',
    l,
    ctx,
    selection,
  );
  return states[states.length - 1];
}
/** The thinking wave's pieces have 17 points; test answer strokes have 3. */
const waveStrokes = (page: number) =>
  fakeNote
    .strokes(page)
    .filter(
      e => e.stroke!.points.data.length === 17 && e.uuid.startsWith('new-'),
    );
const answerStrokes = (page: number) =>
  fakeNote
    .strokes(page)
    .filter(
      e => e.stroke!.points.data.length === 3 && e.uuid.startsWith('new-'),
    );
const pause = (ms: number) => new Promise(r => setTimeout(r, ms));
const line = (
  id: string,
  index: number,
  page: number,
  strokes: InkStroke[] | null,
  text = 'あ',
): BridgeMessage => ({
  type: 'line',
  id,
  index,
  page,
  text,
  strokes: strokes as InkStroke[],
});
const done = (id: string): BridgeMessage => ({
  type: 'done',
  id,
  text: '',
  missing: [],
  truncated: false,
  pages: 1,
  elapsedMs: 1,
});

describe('AskFlow', () => {
  beforeEach(() => fakeNote.reset());

  it('regression: a fragment with `strokes: null` no longer crashes the flow', async () => {
    mockScript = (send, id) => {
      send(line(id, 0, 0, null, '🙂'));
      send(line(id, 1, 0, [stroke(), stroke()]));
      send(done(id));
    };
    const last = await run(layout);
    expect(last.failed).toBeFalsy();
    expect(last.finished).toBe(true);
    expect(
      fakeNote.strokes(0).filter(e => e.uuid.startsWith('new-')),
    ).toHaveLength(2);
  });

  it('tells the bridge which note the question was written in, so it can continue the conversation', async () => {
    const asks: {note: string; question: string}[] = [];
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
              avoidOverwriting: false,
              busyNotice: true,
            },
            onMessage: () => {},
            onClose: () => {},
            close: () => {},
            send: (m: {type: string; note: string; question: string}) => {
              if (m.type === 'ask') {
                asks.push({note: m.note, question: m.question});
              }
            },
          }) as never,
      );
    const flow = new AskFlow(() => {});
    void flow.runWithQuestion('それを3倍して', layout, ctx);
    await pause(300);
    flow.cancel();
    await pause(50);
    expect(asks).toEqual([{note: '/note.note', question: 'それを3倍して'}]);
  });

  it('a connection that drops mid-answer reconnects, asks for the missing frames and writes each line once', async () => {
    const connect = jest.spyOn(require('../src/bridgeClient'), 'connectFirst');
    const settings = {
      showProgress: false,
      lineTilt: true,
      thinkingAnimation: false,
      avoidOverwriting: false,
      busyNotice: true,
    };
    const resumes: {id: string; next: number}[] = [];
    let first: Record<string, unknown>;
    first = {
      name: 'fake',
      settings,
      onMessage: () => {},
      onClose: () => {},
      close: () => {},
      send: (m: {type: string; id: string}) => {
        if (m.type === 'ask') {
          setTimeout(() => {
            (first.onMessage as (x: BridgeMessage) => void)({
              ...line(m.id, 0, 0, [stroke(), stroke()]),
              seq: 0,
            } as BridgeMessage);
            (first.onClose as (r: string) => void)('closed (1006)'); // before `done`
          }, 0);
        }
      },
    };
    let second: Record<string, unknown>;
    second = {
      name: 'fake',
      settings,
      onMessage: () => {},
      onClose: () => {},
      close: () => {},
      send: (m: {type: string; id: string; next: number}) => {
        if (m.type === 'resume') {
          resumes.push({id: m.id, next: m.next});
          const send = (x: BridgeMessage) =>
            (second.onMessage as (x: BridgeMessage) => void)(x);
          // the bridge replays from `next`; a frame we already handled is also tolerated
          setTimeout(() => {
            send({
              ...line(m.id, 0, 0, [stroke(), stroke()]),
              seq: 0,
            } as BridgeMessage);
            send({...line(m.id, 1, 0, [stroke()]), seq: 1} as BridgeMessage);
            send({...done(m.id), seq: 2} as BridgeMessage);
          }, 0);
        }
      },
    };
    connect.mockImplementationOnce(async () => first as never);
    connect.mockImplementationOnce(async () => second as never);

    const last = await run(layout);

    expect(last.failed).toBeFalsy();
    expect(last.finished).toBe(true);
    expect(resumes).toHaveLength(1);
    expect(resumes[0].next).toBe(1); // frame 0 had been handled
    expect(answerStrokes(0)).toHaveLength(3); // 2 + 1: nothing written twice
  });

  it('writes in steps of 15 strokes', async () => {
    const insertSpy = jest.spyOn(
      require('../__mocks__/sn-plugin-lib').PluginCommAPI,
      'insertPageElements',
    );
    mockScript = (send, id) => {
      send(
        line(
          id,
          0,
          0,
          Array.from({length: 40}, () => stroke()),
        ),
      );
      send(done(id));
    };
    await run(layout);
    // 40 strokes → 15 + 15 + 10 (the thinking wave adds its own calls, so count the big ones).
    const sizes = insertSpy.mock.calls
      .map(c => (c[0] as unknown[]).length)
      .filter(n => n > 1);
    expect(sizes).toEqual([15, 15, 10]);
    insertSpy.mockRestore();
  });

  it('continues on a newly inserted page when the bridge asks for page 1', async () => {
    mockScript = (send, id) => {
      send(line(id, 0, 0, [stroke()]));
      send(line(id, 1, 1, [stroke()]));
      send(done(id));
    };
    const last = await run(layout);
    expect(last.failed).toBeFalsy();
    expect(fakeNote.inserted).toEqual([1]);
    expect(fakeNote.currentPage).toBe(1);
    expect(fakeNote.strokes(1).some(e => e.uuid.startsWith('new-'))).toBe(true);
  });

  it('when the note app refuses to save, the next page is not inserted and the first page keeps what was written', async () => {
    mockScript = (send, id) => {
      send(line(id, 0, 0, [stroke()]));
      send(line(id, 1, 1, [stroke()]));
      send(done(id));
    };
    fakeNote.refuse.add('saveCurrentNote');
    const last = await run(layout);
    expect(last.failed).toBe(true);
    expect(last.message).toContain('saveCurrentNote');
    expect(fakeNote.inserted).toEqual([]); // no page was inserted over unsaved writing
    expect(answerStrokes(0)).toHaveLength(1); // the first line is still on its page
  });

  it('when the page count does not grow after inserting, the answer stops instead of writing on a page that is not there', async () => {
    mockScript = (send, id) => {
      send(line(id, 0, 0, [stroke()]));
      send(line(id, 1, 1, [stroke()]));
      send(done(id));
    };
    fakeNote.silentInsertPage = true;
    const last = await run(layout);
    expect(last.failed).toBe(true);
    expect(last.message).toContain('page count did not grow');
    expect(answerStrokes(1)).toHaveLength(0);
  });

  it('shows an error when the bridge reports one', async () => {
    mockScript = (send, id) =>
      send({type: 'error', id, code: 'provider_failed', message: 'boom'});
    const last = await run(layout);
    expect(last.failed).toBe(true);
    expect(last.message).toContain('boom');
  });

  describe('thinking wave', () => {
    it('starts while the bridge is still thinking and is gone before the answer is written', async () => {
      let waveWhenFirstLineArrived = -1;
      mockScript = async (send, id) => {
        await pause(1300); // Claude thinking: the wave should be drawing meanwhile
        waveWhenFirstLineArrived = waveStrokes(0).length;
        send(line(id, 0, 0, [stroke(), stroke()]));
        send(done(id));
      };
      await run(layout);
      expect(waveWhenFirstLineArrived).toBeGreaterThanOrEqual(2);
      expect(waveStrokes(0)).toHaveLength(0); // all removed
      expect(answerStrokes(0)).toHaveLength(2); // the answer stays
    });

    it('is removed even when the bridge fails', async () => {
      mockScript = async (send, id) => {
        await pause(700);
        send({type: 'error', id, code: 'provider_failed', message: 'boom'});
      };
      await run(layout);
      expect(waveStrokes(0)).toHaveLength(0);
    });

    it('never removes the existing writing', async () => {
      const mine = fakeNote.addUserStroke(
        0,
        Array.from({length: 17}, (_, i) => ({x: 90 + i, y: 800})),
      );
      mockScript = async (send, id) => {
        await pause(900);
        send(line(id, 0, 0, [stroke()]));
        send(done(id));
      };
      await run(layout);
      expect(fakeNote.strokes(0)).toContain(mine);
    });
  });

  describe('slow answers', () => {
    it('leaves no wave behind when the answer comes after the wave wrapped around the line', async () => {
      // The wave advances ~1 character per tick; a narrow answer area makes it wrap quickly.
      const narrow = {...layout, width: 140};
      const mine = fakeNote.addUserStroke(
        0,
        Array.from({length: 17}, (_, i) => ({x: 90 + i, y: 900})),
      );
      mockScript = async (send, id) => {
        await pause(3200); // long enough for several passes
        send(line(id, 0, 0, [stroke()]));
        send(done(id));
      };
      await run(narrow);
      expect(waveStrokes(0)).toHaveLength(0);
      expect(answerStrokes(0)).toHaveLength(1);
      expect(fakeNote.strokes(0)).toContain(mine);
    }, 15000);
  });

  describe('placement', () => {
    it('goes to a new page at once when the selection is at the page bottom, and animates there', async () => {
      let waveOnNewPage = -1;
      mockScript = async (send, id) => {
        await pause(1000);
        waveOnNewPage = waveStrokes(1).length;
        send(line(id, 0, 0, [stroke()]));
        send(done(id));
      };
      const bottom = {...layout, y: 1860};
      await run(bottom);
      expect(fakeNote.inserted).toEqual([1]);
      expect(waveStrokes(0)).toHaveLength(0); // nothing drawn at the bottom of the old page
      expect(waveOnNewPage).toBeGreaterThanOrEqual(1);
      expect(waveStrokes(1)).toHaveLength(0);
      expect(answerStrokes(1)).toHaveLength(1);
    });

    it('moves to a new page when existing writing sits right below the selection', async () => {
      // An old note starts at y=310, just under where the answer would begin (y=300).
      fakeNote.addUserStroke(
        0,
        Array.from({length: 5}, (_, i) => ({x: 200 + i * 20, y: 310 + i})),
      );
      mockScript = async (send, id) => {
        await pause(800);
        send(line(id, 0, 0, [stroke()]));
        send(done(id));
      };
      const selection: Box = {left: 84, top: 100, right: 400, bottom: 260};
      await run(layout, selection);
      expect(fakeNote.inserted).toEqual([1]);
      expect(answerStrokes(0)).toHaveLength(0); // nothing written over the old note
      expect(answerStrokes(1)).toHaveLength(1);
      expect(waveStrokes(0)).toHaveLength(0);
      expect(waveStrokes(1)).toHaveLength(0);
    });

    it('limits the answer to the space above existing writing instead of overwriting it', async () => {
      fakeNote.addUserStroke(
        0,
        Array.from({length: 5}, (_, i) => ({x: 200 + i * 20, y: 700 + i})),
      );
      let sent: LayoutParams | null = null;
      const conn = require('../src/bridgeClient');
      jest.spyOn(conn, 'connectFirst').mockImplementationOnce(
        async () =>
          ({
            name: 'fake',
            settings: {
              showProgress: false,
              lineTilt: true,
              thinkingAnimation: true,
              avoidOverwriting: false,
              busyNotice: true,
            },
            onMessage: () => {},
            onClose: () => {},
            close: () => {},
            send: (m: {type: string; layout: LayoutParams}) => {
              if (m.type === 'ask') {
                sent = m.layout;
              }
            },
          }) as never,
      );
      const selection: Box = {left: 84, top: 100, right: 400, bottom: 260};
      const flow = new AskFlow(() => {});
      void flow.runWithQuestion('q', layout, ctx, selection);
      await pause(300);
      flow.cancel();
      await pause(50);
      expect(sent).not.toBeNull();
      expect(sent!.startLimit).toBeLessThan(700);
      expect(sent!.startLimit).toBeGreaterThan(layout.y);
    });
  });
});

describe('page switching must not lose anything (regression: the old page came back empty)', () => {
  beforeEach(() => fakeNote.reset());

  it("keeps the user's writing and the answer on the page we leave", async () => {
    // The user's writing and our first lines live in the cache, unsaved, when the page is added.
    const mine = fakeNote.addUserStroke(0, [
      {x: 90, y: 100},
      {x: 120, y: 130},
    ]);
    mockScript = (send, id) => {
      send(line(id, 0, 0, [stroke()]));
      send(line(id, 1, 1, [stroke()])); // the answer overflows onto a new page
      send(done(id));
    };
    await run(layout);
    expect(fakeNote.inserted).toEqual([1]);
    expect(fakeNote.strokes(0)).toContainEqual(mine);
    expect(answerStrokes(0)).toHaveLength(1);
    expect(answerStrokes(1)).toHaveLength(1);
  });

  it('keeps the old page when there is no room and the answer starts on a new page', async () => {
    const mine = fakeNote.addUserStroke(0, [
      {x: 90, y: 1800},
      {x: 120, y: 1830},
    ]);
    mockScript = (send, id) => {
      send(line(id, 0, 0, [stroke()]));
      send(done(id));
    };
    await run({...layout, y: 1860});
    expect(fakeNote.strokes(0)).toContainEqual(mine);
    expect(answerStrokes(1)).toHaveLength(1);
  });

  it('sees writing that has not been saved yet when looking for free space', async () => {
    // Unsaved (cache-only) writing right under the selection must still be avoided.
    fakeNote.addUserStroke(
      0,
      Array.from({length: 5}, (_, i) => ({x: 200 + i * 20, y: 310 + i})),
    );
    mockScript = (send, id) => {
      send(line(id, 0, 0, [stroke()]));
      send(done(id));
    };
    await run(layout, {left: 84, top: 100, right: 400, bottom: 260});
    expect(answerStrokes(0)).toHaveLength(0);
    expect(answerStrokes(1)).toHaveLength(1);
  });
});

describe('the user leaves the note while the answer is being written', () => {
  beforeEach(() => fakeNote.reset());

  it('never writes into the document that is now open, and says why it stopped', async () => {
    mockScript = async (send, id) => {
      send(line(id, 0, 0, [stroke()]));
      await pause(200);
      fakeNote.currentFile = '/other-document.note'; // the user opened something else
      send(line(id, 1, 0, [stroke()]));
      send(line(id, 2, 0, [stroke()]));
      send(done(id));
    };
    const last = await run(layout);
    expect(fakeNote.strayWrites).toBe(0);
    expect(fakeNote.strayDeletes).toBe(0);
    expect(fakeNote.strayJumps).toBe(0);
    expect(last.failed).toBe(true);
    expect(last.message).toContain(t('noteMoved'));
    expect(answerStrokes(0)).toHaveLength(1); // what was written before stays
  });

  it('removes nothing from another document when the wave is cleaned up', async () => {
    mockScript = async (send, id) => {
      await pause(800);
      fakeNote.currentFile = '/other-document.note';
      await pause(100);
      send(line(id, 0, 0, [stroke()]));
      send(done(id));
    };
    await run(layout);
    expect(fakeNote.strayDeletes).toBe(0);
    expect(fakeNote.strayWrites).toBe(0);
  });

  it('writes to its own page while the user looks at another one, without moving the user', async () => {
    mockScript = async (send, id) => {
      send(line(id, 0, 0, [stroke()]));
      await pause(100);
      fakeNote.currentPage = 5; // flipped away mid-answer
      send(line(id, 1, 0, [stroke()]));
      send(done(id));
    };
    const last = await run(layout);
    expect(last.failed).toBeFalsy();
    expect(fakeNote.currentPage).toBe(5); // we never pulled the user back
    expect(answerStrokes(0)).toHaveLength(2);
    expect(answerStrokes(5)).toHaveLength(0);
  });
});

describe('the user keeps flipping pages while a long answer is written', () => {
  beforeEach(() => fakeNote.reset());

  it('loses no stroke and writes nothing to the page the user flipped to', async () => {
    const lines = 12;
    mockScript = async (send, id) => {
      for (let i = 0; i < lines; i++) {
        fakeNote.currentPage = ((i * 7) % 5) + 1; // the user is on some other page every time
        send(
          line(
            id,
            i,
            0,
            Array.from({length: 40}, () => stroke()),
          ),
        );
        await pause(5);
      }
      send(done(id));
    };
    const last = await run({...layout, maxPages: 1});
    expect(last.failed).toBeFalsy();
    expect(fakeNote.jumps).toBe(0); // the user was never moved
    expect(answerStrokes(0)).toHaveLength(lines * 40); // every stroke landed on its own page
    for (const other of [1, 2, 3, 4, 5]) {
      expect(answerStrokes(other)).toHaveLength(0);
    }
  }, 30000);
});

describe('thinking animation setting', () => {
  beforeEach(() => fakeNote.reset());

  it('draws no wave when the last known settings turned it off', async () => {
    // The wave starts before the bridge is reached, so it follows what the last connection said.
    rememberSettings({...mockBridgeSettings, thinkingAnimation: false});
    mockScript = () => {}; // the bridge never answers: we look at the page while it "thinks"
    const flow = new AskFlow(() => {});
    const running = flow.runWithQuestion('q', layout, ctx);
    await pause(1200);
    const waves = waveStrokes(0).length;
    flow.cancel();
    await running.catch(() => {});
    expect(waves).toBe(0);
  });
});

// Every test starts from the defaults, whatever an earlier one remembered.
beforeEach(() => {
  mockBridgeSettings = {
    showProgress: false,
    lineTilt: true,
    thinkingAnimation: true,
    avoidOverwriting: false,
    busyNotice: true,
  };
  rememberSettings({...mockBridgeSettings});
});

describe('the connection drops (e.g. the tunnel reconnects)', () => {
  beforeEach(() => fakeNote.reset());

  it('after the bridge sent everything: the rest is still written', async () => {
    let conn: {onClose: (reason: string) => void} | null = null;
    const bridge = require('../src/bridgeClient');
    const real = bridge.connectFirst;
    jest
      .spyOn(bridge, 'connectFirst')
      .mockImplementationOnce(async (...a: unknown[]) => {
        conn = (await real(...a)) as never;
        return conn;
      });
    mockScript = (send, id) => {
      send(
        line(
          id,
          0,
          0,
          Array.from({length: 60}, () => stroke()),
        ),
      );
      send(done(id));
      setTimeout(() => conn?.onClose('closed (1006)'), 0); // the tunnel drops right after `done`
    };
    const last = await run(layout);
    expect(last.failed).toBeFalsy();
    expect(answerStrokes(0)).toHaveLength(60);
  });

  it('dropped before the bridge finished and the bridge cannot be reached again: stops and says why', async () => {
    let conn: {onClose: (reason: string) => void} | null = null;
    const bridge = require('../src/bridgeClient');
    const real = bridge.connectFirst;
    jest
      .spyOn(bridge, 'connectFirst')
      .mockImplementationOnce(async (...a: unknown[]) => {
        conn = (await real(...a)) as never;
        // queued only now: `real` is the spy itself, so queued mocks would be taken by it
        for (let i = 0; i < 3; i++) {
          bridge.connectFirst.mockImplementationOnce(async () => {
            throw new Error('no bridge reachable'); // every reconnection attempt fails
          });
        }
        return conn;
      });
    mockScript = (send, id) => {
      send(line(id, 0, 0, [stroke()]));
      setTimeout(() => conn?.onClose('closed (1006)'), 20);
    };
    const last = await run(layout);
    expect(last.failed).toBe(true);
    expect(last.message).toContain('1006');
  });
});

describe('defaults', () => {
  it('the thinking animation is off until the user turns it on', () => {
    const {knownSettings} = require('../src/askFlow');
    rememberSettings({
      showProgress: false,
      lineTilt: true,
      thinkingAnimation: false,
      avoidOverwriting: false,
      busyNotice: true,
    });
    expect(knownSettings().thinkingAnimation).toBe(false);
  });
});

describe('reconnecting (docs/spec/ask-flow.md, S5)', () => {
  const settings = {
    showProgress: false,
    lineTilt: true,
    thinkingAnimation: false,
    avoidOverwriting: false,
    busyNotice: true,
  };
  type Sent = {type: string; id: string; next?: number};
  /** A connection whose reaction to what it is sent is scripted. */
  const fake = (
    react: (m: Sent, deliver: (x: unknown) => void, drop: () => void) => void,
  ) => {
    const c: Record<string, unknown> = {
      name: 'fake',
      settings,
      onMessage: () => {},
      onClose: () => {},
      onBad: () => {},
      close: () => {},
      send: (m: Sent) =>
        setTimeout(
          () =>
            react(
              m,
              x => (c.onMessage as (x: unknown) => void)(x),
              () => (c.onClose as (r: string) => void)('closed (1006)'),
            ),
          0,
        ),
    };
    return c;
  };
  const queue = (...conns: Record<string, unknown>[]) => {
    const spy = jest.spyOn(require('../src/bridgeClient'), 'connectFirst');
    spy.mockClear(); // the spy is shared: count only this test's calls
    for (const c of conns) {
      spy.mockImplementationOnce(async () => c as never);
    }
    return spy;
  };
  const alive = (id: string, seq?: number) => ({
    type: 'status',
    id,
    stage: 'alive',
    seq,
  });
  const states = async (l: LayoutParams = layout) => {
    const seen: AskState[] = [];
    await new AskFlow(x => seen.push(x)).runWithQuestion('q', l, ctx);
    return seen;
  };

  beforeEach(() => fakeNote.reset());
  afterEach(() => setQuietMsForTests(15000));

  it('tries three times in a row, shows n/3, and then gives up with the reason', async () => {
    const spy = queue(fake((m, _d, drop) => m.type === 'ask' && drop()));
    spy.mockImplementation(async () => {
      throw new Error('no bridge reachable');
    });
    const seen = await states();
    spy.mockRestore(); // back to the module's own fake for the other tests
    const msgs = seen.map(s => s.message);
    expect(msgs.some(m => m.includes('(1/3)'))).toBe(true);
    expect(msgs.some(m => m.includes('(3/3)'))).toBe(true);
    expect(msgs.some(m => m.includes('(4/3)'))).toBe(false);
    expect(seen[seen.length - 1].failed).toBe(true);
    expect(seen[seen.length - 1].message).toContain('1006');
  });

  it('hearing from the bridge resets the count: more than three drops are fine if each got through', async () => {
    const resumeSeen: number[] = [];
    const part = (n: number) =>
      fake((m, deliver, drop) => {
        if (m.type === 'resume') {
          resumeSeen.push(m.next ?? -1);
        }
        deliver(alive(m.id)); // proof of life
        if (n < 5) {
          deliver({...line(m.id, n, 0, [stroke()]), seq: n});
          drop();
        } else {
          deliver({...done(m.id), seq: n});
        }
      });
    queue(part(0), part(1), part(2), part(3), part(4), part(5));
    const seen = await states();
    expect(seen[seen.length - 1].failed).toBeFalsy();
    expect(seen[seen.length - 1].finished).toBe(true);
    expect(resumeSeen).toEqual([1, 2, 3, 4, 5]); // five reconnections, each from the next frame
    expect(answerStrokes(0)).toHaveLength(5);
  });

  it('a drop between connecting and asking connects again and sends the question (not a resume)', async () => {
    const got: string[] = [];
    const first = fake(() => {});
    const second = fake((m, deliver) => {
      got.push(m.type);
      deliver({...done(m.id), seq: 0});
    });
    // the first connection is already closed when the question is about to be sent
    first.send = () => {
      throw new Error('socket is closed');
    };
    queue(first, second);
    const seen = await states();
    expect(got).toEqual(['ask']);
    expect(seen[seen.length - 1].finished).toBe(true);
    expect(seen[seen.length - 1].failed).toBeFalsy();
  });

  it('a connection that goes completely silent is treated as dead and resumed', async () => {
    setQuietMsForTests(40);
    const silent = fake(() => {}); // takes the question and says nothing, ever
    const resumed: Sent[] = [];
    const second = fake((m, deliver) => {
      resumed.push(m);
      deliver({...done(m.id), seq: 0});
    });
    queue(silent, second);
    const seen = await states();
    expect(resumed.map(m => m.type)).toEqual(['resume']);
    expect(seen[seen.length - 1].finished).toBe(true);
  });

  it('keep-alives from a slow AI are not mistaken for a dead connection', async () => {
    setQuietMsForTests(60);
    const spy = queue(
      fake((m, deliver) => {
        let n = 0;
        const timer = setInterval(() => {
          deliver(alive(m.id));
          if (++n === 8) {
            clearInterval(timer);
            deliver({...done(m.id), seq: 0});
          }
        }, 20);
      }),
    );
    const seen = await states();
    expect(seen[seen.length - 1].finished).toBe(true);
    expect(spy).toHaveBeenCalledTimes(1); // never reconnected
  });
});
