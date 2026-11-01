import {utf8Decode, utf8Encode} from '../src/utf8';

describe('utf8', () => {
  it.each(['', 'abc', 'あいう 漢字', '🙂 emoji', 'ｱｲｳ ｶﾀｶﾅ'])(
    'round-trips %p',
    s => {
      expect(utf8Decode(utf8Encode(s))).toBe(s);
    },
  );
  it('encodes like the platform', () => {
    expect(Array.from(utf8Encode('あ🙂'))).toEqual(
      Array.from(Buffer.from('あ🙂', 'utf8')),
    );
  });
  it('handles long text', () => {
    const s = 'ストローク'.repeat(5000);
    expect(utf8Decode(utf8Encode(s))).toBe(s);
  });
});
