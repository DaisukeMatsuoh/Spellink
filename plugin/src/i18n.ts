import {PluginManager} from 'sn-plugin-lib';

const STRINGS = {
  en: {
    askButton: 'Ask Claude',
    diagButton: 'Cloute diagnostics',
    permissions: 'Checking permissions…',
    permissionDenied: 'Permission not granted: {list}',
    recognizing: 'Reading your handwriting…',
    noText: 'No handwriting found in the selection.',
    connecting: 'Connecting to the bridge…',
    noBridge:
      'No bridge reachable. Is cloute-bridge running on a computer on this network?',
    thinking: 'Claude is thinking…',
    writing: 'Writing… ({lines} lines)',
    done: 'Done.',
    truncated: 'The answer did not fit on this page and was cut short.',
    noRoom: 'There is no room below the selection on this page.',
    error: 'Error: {message}',
    question: 'Question',
    answer: 'Answer',
    close: 'Close',
    cancel: 'Cancel',
  },
  ja: {
    askButton: 'Claudeに聞く',
    diagButton: 'Cloute 診断',
    permissions: '権限を確認しています…',
    permissionDenied: '権限が許可されていません: {list}',
    recognizing: '手書きを読み取っています…',
    noText: '選択範囲に手書き文字が見つかりませんでした。',
    connecting: 'ブリッジに接続しています…',
    noBridge:
      'ブリッジに接続できません。同じネットワークのPCで cloute-bridge が動いているか確認してください。',
    thinking: 'Claude が考えています…',
    writing: '書いています…（{lines} 行）',
    done: '完了しました。',
    truncated: '回答がこのページに収まらなかったため、途中までになっています。',
    noRoom: 'このページには選択範囲の下に書く余白がありません。',
    error: 'エラー: {message}',
    question: '質問',
    answer: '回答',
    close: '閉じる',
    cancel: 'キャンセル',
  },
} as const;

export type Locale = keyof typeof STRINGS;
export type MessageKey = keyof (typeof STRINGS)['en'];

/** Firmware language codes are like "en", "ja", "zh_CN"; anything unsupported falls back to English. */
function toLocale(code: string | undefined): Locale {
  return code?.toLowerCase().startsWith('ja') ? 'ja' : 'en';
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

/** Button labels are a JSON map of firmware language code → label; the firmware picks one. */
export function buttonName(key: MessageKey): string {
  return JSON.stringify({
    en: STRINGS.en[key],
    ja: STRINGS.ja[key],
    zh_CN: STRINGS.en[key],
    zh_TW: STRINGS.en[key],
  });
}
