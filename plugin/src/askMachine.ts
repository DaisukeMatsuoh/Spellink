// The state machine of one question (docs/spec/ask-flow.md). The table is generated from that
// document; this file only runs it: look up the cell for (state, event), move to the cell's next
// state, and call the cell's action. It knows nothing about notes, sockets or ink.
//
// Like the C original this follows (one function per cell, `table[state][event]`), the state is
// changed in exactly one place — here — so what the machine believes and what the table says
// cannot drift apart.
import {
  ASK_TABLE,
  type AskAction,
  type AskEvent,
  type AskState,
  type Move,
} from './askTable.generated';

export type {AskAction, AskEvent, AskState} from './askTable.generated';

/** One function for every action in the table; a missing one does not compile. */
export type Handlers = Record<AskAction, (arg?: unknown) => void>;

export type Log = (k: string, d?: unknown) => void;

export const TERMINAL: readonly AskState[] = ['OK', 'FAIL', 'CANC'];

/** Whether the guarded cell (re-connection) still has attempts left. */
export type Guard = 'more' | 'spent';

export class AskMachine {
  state: AskState;

  constructor(
    private readonly handlers: Handlers,
    private readonly log: Log,
    start: AskState = 'IDLE',
  ) {
    this.state = start;
  }

  get terminal(): boolean {
    return TERMINAL.includes(this.state);
  }

  /**
   * Looks up the cell and acts on it. Never throws: an event or state the table does not know is
   * recorded and dropped (a bug to find in the record, not a reason to take the note app down).
   */
  dispatch(event: AskEvent, arg?: unknown, guard: Guard = 'more'): void {
    const cell = ASK_TABLE[this.state]?.[event];
    if (!cell) {
      this.log('ask.unknown', {state: this.state, event});
      return;
    }
    switch (cell.k) {
      case 'never':
        // The table says this cannot happen here. It did: record it loudly, change nothing.
        this.log('ask.never', {state: this.state, event, note: cell.note});
        return;
      case 'ignore':
        if (cell.action) {
          this.handlers[cell.action]({event, state: this.state, arg});
        }
        return;
      case 'do':
        this.move(event, cell, arg);
        return;
      case 'guard':
        this.move(event, cell[guard], arg);
    }
  }

  private move(event: AskEvent, to: Move, arg: unknown): void {
    const from = this.state;
    // The state changes first, so an action that raises further events sees the new state.
    this.state = to.next;
    if (from !== to.next) {
      this.log('ask.state', {from, event, action: to.action, to: to.next});
    }
    this.handlers[to.action](arg);
  }
}
