import type {ClientMessage} from './protocol';
import {call} from './sdk';
import {PluginCommAPI} from 'sn-plugin-lib';

/**
 * Asks the bridge to forget the conversation of the note that is open now. The open note's path
 * is the key the bridge keeps its memory under (the same one `ask` sends). When no note is open
 * (the settings were opened from the plugin list, not from inside a note) there is nothing to
 * forget: 'noNote', and nothing is sent.
 */
export async function forgetThisNote(conn: {
  send: (m: ClientMessage) => void;
}): Promise<'sent' | 'noNote'> {
  let path: unknown;
  try {
    path = await call<string>('getCurrentFilePath', () =>
      PluginCommAPI.getCurrentFilePath(),
    );
  } catch {
    return 'noNote'; // not bound to a note app, or the note app does not answer: no key to send
  }
  if (typeof path !== 'string' || path === '') {
    return 'noNote';
  }
  conn.send({type: 'clearHistory', note: path});
  return 'sent';
}
