// Runs "Ask Claude" in the background (the lasso button has no UI of its own) so the answer can
// be watched appearing on the page. The progress panel opens only when the user turned it on
// (Settings → show progress) or when something went wrong.
import {PluginManager} from 'sn-plugin-lib';
import {AskFlow, type AskState} from './askFlow';
import {notify} from './notify';
import {show} from './session';

let state: AskState = {message: '', finished: true};
let flow: AskFlow | null = null;
let panelOpen = false;
let alerted = false;
const listeners = new Set<() => void>();

function openPanel(): void {
  if (panelOpen) {
    return;
  }
  panelOpen = true;
  show('ask');
  PluginManager.showPluginView();
}

function report(next: AskState): void {
  state = next;
  if (next.showProgress) {
    openPanel(); // the user asked to watch progress
  } else if (next.failed && !alerted) {
    // A failure is told in a small dialog; the full-screen panel is only the fallback.
    alerted = true;
    notify(next.message).then(shown => {
      if (!shown) {
        openPanel();
      }
    });
  }
  listeners.forEach(l => l());
}

export function startAsk(): void {
  if (flow && !state.finished) {
    return; // one question at a time
  }
  panelOpen = false;
  alerted = false;
  state = {message: '', finished: false};
  flow = new AskFlow(report);
  flow.run();
}

export function cancelAsk(): void {
  flow?.cancel();
}

/** The panel was closed by the user. */
export function panelClosed(): void {
  panelOpen = false;
}

export function getAskState(): AskState {
  return state;
}

export function subscribeAsk(l: () => void): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}
