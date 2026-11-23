// The version of this plugin, and what to say when it and the bridge it meets are not the same.
// `__tests__/version.test.ts` keeps PLUGIN_VERSION equal to package.json and PluginConfig.json.
export const PLUGIN_VERSION = '0.1.1';

/** Digits of "MAJOR.MINOR.PATCH" (extra text such as "-rc1" is ignored), or null if not that shape. */
export function parseVersion(v: string): [number, number, number] | null {
  const m =
    /^(\d{1,6})\.(\d{1,6})\.(\d{1,6})(?:[-+][0-9A-Za-z.-]{0,32})?$/.exec(v);
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

/**
 * Which side is behind, judged by MAJOR.MINOR only (a patch never changes how they talk):
 * 'plugin' = the plugin is older, 'bridge' = the bridge is older, null = same or cannot tell.
 */
export function olderSide(
  plugin: string,
  bridge: string,
): 'plugin' | 'bridge' | null {
  const p = parseVersion(plugin);
  const b = parseVersion(bridge);
  if (!p || !b) {
    return null;
  }
  for (let i = 0; i < 2; i++) {
    if (p[i] !== b[i]) {
      return p[i] < b[i] ? 'plugin' : 'bridge';
    }
  }
  return null;
}
