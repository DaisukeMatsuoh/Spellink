// The frames of the pairing channel (docs/spec/pairing.md, C), checked as untrusted input.
import {isSecret} from './base64url';
import {isWssUrl} from './pairConfig';
import {MAX_STORED_URLS} from './pairStore';

/** The reasons a bridge may give for refusing (bridge: devices.Deny*). */
export const DENY_REASONS = [
  'code_in_use',
  'pair_busy',
  'grant_pending',
  'window_closed',
  'expired',
  'superseded',
  'unknown_device',
] as const;
export type DenyReason = (typeof DENY_REASONS)[number];

export type PairFrame =
  | {type: 'pairGrant'; key: string; endpoints: string[]}
  | {type: 'pairDenied'; reason: DenyReason | 'other'}
  | {type: 'error'; code: string};

export type ParsedPairFrame =
  {kind: 'frame'; frame: PairFrame} | {kind: 'bad'; reason: string};

/** A frame from the pairing channel, or why it cannot be trusted. */
export function parsePairFrame(raw: unknown): ParsedPairFrame {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) {
    return {kind: 'bad', reason: 'not an object'};
  }
  const r = raw as Record<string, unknown>;
  switch (r.type) {
    case 'pairGrant': {
      if (!isSecret(r.key)) {
        return {kind: 'bad', reason: 'the key is not a 32-byte secret'};
      }
      const eps = r.endpoints;
      if (
        !Array.isArray(eps) ||
        eps.length === 0 ||
        eps.length > MAX_STORED_URLS ||
        !eps.every(isWssUrl)
      ) {
        return {kind: 'bad', reason: 'the addresses cannot be used'};
      }
      return {
        kind: 'frame',
        frame: {type: 'pairGrant', key: r.key, endpoints: eps as string[]},
      };
    }
    case 'pairDenied': {
      const known = (DENY_REASONS as readonly unknown[]).includes(r.reason);
      return {
        kind: 'frame',
        frame: {
          type: 'pairDenied',
          reason: known ? (r.reason as DenyReason) : 'other',
        },
      };
    }
    case 'error':
      return {
        kind: 'frame',
        frame: {
          type: 'error',
          code: typeof r.code === 'string' ? r.code.slice(0, 64) : 'unknown',
        },
      };
  }
  return {kind: 'bad', reason: 'not a pairing frame'};
}
