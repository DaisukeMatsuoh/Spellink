import {encodeBase64Url} from '../src/base64url';
import type {PairSetup} from '../src/pairConfig';
import type {PairChannel} from '../src/pairConnect';
import type {PairFrame} from '../src/pairFrames';
import {PairingController, type PairDeps, type PairView} from '../src/pairing';
import {PAIR_EVENTS, PAIR_STATES, PAIR_TABLE} from '../src/pairTable.generated';
import {PairMachine, type PairState} from '../src/pairingMachine';
import type {StoredBridge} from '../src/pairStore';

const secretOf = (n: number) => encodeBase64Url(new Uint8Array(32).fill(n));
const ID = '0123456789abcdef';
const URL = 'wss://bridge.example.com/v1?d=' + ID;
const setup: PairSetup = {
  id: ID,
  gen: 2,
  b: secretOf(1),
  name: 'Nomad',
  urls: [URL],
  windowSec: 90,
};

class FakeChannel implements PairChannel {
  sent: Record<string, unknown>[] = [];
  closed = 0;
  throwOnSend = false;
  onFrame: (f: PairFrame) => void = () => {};
  onClose: (r: string) => void = () => {};
  send(f: Record<string, unknown>) {
    if (this.throwOnSend) {
      throw new Error('send failed');
    }
    this.sent.push(f);
  }
  close() {
    this.closed++;
  }
}

/** The outside world of the controller, under the test's control. */
class World {
  views: PairView[] = [];
  saved: StoredBridge[] = [];
  saveResult: boolean | Error = true;
  now = 1_000_000;
  timers: {at: number; f: () => void; every?: number; live: boolean}[] = [];
  connects: {
    urls: string[];
    b: string;
    ms: number;
    ok: (c: PairChannel) => void;
    fail: (e: Error) => void;
  }[] = [];
  logs: string[] = [];
  digitsUsed = '12345678';

  deps: PairDeps = {
    connect: (urls, b, ms) =>
      new Promise((ok, fail) => this.connects.push({urls, b, ms, ok, fail})),
    save: async b => {
      this.saved.push(b);
      if (this.saveResult instanceof Error) {
        throw this.saveResult;
      }
      return this.saveResult;
    },
    digits: n => this.digitsUsed.slice(0, n),
    after: (ms, f) => {
      const t = {at: this.now + ms, f, live: true};
      this.timers.push(t);
      return () => (t.live = false);
    },
    every: (ms, f) => {
      const t = {at: this.now + ms, f, every: ms, live: true};
      this.timers.push(t);
      return () => (t.live = false);
    },
    now: () => this.now,
    onView: v => this.views.push(v),
    log: k => this.logs.push(k),
  };

  advance(ms: number) {
    const end = this.now + ms;
    for (;;) {
      const due = this.timers
        .filter(t => t.live && t.at <= end)
        .sort((a, b) => a.at - b.at)[0];
      if (!due) {
        break;
      }
      this.now = due.at;
      if (due.every) {
        due.at += due.every;
      } else {
        due.live = false;
      }
      due.f();
    }
    this.now = end;
  }

  get last(): PairView {
    return this.views[this.views.length - 1];
  }
}

const flush = () => new Promise(r => setImmediate(r));

function controller(start: PairState = 'Unpaired', s: PairSetup = setup) {
  const w = new World();
  return {w, c: new PairingController(s, w.deps, start)};
}

/** Takes a controller to ShowingCode (the code is on the screen). */
async function showing(start: PairState = 'Unpaired') {
  const {w, c} = controller(start);
  c.tap();
  const ch = new FakeChannel();
  w.connects[0].ok(ch);
  await flush();
  return {w, c, ch};
}

describe('pairing, from the press of the button to a stored key', () => {
  it('goes through connecting, showing the code, the grant, storing and the ack', async () => {
    const {w, c} = controller();
    c.tap();
    expect(c.state).toBe('Connecting');
    expect(w.last).toMatchObject({state: 'Connecting', note: 'connecting'});
    expect(w.connects).toHaveLength(1);
    expect(w.connects[0]).toMatchObject({urls: [URL], b: setup.b, ms: 8000});

    const ch = new FakeChannel();
    w.connects[0].ok(ch);
    await flush();
    expect(c.state).toBe('ShowingCode');
    expect(ch.sent).toEqual([
      {type: 'pairRequest', id: ID, gen: 2, code: '12345678'},
    ]);
    expect(w.last).toMatchObject({
      state: 'ShowingCode',
      note: 'showing',
      code: '12345678',
      remainingSec: 90,
      qr: `spellink://pair?v=1&i=${ID}&c=12345678`,
    });

    w.advance(30_000);
    expect(w.last.remainingSec).toBe(60);

    ch.onFrame({type: 'pairGrant', key: secretOf(7), endpoints: [URL]});
    expect(c.state).toBe('Storing');
    expect(w.last.note).toBe('storing');
    await flush();
    expect(w.saved).toEqual([
      {id: ID, gen: 2, key: secretOf(7), name: 'Nomad', urls: [URL]},
    ]);
    expect(c.state).toBe('Paired');
    expect(w.last.note).toBe('paired');
    // the ack is sent only after the key is stored, then the channel is closed
    expect(ch.sent[1]).toEqual({type: 'pairAck', id: ID, gen: 2});
    expect(ch.closed).toBe(1);
    // nothing of the old timers fires any more
    w.advance(200_000);
    expect(c.state).toBe('Paired');
  });

  it('never sends the key or the first secret anywhere but the channel and the store', async () => {
    const {w, c, ch} = await showing();
    ch.onFrame({type: 'pairGrant', key: secretOf(7), endpoints: [URL]});
    await flush();
    const shown = JSON.stringify(w.views);
    expect(shown).not.toContain(secretOf(7));
    expect(shown).not.toContain(setup.b!);
    expect(JSON.stringify(w.logs)).not.toContain(secretOf(7));
    expect(c.state).toBe('Paired');
  });

  it('can pair again from Paired (a device that lost its pairing on the bridge)', async () => {
    const {w, c} = controller('Paired');
    c.tap();
    expect(c.state).toBe('Connecting');
    w.connects[0].ok(new FakeChannel());
    await flush();
    expect(c.state).toBe('ShowingCode');
  });

  it('a package with no first secret cannot pair and says so', () => {
    const noSecret: PairSetup = {...setup};
    delete noSecret.b;
    const {w, c} = controller('Unpaired', noSecret);
    c.tap();
    expect(c.state).toBe('Unpaired');
    expect(w.last).toMatchObject({note: 'disconnected', detail: 'noSecret'});
    expect(w.connects).toHaveLength(0);
  });

  it('shows the window running out', async () => {
    const {w, c, ch} = await showing();
    w.advance(89_999);
    expect(c.state).toBe('ShowingCode');
    w.advance(1);
    expect(c.state).toBe('Unpaired');
    expect(w.last.note).toBe('timeout');
    expect(ch.closed).toBe(1);
  });

  it('takes the window from the package', async () => {
    const {w, c} = controller('Unpaired', {...setup, windowSec: 10});
    c.tap();
    w.connects[0].ok(new FakeChannel());
    await flush();
    expect(w.last.remainingSec).toBe(10);
    w.advance(10_000);
    expect(c.state).toBe('Unpaired');
  });
});

describe('everything that can go wrong on the way', () => {
  it('a connection that fails ends the attempt with the reason, and no retry is made', async () => {
    const {w, c} = controller();
    c.tap();
    w.connects[0].fail(new Error('no address could be reached'));
    await flush();
    expect(c.state).toBe('Unpaired');
    expect(w.last).toMatchObject({
      note: 'disconnected',
      detail: 'no address could be reached',
    });
    expect(w.connects).toHaveLength(1);
  });

  it('a connection that is lost while the code is shown ends the attempt', async () => {
    const {w, c, ch} = await showing();
    ch.onClose('closed (1006)');
    expect(c.state).toBe('Unpaired');
    expect(w.last).toMatchObject({
      note: 'disconnected',
      detail: 'closed (1006)',
    });
    w.advance(200_000);
    expect(w.views.filter(v => v.note === 'timeout')).toHaveLength(0);
  });

  it.each([
    ['code_in_use', 'denied'],
    ['window_closed', 'denied'],
    ['unknown_device', 'denied'],
  ])('a refusal (%s) ends the attempt', async reason => {
    const {w, c, ch} = await showing();
    ch.onFrame({type: 'pairDenied', reason: reason as never});
    expect(c.state).toBe('Unpaired');
    expect(w.last).toMatchObject({note: 'denied', detail: reason});
  });

  it('an error frame is treated as a refusal', async () => {
    const {w, c, ch} = await showing();
    ch.onFrame({type: 'error', code: 'bad_request'});
    expect(c.state).toBe('Unpaired');
    expect(w.last.detail).toBe('error:bad_request');
  });

  it('a grant whose key is not a secret is not stored', async () => {
    const {w, c, ch} = await showing();
    ch.onFrame({type: 'pairGrant', key: 'short', endpoints: [URL]});
    await flush();
    expect(w.saved).toEqual([]);
    expect(c.state).toBe('Unpaired');
  });

  it('a key that cannot be stored is not acknowledged', async () => {
    const {w, c, ch} = await showing();
    w.saveResult = false;
    ch.onFrame({type: 'pairGrant', key: secretOf(7), endpoints: [URL]});
    await flush();
    expect(c.state).toBe('Unpaired');
    expect(w.last.note).toBe('storeFailed');
    expect(ch.sent.some(f => f.type === 'pairAck')).toBe(false);
    expect(ch.closed).toBe(1);
  });

  it('a store that throws is the same as one that fails', async () => {
    const {w, c, ch} = await showing();
    w.saveResult = new Error('disk');
    ch.onFrame({type: 'pairGrant', key: secretOf(7), endpoints: [URL]});
    await flush();
    expect(c.state).toBe('Unpaired');
    expect(w.last.note).toBe('storeFailed');
  });

  it('an ack that cannot be sent does not undo a stored key', async () => {
    const {c, ch} = await showing();
    ch.onFrame({type: 'pairGrant', key: secretOf(7), endpoints: [URL]});
    ch.throwOnSend = true; // the connection died while the key was being stored
    await flush();
    expect(c.state).toBe('Paired');
  });

  it('a request that cannot be sent ends the attempt', async () => {
    const {w, c} = controller();
    c.tap();
    const ch = new FakeChannel();
    ch.throwOnSend = true;
    w.connects[0].ok(ch);
    await flush();
    expect(c.state).toBe('Unpaired');
    expect(w.last).toMatchObject({note: 'disconnected', detail: 'send failed'});
  });

  it('a second press while connecting or showing changes nothing', async () => {
    const {w, c} = controller();
    c.tap();
    c.tap();
    expect(w.connects).toHaveLength(1);
    w.connects[0].ok(new FakeChannel());
    await flush();
    c.tap();
    expect(c.state).toBe('ShowingCode');
    expect(w.connects).toHaveLength(1);
  });

  it('a second grant while storing is ignored', async () => {
    const {w, c, ch} = await showing();
    ch.onFrame({type: 'pairGrant', key: secretOf(7), endpoints: [URL]});
    ch.onFrame({type: 'pairGrant', key: secretOf(8), endpoints: [URL]});
    await flush();
    expect(w.saved).toHaveLength(1);
    expect(c.state).toBe('Paired');
  });

  it('a grant before any request, and other events out of turn, are recorded and change nothing', async () => {
    const {w, c} = controller();
    c.tap();
    const ch = new FakeChannel();
    w.connects[0].ok(ch);
    await flush();
    c.keyRefused(); // not paired yet: cannot happen
    expect(c.state).toBe('ShowingCode');
    expect(w.logs).toContain('pair.never');
  });
});

describe('giving up', () => {
  it('cancelling while connecting drops the channel that arrives later', async () => {
    const {w, c} = controller();
    c.tap();
    c.cancel();
    expect(c.state).toBe('Unpaired');
    expect(w.last.note).toBe('cancelled');
    const late = new FakeChannel();
    w.connects[0].ok(late);
    await flush();
    expect(late.closed).toBe(1);
    expect(c.state).toBe('Unpaired');
  });

  it('a failure that arrives after cancelling is not shown', async () => {
    const {w, c} = controller();
    c.tap();
    c.cancel();
    w.connects[0].fail(new Error('late'));
    await flush();
    expect(w.last.note).toBe('cancelled');
  });

  it('cancelling while the code is shown closes the channel and stops the timers', async () => {
    const {w, c, ch} = await showing();
    c.cancel();
    expect(ch.closed).toBe(1);
    expect(c.state).toBe('Unpaired');
    const n = w.views.length;
    w.advance(200_000);
    expect(w.views.length).toBe(n);
  });

  it('frames and closes of a cancelled attempt are dropped, even if a new attempt began', async () => {
    const {w, c, ch} = await showing();
    c.cancel();
    c.tap();
    w.connects[1].ok(new FakeChannel());
    await flush();
    expect(c.state).toBe('ShowingCode');
    ch.onFrame({type: 'pairGrant', key: secretOf(7), endpoints: [URL]}); // from the first attempt
    ch.onClose('late close');
    await flush();
    expect(c.state).toBe('ShowingCode');
    expect(w.saved).toEqual([]);
  });

  it('cancelling does not interrupt storing (a half-written key would be worse)', async () => {
    const {c, ch} = await showing();
    ch.onFrame({type: 'pairGrant', key: secretOf(7), endpoints: [URL]});
    c.cancel();
    await flush();
    expect(c.state).toBe('Paired');
  });

  it('cancelling when nothing is going on changes nothing', () => {
    const {c} = controller('Unpaired');
    c.cancel();
    expect(c.state).toBe('Unpaired');
  });
});

describe('a key the bridge refuses', () => {
  it('sends a paired device back to pairing', () => {
    const {w, c} = controller('Paired');
    c.keyRefused();
    expect(c.state).toBe('Unpaired');
    expect(w.last.note).toBe('refused');
  });
});

describe('the table of the device', () => {
  it('has a handler for every action and a cell for every state and event', () => {
    for (const s of PAIR_STATES) {
      for (const e of PAIR_EVENTS) {
        expect(PAIR_TABLE[s][e]).toBeDefined();
      }
    }
  });

  // every cell of the table, run on the machine itself: what the document says is what happens
  const handlers = () => {
    const called: string[] = [];
    const h = new Proxy(
      {},
      {get: (_t, k) => () => called.push(String(k))},
    ) as never;
    return {called, h};
  };
  it.each(PAIR_STATES.flatMap(s => PAIR_EVENTS.map(e => [s, e] as const)))(
    '%s × %s',
    (s, e) => {
      const {called, h} = handlers();
      const logs: string[] = [];
      const m = new PairMachine(h, k => logs.push(k), s);
      const cell = PAIR_TABLE[s][e];
      m.dispatch(e, 'arg');
      switch (cell.k) {
        case 'never':
          expect(m.state).toBe(s);
          expect(called).toEqual([]);
          expect(logs).toContain('pair.never');
          break;
        case 'ignore':
          expect(m.state).toBe(s);
          expect(called).toEqual([]);
          break;
        case 'do':
          expect(m.state).toBe(cell.next);
          expect(called).toEqual([cell.action]);
          break;
        default:
          throw new Error('this table has no guarded cells');
      }
    },
  );
});

describe('the first frame of a pairing', () => {
  it('names the device and the code, and asks for nothing else', async () => {
    const {ch} = await showing();
    expect(ch.sent[0]).toEqual({
      type: 'pairRequest',
      id: ID,
      gen: 2,
      code: '12345678',
    });
  });
});
