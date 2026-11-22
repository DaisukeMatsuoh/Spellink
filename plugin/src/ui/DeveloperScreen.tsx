import React, {useCallback, useRef, useState} from 'react';
import {ScrollView, Text, View} from 'react-native';
import * as diag from '../diagnostics';
import {t} from '../i18n';
import {closeView} from '../sdk';
import * as probe from '../pairingProbe';
import * as net from '../netProbe';
import {QrView} from './QrView';
import {Button, styles} from './common';

/**
 * Developer tools for verifying device behaviour (experiments about the SDK and the device). Not for
 * users: it is reached from the diagnostics screen by tapping its title five times. Results can be
 * sent to the bridge's log.
 */
export function DeveloperScreen({onBack}: {onBack: () => void}) {
  const [lines, setLines] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [qr, setQr] = useState<{text: string; size: number} | null>(null);
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
      <Text style={styles.title}>Spellink developer tools</Text>
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
          label="9. Marker (userData)"
          onPress={run('marker', diag.markerTest)}
        />
        <Button
          disabled={busy}
          label="10. Cache number"
          onPress={run('cache number', diag.cacheNumberTest)}
        />
        <Button
          disabled={busy}
          label="11. Seed in package"
          onPress={run('seed', probe.probeSeed)}
        />
        <Button
          disabled={busy}
          label="12. SDK names"
          onPress={run('sdk names', probe.probeApis)}
        />
        <Button
          disabled={busy}
          label="12b. Network channels"
          onPress={run('channels', net.probeChannels)}
        />
        <Button
          disabled={busy}
          label="13. Files (file://)"
          onPress={run('files', probe.probeFiles)}
        />
        <Button
          disabled={busy}
          label="15. Name store"
          onPress={run('name store', probe.probeNameStore)}
        />
        <Button
          disabled={busy}
          label="16. Read existing"
          onPress={run('read existing', probe.probeReadExisting)}
        />
        <Button
          disabled={busy}
          label={qr ? '14. QR: hide' : '14. QR: show'}
          onPress={() => {
            if (qr) {
              return setQr(null);
            }
            const t0 = Date.now();
            setQr({
              text: 'spellink://pair?v=1&n=0123456789abcdef&h=192.168.2.180:8766',
              size: 640,
            });
            requestAnimationFrame(() =>
              log(`qr: first frame after ${Date.now() - t0}ms`),
            );
          }}
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
        <Button label={t('back')} onPress={onBack} />
        <Button label={t('close')} onPress={() => void closeView()} />
      </View>
      {qr ? <QrView text={qr.text} size={qr.size} /> : null}
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
