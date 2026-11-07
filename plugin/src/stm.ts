// A table-driven state machine (see docs/spec/README.md): look up the cell for (state, event),
// move to the cell's next state, and call the cell's action. It knows nothing about notes,
// sockets or ink; the tables are generated from the state transition tables in docs/spec.
//
// As in the C original this follows (`table[state][event]`, one function per cell), the state is
// changed in exactly one place — here — so what the machine believes and what the table says
// cannot drift apart.

export interface Move<S extends string, A extends string> {
  action: A;
  next: S;
}

/** What happens in one state on one event. */
export type Cell<S extends string, A extends string> =
  | {k: 'never'; note: string}
  | {k: 'ignore'; note?: string; action?: A}
  | ({k: 'do'} & Move<S, A>)
  | {k: 'guard'; more: Move<S, A>; spent: Move<S, A>};

export type Table<S extends string, E extends string, A extends string> = Record<
  S,
  Record<E, Cell<S, A>>
>;

/** One function for every action in the table; a missing one does not compile. */
export type Handlers<A extends string> = Record<A, (arg?: unknown) => void>;

export type Log = (k: string, d?: unknown) => void;

/** Whether the guarded cell (e.g. re-connection) still has attempts left. */
export type Guard = 'more' | 'spent';

export class Stm<S extends string, E extends string, A extends string> {
  state: S;

  constructor(
    private readonly table: Table<S, E, A>,
    private readonly handlers: Handlers<A>,
    private readonly log: Log,
    private readonly ends: readonly S[],
    start: S,
    private readonly name = 'stm',
  ) {
    this.state = start;
  }

  get terminal(): boolean {
    return this.ends.includes(this.state);
  }

  /**
   * Looks up the cell and acts on it. Never throws: an event or state the table does not know is
   * recorded and dropped (a bug to find in the record, not a reason to take the note app down).
   */
  dispatch(event: E, arg?: unknown, guard: Guard = 'more'): void {
    const cell = this.table[this.state]?.[event];
    if (!cell) {
      this.log(`${this.name}.unknown`, {state: this.state, event});
      return;
    }
    switch (cell.k) {
      case 'never':
        // The table says this cannot happen here. It did: record it loudly, change nothing.
        this.log(`${this.name}.never`, {state: this.state, event, note: cell.note});
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

  private move(event: E, to: Move<S, A>, arg: unknown): void {
    const from = this.state;
    // The state changes first, so an action that raises further events sees the new state.
    this.state = to.next;
    if (from !== to.next) {
      this.log(`${this.name}.state`, {from, event, action: to.action, to: to.next});
    }
    this.handlers[to.action](arg);
  }
}
