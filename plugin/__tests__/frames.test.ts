import {BridgeConnection} from '../src/bridgeClient';
import {
  MAX_EXTRA_PAGES,
  MAX_POINTS_PER_STROKE,
  MAX_STROKES_PER_LINE,
  parseFrame,
  parseSettings,
} from '../src/frames';

const goodStroke = {p: [1, 2, 3, 4], w: [10, 10]};
const line = (over: Record<string, unknown> = {}) => ({
  type: 'line',
  id: 'a',
  index: 0,
  seq: 0,
  page: 0,
  text: 'あ',
  strokes: [goodStroke],
  ...over,
});
const reason = (raw: unknown) => {
  const p = parseFrame(raw, MAX_EXTRA_PAGES);
  return p.kind === 'bad' ? p.reason : p.kind;
};

describe('a line frame is checked before anything is written', () => {
  it('accepts a normal line, and a line with no glyphs (null or missing strokes)', () => {
    expect(parseFrame(line(), MAX_EXTRA_PAGES).kind).toBe('frame');
    for (const strokes of [null, undefined, []]) {
      const p = parseFrame(line({strokes}), MAX_EXTRA_PAGES);
      expect(p.kind).toBe('frame');
      if (p.kind === 'frame' && p.frame.type === 'line') {
        expect(p.frame.strokes).toEqual([]);
      }
    }
  });

  it.each([
    [
      'a page past the allowed extra pages',
      line({page: MAX_EXTRA_PAGES + 1}),
      /page/,
    ],
    ['a negative page', line({page: -1}), /page/],
    ['a fractional page', line({page: 0.5}), /page/],
    ['a page that is not a number', line({page: '1'}), /page/],
    ['an index that is not an integer', line({index: 'x'}), /index/],
    ['a seq that is negative', line({seq: -1}), /seq/],
    ['text that is not a string', line({text: 7}), /text/],
    ['strokes that are not a list', line({strokes: {p: []}}), /strokes/],
    [
      'too many strokes',
      line({strokes: Array(MAX_STROKES_PER_LINE + 1).fill(goodStroke)}),
      /strokes/,
    ],
    ['a stroke without pressures', line({strokes: [{p: [1, 2]}]}), /stroke/],
    [
      'a stroke with an odd number of coordinates',
      line({strokes: [{p: [1, 2, 3], w: [1]}]}),
      /odd/,
    ],
    ['a stroke with no points', line({strokes: [{p: [], w: []}]}), /no points/],
    [
      'pressures that do not match the points',
      line({strokes: [{p: [1, 2, 3, 4], w: [1]}]}),
      /match/,
    ],
    ['a NaN coordinate', line({strokes: [{p: [NaN, 2], w: [1]}]}), /sane/],
    [
      'an infinite coordinate',
      line({strokes: [{p: [Infinity, 2], w: [1]}]}),
      /sane/,
    ],
    [
      'a coordinate far off any page',
      line({strokes: [{p: [1e9, 2], w: [1]}]}),
      /sane/,
    ],
    [
      'a coordinate that is a string',
      line({strokes: [{p: ['1', 2], w: [1]}]}),
      /sane/,
    ],
    [
      'a negative pressure',
      line({strokes: [{p: [1, 2], w: [-1]}]}),
      /pressure/,
    ],
    [
      'a stroke with a huge number of points',
      line({
        strokes: [
          {
            p: Array(MAX_POINTS_PER_STROKE * 2 + 2).fill(1),
            w: Array(MAX_POINTS_PER_STROKE + 1).fill(1),
          },
        ],
      }),
      /too many/,
    ],
  ])('rejects %s', (_name, frame, why) => {
    expect(reason(frame)).toMatch(why);
  });
});

describe('the other frames', () => {
  it('rejects things that are not frames at all', () => {
    for (const raw of [null, undefined, 3, 'x', [], {}, {type: 5}]) {
      expect(reason(raw)).toMatch(/not an object|type/);
    }
  });

  it('skips an unknown type instead of failing (a newer bridge may add messages)', () => {
    const p = parseFrame({type: 'sparkle', id: 'a'}, MAX_EXTRA_PAGES);
    expect(p).toEqual({kind: 'ignore', type: 'sparkle'});
  });

  it('checks done, error and status', () => {
    expect(reason({type: 'done', id: 'a'})).toMatch(/truncated/);
    expect(parseFrame({type: 'done', id: 'a', truncated: false}, 2).kind).toBe(
      'frame',
    );
    expect(reason({type: 'error', code: 'x'})).toMatch(/message/);
    expect(parseFrame({type: 'error', code: 'x', message: 'm'}, 2).kind).toBe(
      'frame',
    );
    expect(reason({type: 'status', id: 'a'})).toMatch(/stage/);
    expect(parseFrame({type: 'status', id: 'a', stage: 'alive'}, 2).kind).toBe(
      'frame',
    );
    expect(reason({type: 'historyCleared', notes: 'many'})).toMatch(/count/);
    expect(reason({type: 'probeAck', id: 'p', bytes: -1})).toMatch(/size/);
    expect(reason({type: 'probeAck', bytes: 5})).toMatch(/id/);
    expect(parseFrame({type: 'probeAck', id: 'p', bytes: 5}, 2).kind).toBe(
      'frame',
    );
  });
});

describe('settings from the bridge', () => {
  it('replace anything wrong with the default', () => {
    const s = parseSettings({
      showProgress: 'yes',
      lineTilt: null,
      tiltDegrees: 999,
      thinkingAnimation: 1,
      fullWidthSpacing: 5,
      halfWidthSpacing: '120',
      busyNotice: 'no',
    });
    expect(s).toEqual({
      showProgress: false,
      lineTilt: true,
      tiltDegrees: undefined,
      thinkingAnimation: false,
      fullWidthSpacing: undefined,
      halfWidthSpacing: undefined,
      avoidOverwriting: false,
      busyNotice: true,
    });
  });

  it('keep good values, and survive no settings at all', () => {
    const s = parseSettings({
      fullWidthSpacing: 120,
      halfWidthSpacing: 150,
      tiltDegrees: -0.4,
      busyNotice: false,
    });
    expect(s.fullWidthSpacing).toBe(120);
    expect(s.halfWidthSpacing).toBe(150);
    expect(s.tiltDegrees).toBe(-0.4);
    expect(s.busyNotice).toBe(false);
    expect(parseSettings(undefined).lineTilt).toBe(true);
  });
});

describe('BridgeConnection hands on only valid frames', () => {
  const make = () => {
    const ws = {
      onmessage: null as null | ((e: {data: string}) => void),
      onclose: null as unknown,
      send: jest.fn(),
      close: jest.fn(),
    };
    const c = new BridgeConnection(
      ws as never,
      null,
      'n',
      'u',
      'plaintext',
      parseSettings({}),
    );
    const got: unknown[] = [];
    c.onMessage = m => got.push(m);
    return {
      ws,
      c,
      got,
      push: (v: unknown) => ws.onmessage!({data: JSON.stringify(v)}),
    };
  };

  it('passes a valid frame, skips an unknown one, and drops the connection on a bad one', () => {
    const {ws, got, push} = make();
    push(line());
    push({type: 'sparkle'});
    expect(got).toHaveLength(1);
    expect(ws.close).not.toHaveBeenCalled();
    push(line({page: 99}));
    expect(got).toHaveLength(1); // the bad frame never reached the handler
    expect(ws.close).toHaveBeenCalled();
  });

  it('lets the owner decide what a bad frame means', () => {
    const {ws, c, push} = make();
    const seen: string[] = [];
    c.onBad = r => seen.push(r);
    push(line({strokes: [{p: [NaN, 1], w: [1]}]}));
    expect(seen).toHaveLength(1);
    expect(ws.close).not.toHaveBeenCalled();
  });

  it('drops the connection when the frame is not even JSON', () => {
    const {ws} = make();
    ws.onmessage!({data: 'not json {'});
    expect(ws.close).toHaveBeenCalled();
  });
});
