// What the pairing screen says for each situation (the words are in i18n.ts).
import {t} from './i18n';
import type {PairView} from './pairing';

/** The one line the screen shows under the bridge's name. */
export function pairMessage(v: PairView, name: string): string {
  switch (v.note) {
    case 'idle':
      return t('pairIdle');
    case 'connecting':
      return t('pairConnecting');
    case 'showing':
      return t('pairShowing');
    case 'storing':
      return t('pairStoring');
    case 'paired':
      return t('pairPaired', {name});
    case 'timeout':
      return t('pairTimeout');
    case 'cancelled':
      return t('pairCancelled');
    case 'storeFailed':
      return t('pairStoreFailed');
    case 'refused':
      return t('pairRefused');
    case 'noSecret':
      return t('pairNoSecret');
    case 'denied':
      return t('pairDenied', {detail: v.detail ?? ''});
    case 'disconnected':
      // the controller reports a package with no first secret as a lost connection with this detail
      return v.detail === 'noSecret'
        ? t('pairNoSecret')
        : t('pairDisconnected', {detail: v.detail ?? ''});
  }
}

/** "1234 5678" for an eight digit code: easier to read and to type. */
export function groupCode(code: string): string {
  return code.replace(/^(\d{4})(\d+)$/, '$1 $2');
}
