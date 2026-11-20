// The pairing flow of the device (docs/spec/pairing.md, B): the generic machine (stm.ts) on the
// generated table. `pair.state` / `pair.never` / `pair.unknown` are what it writes to the record.
import {
  PAIR_TABLE,
  type PairAction,
  type PairEvent,
  type PairState,
} from './pairTable.generated';
import {Stm, type Handlers as StmHandlers, type Log} from './stm';

export type {PairAction, PairEvent, PairState} from './pairTable.generated';

/** One function for every action in the table; a missing one does not compile. */
export type PairHandlers = StmHandlers<PairAction>;

/** No state is final: a pairing can always be tried again. */
const NO_END: readonly PairState[] = [];

export class PairMachine extends Stm<PairState, PairEvent, PairAction> {
  constructor(handlers: PairHandlers, log: Log, start: PairState) {
    super(PAIR_TABLE, handlers, log, NO_END, start, 'pair');
  }
}
