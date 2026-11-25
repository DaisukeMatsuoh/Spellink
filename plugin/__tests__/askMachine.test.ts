import fs from 'fs';
import path from 'path';
import {AskFlow} from '../src/askFlow';
import {AskMachine, TERMINAL, type Handlers} from '../src/askMachine';
import {
  ASK_ACTIONS,
  ASK_EVENTS,
  ASK_STATES,
  ASK_TABLE,
  type AskAction,
  type AskEvent,
  type AskState,
} from '../src/askTable.generated';

// eslint-disable-next-line @typescript-eslint/no-var-requires
const {parse} = require('../scripts/stmTable.js');
const SPEC = path.join(__dirname, '../../docs/spec/ask-flow.md');

describe('the table in the document is the table in the code', () => {
  const doc = parse(fs.readFileSync(SPEC, 'utf8'));

  it('lists the same states, events and actions', () => {
    expect(doc.states).toEqual([...ASK_STATES]);
    expect(doc.events).toEqual([...ASK_EVENTS]);
    expect(doc.actions).toEqual([...ASK_ACTIONS]);
  });

  it('has the same cell in every one of the 250 places (if this fails: run `npm run spec`)', () => {
    expect(ASK_STATES.length * ASK_EVENTS.length).toBe(250);
    for (const s of ASK_STATES) {
      for (const e of ASK_EVENTS) {
        expect({at: `${s} × ${e}`, cell: ASK_TABLE[s][e]}).toEqual({
          at: `${s} × ${e}`,
          cell: doc.table[s][e],
        });
      }
    }
  });

  it('is strict: a hole, an unknown name or an unreadable cell is an error naming the place', () => {
    const md = fs.readFileSync(SPEC, 'utf8');
    expect(() =>
      parse(
        md.replace(
          '| **Idle** | A1→CheckingPermission |',
          '| **Idle** | A99→CheckingPermission |',
        ),
      ),
    ).toThrow(/Idle × Pressed.*unknown action A99/);
    expect(() =>
      parse(
        md.replace(
          '| **Idle** | A1→CheckingPermission |',
          '| **Idle** | A1→NOWHERE |',
        ),
      ),
    ).toThrow(/unknown state NOWHERE/);
    expect(() =>
      parse(
        md.replace(
          '| **Idle** | A1→CheckingPermission |',
          '| **Idle** | maybe |',
        ),
      ),
    ).toThrow(/Idle × Pressed.*notation/);
    expect(() => parse(md.replace(/^\| \*\*Completed\*\* .*$/m, ''))).toThrow(
      /Completed × .*no cell/,
    );
  });
});

describe('every cell does what the table says (all 250)', () => {
  const calls: {action: AskAction; arg: unknown}[] = [];
  const logs: string[] = [];
  const handlers = Object.fromEntries(
    ASK_ACTIONS.map(a => [a, (arg?: unknown) => calls.push({action: a, arg})]),
  ) as unknown as Handlers;
  const machine = (state: AskState) =>
    new AskMachine(handlers, k => logs.push(k), state);
  beforeEach(() => {
    calls.length = 0;
    logs.length = 0;
  });

  const cases = ASK_STATES.flatMap(s => ASK_EVENTS.map(e => [s, e] as const));
  it.each(cases)('%s × %s', (s, e) => {
    const cell = ASK_TABLE[s][e];
    const m = machine(s);
    m.dispatch(e, 'arg');
    switch (cell.k) {
      case 'never':
        expect(calls).toEqual([]); // nothing is done…
        expect(m.state).toBe(s); // …nothing moves…
        expect(logs).toContain('ask.never'); // …and it is recorded loudly
        break;
      case 'ignore':
        expect(m.state).toBe(s);
        expect(calls.map(c => c.action)).toEqual(
          cell.action ? [cell.action] : [],
        );
        break;
      case 'do':
        expect(m.state).toBe(cell.next);
        expect(calls.map(c => c.action)).toEqual([cell.action]);
        expect(calls[0].arg).toBe('arg');
        break;
      case 'guard': {
        expect(m.state).toBe(cell.more.next);
        expect(calls.map(c => c.action)).toEqual([cell.more.action]);
        calls.length = 0;
        const spent = machine(s);
        spent.dispatch(e, 'arg', 'spent');
        expect(spent.state).toBe(cell.spent.next);
        expect(calls.map(c => c.action)).toEqual([cell.spent.action]);
      }
    }
  });
});

describe('the shape of the table', () => {
  const nexts = (s: AskState): AskState[] =>
    ASK_EVENTS.flatMap(e => {
      const c = ASK_TABLE[s][e];
      return c.k === 'do'
        ? [c.next]
        : c.k === 'guard'
          ? [c.more.next, c.spent.next]
          : [];
    });

  it('has no way out of an end state', () => {
    for (const s of TERMINAL) {
      expect(nexts(s).filter(n => n !== s)).toEqual([]);
    }
  });

  it('reaches every state from Idle', () => {
    const seen = new Set<AskState>(['Idle']);
    const todo: AskState[] = ['Idle'];
    while (todo.length) {
      for (const n of nexts(todo.pop()!)) {
        if (!seen.has(n)) {
          seen.add(n);
          todo.push(n);
        }
      }
    }
    expect([...seen].sort()).toEqual([...ASK_STATES].sort());
  });

  it('lets every state that is not an end state finish: an end state can be reached from it', () => {
    for (const s of ASK_STATES) {
      const seen = new Set<AskState>([s]);
      const todo: AskState[] = [s];
      while (todo.length) {
        for (const n of nexts(todo.pop()!)) {
          if (!seen.has(n)) {
            seen.add(n);
            todo.push(n);
          }
        }
      }
      expect([...seen].some(x => TERMINAL.includes(x))).toBe(true);
    }
  });

  it('lets the user cancel, and the user press again, in every state in which a question runs', () => {
    for (const s of [
      'CheckingPermission',
      'Recognizing',
      'Preparing',
      'Streaming',
      'Reconnecting',
      'Draining',
    ] as AskState[]) {
      const cancel = ASK_TABLE[s].CancelRequested;
      expect(cancel.k === 'do' && cancel.next === 'Cancelled').toBe(true);
      const press = ASK_TABLE[s].Pressed;
      expect(
        press.k === 'do' && press.action === 'A2' && press.next === s,
      ).toBe(true);
    }
  });

  it('answers a failed or refused note app in every state that writes or reads the note', () => {
    for (const s of [
      'CheckingPermission',
      'Recognizing',
      'Preparing',
      'Streaming',
      'Reconnecting',
      'Draining',
    ] as AskState[]) {
      for (const e of ['Refused', 'NoAnswer'] as AskEvent[]) {
        const c = ASK_TABLE[s][e];
        expect(c.k === 'do' && c.next === 'Failed').toBe(true);
      }
    }
  });
});

describe('the code has an action for every action in the table', () => {
  it('(a missing one does not even compile; this checks the running object)', () => {
    const flow = new AskFlow(() => {}) as unknown as {
      machine: {handlers: Handlers};
    };
    expect(Object.keys(flow.machine.handlers).sort()).toEqual(
      [...ASK_ACTIONS].sort(),
    );
  });
});
