import {fakeNote} from '../__mocks__/sn-plugin-lib';
import {AskFlow, type AskState} from '../src/askFlow';
import type {Box} from '../src/freeSpace';
import type {BridgeMessage, InkStroke, LayoutParams} from '../src/protocol';

// The bridge connection is replaced by a script of messages.
let mockScript: (
  send: (m: BridgeMessage) => void,
  id: string,
) => void | Promise<void>;
jest.mock('../src/bridgeConfig', () => ({
  loadBridges: async () => ({endpoints: [], source: 'baked'}),
}));
jest.mock('../src/bridgeClient', () => ({
  connectFirst: async () => {
    const conn: Record<string, unknown> = {
      name: 'fake',
      settings: {showProgress: false, lineTilt: true},
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

  it('shows an error when the bridge reports one', async () => {
    mockScript = (send, id) =>
      send({type: 'error', id, code: 'claude_failed', message: 'boom'});
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
        send({type: 'error', id, code: 'claude_failed', message: 'boom'});
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
            settings: {showProgress: false, lineTilt: true},
            onMessage: () => {},
            onClose: () => {},
            close: () => {},
            send: (m: {type: string; layout: LayoutParams}) => {
              if (m.type === 'ask') {
                sent = m.layout;
              }
            },
          } as never),
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
