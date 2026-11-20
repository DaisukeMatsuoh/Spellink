// The Spellink Bridge App page (bridge/internal/server/appui): its whole flow in a fake browser.
/* eslint-disable @typescript-eslint/no-var-requires */
export {}; // a module of its own: the page tests share nothing with each other
const dir = '../../bridge/internal/server';
const logic = require(`${dir}/pairpage/pairlogic.js`);
const app = require(`${dir}/appui/app.js`);

// every message is "<lang>:<key>" with the values after it, so a test sees which message was used
const words: Record<string, Record<string, string>> = {
  en: new Proxy(
    {},
    {get: (_t, k: string) => `${k} {name}{why}{path}{version}{until}{last}`},
  ) as never,
};

class Element {
  children: Element[] = [];
  attrs: Record<string, string> = {};
  className = '';
  textContent = '';
  value = '';
  onclick: (() => void) | null = null;
  constructor(readonly tag = 'div') {}
  appendChild(c: Element) {
    this.children.push(c);
  }
  replaceChildren() {
    this.children = [];
  }
  setAttribute(k: string, v: string) {
    this.attrs[k] = v;
  }
  set innerHTML(_v: string) {
    throw new Error(
      'the page must build its elements, never parse HTML (a device name could carry markup)',
    );
  }
  /** Every text in this subtree, in order. */
  texts(): string[] {
    return [this.textContent, ...this.children.flatMap(c => c.texts())].filter(
      Boolean,
    );
  }
  find(pred: (e: Element) => boolean): Element[] {
    return [
      ...(pred(this) ? [this] : []),
      ...this.children.flatMap(c => c.find(pred)),
    ];
  }
}

interface Dev {
  id: string;
  name: string;
  status: string;
  expires?: string;
  lastSeen?: string;
  waiting?: number;
}
const status = (over: Record<string, unknown> = {}) => ({
  version: '1.0',
  name: 'Rock',
  provider: {name: 'Claude Code', found: true, path: '/usr/bin/claude'},
  relay: true,
  addresses: ['wss://x/v1'],
  ...over,
});

type Reply = {status?: number; json?: unknown; text?: string} | Error;

function browser(
  o: {
    hash?: string;
    devices?: Dev[];
    status?: object;
    confirm?: boolean;
    replies?: Record<string, Reply[]>; // by "METHOD path"; consumed in order, then the default
    languages?: string[];
  } = {},
) {
  const els: Record<string, Element> = {};
  const el = (id: string) => (els[id] ??= new Element());
  const calls: Array<{
    method: string;
    path: string;
    body?: unknown;
    auth?: string;
  }> = [];
  const downloads: Array<{name: string; blob: unknown}> = [];
  const timers: Array<{at: number; f: () => void}> = [];
  let now = 0;
  const location = {hash: o.hash ?? '#tok', href: '/app'};
  const confirms: string[] = [];
  const queued = Object.fromEntries(
    Object.entries(o.replies ?? {}).map(([k, v]) => [k, [...v]]),
  );

  const respond = (r: Reply) => {
    if (r instanceof Error) {
      throw r;
    }
    const code = r.status ?? 200;
    return {
      ok: code >= 200 && code < 300,
      status: code,
      json: async () => r.json,
      text: async () => r.text ?? '',
      blob: async () => ({pkg: true}),
    };
  };

  const env = {
    logic,
    document: {
      title: '',
      getElementById: el,
      createElement: (tag: string) => new Element(tag),
    },
    navigator: {languages: o.languages ?? ['en'], language: 'en'},
    location,
    setTimeout: (f: () => void, ms = 0) => timers.push({at: now + ms, f}),
    confirm: (m: string) => {
      confirms.push(m);
      return o.confirm ?? true;
    },
    download: (blob: unknown, name: string) => downloads.push({name, blob}),
    fetch: async (
      path: string,
      init?: {method?: string; headers?: Record<string, string>; body?: string},
    ) => {
      if (path === '/app/strings.json') {
        return {json: async () => words};
      }
      const method = init?.method ?? 'GET';
      calls.push({
        method,
        path,
        body: init?.body ? JSON.parse(init.body) : undefined,
        auth: init?.headers?.Authorization,
      });
      const key = `${method} ${path}`;
      const q = queued[key];
      if (q?.length) {
        return respond(q.shift()!);
      }
      if (path === '/admin/status') {
        return respond({json: o.status ?? status()});
      }
      if (path === '/admin/devices' && method === 'GET') {
        return respond({json: o.devices ?? []});
      }
      if (path === '/admin/devices' && method === 'POST') {
        return respond({
          status: 201,
          json: {id: 'ffffffffffffffff', name: 'x'},
        });
      }
      return respond({status: path.endsWith('/package') ? 200 : 204});
    },
  };
  const flush = () => new Promise(r => setImmediate(r));
  const advance = async (ms: number) => {
    now += ms;
    const due = timers.filter(t => t.at <= now);
    due.forEach(t => timers.splice(timers.indexOf(t), 1));
    due.forEach(t => t.f());
    await flush();
    await flush();
  };
  const started = async () => {
    await app.start(env);
    await flush();
  };
  const says = () => els.message?.attrs['data-key'];
  return {
    env,
    el,
    els,
    calls,
    downloads,
    confirms,
    location,
    started,
    advance,
    flush,
    says,
    timers,
  };
}

const dev = (status: string, extra: Partial<Dev> = {}): Dev => ({
  id: '0123456789abcdef',
  name: 'Nomad',
  status,
  ...extra,
});
const buttons = (b: ReturnType<typeof browser>) =>
  b.el('devices').find(e => e.tag === 'button');
const labels = (b: ReturnType<typeof browser>) =>
  buttons(b).map(x => x.textContent.split(' ')[0]);

describe('what the page shows', () => {
  it('draws the bridge, the AI program, the relay and no warning when all is well', async () => {
    const b = browser();
    await b.started();
    const lines = b.el('status').children;
    expect(lines.map(l => l.className)).toEqual([
      'line info',
      'line ok',
      'line ok',
    ]);
    expect(lines[0].textContent).toMatch(/^bridgeLine /);
    expect(b.calls.map(c => c.auth)).toEqual(['Bearer tok', 'Bearer tok']); // the token goes in a header, nowhere else
    expect(b.env.document.title).toMatch(/^title /);
  });

  it('warns when the AI program is missing, when there is no relay, and when no address is set', async () => {
    const b = browser({
      status: status({
        provider: {name: 'Claude Code', found: false},
        relay: false,
        addresses: [],
      }),
    });
    await b.started();
    expect(b.el('status').children.map(l => l.className)).toEqual([
      'line info',
      'line bad',
      'line info',
      'line bad',
    ]);
    expect(b.el('status').children[1].textContent).toMatch(/^providerMissing /);
    expect(b.el('status').children[3].textContent).toMatch(/^noAddress /);
  });

  it('says there are no devices when there are none', async () => {
    const b = browser();
    await b.started();
    expect(b.el('devices').children).toHaveLength(1);
    expect(b.el('devices').children[0].textContent).toMatch(/^noDevices /);
  });

  it.each([
    ['issued', ['actionSave', 'actionRevoke', 'actionReissue']],
    ['waiting', ['actionPair', 'actionRevoke']],
    ['granting', ['actionRevoke']],
    ['active', ['actionRevoke', 'actionReissue']],
    ['expired', ['actionReissue', 'actionForget']],
    ['revoked', ['actionForget']],
    ['something new', []],
  ])('offers the right actions for a device that is %s', async (st, want) => {
    const b = browser({devices: [dev(st)]});
    await b.started();
    expect(labels(b)).toEqual(want);
    expect(b.el('devices').children[0].children[0].textContent).toBe('Nomad');
    expect(b.el('devices').children[0].children[1].textContent).toMatch(
      new RegExp(`^state${st[0].toUpperCase()}${st.slice(1)} `),
    );
  });

  it('shows the dates, and "never" for a date it cannot read', async () => {
    const b = browser({
      devices: [
        dev('active', {lastSeen: 'not a date'}),
        dev('issued', {
          id: 'ffffffffffffffff',
          name: 'B',
          expires: '2026-11-21T00:00:00Z',
        }),
      ],
    });
    await b.started();
    const states = b.el('devices').children.map(r => r.children[1].textContent);
    expect(states[0]).toMatch(/^stateActive /);
    expect(states[0]).toContain('never');
    expect(states[1]).not.toContain('Invalid');
  });

  it('never parses markup from a device name', async () => {
    const evil = '<img src=x onerror=alert(1)>';
    const b = browser({devices: [dev('active', {name: evil})]});
    await b.started(); // the fake element throws if innerHTML is ever set
    expect(b.el('devices').children[0].children[0].textContent).toBe(evil);
  });

  it('refreshes every few seconds', async () => {
    const b = browser();
    await b.started();
    const n = b.calls.length;
    await b.advance(4999);
    expect(b.calls.length).toBe(n);
    await b.advance(1);
    expect(b.calls.length).toBe(n + 2);
  });
});

describe('when the page cannot do its work', () => {
  it('says how to open it, and asks the bridge nothing, without the token', async () => {
    const b = browser({hash: ''});
    await b.started();
    expect(b.says()).toBe('noToken');
    expect(b.calls).toHaveLength(0);
  });

  it('says it is not allowed on a 401 and stops asking', async () => {
    const b = browser({replies: {'GET /admin/status': [{status: 401}]}});
    await b.started();
    expect(b.says()).toBe('unauthorized');
    const n = b.calls.length;
    await b.advance(20000);
    expect(b.calls.length).toBe(n);
  });

  it('says the bridge does not answer when the request fails or is refused, and keeps trying', async () => {
    const b = browser({
      replies: {
        'GET /admin/status': [new Error('offline')],
        'GET /admin/devices': [new Error('offline')],
      },
    });
    await b.started();
    expect(b.says()).toBe('unreachable');
    const refused = browser({
      replies: {'GET /admin/devices': [{status: 500, text: 'boom'}]},
    });
    await refused.started();
    expect(refused.says()).toBe('unreachable');
    const first = browser({
      replies: {'GET /admin/status': [{status: 500, text: 'boom'}]},
    });
    await first.started();
    expect(first.says()).toBe('unreachable');
    await b.advance(5000);
    expect(
      b.calls.filter(c => c.path === '/admin/status').length,
    ).toBeGreaterThan(1);
  });
});

describe('what the owner can do', () => {
  it('removes a device only after confirming, then reloads', async () => {
    const no = browser({devices: [dev('active')], confirm: false});
    await no.started();
    buttons(no)[0].onclick!();
    await no.flush();
    expect(no.confirms[0]).toMatch(/^confirmRevoke /);
    expect(no.calls.some(c => c.method === 'POST')).toBe(false);

    const yes = browser({devices: [dev('active')]});
    await yes.started();
    const loads = yes.calls.length;
    buttons(yes)[0].onclick!();
    await yes.flush();
    await yes.flush();
    expect(yes.calls.find(c => c.method === 'POST')?.path).toBe(
      '/admin/devices/0123456789abcdef/revoke',
    );
    expect(yes.says()).toBe('doneRevoke');
    expect(yes.calls.length).toBeGreaterThan(loads + 1); // reloaded
  });

  it('forgets a device after confirming', async () => {
    const b = browser({devices: [dev('revoked')]});
    await b.started();
    buttons(b)[0].onclick!();
    await b.flush();
    await b.flush();
    expect(b.confirms[0]).toMatch(/^confirmForget /);
    expect(b.says()).toBe('doneForget');
  });

  it('makes a new setup file when it reissues, and saves it at once', async () => {
    const b = browser({devices: [dev('active')]});
    await b.started();
    buttons(b)[1].onclick!();
    for (let i = 0; i < 5; i++) {
      await b.flush();
    }
    expect(b.confirms[0]).toMatch(/^confirmReissue /);
    expect(b.calls.map(c => `${c.method} ${c.path}`)).toContain(
      'POST /admin/devices/0123456789abcdef/reissue',
    );
    expect(b.downloads).toHaveLength(1);
    expect(b.downloads[0].name).toBe('Spellink-Nomad.snplg');
    expect(b.says()).toBe('doneReissue');
  });

  it('shows what the bridge refused to do, in its own words', async () => {
    const b = browser({
      devices: [dev('active')],
      replies: {
        'POST /admin/devices/0123456789abcdef/revoke': [
          {status: 409, text: 'the device is already revoked\n'},
        ],
      },
    });
    await b.started();
    buttons(b)[0].onclick!();
    await b.flush();
    await b.flush();
    expect(b.says()).toBe('failed');
    expect(b.el('message').textContent).toContain(
      'the device is already revoked',
    );
  });

  it('says so when an action cannot reach the bridge', async () => {
    const b = browser({
      devices: [dev('active')],
      replies: {
        'POST /admin/devices/0123456789abcdef/revoke': [new Error('offline')],
      },
    });
    await b.started();
    buttons(b)[0].onclick!();
    await b.flush();
    await b.flush();
    expect(b.says()).toBe('unreachable');
  });

  it('saves the setup file under a safe name, or says why it could not', async () => {
    const b = browser({devices: [dev('issued', {name: 'My Nomad/../x'})]});
    await b.started();
    buttons(b)[0].onclick!();
    await b.flush();
    await b.flush();
    expect(b.downloads.map(d => d.name)).toEqual([
      'Spellink-My_Nomad_.._x.snplg',
    ]);
    expect(b.says()).toBe('doneSave');

    const bad = browser({
      devices: [dev('issued')],
      replies: {
        'GET /admin/devices/0123456789abcdef/package': [
          {status: 409, text: 'the first secret has expired'},
        ],
      },
    });
    await bad.started();
    buttons(bad)[0].onclick!();
    await bad.flush();
    await bad.flush();
    expect(bad.says()).toBe('failed');
    expect(bad.downloads).toHaveLength(0);
  });

  it('a refused body that cannot be read is reported as the status', async () => {
    const b = browser({devices: [dev('issued')]});
    await b.started();
    (b.env as {fetch: unknown}).fetch = async (path: string) =>
      path.endsWith('/package')
        ? {
            ok: false,
            status: 503,
            text: async () => {
              throw new Error('x');
            },
          }
        : {ok: true, status: 200, json: async () => []};
    buttons(b)[0].onclick!();
    await b.flush();
    await b.flush();
    expect(b.el('message').textContent).toContain('HTTP 503');
  });

  it('opens the pairing page for a device that is showing its code', async () => {
    const b = browser({devices: [dev('waiting')]});
    await b.started();
    buttons(b)[0].onclick!();
    expect(b.location.href).toBe('/pair#tok');
  });
});

describe('adding a device', () => {
  const type = async (b: ReturnType<typeof browser>, name: string) => {
    b.el('addName').value = name;
    b.el('addButton').onclick!();
    for (let i = 0; i < 6; i++) {
      await b.flush();
    }
  };

  it('asks for a name', async () => {
    const b = browser();
    await b.started();
    await type(b, '   ');
    expect(b.says()).toBe('nameEmpty');
    expect(b.calls.some(c => c.method === 'POST')).toBe(false);
  });

  it('adds it, saves its setup file, explains what to do next and links to the pairing page', async () => {
    const b = browser();
    await b.started();
    await type(b, '  Nomad ');
    expect(b.calls.find(c => c.method === 'POST')?.body).toEqual({
      name: 'Nomad',
    });
    expect(b.downloads.map(d => d.name)).toEqual(['Spellink-Nomad.snplg']);
    expect(b.el('addName').value).toBe('');
    expect(b.says()).toBe('added');
    const steps = b.el('steps').children;
    expect(steps.slice(0, 3).map(s => s.textContent.split(' ')[0])).toEqual([
      'step1',
      'step2',
      'step3',
    ]);
    expect(steps[3].attrs.href).toBe('/pair#tok');
  });

  it('shows what the bridge answered when it will not add the device', async () => {
    const b = browser({
      replies: {
        'POST /admin/devices': [
          {status: 409, text: 'a device with this name already exists'},
        ],
      },
    });
    await b.started();
    await type(b, 'Nomad');
    expect(b.says()).toBe('failed');
    expect(b.el('message').textContent).toContain('already exists');
    expect(b.downloads).toHaveLength(0);
    expect(b.el('steps').children).toHaveLength(0);
  });

  it('does not explain the next steps when the setup file could not be saved', async () => {
    const b = browser({
      replies: {
        'GET /admin/devices/ffffffffffffffff/package': [
          {status: 500, text: 'no base package'},
        ],
      },
    });
    await b.started();
    await type(b, 'Nomad');
    expect(b.says()).toBe('failed');
    expect(b.el('steps').children).toHaveLength(0);
  });

  it('says so when the bridge cannot be reached', async () => {
    const b = browser({
      replies: {'POST /admin/devices': [new Error('offline')]},
    });
    await b.started();
    await type(b, 'Nomad');
    expect(b.says()).toBe('unreachable');
    const noMessage = browser({
      replies: {
        'POST /admin/devices': [Object.assign(new Error(''), {message: ''})],
      },
    });
    await noMessage.started();
    await type(noMessage, 'Nomad');
    expect(noMessage.el('message').textContent).toContain('network');
  });
});

describe('the small pure parts', () => {
  it('makes a safe file name, falling back to the id', () => {
    expect(app.fileName('Nomad', 'i')).toBe('Spellink-Nomad.snplg');
    expect(app.fileName('My Nomad!', 'i')).toBe('Spellink-My_Nomad.snplg');
    expect(app.fileName('../../etc/passwd', 'i')).toBe(
      'Spellink-etc_passwd.snplg',
    );
    expect(app.fileName('ノート', 'abc')).toBe('Spellink-abc.snplg');
    expect(app.fileName('...', 'abc')).toBe('Spellink-abc.snplg');
  });

  it('knows the actions of every state and none for an unknown one', () => {
    expect(app.actionsFor('issued')).toEqual(['save', 'revoke', 'reissue']);
    expect(app.actionsFor('nonsense')).toEqual([]);
  });
});
