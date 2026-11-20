// The pairing channel: a Noise session opened with the device's first secret B, to every address of
// the package at once (the one that answers first wins). Nothing but pairing travels on it.
import {NoiseInitiator, pskFromToken, type SecureChannel} from './noise';
import {parsePairFrame, type PairFrame} from './pairFrames';
import {utf8Decode, utf8Encode} from './utf8';

export interface PairChannel {
  send(frame: Record<string, unknown>): void;
  /** Frames that passed `parsePairFrame`; anything else closes the channel. */
  onFrame: (f: PairFrame) => void;
  /** The channel ended (after it was established). */
  onClose: (reason: string) => void;
  close(): void;
}

type WebSocketCtor = new (url: string) => WebSocket;

/**
 * Opens the channel. Rejects when no address completes the handshake within `timeoutMs`, with
 * the reasons of the failures (the first secret may be expired or already used).
 */
export function connectPairing(
  urls: string[],
  b: string,
  timeoutMs: number,
  Socket: WebSocketCtor = WebSocket,
): Promise<PairChannel> {
  return new Promise((resolve, reject) => {
    const sockets: WebSocket[] = [];
    const reasons: string[] = [];
    let settled = false;
    let pending = urls.length;
    const finish = (winner: WebSocket | null) => {
      settled = true;
      clearTimeout(timer);
      sockets.forEach(s => s !== winner && s.close());
    };
    const timer = setTimeout(() => {
      if (!settled) {
        finish(null);
        reject(
          new Error(`no address answered in time (${reasons.join('; ')})`),
        );
      }
    }, timeoutMs);
    const lost = (why: string) => {
      reasons.push(why);
      if (--pending === 0 && !settled) {
        finish(null);
        reject(
          new Error(`no address could be reached (${reasons.join('; ')})`),
        );
      }
    };
    if (urls.length === 0) {
      finish(null);
      reject(new Error('no address to connect to'));
      return;
    }
    for (const url of urls) {
      let initiator: NoiseInitiator;
      let msg1: Uint8Array;
      try {
        initiator = new NoiseInitiator(pskFromToken(b));
        msg1 = initiator.writeMessage1();
      } catch (e) {
        lost(`encryption unavailable: ${(e as Error).message}`);
        continue;
      }
      const ws = new Socket(url);
      ws.binaryType = 'arraybuffer';
      sockets.push(ws);
      let done = false;
      const fail = (why: string) => {
        if (!done) {
          done = true;
          lost(why);
        }
      };
      ws.onopen = () => ws.send(msg1);
      ws.onmessage = e => {
        if (done) {
          return;
        }
        done = true;
        let secure: SecureChannel;
        try {
          if (!(e.data instanceof ArrayBuffer)) {
            throw new Error('the bridge did not answer the handshake');
          }
          secure = initiator.readMessage2(new Uint8Array(e.data));
        } catch (err) {
          done = false;
          return fail((err as Error).message);
        }
        if (settled) {
          ws.close();
          return;
        }
        finish(ws);
        resolve(adopt(ws, secure));
      };
      ws.onerror = () => fail('connection error');
      ws.onclose = ev => fail(ev.reason || `closed (${ev.code})`);
    }
  });
}

function adopt(ws: WebSocket, secure: SecureChannel): PairChannel {
  const ch: PairChannel = {
    onFrame: () => {},
    onClose: () => {},
    send(frame) {
      for (const f of secure.seal(utf8Encode(JSON.stringify(frame)))) {
        ws.send(f);
      }
    },
    close: () => ws.close(),
  };
  ws.onmessage = e => {
    try {
      if (!(e.data instanceof ArrayBuffer)) {
        throw new Error('expected a binary frame');
      }
      const message = secure.open(new Uint8Array(e.data));
      if (!message) {
        return; // a long message is still arriving
      }
      const parsed = parsePairFrame(JSON.parse(utf8Decode(message)));
      if (parsed.kind === 'bad') {
        throw new Error(parsed.reason);
      }
      ch.onFrame(parsed.frame);
    } catch {
      // a frame that does not decrypt or does not check out: this channel can no longer be trusted
      ws.close();
    }
  };
  ws.onclose = ev => ch.onClose(ev.reason || `closed (${ev.code})`);
  ws.onerror = () => {};
  return ch;
}
