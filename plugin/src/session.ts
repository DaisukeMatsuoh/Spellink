// Which screen the plugin view should show. Set from the button listener in index.js,
// read by App; each press bumps `seq` so the same screen restarts on a new press.
export type Screen = 'ask' | 'diagnostics';

let current: {screen: Screen; seq: number} = {screen: 'diagnostics', seq: 0};
const listeners = new Set<() => void>();

export function show(screen: Screen): void {
  current = {screen, seq: current.seq + 1};
  listeners.forEach(l => l());
}

export function getScreen(): {screen: Screen; seq: number} {
  return current;
}

export function subscribe(l: () => void): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}
