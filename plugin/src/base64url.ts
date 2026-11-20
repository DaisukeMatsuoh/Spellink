// Unpadded base64url (RFC 4648 §5), strict: decoding refuses anything that is not exactly what
// encoding would have written, so one secret has one spelling.
const ALPHABET =
  'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
const INDEX: Record<string, number> = {};
for (let i = 0; i < ALPHABET.length; i++) {
  INDEX[ALPHABET[i]] = i;
}

export function encodeBase64Url(bytes: Uint8Array): string {
  let out = '';
  for (let i = 0; i < bytes.length; i += 3) {
    const n =
      (bytes[i] << 16) | ((bytes[i + 1] ?? 0) << 8) | (bytes[i + 2] ?? 0);
    out += ALPHABET[(n >> 18) & 63] + ALPHABET[(n >> 12) & 63];
    if (i + 1 < bytes.length) {
      out += ALPHABET[(n >> 6) & 63];
    }
    if (i + 2 < bytes.length) {
      out += ALPHABET[n & 63];
    }
  }
  return out;
}

/** The bytes of `text`, or null if it is not canonical unpadded base64url. */
export function decodeBase64Url(text: string): Uint8Array | null {
  if (text.length % 4 === 1) {
    return null;
  }
  const out = new Uint8Array(Math.floor((text.length * 3) / 4));
  let bits = 0;
  let acc = 0;
  let at = 0;
  for (const ch of text) {
    const v = INDEX[ch];
    if (v === undefined) {
      return null;
    }
    acc = (acc << 6) | v;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out[at++] = (acc >> bits) & 255;
      acc &= (1 << bits) - 1;
    }
  }
  // leftover bits must be zero, or two spellings would mean the same bytes
  return acc === 0 && encodeBase64Url(out) === text ? out : null;
}

/** A secret as the bridge makes it: 32 random bytes in base64url (43 characters). */
export function isSecret(text: unknown): text is string {
  return (
    typeof text === 'string' &&
    text.length === 43 &&
    decodeBase64Url(text)?.length === 32
  );
}
