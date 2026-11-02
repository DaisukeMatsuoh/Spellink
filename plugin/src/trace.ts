// A flight recorder for the SDK calls the plugin makes. When something goes wrong on the device
// (the note app drops out, strokes go missing) there is otherwise no trace of what happened
// just before. Events are kept in a ring buffer and, while a bridge connection is open, sent to
// the bridge's report log (reports.jsonl) as they happen.

export interface TraceEvent {
  /** Milliseconds since the plugin started. */
  t: number;
  /** What happened: an SDK call ("insertPageElements"), or a step ("ask.start"). */
  k: string;
  /** How long the call took. */
  ms?: number;
  /** The error, when it failed. */
  err?: string;
  /** Small extra facts (counts, page numbers). Never handwriting content. */
  d?: unknown;
}

const MAX_EVENTS = 400;
const started = Date.now();
const ring: TraceEvent[] = [];
let unsent: TraceEvent[] = [];
let sink: ((events: TraceEvent[]) => void) | null = null;

export function trace(
  k: string,
  extra: Omit<TraceEvent, 't' | 'k'> = {},
): void {
  const e: TraceEvent = {t: Date.now() - started, k, ...extra};
  ring.push(e);
  if (ring.length > MAX_EVENTS) {
    ring.shift();
  }
  unsent.push(e);
}

/** Where new events go while a bridge connection is open (null = keep them in memory only). */
export function setTraceSink(
  fn: ((events: TraceEvent[]) => void) | null,
): void {
  sink = fn;
}

/** Sends everything not yet sent, in chunks the bridge accepts. */
export function flushTrace(): void {
  if (!sink || unsent.length === 0) {
    return;
  }
  const batch = unsent;
  unsent = [];
  try {
    for (let i = 0; i < batch.length; i += 100) {
      sink(batch.slice(i, i + 100));
    }
  } catch {
    // The connection is gone; the events stay in the ring buffer.
  }
}

/** The last events, oldest first (for the diagnostics screen). */
export function dumpTrace(): TraceEvent[] {
  return [...ring];
}

export function resetTraceForTests(): void {
  ring.length = 0;
  unsent = [];
  sink = null;
}
