// The state machine of one question (docs/spec/ask-flow.md): the generic machine (stm.ts) on the
// generated table. `ask.state` / `ask.never` / `ask.unknown` are what it writes to the record.
import {Stm, type Guard, type Handlers as StmHandlers, type Log} from './stm';
import {
  ASK_TABLE,
  type AskAction,
  type AskEvent,
  type AskState,
} from './askTable.generated';

export type {AskAction, AskEvent, AskState} from './askTable.generated';

/** One function for every action in the table; a missing one does not compile. */
export type Handlers = StmHandlers<AskAction>;

export const TERMINAL: readonly AskState[] = ['OK', 'FAIL', 'CANC'];

export class AskMachine extends Stm<AskState, AskEvent, AskAction> {
  constructor(handlers: Handlers, log: Log, start: AskState = 'IDLE') {
    super(ASK_TABLE, handlers, log, TERMINAL, start, 'ask');
  }
}

export type {Guard};
