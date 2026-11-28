// Runs a procedure table (docs/spec/page-turn.md): rows are the steps still to do, columns are the
// outcomes of a step. The table says what follows each outcome — the next step, an abort that
// left the note untouched, an abort that may have left it changed, or a recorded fallback — so
// the steps themselves only do the work and report what happened.
import {Stm, type Handlers, type Table} from './stm';
import {trace} from './trace';

/** What a step reports (see the "結果" table of the spec). */
export type Outcome =
  'Ok' | 'Failed' | 'Refused' | 'NoAnswer' | 'NoteChanged' | 'Unexpected';

export async function runProcedure<S extends string, A extends string>(opts: {
  name: string;
  table: Table<S, Outcome, A>;
  start: S;
  ends: readonly S[];
  steps: Partial<Record<S, () => Promise<Outcome>>>;
  handlers: Handlers<A>;
}): Promise<S> {
  const m = new Stm(
    opts.table,
    opts.handlers,
    (k, d) => trace(k, {d}),
    opts.ends,
    opts.start,
    opts.name,
  );
  while (!m.terminal) {
    const at = m.state;
    const step = opts.steps[at];
    if (!step) {
      throw new Error(`${opts.name}: no step for ${at}`);
    }
    const outcome = await step();
    m.dispatch(outcome, {at, outcome});
    if (m.state === at) {
      // The table has no move for this (a "never" or "ignore" cell): do not loop on it.
      throw new Error(`${opts.name}: stuck at ${at} after ${outcome}`);
    }
  }
  return m.state;
}
