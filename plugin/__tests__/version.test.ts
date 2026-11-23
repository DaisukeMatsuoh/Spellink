import {olderSide, parseVersion, PLUGIN_VERSION} from '../src/version';

describe('the plugin version', () => {
  it('is the one in package.json and in PluginConfig.json', () => {
    expect(require('../package.json').version).toBe(PLUGIN_VERSION);
    expect(require('../PluginConfig.json').versionName).toBe(PLUGIN_VERSION);
  });
  it('is a version we can read', () => {
    expect(parseVersion(PLUGIN_VERSION)).not.toBeNull();
  });
});

describe('parseVersion', () => {
  it.each([
    ['0.1.0', [0, 1, 0]],
    ['12.0.3-rc1', [12, 0, 3]],
    ['1.2.3+abc', [1, 2, 3]],
  ])('reads %s', (v, want) => expect(parseVersion(v)).toEqual(want));
  it.each([
    '',
    '1',
    '1.2',
    'a.b.c',
    '1.2.3.4',
    '1.2.x',
    '-1.0.0',
    '1234567.0.0',
  ])('refuses %j', v => expect(parseVersion(v)).toBeNull());
});

describe('olderSide', () => {
  it.each([
    ['0.1.0', '0.1.0', null],
    ['0.1.0', '0.1.7', null], // a patch is not a difference
    ['0.1.9', '0.2.0', 'plugin'],
    ['0.2.0', '0.1.9', 'bridge'],
    ['1.0.0', '0.9.0', 'bridge'],
    ['0.9.0', '1.0.0', 'plugin'],
    ['0.1.0', '', null], // an old bridge says nothing: we cannot tell
    ['0.1.0', 'dev', null],
  ])('plugin %s, bridge %s -> %s', (p, b, want) =>
    expect(olderSide(p, b)).toBe(want),
  );
});
