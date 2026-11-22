// The plugin against the real bridge: the plugin's own code (Noise, frame validation, the ask
// flow, the writing into a note) over a real WebSocket to the bridge built from ../bridge, with
// a fake `claude` that answers from a script. It checks the two programs agree — what no unit
// test of either side can — and, if they ever stop agreeing, says so before a device does.
//
// It needs the Go toolchain (to build the bridge); without it the tests are skipped.
import {execFileSync, spawn, type ChildProcess} from 'child_process';
import fs from 'fs';
import net from 'net';
import os from 'os';
import path from 'path';
import {fakeNote} from '../__mocks__/sn-plugin-lib';
import {AskFlow, rememberSettings, type AskState} from '../src/askFlow';
import {connectFirst} from '../src/bridgeClient';
import type {LayoutParams} from '../src/protocol';
import {resetHostQueueForTests} from '../src/sdk';
import {resetTraceForTests} from '../src/trace';

// eslint-disable-next-line @typescript-eslint/no-var-requires
const WS = require('ws');

let mockEndpoints: {name: string; token: string; urls: string[]}[] = [];
jest.mock('../src/bridgeConfig', () => ({
  loadBridges: async () => ({endpoints: mockEndpoints, source: 'none'}),
}));

const REPO = path.join(__dirname, '../..');
const TOKEN = 'integration-token';

function goTool(): string | null {
  // (the name is left from the Go bridge: it is the Rust toolchain that builds it now)
  for (const go of [path.join(os.homedir(), '.cargo/bin/cargo'), 'cargo']) {
    try {
      execFileSync(go, ['--version'], {stdio: 'ignore'});
      return go;
    } catch {
      // try the next
    }
  }
  return null;
}

const free = () =>
  new Promise<number>(resolve => {
    const s = net.createServer();
    s.listen(0, '127.0.0.1', () => {
      const {port} = s.address() as net.AddressInfo;
      s.close(() => resolve(port));
    });
  });

const pause = (ms: number) => new Promise(r => setTimeout(r, ms));
const go = goTool();
const suite = go ? describe : describe.skip;

suite('the plugin and the bridge, for real', () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'spellink-it-'));
  let bridge: ChildProcess | null = null;
  let port = 0;
  let log = '';
  const url = () => `ws://127.0.0.1:${port}/v1`;
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

  beforeAll(async () => {
    (globalThis as unknown as {WebSocket: unknown}).WebSocket = WS;
    const exe = path.join(tmp, 'bridge');
    execFileSync(go!, ['build', '--locked'], {
      cwd: path.join(REPO, 'bridge'),
      timeout: 600000,
    });
    fs.copyFileSync(
      path.join(REPO, 'bridge/target/debug/spellink-bridge'),
      exe,
    );
    fs.chmodSync(exe, 0o755);
    const cfgDir = path.join(tmp, 'cfg', 'spellink-bridge');
    fs.mkdirSync(cfgDir, {recursive: true});
    execFileSync(exe, ['testfont', path.join(cfgDir, 'glyphs.bin')]);
    // a fake Claude Code: counts the earlier turns it was given, answers in two lines
    const claude = path.join(tmp, 'claude');
    fs.writeFileSync(
      claude,
      `#!/bin/sh
cat > "${tmp}/stdin.txt"
if grep -q slowly "${tmp}/stdin.txt"; then sleep 20; fi
n=$(grep -c '^\\[質問\\]' "${tmp}/stdin.txt")
printf '%s\\n' '{"type":"system","subtype":"init"}'
printf '%s\\n' '{"type":"stream_event","event":{"type":"message_start"}}'
printf '%s\\n' "{\\"type\\":\\"stream_event\\",\\"event\\":{\\"type\\":\\"content_block_delta\\",\\"delta\\":{\\"type\\":\\"text_delta\\",\\"text\\":\\"turns:$n\\\\nはい。\\"}}}"
printf '%s\\n' '{"type":"result","is_error":false}'
`,
      {mode: 0o755},
    );
    port = await free();
    fs.writeFileSync(
      path.join(cfgDir, 'config.json'),
      JSON.stringify({
        name: 'integration',
        port,
        token: TOKEN,
        claudeExecutable: claude,
        tiltDegrees: 0,
      }),
    );
    bridge = spawn(exe, ['serve'], {
      cwd: tmp,
      env: {
        ...process.env,
        HOME: tmp,
        XDG_CONFIG_HOME: path.join(tmp, 'cfg'),
        LC_ALL: 'C',
      },
    });
    bridge.stdout!.on('data', d => (log += d));
    bridge.stderr!.on('data', d => (log += d));
    for (let i = 0; i < 100; i++) {
      try {
        if ((await fetch(`http://127.0.0.1:${port}/health`)).ok) {
          return;
        }
      } catch {
        await pause(100);
      }
    }
    throw new Error(`the bridge did not start:\n${log}`);
  }, 240000);

  afterAll(() => {
    bridge?.kill();
    fs.rmSync(tmp, {recursive: true, force: true});
  });

  /** Each test starts with a bridge that remembers nothing (the same protocol message the settings use). */
  const forgetEverything = async () => {
    const conn = await connectFirst([
      {name: 'integration', token: TOKEN, urls: [url()]},
    ]);
    await new Promise<void>(resolve => {
      conn.onMessage = m => m.type === 'historyCleared' && resolve();
      conn.send({type: 'clearHistory'});
    });
    conn.close();
  };

  beforeEach(async () => {
    await forgetEverything();
    fakeNote.reset();
    resetTraceForTests();
    resetHostQueueForTests();
    mockEndpoints = [{name: 'integration', token: TOKEN, urls: [url()]}];
    rememberSettings({
      showProgress: false,
      lineTilt: true,
      thinkingAnimation: false,
      avoidOverwriting: false,
      busyNotice: true,
    });
  });

  const ask = async (
    question: string,
    afterMs?: number,
    then?: (f: AskFlow) => void,
  ) => {
    const seen: AskState[] = [];
    const flow = new AskFlow(s => seen.push(s));
    const done = flow.runWithQuestion(question, layout, ctx);
    if (afterMs !== undefined) {
      await pause(afterMs);
      then?.(flow);
    }
    await done;
    return seen[seen.length - 1];
  };
  const strokes = () =>
    fakeNote.strokes(0).filter(e => e.uuid.startsWith('new-')).length;

  it('connects the way a device does: encrypted, and the bridge says who it is and which AI it asks', async () => {
    const conn = await connectFirst(mockEndpoints);
    expect(conn.mode).toBe('noise');
    expect(conn.name).toBe('integration');
    expect(conn.provider).toBe('Claude Code');
    expect(conn.settings.busyNotice).toBe(true);
    conn.close();
  }, 20000);

  it('a wrong token cannot get in', async () => {
    await expect(
      connectFirst([{name: 'x', token: 'not-the-token', urls: [url()]}], 3000),
    ).rejects.toThrow();
  }, 20000);

  it('a question is answered with handwriting that ends up in the note', async () => {
    const last = await ask('こんにちは');
    expect(last.failed).toBeFalsy();
    expect(last.finished).toBe(true);
    expect(last.answer).toContain('turns:0'); // nothing earlier in this note
    expect(strokes()).toBeGreaterThan(0);
  }, 30000);

  it('a follow-up in the same note is given the conversation so far, by the real bridge', async () => {
    await ask('一つ目の質問');
    const second = await ask('二つ目の質問');
    expect(second.failed).toBeFalsy();
    expect(second.answer).toContain('turns:1');
  }, 40000);

  it('a cancelled answer ends cleanly, and the bridge goes on serving', async () => {
    const last = await ask('slowly please', 800, f => f.cancel());
    expect(last.finished).toBe(true);
    expect(last.failed).toBe(true);
    expect((await fetch(`http://127.0.0.1:${port}/health`)).ok).toBe(true);
    const again = await ask('まだ動く？');
    expect(again.failed).toBeFalsy();
  }, 60000);

  it('the bridge never saw a frame it could not read, and the plugin never hit an impossible cell', () => {
    expect(log).not.toMatch(/panic|unknown message type|bad_request/);
  });
});
