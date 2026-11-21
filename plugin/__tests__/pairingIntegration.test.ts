// The pairing against the real bridge: the owner's side is the real command line (`spellink-bridge
// device …`), the device's side is the plugin's own code (setup parsing, the channel with Noise,
// the controller, the store, the connection with K). It checks that the two programs agree on
// what no unit test of either side can: the setup line in the package, the frames, the secrets.
//
// It needs the Go toolchain and `zip`/`unzip`; without them the tests are skipped.
import {execFileSync, spawn, type ChildProcess} from 'child_process';
import fs from 'fs';
import net from 'net';
import os from 'os';
import path from 'path';
import {connectFirst, NoBridgeError} from '../src/bridgeClient';
import {parsePairSetup, type PairSetup} from '../src/pairConfig';
import {connectPairing} from '../src/pairConnect';
import {PairingController, type PairDeps, type PairView} from '../src/pairing';
import {planBridges} from '../src/pairPlan';
import {readStore, saveToStore, type FileOps} from '../src/pairStore';
import {resetTraceForTests} from '../src/trace';

// eslint-disable-next-line @typescript-eslint/no-var-requires
const WS = require('ws');

const REPO = path.join(__dirname, '../..');

function tool(candidates: string[], probe: string[]): string | null {
  for (const c of candidates) {
    try {
      execFileSync(c, probe, {stdio: 'ignore'});
      return c;
    } catch {
      // try the next
    }
  }
  return null;
}
// (the name is left from the Go bridge: it is the Rust toolchain that builds it now)
const go = tool(
  [path.join(os.homedir(), '.cargo/bin/cargo'), 'cargo'],
  ['--version'],
);
const zip = tool(['zip'], ['-v']);
const unzip = tool(['unzip'], ['-v']);
const suite = go && zip && unzip ? describe : describe.skip;

const free = () =>
  new Promise<number>(resolve => {
    const s = net.createServer();
    s.listen(0, '127.0.0.1', () => {
      const {port} = s.address() as net.AddressInfo;
      s.close(() => resolve(port));
    });
  });
const pause = (ms: number) => new Promise(r => setTimeout(r, ms));
async function until<T>(
  what: string,
  f: () => T | undefined | false,
  ms = 8000,
): Promise<T> {
  const end = Date.now() + ms;
  for (;;) {
    const v = f();
    if (v) {
      return v;
    }
    if (Date.now() > end) {
      throw new Error(`timed out waiting for ${what}`);
    }
    await pause(20);
  }
}

suite('pairing a device, for real', () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'spellink-pair-'));
  const home = path.join(tmp, 'cfg');
  const env = {...process.env, HOME: tmp, XDG_CONFIG_HOME: home};
  let exe = '';
  let bridge: ChildProcess | null = null;
  let port = 0;
  let log = '';

  /** Runs the bridge's command line as the owner would; returns what it printed (or the error). */
  const cli = (...args: string[]): string => {
    try {
      return execFileSync(exe, ['device', ...args], {
        env,
        cwd: tmp,
        encoding: 'utf8',
        stdio: 'pipe',
        timeout: 20000,
      });
    } catch (e) {
      const err = e as {stdout?: string; stderr?: string};
      return `FAILED: ${err.stderr ?? ''}${err.stdout ?? ''}`;
    }
  };

  /** What is in the folder the device keeps its pairing in (a plain map instead of the SDK). */
  const folder = new Set<string>();
  const ops: FileOps = {
    async list() {
      return folder.size ? [...folder] : null;
    },
    async makeDir(p) {
      folder.add(p.split('/').pop()!);
      return true;
    },
  };

  /** The setup line the bridge put at the top of a package, as the plugin reads it. */
  function setupOf(snplg: string): PairSetup {
    const bundle = execFileSync(unzip!, ['-p', snplg, 'Spellink.bundle'], {
      encoding: 'utf8',
    });
    const line = bundle.split('\n')[0];
    const prefix = 'globalThis.__SPELLINK_PAIR__=';
    expect(line.startsWith(prefix) && line.endsWith(';')).toBe(true);
    const parsed = parsePairSetup(JSON.parse(line.slice(prefix.length, -1)));
    if (!('setup' in parsed)) {
      throw new Error(
        `the plugin cannot read the bridge's setup: ${parsed.problem}`,
      );
    }
    return parsed.setup;
  }

  const realDeps = (views: PairView[]): PairDeps => ({
    connect: (urls, b, ms) => connectPairing(urls, b, ms, WS),
    save: b => saveToStore(ops, b),
    digits: n => '48151623'.slice(0, n),
    after: (ms, f) => {
      const t = setTimeout(f, ms);
      return () => clearTimeout(t);
    },
    every: (ms, f) => {
      const t = setInterval(f, ms);
      return () => clearInterval(t);
    },
    now: () => Date.now(),
    banner: () => ({text: 'ああ ああ', width: 600, charSize: 40}),
    onView: v => views.push(v),
    log: () => {},
  });

  beforeAll(async () => {
    (globalThis as unknown as {WebSocket: unknown}).WebSocket = WS;
    exe = path.join(tmp, 'bridge');
    execFileSync(go!, ['build', '--locked'], {
      cwd: path.join(REPO, 'bridge'),
      timeout: 600000,
    });
    fs.copyFileSync(path.join(REPO, 'bridge/target/debug/spellink-bridge'), exe);
    fs.chmodSync(exe, 0o755);
    const cfgDir = path.join(home, 'spellink-bridge');
    fs.mkdirSync(cfgDir, {recursive: true});
    execFileSync(exe, ['testfont', path.join(cfgDir, 'glyphs.bin')], {env});
    // the public plugin package the owner's setup files are made from
    const pkg = path.join(tmp, 'pkg');
    fs.mkdirSync(pkg);
    fs.writeFileSync(
      path.join(pkg, 'PluginConfig.json'),
      '{"name":"Spellink"}',
    );
    fs.writeFileSync(
      path.join(pkg, 'Spellink.bundle'),
      'var __BUNDLE_START_TIME__=1;\n',
    );
    execFileSync(zip!, ['-q', '-r', path.join(tmp, 'Spellink.snplg'), '.'], {
      cwd: pkg,
    });
    port = await free();
    fs.writeFileSync(
      path.join(cfgDir, 'config.json'),
      JSON.stringify({
        name: 'Rock',
        port,
        token: 'unused-shared-token',
        claudeExecutable: '/bin/false',
        tiltDegrees: 0,
        extraUrls: ['wss://bridge.test/v1'],
        pairing: {
          windowSec: 30,
          pluginPackage: path.join(tmp, 'Spellink.snplg'),
        },
      }),
    );
    bridge = spawn(exe, ['serve'], {cwd: tmp, env});
    bridge.stdout!.on('data', d => (log += d));
    bridge.stderr!.on('data', d => (log += d));
    for (let i = 0; i < 100; i++) {
      try {
        if ((await fetch(`http://127.0.0.1:${port}/health`)).ok) {
          return;
        }
      } catch {
        // not up yet
      }
      await pause(100);
    }
    throw new Error('the bridge did not start\n' + log);
  }, 240000);

  afterAll(() => {
    bridge?.kill();
    fs.rmSync(tmp, {recursive: true, force: true});
  });

  beforeEach(() => resetTraceForTests());

  it('adds a device, pairs it with the code, connects with the new key, and revokes it', async () => {
    // 1. the owner adds a device: the setup file the plugin will be installed from
    const added = cli('add', 'Nomad', tmp);
    expect(added).toContain('Spellink-Nomad.snplg');
    const file = path.join(tmp, 'Spellink-Nomad.snplg');
    expect((fs.statSync(file).mode & 0o777).toString(8)).toBe('600');
    const setup = setupOf(file);
    expect(setup).toMatchObject({gen: 1, name: 'Rock', windowSec: 30});
    expect(setup.b).toBeDefined();
    expect(setup.urls).toEqual([`wss://bridge.test/v1?d=${setup.id}`]);

    // 2. what the device would do first: it has a package and an empty store
    const plan = planBridges(setup, await readStore(ops));
    expect(plan.bridges).toMatchObject([{id: setup.id, state: 'Unpaired'}]);

    // 3. the device presses Pair (the test reaches the bridge over ws on localhost: the `wss`
    //    address in the package is not a real host)
    const views: PairView[] = [];
    const local: PairSetup = {
      ...setup,
      urls: [`ws://127.0.0.1:${port}/v1?d=${setup.id}`],
    };
    const controller = new PairingController(
      local,
      realDeps(views),
      'Unpaired',
    );
    controller.tap();
    await until(
      'the code on the screen',
      () => controller.state === 'ShowingCode',
    );
    const shown = views[views.length - 1];
    expect(shown).toMatchObject({code: '48151623', note: 'showing'});
    expect(shown.qr).toBe(`spellink://pair?v=1&i=${setup.id}&c=48151623`);
    expect(cli('list')).toMatch(/Nomad.*waiting/);
    // the bridge wrote the phrase in its own handwriting (the test font has hiragana)
    const banner = await until(
      'the handwritten banner',
      () => views.find(v => v.banner)?.banner,
    );
    expect(banner.strokes.length).toBeGreaterThan(0);
    expect(banner.width).toBeGreaterThan(40);
    expect(banner.height).toBeGreaterThan(20);

    // 4. a wrong code is refused and tells nobody who is waiting
    const wrong = cli('pair', '00000000');
    expect(wrong).toMatch(/not accepted/);
    expect(wrong).not.toContain('Nomad');
    expect(controller.state).toBe('ShowingCode');

    // 5. the right code
    expect(cli('pair', '48151623')).toContain('Code accepted');
    await until('the pairing to finish', () => controller.state === 'Paired');
    expect(views[views.length - 1].note).toBe('paired');

    // 6. the device has stored the key and where the bridge is; nothing else was needed
    const stored = (await readStore(ops)).bridges;
    expect(stored).toHaveLength(1);
    expect(stored[0]).toMatchObject({
      id: setup.id,
      gen: 1,
      name: 'Rock',
      urls: [`wss://bridge.test/v1?d=${setup.id}`],
    });
    expect(stored[0].key).not.toBe(setup.b);
    await until('the bridge to see the device as active', () =>
      /Nomad\s+\S+\s+active/.test(cli('list')),
    );

    // 7. an update with a plain package (no setup at all) still knows the bridge
    expect(planBridges(null, await readStore(ops)).bridges).toMatchObject([
      {id: setup.id, state: 'Paired'},
    ]);

    // 8. the first secret no longer opens anything
    await expect(
      connectPairing(local.urls, setup.b!, 3000, WS),
    ).rejects.toThrow();

    // 9. the key opens a normal session (welcome and all)
    const k = stored[0].key;
    const conn = await connectFirst(
      [
        {
          id: setup.id,
          name: 'Rock',
          token: k,
          urls: [`ws://127.0.0.1:${port}/v1?d=${setup.id}`],
        },
      ],
      5000,
    );
    expect(conn.name).toBe('Rock');
    expect(conn.mode).toBe('noise');

    // 10. the owner revokes the device: the live session ends and the key stops working
    const closed = new Promise<string>(resolve => (conn.onClose = resolve));
    expect(cli('revoke', 'Nomad')).toContain('Revoked');
    await closed;
    // ...and the plugin can tell that it was turned away (not that the bridge is gone)
    const refused = await connectFirst(
      [
        {
          id: setup.id,
          name: 'Rock',
          token: k,
          urls: [`ws://127.0.0.1:${port}/v1?d=${setup.id}`],
        },
      ],
      3000,
    ).catch((e: unknown) => e);
    expect(refused).toBeInstanceOf(NoBridgeError);
    expect((refused as NoBridgeError).refused).toEqual(['Rock']);
    expect(cli('list')).toMatch(/Nomad.*revoked/);
  }, 60000);

  it('a request the owner never confirms is dropped by the bridge when the device gives up', async () => {
    cli('add', 'Manta', tmp);
    const setup = setupOf(path.join(tmp, 'Spellink-Manta.snplg'));
    const views: PairView[] = [];
    const controller = new PairingController(
      {
        ...setup,
        urls: [`ws://127.0.0.1:${port}/v1?d=${setup.id}`],
        windowSec: 10,
      },
      {...realDeps(views), digits: () => '11112222'},
      'Unpaired',
    );
    controller.tap();
    await until('the code', () => controller.state === 'ShowingCode');
    expect(cli('list')).toMatch(/Manta.*waiting/);
    controller.cancel(); // the owner never typed it; the user gives up
    expect(controller.state).toBe('Unpaired');
    await until('the bridge to drop the request', () =>
      /Manta\s+\S+\s+issued/.test(cli('list')),
    );
  }, 60000);

  it('a reissued device needs its new file; the old one is refused', async () => {
    cli('add', 'Boox', tmp);
    const oldFile = path.join(tmp, 'Spellink-Boox.snplg');
    const old = setupOf(oldFile);
    expect(cli('reissue', 'Boox')).toContain('Reissued');
    fs.rmSync(oldFile);
    expect(cli('package', 'Boox', tmp)).toContain('Wrote');
    const fresh = setupOf(oldFile);
    expect(fresh.id).toBe(old.id);
    expect(fresh.gen).toBe(old.gen + 1);
    expect(fresh.b).not.toBe(old.b);
    // the old package's first secret is not valid any more
    await expect(
      connectPairing(
        [`ws://127.0.0.1:${port}/v1?d=${old.id}`],
        old.b!,
        3000,
        WS,
      ),
    ).rejects.toThrow();
    // the new one pairs
    const views: PairView[] = [];
    const controller = new PairingController(
      {...fresh, urls: [`ws://127.0.0.1:${port}/v1?d=${fresh.id}`]},
      {...realDeps(views), digits: () => '33334444'},
      'Unpaired',
    );
    controller.tap();
    await until('the code', () => controller.state === 'ShowingCode');
    expect(cli('pair', '33334444')).toContain('Code accepted');
    await until('the pairing', () => controller.state === 'Paired');
    const mine = (await readStore(ops)).bridges.find(b => b.id === fresh.id);
    expect(mine).toMatchObject({gen: 2});
  }, 60000);
});
