import {NativeUIUtils} from 'sn-plugin-lib';
import {t} from './i18n';

/**
 * A small native dialog over the note (the ones Supernote itself shows), instead of taking over
 * the whole screen. Resolves to false when the dialog could not be shown, so the caller can fall
 * back to something else.
 */
export async function notify(message: string): Promise<boolean> {
  try {
    await NativeUIUtils.showRattaDialog(message, t('close'), 'OK', false);
    return true;
  } catch {
    return false;
  }
}
