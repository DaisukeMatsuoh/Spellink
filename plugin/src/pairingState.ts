// What this device knows about its bridges, from its package and from its store, and the real
// parts a pairing needs (connection, store, randomness, timers).
import {embeddedSetup} from './pairConfig';
import {connectPairing} from './pairConnect';
import {realFileOps} from './fileOps';
import {randomBytes} from './noise';
import type {PairDeps} from './pairing';
import {planBridges, type Plan} from './pairPlan';
import {readStore, saveToStore} from './pairStore';
import {t} from './i18n';
import {trace} from './trace';

/** The handwritten phrase beside the QR code: the width it may use and the size of its letters (pixels). */

export interface PairingState {
  plan: Plan;
  /** Why the package's own setup could not be used, if it carried one that was wrong. */
  setupProblem?: string;
}

/** Reads the package's setup and the store and says which bridges there are. */
export async function loadPairing(): Promise<PairingState> {
  const {setup, problem} = embeddedSetup();
  const reading = await readStore(realFileOps());
  const plan = planBridges(setup, reading);
  for (const p of plan.problems) {
    trace('pair.store.problem', {err: p});
  }
  if (problem) {
    trace('pair.setup.problem', {err: problem});
  }
  return {plan, setupProblem: problem};
}

/** `n` decimal digits, uniformly: bytes of 250 or more are skipped so every digit is equally likely. */
export function randomDigits(n: number): string {
  let out = '';
  while (out.length < n) {
    for (const b of randomBytes(n * 2)) {
      if (b < 250 && out.length < n) {
        out += String(b % 10);
      }
    }
  }
  return out;
}

/** The real outside world of a pairing; `onView` is where the screen listens. */
export function realPairDeps(onView: PairDeps['onView']): PairDeps {
  return {
    connect: (urls, b, ms) => connectPairing(urls, b, ms),
    save: stored => saveToStore(realFileOps(), stored),
    digits: randomDigits,
    after: (ms, f) => {
      const t = setTimeout(f, ms);
      return () => clearTimeout(t);
    },
    every: (ms, f) => {
      const t = setInterval(f, ms);
      return () => clearInterval(t);
    },
    now: () => Date.now(),
    onView,
    log: (k, d) => trace(k, {d}),
  };
}
