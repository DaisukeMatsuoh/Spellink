import {fakeNote} from '../__mocks__/sn-plugin-lib';
import {AskFlow, type AskState} from '../src/askFlow';
import type {BridgeMessage, InkStroke, LayoutParams} from '../src/protocol';

// The bridge connection is replaced by a script of messages.
let mockScript: (send: (m: BridgeMessage) => void, id: string) => void;
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

async function run(l: LayoutParams) {
  const states: AskState[] = [];
  await new AskFlow(s => states.push(s)).runWithQuestion('q', l, ctx);
  return states[states.length - 1];
}
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
});
