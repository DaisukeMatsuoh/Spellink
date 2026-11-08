import fs from 'fs';
import path from 'path';
import {MAX_EXTRA_PAGES, parseFrame, parseSettings} from '../src/frames';

// The frames the Go bridge really sends (recorded by its test, bridge/internal/server/
// contract_test.go). The plugin's validator must accept every one of them: if the bridge renames
// a field or changes a type, this fails here instead of on the device. After a deliberate
// change on the bridge: `cd bridge && go test ./internal/server -run Contract -update`.
const golden: Record<string, unknown>[] = JSON.parse(
  fs.readFileSync(
    path.join(
      __dirname,
      '../../bridge/internal/server/testdata/frames.golden.json',
    ),
    'utf8',
  ),
);

const label = (f: Record<string, unknown>) =>
  [f.type, f.stage, f.code, f.phase].filter(Boolean).join(':');

describe('the frames the bridge sends are accepted by the plugin', () => {
  it('has a recording of every kind of frame there is', () => {
    const kinds = golden.map(label);
    for (const k of [
      'welcome',
      'status:thinking',
      'status:writing',
      'status:alive:starting',
      'status:alive:writing',
      'line',
      'done',
      'error:cancelled',
      'error:unknown_ask',
      'settings',
      'historyCleared',
    ]) {
      expect(kinds).toContain(k);
    }
  });

  it.each(
    golden.filter(f => f.type !== 'welcome').map(f => [label(f), f] as const),
  )('accepts %s', (_name, frame) => {
    const parsed = parseFrame(frame, MAX_EXTRA_PAGES);
    expect(parsed.kind).toBe('frame');
  });

  it('welcome: the name, and settings that are all in range', () => {
    const w = golden.find(f => f.type === 'welcome') as {
      bridge: string;
      protocol: number;
      settings: Record<string, unknown>;
    };
    expect(typeof w.bridge).toBe('string');
    expect(w.protocol).toBe(1);
    const s = parseSettings(w.settings);
    // what the bridge sent survives the plugin's checks unchanged
    expect(s.showProgress).toBe(w.settings.showProgress);
    expect(s.lineTilt).toBe(w.settings.lineTilt);
    expect(s.busyNotice).toBe(w.settings.busyNotice);
    expect(s.avoidOverwriting).toBe(w.settings.avoidOverwriting);
  });

  it('a line keeps its strokes, and an answer-ending error and done carry a seq', () => {
    const line = golden.find(f => f.type === 'line')!;
    const parsed = parseFrame(line, MAX_EXTRA_PAGES);
    expect(
      parsed.kind === 'frame' &&
        parsed.frame.type === 'line' &&
        parsed.frame.strokes.length,
    ).toBeGreaterThan(0);
    for (const f of golden.filter(
      f => f.type === 'done' || (f.type === 'error' && f.code === 'cancelled'),
    )) {
      expect(typeof f.seq).toBe('number');
    }
  });

  it('keep-alives carry the phase and how long the AI has been idle', () => {
    for (const f of golden.filter(f => f.stage === 'alive')) {
      expect(['starting', 'working', 'writing']).toContain(f.phase);
      expect(typeof f.idleMs).toBe('number');
    }
  });
});
