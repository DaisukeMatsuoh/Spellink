jest.mock('../src/bridges.local.json', () => ({bridges: []}), {virtual: true});
import {setFakeLanguage} from '../__mocks__/sn-plugin-lib';
import {
  connectFirst,
  NoBridgeError,
  noBridgeMessage,
  refusedBridges,
  type ConnectAttempt,
} from '../src/bridgeClient';
import {startLocaleTracking} from '../src/i18n';

// How `connectFirst` tells a refused device from an absent bridge (the handshake itself is checked
// against the real bridge in pairingIntegration.test).
class FakeSocket {
  static all: FakeSocket[] = [];
  binaryType = '';
  closed = 0;
  onopen: () => void = () => {};
  onmessage: (e: {data: unknown}) => void = () => {};
  onerror: (e: unknown) => void = () => {};
  onclose: (e: {reason: string; code: number}) => void = () => {};
  constructor(readonly url: string) {
    FakeSocket.all.push(this);
  }
  send() {}
  close() {
    this.closed++;
  }
}

const OK = 'wss://a.example/v1?d=0123456789abcdef';
const OK2 = 'wss://b.example/v1/relay/client?room=abc';
const paired = {id: '0123456789abcdef', name: 'Rock', token: 'T', urls: [OK]};

beforeAll(() => {
  (globalThis as unknown as {WebSocket: unknown}).WebSocket = FakeSocket;
  startLocaleTracking();
  setFakeLanguage('en');
});
beforeEach(() => {
  FakeSocket.all = [];
});

async function failWith(
  endpoints: Parameters<typeof connectFirst>[0],
  reasons: Array<string | null>,
) {
  const attempts: ConnectAttempt[] = [];
  const p = connectFirst(endpoints, 1000, attempts);
  FakeSocket.all.forEach((s, i) => {
    if (reasons[i] !== null) {
      s.onclose({reason: reasons[i]!, code: 1008});
    }
  });
  return p.then(
    () => {
      throw new Error('should have failed');
    },
    (e: unknown) => ({e, attempts}),
  );
}

describe('a bridge that turns this device away', () => {
  it.each(['unauthorized', 'device_removed'])(
    'is recognised by %s on its only address',
    async reason => {
      const {e} = await failWith([paired], [reason]);
      expect(e).toBeInstanceOf(NoBridgeError);
      expect((e as NoBridgeError).refused).toEqual(['Rock']);
    },
  );

  it('is recognised even when another of its addresses was simply unreachable', async () => {
    const {e} = await failWith(
      [{...paired, urls: [OK, OK2]}],
      ['unauthorized', 'closed (1006)'],
    );
    expect((e as NoBridgeError).refused).toEqual(['Rock']);
  });

  it('is not recognised when every address fails for another reason', async () => {
    const {e} = await failWith(
      [{...paired, urls: [OK, OK2]}],
      ['closed (1006)', 'no bridge'],
    );
    expect((e as NoBridgeError).refused).toEqual([]);
  });

  it('is not recognised through a relay alone: there a refusal looks like an absent bridge', async () => {
    const {e} = await failWith([{...paired, urls: [OK2]}], ['closed (4404)']);
    expect((e as NoBridgeError).refused).toEqual([]);
  });

  it('is not recognised for a bridge that was not paired one by one (no id)', async () => {
    const old: Parameters<typeof connectFirst>[0][number] = {...paired};
    delete old.id;
    const {e} = await failWith([old], ['unauthorized']);
    expect((e as NoBridgeError).refused).toEqual([]);
  });

  it('is not recognised while an address has not answered at all', async () => {
    jest.useFakeTimers();
    try {
      const attempts: ConnectAttempt[] = [];
      const p = connectFirst([{...paired, urls: [OK, OK2]}], 500, attempts);
      const assertion = expect(p).rejects.toMatchObject({refused: []});
      FakeSocket.all[0].onclose({reason: 'unauthorized', code: 1008}); // one answered, the other is silent
      jest.advanceTimersByTime(500);
      await assertion;
    } finally {
      jest.useRealTimers();
    }
  });

  it('names only the refusing bridge when there are several', async () => {
    const other = {
      id: 'fedcba9876543210',
      name: 'Other',
      token: 'U',
      urls: ['wss://c.example/v1?d=fedcba9876543210'],
    };
    const {e} = await failWith(
      [paired, other],
      ['unauthorized', 'closed (1006)'],
    );
    expect((e as NoBridgeError).refused).toEqual(['Rock']);
  });

  it("is told to the user with the bridge's name, and an ordinary failure is not", () => {
    expect(noBridgeMessage(new NoBridgeError('x', ['Rock']))).toMatch(
      /Rock no longer accepts this device/,
    );
    expect(noBridgeMessage(new NoBridgeError('x', []))).toMatch(
      /registered bridge was not found/,
    );
    expect(noBridgeMessage(new Error('boom'))).toMatch(/registered bridge was not found/);
    expect(noBridgeMessage('a string')).toMatch(/registered bridge was not found/);
  });

  it('refusedBridges works on the attempts alone', () => {
    const attempts: ConnectAttempt[] = [
      {url: OK, ms: 5, error: 'unauthorized'},
    ];
    expect(refusedBridges([paired], attempts)).toEqual(['Rock']);
    expect(refusedBridges([paired], [])).toEqual([]);
    expect(refusedBridges([], attempts)).toEqual([]);
  });
});
