import {PluginManager} from 'sn-plugin-lib';

const STRINGS = {
  en: {
    askButton: 'Ask Claude',
    diagButton: 'Spellink diagnostics',
    permissions: 'Checking permissions…',
    permissionDenied: 'Permission not granted: {list}',
    recognizing: 'Reading your handwriting…',
    noText: "This can't be read as text.",
    connecting: 'Connecting to the bridge…',
    noBridge:
      'No bridge reachable. Is spellink-bridge running on a computer on this network?',
    thinking: 'Claude is thinking…',
    writing: 'Writing… ({lines} lines)',
    done: 'Done.',
    truncated: 'The answer did not fit on this page and was cut short.',
    noRoom: 'There is no room below the selection on this page.',
    noteMoved:
      'The note was switched or closed, so the answer was stopped. Nothing was written to the other document.',
    error: 'Error: {message}',
    question: 'Question',
    answer: 'Answer',
    close: 'Close',
    cancel: 'Cancel',
    settingsTitle: 'Spellink settings',
    showProgress: 'Show progress panel (debug)',
    showProgressHelp:
      'Off: the answer is simply written onto the page. The panel still opens when something goes wrong.',
    thinkingAnimation: 'Thinking animation',
    thinkingAnimationHelp:
      'Draws a wavy line on the page while Claude is thinking, and removes it before the answer.',
    fullWidthSpacing: 'Spacing: kanji / kana',
    halfWidthSpacing: 'Spacing: letters / digits',
    spacingHelp:
      '100% is the tightest. Widen it if characters touch; full-width and half-width are set separately.',
    lineTilt: 'Writing slope',
    lineTiltHelp:
      "Like a person's handwriting habit, the whole answer is written at this agent's own angle: {angle}.",
    risesRight: 'rises to the right {deg}°',
    fallsRight: 'falls to the right {deg}°',
    level: 'level',
    connectedTo: 'Connected to {name}',
    saved: 'Saved.',
  },
  ja: {
    askButton: 'Claudeに聞く',
    diagButton: 'Spellink 診断',
    permissions: '権限を確認しています…',
    permissionDenied: '権限が許可されていません: {list}',
    recognizing: '手書きを読み取っています…',
    noText: '文字として読み取れない内容です。',
    connecting: 'ブリッジに接続しています…',
    noBridge:
      'ブリッジに接続できません。同じネットワークのPCで spellink-bridge が動いているか確認してください。',
    thinking: 'Claude が考えています…',
    writing: '書いています…（{lines} 行）',
    done: '完了しました。',
    truncated: '回答がこのページに収まらなかったため、途中までになっています。',
    noRoom: 'このページには選択範囲の下に書く余白がありません。',
    noteMoved:
      'ノートが切り替わった（または閉じられた）ため、回答を中断しました。別のドキュメントには何も書き込んでいません。',
    error: 'エラー: {message}',
    question: '質問',
    answer: '回答',
    close: '閉じる',
    cancel: 'キャンセル',
    settingsTitle: 'Spellink の設定',
    showProgress: '進捗パネルを表示（デバッグ）',
    showProgressHelp:
      'OFF のときは、回答をそのままページに書き込みます。エラーのときだけパネルが開きます。',
    thinkingAnimation: '待機中のアニメーション',
    thinkingAnimationHelp:
      'Claude が考えている間、ページに波線を描き、回答の直前に消します。',
    fullWidthSpacing: '字間: 漢字・かな',
    halfWidthSpacing: '字間: 英字・数字',
    spacingHelp:
      '100% が最も詰まった状態です。文字が重なるときは広げてください。全角と半角は別々に設定できます。',
    lineTilt: '書き癖の傾き',
    lineTiltHelp:
      '人の書き癖のように、回答全体をこのエージェント固有の角度で書きます: {angle}',
    risesRight: '右上がり {deg}°',
    fallsRight: '右下がり {deg}°',
    level: '水平',
    connectedTo: '{name} に接続しました',
    saved: '保存しました。',
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
