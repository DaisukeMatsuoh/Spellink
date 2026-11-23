import React, {useCallback, useRef, useState} from 'react';
import {ScrollView, Text, TouchableWithoutFeedback, View} from 'react-native';
import * as diag from '../diagnostics';
import {t} from '../i18n';
import {closeView} from '../sdk';
import {DeveloperScreen} from './DeveloperScreen';
import {Button, styles} from './common';

/** Taps on the title that open the developer tools (not advertised: they are for the author). */
const DEVELOPER_TAPS = 5;

/**
 * What a user may need when something does not work: check the connection, and send what the
 * check found to their own bridge (their computer), where it is kept in a log file and nowhere else.
 */
export function DiagnosticsScreen() {
  const [lines, setLines] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [developer, setDeveloper] = useState(false);
  const taps = useRef(0);
  const all = useRef<string[]>([]);
  const log = useCallback((line: string) => {
    all.current = [...all.current, line];
    setLines(all.current);
  }, []);
  const run = (label: string, f: () => Promise<void>) => async () => {
    setBusy(true);
    log(`--- ${label}`);
    try {
      await f();
    } catch (e) {
      log(`ERROR ${(e as Error).message}`);
    }
    setBusy(false);
  };

  if (developer) {
    return <DeveloperScreen onBack={() => setDeveloper(false)} />;
  }
  return (
    <View style={styles.screen}>
      <TouchableWithoutFeedback
        onPress={() => {
          taps.current += 1;
          if (taps.current >= DEVELOPER_TAPS) {
            taps.current = 0;
            setDeveloper(true);
          }
        }}>
        <Text style={styles.title}>{t('diagTitle')}</Text>
      </TouchableWithoutFeedback>
      <Text style={styles.text}>{t('diagHelp')}</Text>
      <View style={styles.row}>
        <Button
          disabled={busy}
          label={t('diagCheck')}
          onPress={run('check', async () => {
            await diag.environment(log);
            await diag.network(log);
            await diag.noteProbe(log);
            log(t('diagDone'));
          })}
        />
        <Button
          disabled={busy || lines.length === 0}
          label={t('diagSend')}
          onPress={run('report', () => diag.sendReport(all.current, log))}
        />
        <Button label={t('close')} onPress={() => void closeView()} />
      </View>
      <ScrollView style={styles.fill}>
        {lines.map((l, i) => (
          <Text key={i} style={styles.small}>
            {l}
          </Text>
        ))}
      </ScrollView>
    </View>
  );
}
