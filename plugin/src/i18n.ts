import {PluginManager} from 'sn-plugin-lib';

import {LOCALES, STRINGS} from './strings.generated';

export type Locale = keyof typeof STRINGS;
export type MessageKey = keyof (typeof STRINGS)['en'];

/**
 * Firmware language codes are like "en", "ja", "zh_CN". An exact match wins, then the language part
 * ("zh" for "zh_CN"); anything else falls back to English.
 */
export function toLocale(code: string | undefined): Locale {
  const c = (code ?? '').replace('-', '_');
  const exact = LOCALES.find(l => l.toLowerCase() === c.toLowerCase());
  if (exact) {
    return exact;
  }
  const lang = c.split('_')[0].toLowerCase();
  return LOCALES.find(l => l.toLowerCase() === lang) ?? 'en';
}

function systemLocale(): Locale {
  try {
    return toLocale(new Intl.Collator().resolvedOptions().locale);
  } catch {
    return 'en';
  }
}

let locale: Locale = systemLocale();
const listeners = new Set<() => void>();

export function startLocaleTracking(): void {
  PluginManager.registerLangListener({
    onMsg(code: string) {
      locale = toLocale(code);
      listeners.forEach(l => l());
    },
  });
}

export function onLocaleChange(l: () => void): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}

export function t(
  key: MessageKey,
  vars: Record<string, string | number> = {},
): string {
  return STRINGS[locale][key].replace(/\{(\w+)\}/g, (_, k) =>
    String(vars[k] ?? ''),
  );
}

/**
 * Button labels are a JSON map of firmware language code → label; the firmware picks one. Every
 * language of the TSV is offered, and the Chinese codes the firmware may ask for fall back to the
 * Chinese column if there is one, else to English.
 */
export function buttonName(key: MessageKey): string {
  const names: Record<string, string> = {};
  for (const l of LOCALES) {
    names[l] = STRINGS[l][key];
  }
  for (const code of ['zh_CN', 'zh_TW']) {
    names[code] ??= STRINGS[toLocale(code)][key];
  }
  return JSON.stringify(names);
}
