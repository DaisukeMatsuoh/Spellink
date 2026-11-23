/* eslint-disable no-bitwise -- byte-level encoding */
// UTF-8 conversion without TextEncoder/TextDecoder, which Hermes does not fully provide.

export function utf8Encode(s: string): Uint8Array {
  const out: number[] = [];
  for (const ch of s) {
    const c = ch.codePointAt(0)!;
    if (c < 0x80) {
      out.push(c);
    } else if (c < 0x800) {
      out.push(0xc0 | (c >> 6), 0x80 | (c & 63));
    } else if (c < 0x10000) {
      out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
    } else {
      out.push(
        0xf0 | (c >> 18),
        0x80 | ((c >> 12) & 63),
        0x80 | ((c >> 6) & 63),
        0x80 | (c & 63),
      );
    }
  }
  return Uint8Array.from(out);
}

export function utf8Decode(b: Uint8Array): string {
  const parts: string[] = [];
  let codes: number[] = [];
  for (let i = 0; i < b.length;) {
    const x = b[i];
    let c: number;
    if (x < 0x80) {
      c = x;
      i += 1;
    } else if (x < 0xe0) {
      c = ((x & 31) << 6) | (b[i + 1] & 63);
      i += 2;
    } else if (x < 0xf0) {
      c = ((x & 15) << 12) | ((b[i + 1] & 63) << 6) | (b[i + 2] & 63);
      i += 3;
    } else {
      c =
        ((x & 7) << 18) |
        ((b[i + 1] & 63) << 12) |
        ((b[i + 2] & 63) << 6) |
        (b[i + 3] & 63);
      i += 4;
    }
    codes.push(c);
    if (codes.length >= 8192) {
      parts.push(String.fromCodePoint(...codes));
      codes = [];
    }
  }
  parts.push(String.fromCodePoint(...codes));
  return parts.join('');
}
