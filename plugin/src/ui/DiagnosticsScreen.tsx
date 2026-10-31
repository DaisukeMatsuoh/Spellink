import React, {useCallback, useRef, useState} from 'react';
import {ScrollView, Text, View} from 'react-native';
import {PluginManager} from 'sn-plugin-lib';
import * as diag from '../diagnostics';
import {t} from '../i18n';
import {Button, styles} from './common';

/** Developer screen for verifying device behaviour; results can be sent to the bridge's log. */
export function DiagnosticsScreen() {
  const [lines, setLines] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const all = useRef<string[]>([]);
  const log = useCallback((line: string) => {
    all.current = [...all.current, line];
    setLines(all.current);
  }, []);
  const run =
    (label: string, f: (log: diag.Log) => Promise<void>) => async () => {
      setBusy(true);
      log(`--- ${label}`);
      try {
        await f(log);
      } catch (e) {
        log(`ERROR ${(e as Error).message}`);
      }
      setBusy(false);
    };

  return (
    <View style={styles.screen}>
      <Text style={styles.title}>Spellink diagnostics</Text>
      <View style={styles.row}>
        <Button
          disabled={busy}
          label="1. Environment"
          onPress={run('environment', diag.environment)}
        />
        <Button
          disabled={busy}
          label="2. Permissions"
          onPress={run('permissions', diag.requestPermissions)}
        />
        <Button
          disabled={busy}
          label="3. Network"
          onPress={run('network', diag.network)}
        />
        <Button
          disabled={busy}
          label="4. Strokes ×10"
          onPress={run('strokes 10', l => diag.strokeBenchmark(10, l))}
        />
        <Button
          disabled={busy}
          label="5. Strokes ×200"
          onPress={run('strokes 200', l => diag.strokeBenchmark(200, l))}
        />
        <Button
          disabled={busy}
          label="6. Text box"
          onPress={run('textbox', diag.textBoxTest)}
        />
        <Button
          disabled={busy}
          label="7. Ask (no lasso)"
          onPress={run('ask', diag.askWithoutLasso)}
        />
        <Button
          disabled={busy}
          label="8. New page"
          onPress={run('new page', diag.newPageTest)}
        />
        <Button
          disabled={busy}
          label="Reload file"
          onPress={run('reload', diag.reloadFile)}
        />
        <Button
          disabled={busy}
          label="Send log to bridge"
          onPress={run('report', l => diag.sendReport(all.current, l))}
        />
        <Button
          label={t('close')}
          onPress={() => PluginManager.closePluginView()}
        />
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
