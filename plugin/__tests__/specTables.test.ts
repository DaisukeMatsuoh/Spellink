import fs from 'fs';
import path from 'path';
import {
  INSERT_STATES,
  INSERT_EVENTS,
  INSERT_ACTIONS,
  INSERT_TABLE,
} from '../src/insertTable.generated';
import {
  WAVE_STATES,
  WAVE_EVENTS,
  WAVE_ACTIONS,
  WAVE_TABLE,
} from '../src/waveTable.generated';
import {
  WRITE_STATES,
  WRITE_EVENTS,
  WRITE_ACTIONS,
  WRITE_TABLE,
} from '../src/writeTable.generated';

// eslint-disable-next-line @typescript-eslint/no-var-requires
const {parse, DEFAULT_SECTIONS} = require('../scripts/stmTable.js');
const read = (f: string) =>
  fs.readFileSync(path.join(__dirname, '../../docs/spec', f), 'utf8');
const pageSections = (t: string) => ({
  states: `手順（${t}）`,
  events: '結果',
  actions: '処理',
  matrix: `手順表（${t}）`,
});

// The documents are the source of the tables; the generated code must say the same. If this
// fails: change the document, then run `npm run spec`.
const cases = [
  [
    'wave.md',
    DEFAULT_SECTIONS,
    WAVE_STATES,
    WAVE_EVENTS,
    WAVE_ACTIONS,
    WAVE_TABLE,
  ],
  [
    'page-turn.md (insert)',
    pageSections('挿入'),
    INSERT_STATES,
    INSERT_EVENTS,
    INSERT_ACTIONS,
    INSERT_TABLE,
  ],
  [
    'page-turn.md (write)',
    pageSections('書き込み'),
    WRITE_STATES,
    WRITE_EVENTS,
    WRITE_ACTIONS,
    WRITE_TABLE,
  ],
] as const;

describe.each(cases)(
  '%s: the table in the document is the table in the code',
  (name, sections, states, events, actions, table) => {
    const doc = parse(read(name.split(' ')[0]), sections);

    it('lists the same states, events and actions', () => {
      expect(doc.states).toEqual([...states]);
      expect(doc.events).toEqual([...events]);
      expect(doc.actions).toEqual([...actions]);
    });

    it('has the same cell in every place', () => {
      for (const s of states) {
        for (const e of events) {
          expect({at: `${s} × ${e}`, cell: (table as any)[s][e]}).toEqual({
            at: `${s} × ${e}`,
            cell: doc.table[s][e],
          });
        }
      }
    });

    it('never moves to a state that is not defined, and has no way out of an end state', () => {
      for (const s of states) {
        for (const e of events) {
          const c = (table as any)[s][e];
          const nexts =
            c.k === 'do'
              ? [c.next]
              : c.k === 'guard'
              ? [c.more.next, c.spent.next]
              : [];
          for (const n of nexts) {
            expect(states as readonly string[]).toContain(n);
          }
        }
      }
    });
  },
);
