// The pairing page of the bridge (bridge/assets/pair): its logic, and its whole flow
// from the camera to the request, with a fake browser.
/* eslint-disable @typescript-eslint/no-var-requires */
const dir = '../../bridge/assets/pair';
const logic = require(`${dir}/pairlogic.js`);
const page = require(`${dir}/pair.js`);

const ID = '0123456789abcdef';
const QR = `spellink://pair?v=1&i=${ID}&c=12345678`;

describe('what the page understands', () => {
  it('reads the QR code a device shows', () => {
    expect(logic.parseQr(QR)).toEqual({id: ID, code: '12345678'});
  });

  it.each([
    ['another scheme', `https://pair?v=1&i=${ID}&c=12345678`],
    ['another version', `spellink://pair?v=2&i=${ID}&c=12345678`],
    ['an id of the wrong length', `spellink://pair?v=1&i=0123&c=12345678`],
    [
      'an upper case id',
      `spellink://pair?v=1&i=${ID.toUpperCase()}&c=12345678`,
    ],
    ['a short code', `spellink://pair?v=1&i=${ID}&c=1234567`],
    ['a code with letters', `spellink://pair?v=1&i=${ID}&c=1234567a`],
    ['extra parts', `${QR}&x=1`],
    ['text around it', ` ${QR}`],
    ['not a string', 5],
    ['nothing', undefined],
    ['a web address', 'https://example.com/'],
  ])('does not take %s for a Spellink code', (_n, text) => {
    expect(logic.parseQr(text)).toBeNull();
  });

  it('reads a typed code, with the spaces and dash a person may add', () => {
    expect(logic.parseTyped('12345678')).toBe('12345678');
    expect(logic.parseTyped('1234 5678')).toBe('12345678');
    expect(logic.parseTyped(' 1234-5678 ')).toBe('12345678');
    for (const bad of ['', '1234567', '123456789', 'abcdefgh', '1234 567a']) {
      expect(logic.parseTyped(bad)).toBeNull();
    }
  });

  it("picks the browser's language, the language part, or English", () => {
    const have = ['en', 'ja', 'zh_CN'];
    expect(logic.pickLocale(['ja-JP', 'en'], have)).toBe('ja');
    expect(logic.pickLocale(['zh-CN'], have)).toBe('zh_CN');
    expect(logic.pickLocale(['ZH_cn'], have)).toBe('zh_CN');
    expect(logic.pickLocale(['fr', 'ja'], have)).toBe('ja'); // the next wish
    expect(logic.pickLocale(['fr'], have)).toBe('en');
    expect(logic.pickLocale([], have)).toBe('en');
    expect(logic.pickLocale([undefined, ''], have)).toBe('en');
  });

  it('fills in values and leaves a missing one empty', () => {
    expect(logic.format('Hi {name}, {n}!', {name: 'Ann', n: 0})).toBe(
      'Hi Ann, 0!',
    );
    expect(logic.format('Hi {name}', {})).toBe('Hi ');
    expect(logic.format('Hi {name}')).toBe('Hi ');
    expect(logic.format('plain', {name: 'x'})).toBe('plain');
  });

  it.each([
    [200, 'paired'],
    [400, 'badCode'],
    [401, 'unauthorized'],
    [403, 'notAccepted'],
    [404, 'nobodyWaiting'],
    [500, 'failed'],
    [0, 'failed'],
  ])('answers status %i with %s', (status, key) => {
    expect(logic.messageFor(status)).toBe(key);
  });
});

// ---- the page, in a fake browser ----

const STRINGS = {
  en: {
    title: 'Pair',
    manualLabel: 'Type',
    manualButton: 'Go',
    hint: 'Show the QR',
    sending: 'Pairing…',
    paired: 'Paired: {name}',
    notSpellink: 'Not Spellink',
    badCode: 'Eight digits',
    notAccepted: 'Not accepted',
    nobodyWaiting: 'Nobody',
    unauthorized: 'Not allowed',
    noToken: 'Open with --web',
    cameraDenied: 'No camera ({why})',
    failed: 'Failed ({why})',
  },
  ja: {
    title: 'ペア',
    manualLabel: '入力',
    manualButton: '実行',
    hint: 'QRを見せて',
    sending: '送信中',
    paired: '完了: {name}',
    notSpellink: '違う',
    badCode: '8桁',
    notAccepted: '不可',
    nobodyWaiting: 'なし',
    unauthorized: '権限なし',
    noToken: '--webで開く',
    cameraDenied: 'カメラ不可（{why}）',
    failed: '失敗（{why}）',
  },
};

interface Opts {
  hash?: string;
  languages?: string[];
  getUserMedia?: unknown;
  detector?: 'qr' | 'none' | 'throws';
  decode?: string | null; // what jsQR / the detector finds
  responses?: Array<{status: number; json?: unknown} | Error>;
  noMediaDevices?: boolean;
}

function browser(o: Opts = {}) {
  const els: Record<string, any> = {};
  const el = (id: string) =>
    (els[id] ??= {
      id,
      textContent: '',
      value: '',
      attrs: {} as Record<string, string>,
      setAttribute(k: string, v: string) {
        this.attrs[k] = v;
      },
      play: jest.fn(),
      videoWidth: 640,
      videoHeight: 480,
    });
  let now = 0;
  const timers: Array<{at: number; f: () => void}> = [];
  const requests: Array<{url: string; init?: any}> = [];
  const tracks = {stopped: 0};
  const responses = [...(o.responses ?? [])];
  const doc = {
    title: '',
    getElementById: el,
    createElement: () => ({
      width: 0,
      height: 0,
      getContext: () => ({
        drawImage() {},
        getImageData: () => ({
          data: new Uint8ClampedArray(4),
          width: 1,
          height: 1,
        }),
      }),
    }),
  };
  const env: any = {
    logic,
    document: doc,
    navigator: {
      languages: o.languages ?? ['en'],
      language: 'en',
      ...(o.noMediaDevices
        ? {}
        : {
            mediaDevices: {
              getUserMedia:
                o.getUserMedia ??
                (async () => ({
                  getTracks: () => [{stop: () => tracks.stopped++}],
                })),
            },
          }),
    },
    location: {hash: o.hash ?? '#tok123'},
    setTimeout: (f: () => void, ms = 0) => timers.push({at: now + ms, f}),
    fetch: async (url: string, init?: any) => {
      requests.push({url, init});
      if (url === '/pair/strings.json') {
        return {json: async () => STRINGS};
      }
      const r = responses.shift() ?? {status: 200, json: {name: 'Nomad'}};
      if (r instanceof Error) {
        throw r;
      }
      return {status: r.status, json: async () => r.json ?? {}};
    },
    jsQR: () => (o.decode ? {data: o.decode} : null),
    BarcodeDetector:
      o.detector === 'qr' || o.detector === 'throws'
        ? {
            detect: async () => {
              if (o.detector === 'throws') {
                throw new Error('detector');
              }
              return o.decode ? [{rawValue: o.decode}] : [];
            },
          }
        : null,
  };
  const flush = () => new Promise(r => setImmediate(r));
  /** Runs the scan loop for a few rounds, letting promises settle in between. */
  const rounds = async (n = 4, stepMs = 150) => {
    for (let i = 0; i < n; i++) {
      await flush();
      now += stepMs;
      const due = timers.filter(t => t.at <= now);
      due.forEach(t => timers.splice(timers.indexOf(t), 1));
      due.forEach(t => t.f());
    }
    await flush();
  };
  return {env, els, el, requests, tracks, timers, rounds, flush};
}

const post = (b: ReturnType<typeof browser>) =>
  b.requests.filter(r => r.url === '/admin/pair');

describe('the pairing page in a browser', () => {
  it("speaks the browser's language and starts the camera", async () => {
    const b = browser({languages: ['ja-JP']});
    await page.start(b.env);
    await b.flush();
    expect(b.els.title.textContent).toBe('ペア');
    expect(b.env.document.title).toBe('ペア');
    expect(b.el('video').play).toHaveBeenCalled();
    expect(b.els.status.textContent).toBe('QRを見せて');
  });

  it("reads a QR code and sends the code with the device's id and the token", async () => {
    const b = browser({
      decode: QR,
      detector: 'qr',
      responses: [{status: 200, json: {name: 'Nomad'}}],
    });
    await page.start(b.env);
    await b.rounds();
    expect(post(b)).toHaveLength(1);
    const {init} = post(b)[0];
    expect(init.method).toBe('POST');
    expect(init.headers.Authorization).toBe('Bearer tok123');
    expect(JSON.parse(init.body)).toEqual({id: ID, code: '12345678'});
    expect(b.els.status.textContent).toBe('Paired: Nomad');
    expect(b.tracks.stopped).toBe(1); // the camera is switched off
    // and nothing more is sent however long the page stays open
    await b.rounds(3);
    expect(post(b)).toHaveLength(1);
  });

  it('reads with jsQR when the browser has no detector', async () => {
    const b = browser({decode: QR, detector: 'none'});
    await page.start(b.env);
    await b.rounds();
    expect(post(b)).toHaveLength(1);
    expect(b.els.status.textContent).toBe('Paired: Nomad');
  });

  it("a QR code that is not Spellink's is said so and never sent", async () => {
    const b = browser({decode: 'https://example.com/', detector: 'qr'});
    await page.start(b.env);
    await b.rounds();
    expect(b.els.status.attrs['data-key']).toBe('notSpellink');
    expect(post(b)).toHaveLength(0);
  });

  it('a refused code is said so, waits before it tries the same QR code again, and is not sent in a burst', async () => {
    const b = browser({
      decode: QR,
      detector: 'qr',
      responses: [
        {status: 403},
        {status: 403},
        {status: 200, json: {name: 'Nomad'}},
      ],
    });
    await page.start(b.env);
    await b.flush();
    await b.rounds(1);
    expect(b.els.status.attrs['data-key']).toBe('notAccepted');
    expect(post(b)).toHaveLength(1);
    // while paused no scan reads the camera again
    await b.rounds(2);
    expect(post(b)).toHaveLength(1);
  });

  it.each([
    [404, 'nobodyWaiting'],
    [401, 'unauthorized'],
    [400, 'badCode'],
    [500, 'failed'],
  ])('answers a %i from the bridge with %s', async (status, key) => {
    const b = browser({decode: QR, detector: 'qr', responses: [{status}]});
    await page.start(b.env);
    await b.rounds(2);
    expect(b.els.status.attrs['data-key']).toBe(key);
    if (key === 'failed') {
      expect(b.els.status.textContent).toBe('Failed (HTTP 500)');
    }
  });

  it('a network failure is said so and the page keeps working', async () => {
    const b = browser({
      decode: QR,
      detector: 'qr',
      responses: [new Error('offline')],
    });
    await page.start(b.env);
    await b.rounds(2);
    expect(b.els.status.textContent).toBe('Failed (offline)');
  });

  it('a network failure without a message still says something', async () => {
    const b = browser({
      decode: QR,
      detector: 'qr',
      responses: [Object.assign(new Error(''), {message: ''})],
    });
    await page.start(b.env);
    await b.rounds(2);
    expect(b.els.status.textContent).toBe('Failed (network)');
  });

  it('takes a typed code, and refuses a malformed one without asking the bridge', async () => {
    const b = browser({detector: 'none'});
    await page.start(b.env);
    await b.flush();
    b.el('manual').value = '1234';
    b.el('manualButton').onclick();
    expect(b.els.status.attrs['data-key']).toBe('badCode');
    expect(post(b)).toHaveLength(0);
    b.el('manual').value = '1234 5678';
    b.el('manualButton').onclick();
    await b.flush();
    expect(JSON.parse(post(b)[0].init.body)).toEqual({code: '12345678'}); // no id: the bridge looks at every waiting device
    expect(b.els.status.textContent).toBe('Paired: Nomad');
  });

  it('does not send twice while a request is on its way', async () => {
    const b = browser({detector: 'none'});
    await page.start(b.env);
    await b.flush();
    b.el('manual').value = '12345678';
    b.el('manualButton').onclick();
    b.el('manualButton').onclick();
    await b.flush();
    expect(post(b)).toHaveLength(1);
  });

  it('a page opened without the token says how to open it, and starts no camera', async () => {
    const b = browser({hash: ''});
    await page.start(b.env);
    await b.flush();
    expect(b.els.status.attrs['data-key']).toBe('noToken');
    expect(b.el('video').play).not.toHaveBeenCalled();
  });

  it('says so when the camera is refused or missing, and the typed code still works', async () => {
    const denied = browser({
      getUserMedia: async () => {
        throw Object.assign(new Error('x'), {name: 'NotAllowedError'});
      },
    });
    await page.start(denied.env);
    await denied.flush();
    expect(denied.els.status.textContent).toBe('No camera (NotAllowedError)');
    denied.el('manual').value = '12345678';
    denied.el('manualButton').onclick();
    await denied.flush();
    expect(denied.els.status.textContent).toBe('Paired: Nomad');

    const unnamed = browser({
      getUserMedia: async () => {
        throw {};
      },
    });
    await page.start(unnamed.env);
    await unnamed.flush();
    expect(unnamed.els.status.textContent).toBe('No camera (unknown)');

    const none = browser({noMediaDevices: true});
    await page.start(none.env);
    await none.flush();
    expect(none.els.status.textContent).toBe('No camera (no camera API)');
  });

  it('keeps looking when the detector fails on a frame', async () => {
    const b = browser({decode: QR, detector: 'throws'});
    await page.start(b.env);
    await b.rounds(3);
    expect(post(b)).toHaveLength(0);
    expect(b.timers.length + 1).toBeGreaterThan(0); // the loop is still alive
  });

  it('shows the name field as empty if the bridge answers oddly', async () => {
    const b = browser({
      decode: QR,
      detector: 'qr',
      responses: [{status: 200, json: {name: 5}}],
    });
    await page.start(b.env);
    await b.rounds();
    expect(b.els.status.textContent).toBe('Paired: ');
  });

  it('waits for a camera frame before it reads', async () => {
    const b = browser({decode: QR, detector: 'qr'});
    b.el('video').videoWidth = 0; // no frame yet
    await page.start(b.env);
    await b.flush();
    await b.rounds(2);
    expect(post(b)).toHaveLength(0);
    b.el('video').videoWidth = 640;
    await b.rounds(2);
    expect(post(b)).toHaveLength(1);
  });

  it('falls back to its English words key when a message is missing', async () => {
    const b = browser({decode: QR, detector: 'qr', responses: [{status: 418}]});
    delete (STRINGS.en as Record<string, string>).failed;
    try {
      await page.start(b.env);
      await b.rounds(2);
      expect(b.els.status.textContent).toBe('failed');
    } finally {
      (STRINGS.en as Record<string, string>).failed = 'Failed ({why})';
    }
  });
});
