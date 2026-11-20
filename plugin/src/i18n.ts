import {PluginManager} from 'sn-plugin-lib';

const STRINGS = {
  en: {
    askButton: 'Ask AI',
    diagButton: 'Spellink diagnostics',
    permissions: 'Checking permissions…',
    permissionDenied: 'Permission not granted: {list}',
    recognizing: 'Reading your handwriting…',
    noText: "This can't be read as text.",
    connecting: 'Connecting to the bridge…',
    noBridge:
      'No bridge reachable. Is spellink-bridge running on a computer on this network?',
    thinking: 'The AI is thinking…',
    reconnecting: 'Reconnecting ({n}/{max})…',
    badFrame: 'The bridge sent data that could not be used.',
    busy: 'An answer is already being written.',
    avoidOverwritingLabel: 'Avoid existing writing',
    avoidOverwritingHelp:
      'Reads the page before answering so the answer does not run over what is written below (and moves to a new page when there is no room). Slower: the note is saved and read each time.',
    busyNoticeLabel: 'Notice when busy',
    busyNoticeHelp:
      'If you ask again while an answer is being written, show a short notice (it is otherwise ignored).',
    writing: 'Writing… ({lines} lines)',
    done: 'Done.',
    truncated: 'The answer did not fit on this page and was cut short.',
    noRoom: 'There is no room below the selection on this page.',
    hostBusy:
      'Supernote did not respond ({api}), so the answer was stopped. Wait a moment and try again.',
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
      'Draws a wavy line on the page while the AI is thinking, and removes it before the answer.',
    fullWidthSpacing: 'Spacing: kanji / kana',
    halfWidthSpacing: 'Spacing: letters / digits',
    spacingHelp:
      '100% is the tightest. Widen it if characters touch; full-width and half-width are set separately.',
    forgetThisNote: 'Forget this note’s conversation',
    forgetThisNoteHelp:
      'Only for the note that is open now. Open the settings from inside a note to use it.',
    noNoteOpen: 'No note is open: open the settings from inside a note.',
    forgetHistory: 'Forget all conversations',
    forgetHistoryHelp:
      'The AI is given earlier questions written in the same note. This makes it forget them in every note.',
    forgotten: 'Forgot the conversations of {n} note(s).',
    lineTilt: 'Writing slope',
    lineTiltHelp:
      "Like a person's handwriting habit, the whole answer is written at this agent's own angle: {angle}.",
    risesRight: 'rises to the right {deg}°',
    fallsRight: 'falls to the right {deg}°',
    level: 'level',
    connectedTo: 'Connected to {name}',
    destination: 'Asking: {name}',
    saved: 'Saved.',
    pairButton: 'Pair with a bridge',
    pairTitle: 'Pair with a bridge',
    pairStart: 'Pair',
    pairCancel: 'Cancel',
    pairNone:
      'This plugin has no bridge yet. On the computer that runs spellink-bridge, run "spellink-bridge device add <name>", then install the file it writes on this device.',
    pairBridgePaired: '{name}: paired',
    pairBridgeReady: '{name}: ready to pair',
    pairIdle:
      'Press Pair, then type the code that appears into spellink-bridge on your computer.',
    pairConnecting: 'Connecting to the bridge…',
    pairShowing: 'Type this code on your computer: spellink-bridge device pair',
    pairSeconds: '{n} seconds left',
    pairStoring: 'Saving the key…',
    pairPaired: 'Paired with {name}.',
    pairTimeout: 'The code was not entered in time. Press Pair to try again.',
    pairCancelled: 'Cancelled.',
    pairStoreFailed:
      'The key could not be saved on this device. Press Pair to try again.',
    pairDisconnected: 'The connection ended: {detail}',
    pairNoSecret:
      'This file has already been used to pair. On the computer, run "spellink-bridge device reissue <name>" and install the new file.',
    pairDenied: 'The bridge refused: {detail}',
    pairRefused:
      'The bridge no longer accepts this device. Ask for a new file with "spellink-bridge device reissue <name>".',
    pairProblems: '{n} saved pairing item(s) could not be used.',
  },
  ja: {
    askButton: 'AIに聞く',
    diagButton: 'Spellink 診断',
    permissions: '権限を確認しています…',
    permissionDenied: '権限が許可されていません: {list}',
    recognizing: '手書きを読み取っています…',
    noText: '文字として読み取れない内容です。',
    connecting: 'ブリッジに接続しています…',
    noBridge:
      'ブリッジに接続できません。同じネットワークのPCで spellink-bridge が動いているか確認してください。',
    thinking: 'AI が考えています…',
    reconnecting: '再接続中 ({n}/{max})…',
    badFrame: 'ブリッジから、使えないデータが届きました。',
    busy: '回答を書いている最中です。',
    avoidOverwritingLabel: '既存の字を避ける',
    avoidOverwritingHelp:
      '回答の前にページを読み、下にある字に重ならないようにします（余白がなければ新しいページへ）。そのたびにノートを保存して読むので、少し遅くなります。',
    busyNoticeLabel: '回答中の通知',
    busyNoticeHelp:
      '回答を書いている最中にもう一度質問を始めたとき、短い通知を出します（オフのときは何も表示せず無視します）。',
    writing: '書いています…（{lines} 行）',
    done: '完了しました。',
    truncated: '回答がこのページに収まらなかったため、途中までになっています。',
    noRoom: 'このページには選択範囲の下に書く余白がありません。',
    hostBusy:
      'Supernote が応答しなかったため（{api}）、回答を中断しました。しばらくしてからもう一度お試しください。',
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
      'AI が考えている間、ページに波線を描き、回答の直前に消します。',
    fullWidthSpacing: '字間: 漢字・かな',
    halfWidthSpacing: '字間: 英字・数字',
    spacingHelp:
      '100% が最も詰まった状態です。文字が重なるときは広げてください。全角と半角は別々に設定できます。',
    forgetThisNote: 'このノートの会話を忘れさせる',
    forgetThisNoteHelp:
      'いま開いているノートの会話だけを消します。ノートの中から設定を開いたときに使えます。',
    noNoteOpen:
      'ノートが開かれていません。ノートの中から設定を開いてください。',
    forgetHistory: 'すべての会話を忘れさせる',
    forgetHistoryHelp:
      '同じノートに続けて書いた質問は、前のやり取りを踏まえて答えます。これを押すと、すべてのノートの会話を忘れます。',
    forgotten: '{n} 冊のノートの会話を忘れました。',
    lineTilt: '書き癖の傾き',
    lineTiltHelp:
      '人の書き癖のように、回答全体をこのエージェント固有の角度で書きます: {angle}',
    risesRight: '右上がり {deg}°',
    fallsRight: '右下がり {deg}°',
    level: '水平',
    connectedTo: '{name} に接続しました',
    destination: '接続先の AI: {name}',
    saved: '保存しました。',
    pairButton: 'ブリッジとペアリング',
    pairTitle: 'ブリッジとペアリング',
    pairStart: 'ペアリング',
    pairCancel: 'やめる',
    pairNone:
      'このプラグインには、まだブリッジがありません。spellink-bridge を動かしているパソコンで「spellink-bridge device add <名前>」を実行し、できたファイルをこの端末に入れてください。',
    pairBridgePaired: '{name}: ペアリング済み',
    pairBridgeReady: '{name}: ペアリング待ち',
    pairIdle:
      '「ペアリング」を押し、表示されるコードを、パソコンの spellink-bridge に入力してください。',
    pairConnecting: 'ブリッジに接続しています…',
    pairShowing:
      'パソコンでこのコードを入力してください: spellink-bridge device pair',
    pairSeconds: '残り {n} 秒',
    pairStoring: '鍵を保存しています…',
    pairPaired: '{name} とペアリングしました。',
    pairTimeout:
      '時間内にコードが入力されませんでした。もう一度「ペアリング」を押してください。',
    pairCancelled: 'やめました。',
    pairStoreFailed:
      '鍵をこの端末に保存できませんでした。もう一度「ペアリング」を押してください。',
    pairDisconnected: '接続が終わりました: {detail}',
    pairNoSecret:
      'このファイルは、すでにペアリングに使われています。パソコンで「spellink-bridge device reissue <名前>」を実行し、新しいファイルを入れてください。',
    pairDenied: 'ブリッジに拒否されました: {detail}',
    pairRefused:
      'ブリッジがこの端末を受け付けなくなりました。「spellink-bridge device reissue <名前>」で新しいファイルを作ってください。',
    pairProblems: '保存されたペアリングの情報のうち {n} 件は使えませんでした。',
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
