import type {BridgeEndpoint} from './bridgeConfig';
import type {BridgeMessage, ClientMessage} from './protocol';

const CLIENT = 'cloute-plugin/0.1.0';

export class BridgeConnection {
  onMessage: (m: BridgeMessage) => void = () => {};
  onClose: (reason: string) => void = () => {};

  constructor(
    private readonly ws: WebSocket,
    readonly name: string,
    readonly url: string,
  ) {
    ws.onmessage = e => {
      try {
        this.onMessage(JSON.parse(String(e.data)) as BridgeMessage);
      } catch {
        // Ignore malformed frames.
      }
    };
    ws.onclose = e => this.onClose(e.reason || `closed (${e.code})`);
  }

  send(m: ClientMessage): void {
    this.ws.send(JSON.stringify(m));
  }

  close(): void {
    this.ws.close();
  }
}

export interface ConnectAttempt {
  url: string;
  ms: number;
  error?: string;
}

/**
 * Connects to every URL of every paired bridge at once and keeps the first one that accepts
 * our token. Whichever network the device is on, the reachable bridge wins without any setup.
 */
export function connectFirst(
  endpoints: BridgeEndpoint[],
  timeoutMs = 5000,
  attempts: ConnectAttempt[] = [],
): Promise<BridgeConnection> {
  return new Promise((resolve, reject) => {
    const started = Date.now();
    const sockets: WebSocket[] = [];
    let settled = false;
    let pending = 0;

    const finish = (winner: WebSocket | null) => {
      settled = true;
      clearTimeout(timer);
      for (const s of sockets) {
        if (s !== winner) {
          s.close();
        }
      }
    };
    const timer = setTimeout(() => {
      if (settled) {
        return;
      }
      finish(null);
      reject(new Error('no bridge reachable'));
    }, timeoutMs);
    const failed = (url: string, error: string) => {
      attempts.push({
        url,
        ms: Date.now() - started,
        error: settled
          ? 'not needed (another URL answered first)'
          : error || 'unreachable',
      });
      if (--pending === 0 && !settled) {
        finish(null);
        reject(new Error('no bridge reachable'));
      }
    };

    for (const ep of endpoints) {
      for (const url of ep.urls) {
        pending++;
        const ws = new WebSocket(url);
        sockets.push(ws);
        let done = false;
        ws.onopen = () =>
          ws.send(
            JSON.stringify({type: 'hello', token: ep.token, client: CLIENT}),
          );
        ws.onmessage = e => {
          if (done) {
            return;
          }
          done = true;
          let m: BridgeMessage | null = null;
          try {
            m = JSON.parse(String(e.data)) as BridgeMessage;
          } catch {
            // handled below
          }
          if (m?.type !== 'welcome') {
            failed(url, m?.type === 'error' ? m.message : 'unexpected reply');
            return;
          }
          attempts.push({url, ms: Date.now() - started});
          if (settled) {
            ws.close();
            return;
          }
          finish(ws);
          resolve(new BridgeConnection(ws, m.bridge, url));
        };
        ws.onerror = e => {
          if (done) {
            return;
          }
          done = true;
          failed(url, (e as {message?: string}).message ?? 'error');
        };
        ws.onclose = e => {
          if (done) {
            return;
          }
          done = true;
          failed(url, `closed (${e.code})`);
        };
      }
    }
    if (pending === 0) {
      finish(null);
      reject(new Error('no bridges configured'));
    }
  });
}
