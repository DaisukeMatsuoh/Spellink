// Which bridges this device knows, and in what state each is (pairing.md, "起動時の初期状態").
// A bridge is known from the package (ready to pair, with its first secret) and/or from the store
// (paired, with its key). Bridges are judged one by one, by id.
import type {PairSetup} from './pairConfig';
import type {Reading, StoredBridge} from './pairStore';

export interface PlannedBridge {
  id: string;
  name: string;
  /** U5: paired, use `stored`. U1: has to pair, use `setup`. */
  state: 'U1' | 'U5';
  setup?: PairSetup;
  stored?: StoredBridge;
}

export interface Plan {
  bridges: PlannedBridge[];
  problems: string[];
}

export function planBridges(setup: PairSetup | null, reading: Reading): Plan {
  const bridges: PlannedBridge[] = [];
  const seen = new Set<string>();
  for (const stored of reading.bridges) {
    seen.add(stored.id);
    // The package of an older issue does not undo a newer pairing; a package of a newer issue
    // (a reissue) makes the stored key useless.
    if (setup && setup.id === stored.id && stored.gen < setup.gen) {
      bridges.push({id: stored.id, name: setup.name, state: 'U1', setup});
    } else {
      bridges.push({id: stored.id, name: stored.name, state: 'U5', stored});
    }
  }
  if (setup && !seen.has(setup.id)) {
    bridges.push({id: setup.id, name: setup.name, state: 'U1', setup});
  }
  return {bridges, problems: reading.problems};
}
