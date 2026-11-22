import {
  channelReport,
  navigatorNames,
  spread,
  spreadText,
  timeRequests,
} from '../src/netProbe';

describe('spread', () => {
  it('gives min, median and max of the samples', () => {
    expect(spread([30, 10, 20])).toEqual({n: 3, min: 10, median: 20, max: 30});
    expect(spread([10, 20, 30, 40])).toEqual({
      n: 4,
      min: 10,
      median: 25,
      max: 40,
    });
  });
  it('ignores NaN, Infinity and negative samples; no sample gives null', () => {
    expect(spread([NaN, Infinity, -1, 5])).toEqual({
      n: 1,
      min: 5,
      median: 5,
      max: 5,
    });
    expect(spread([])).toBeNull();
    expect(spread([NaN])).toBeNull();
    expect(spreadText(null)).toBe('no sample');
    expect(spreadText(spread([7]))).toBe('n=1 min 7ms median 7ms max 7ms');
  });
});

describe('timeRequests', () => {
  it('reports the first request apart and counts failures', async () => {
    let clock = 0;
    const costs = [100, 10, 12, 0];
    let i = 0;
    const out = await timeRequests(
      async () => {
        const c = costs[i++];
        clock += c;
        if (c === 0) {
          throw new Error('boom');
        }
      },
      4,
      () => clock,
    );
    expect(out).toEqual({first: 100, rest: [10, 12], failures: ['boom']});
  });
  it('a failed first request leaves first null', async () => {
    const out = await timeRequests(async () => {
      throw new Error('down');
    }, 2);
    expect(out.first).toBeNull();
    expect(out.rest).toEqual([]);
    expect(out.failures).toHaveLength(2);
  });
});

describe('channelReport / navigatorNames', () => {
  it('says what typeof gives for each name, absent ones as undefined', () => {
    const lines = channelReport({fetch() {}, WebSocket: class {}});
    expect(lines).toContain('fetch: function');
    expect(lines).toContain('WebSocket: function');
    expect(lines).toContain('RTCPeerConnection: undefined');
  });
  it('lists navigator names, and nothing when it is absent or not an object', () => {
    expect(
      navigatorNames({navigator: {product: 'ReactNative', onLine: true}}),
    ).toEqual(['onLine', 'product']);
    expect(navigatorNames({})).toEqual([]);
    expect(navigatorNames({navigator: 3})).toEqual([]);
  });
});
