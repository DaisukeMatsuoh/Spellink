// The real FileOps (pairStore.ts): the SDK's file module. Every call has a time limit, and what
// comes back is checked, not trusted.
import {TurboModuleRegistry} from 'react-native';
import type {FileOps} from './pairStore';

const TIMEOUT_MS = 5000;

interface FileUtils {
  exists(path: string): Promise<boolean>;
  listFiles(
    dir: string,
  ): Promise<Array<{type?: unknown; path?: unknown}> | null | undefined>;
  makeDir(path: string): Promise<boolean>;
}

const limited = <T>(p: Promise<T>, what: string): Promise<T> =>
  new Promise<T>((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error(`${what}: no answer in ${TIMEOUT_MS}ms`)),
      TIMEOUT_MS,
    );
    p.then(
      v => {
        clearTimeout(timer);
        resolve(v);
      },
      e => {
        clearTimeout(timer);
        reject(e);
      },
    );
  });

/** `module` is the native file module; a test passes its own. */
export function realFileOps(
  module: unknown = TurboModuleRegistry.get('RTNFileUtils'),
): FileOps {
  const fu = module as FileUtils | null;
  const need = (): FileUtils => {
    if (!fu) {
      throw new Error('the file module is not available');
    }
    return fu;
  };
  return {
    async list(dir) {
      const m = need();
      if ((await limited(m.exists(dir), 'exists')) !== true) {
        return null;
      }
      const entries = await limited(m.listFiles(dir), 'listFiles');
      if (!Array.isArray(entries)) {
        return [];
      }
      const names: string[] = [];
      for (const e of entries) {
        if (typeof e?.path === 'string') {
          const name = e.path.split('/').pop();
          if (name) {
            names.push(name);
          }
        }
      }
      return names;
    },
    async makeDir(path) {
      return (await limited(need().makeDir(path), 'makeDir')) === true;
    },
  };
}
