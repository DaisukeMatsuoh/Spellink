import type {BridgeEndpoint} from './bridgeConfig';
import {MAX_EXTRA_PAGES, parseFrame, parseSettings} from './frames';
import {NoiseInitiator, pskFromToken, type SecureChannel} from './noise';
import type {BridgeMessage, ClientMessage, Settings} from './protocol';
import {trace} from './trace';
import {utf8Decode, utf8Encode} from './utf8';

const CLIENT = 'spellink-plugin/0.1.0';

/** 'noise' = end-to-end encrypted (ADR 0007); 'plaintext' = legacy LAN-only fallback. */
export type ConnectionMode = 'noise' | 'plaintext';

/** Decodes one frame; returns null while an encrypted message is still incomplete. */
function decode(
  data: unknown,
  channel: SecureChannel | null,
): {value: unknown} | null {
  if (!channel) {
    return {value: JSON.parse(String(data))};
  }
  if (!(data instanceof ArrayBuffer)) {
    throw new Error('expected a binary frame');
  }
  const message = channel.open(new Uint8Array(data));
  return message ? {value: JSON.parse(utf8Decode(message))} : null;
}

export class BridgeConnection {
  /** Only frames that passed `parseFrame` arrive here. */
  onMessage: (m: BridgeMessage) => void = () => {};
  onClose: (reason: string) => void = () => {};
  /** A frame that is not valid (wrong shape, out of range): by default the connection is dropped. */
  onBad: (reason: string) => void = () => this.ws.close();

  constructor(
    private readonly ws: WebSocket,
    private readonly channel: SecureChannel | null,
    readonly name: string,
    readonly url: string,
    readonly mode: ConnectionMode,
    readonly settings: Settings,
    /** The AI the bridge asks, as the bridge names it ("" if it did not say). */
    readonly provider = '',
  ) {
    ws.onmessage = e => {
      let m: {value: unknown} | null;
      try {
        m = decode(e.data, channel);
      } catch {
        // A frame that fails to decrypt or to parse means the session can no longer be trusted.
        ws.close();
        return;
      }
      if (!m) {
        return;
      }
      const parsed = parseFrame(m.value, MAX_EXTRA_PAGES);
      if (parsed.kind === 'frame') {
        this.onMessage(parsed.frame);
      } else if (parsed.kind === 'ignore') {
        trace('frame.ignored', {d: {type: parsed.type}}); // e.g. a newer bridge's message
      } else {
        trace('frame.bad', {err: parsed.reason});
        this.onBad(parsed.reason);
      }
    };
    ws.onclose = e => this.onClose(e.reason || `closed (${e.code})`);
  }

  send(m: ClientMessage): void {
    const json = JSON.stringify(m);
    if (!this.channel) {
      this.ws.send(json);
      return;
    }
    for (const frame of this.channel.seal(utf8Encode(json))) {
      this.ws.send(frame);
    }
  }

  close(): void {
    this.ws.close();
  }
}

export interface ConnectAttempt {
  url: string;
  ms: number;
  mode?: ConnectionMode;
  error?: string;
}

const isRelay = (url: string) => url.includes('/v1/relay/');

const isRecord = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v);

/** What an unexpected first reply says: the bridge's own error message, or a generic one. */
function replyProblem(reply: unknown): string {
  if (
    isRecord(reply) &&
    reply.type === 'error' &&
    typeof reply.message === 'string'
  ) {
    return reply.message.slice(0, 200);
  }
  return 'unexpected reply';
}

/**
 * Connects to every URL of every paired bridge at once and keeps the first one that completes
 * the handshake. Whichever network the device is on, the reachable bridge wins without setup.
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
        // Prefer Noise. Plaintext is only a fallback for when the crypto code cannot run on
        // this device, and never through the relay (the bridge refuses it there anyway).
        let initiator: NoiseInitiator | null = null;
        let msg1: Uint8Array | null = null;
        try {
          initiator = new NoiseInitiator(pskFromToken(ep.token));
          msg1 = initiator.writeMessage1();
        } catch (e) {
          if (isRelay(url)) {
            failed(url, `encryption unavailable: ${(e as Error).message}`);
            continue;
          }
          initiator = null;
        }
        const mode: ConnectionMode = initiator ? 'noise' : 'plaintext';
        const ws = new WebSocket(url);
        ws.binaryType = 'arraybuffer';
        sockets.push(ws);
        let channel: SecureChannel | null = null;
        let done = false;
        const fail = (error: string) => {
          if (!done) {
            done = true;
            failed(url, error);
          }
        };

        ws.onopen = () =>
          ws.send(
            msg1 ??
              JSON.stringify({type: 'hello', token: ep.token, client: CLIENT}),
          );
        ws.onmessage = e => {
          if (done) {
            return;
          }
          let decoded: {value: unknown} | null;
          try {
            if (initiator && !channel) {
              if (!(e.data instanceof ArrayBuffer)) {
                // The bridge answered in plaintext: it rejected us (e.g. an old bridge).
                return fail(replyProblem(JSON.parse(String(e.data))));
              }
              channel = initiator.readMessage2(new Uint8Array(e.data));
              return; // the encrypted welcome follows
            }
            decoded = decode(e.data, channel);
          } catch (err) {
            return fail((err as Error).message);
          }
          if (!decoded) {
            return;
          }
          const w = decoded.value;
          if (!isRecord(w) || w.type !== 'welcome') {
            return fail(replyProblem(w));
          }
          done = true;
          attempts.push({url, ms: Date.now() - started, mode});
          if (settled) {
            ws.close();
            return;
          }
          finish(ws);
          const name =
            typeof w.bridge === 'string' && w.bridge.length <= 128
              ? w.bridge
              : 'bridge';
          resolve(
            new BridgeConnection(
              ws,
              channel,
              name,
              url,
              mode,
              parseSettings(w.settings),
              typeof w.provider === 'string' && w.provider.length <= 128
                ? w.provider
                : '',
            ),
          );
        };
        ws.onerror = e => fail((e as {message?: string}).message ?? 'error');
        ws.onclose = e => fail(e.reason || `closed (${e.code})`);
      }
    }
    if (pending === 0) {
      finish(null);
      reject(new Error('no bridges configured'));
    }
  });
}
