jest.mock('../src/bridges.local.json', () => ({bridges: []}), {virtual: true});
import {parse} from '../src/bridgeConfig';

const ok = {name: 'home', token: 'abc', urls: ['wss://bridge.example/v1']};

describe('a pairing file is checked entry by entry', () => {
  it('keeps good entries', () => {
    expect(parse({bridges: [ok]})).toEqual([ok]);
  });

  it.each([
    ['not an object', null],
    ['no bridges key', {}],
    ['bridges that is not a list', {bridges: 'x'}],
    ['an empty token', {bridges: [{...ok, token: ''}]}],
    ['a token that is not a string', {bridges: [{...ok, token: 5}]}],
    ['a nameless bridge', {bridges: [{...ok, name: ''}]}],
    ['no URLs', {bridges: [{...ok, urls: []}]}],
    ['a URL that is not wss or ws', {bridges: [{...ok, urls: ['https://x']}]}],
    ['a URL that is not a string', {bridges: [{...ok, urls: [7]}]}],
    ['a huge token', {bridges: [{...ok, token: 'a'.repeat(300)}]}],
    ['too many URLs', {bridges: [{...ok, urls: Array(9).fill('wss://x')}]}],
  ])('leaves out %s', (_name, json) => {
    expect(parse(json)).toEqual([]);
  });

  it('keeps the good entries next to a bad one, and caps the number of bridges', () => {
    expect(parse({bridges: [{...ok, token: ''}, ok]})).toEqual([ok]);
    expect(parse({bridges: Array(20).fill(ok)})).toHaveLength(8);
  });
});
