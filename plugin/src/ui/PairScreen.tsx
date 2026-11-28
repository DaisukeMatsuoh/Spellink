import React, {useCallback, useEffect, useRef, useState} from 'react';
import {ScrollView, Text, View} from 'react-native';
import {t} from '../i18n';
import {PairingController, type PairView} from '../pairing';
import {loadPairing, realPairDeps, type PairingState} from '../pairingState';
import {groupCode, pairMessage} from '../pairText';
import {closeView} from '../sdk';
import {Button, styles} from './common';
import {QrView} from './QrView';

/** Pairing this device with a bridge (docs/spec/pairing.md, B). */
export function PairScreen() {
  const [state, setState] = useState<PairingState | null>(null);
  const [view, setView] = useState<PairView | null>(null);
  const controller = useRef<PairingController | null>(null);

  const load = useCallback(async (alive: () => boolean) => {
    const s = await loadPairing();
    if (!alive()) {
      return;
    }
    setState(s);
    const ready = s.plan.bridges.find(b => b.state === 'Unpaired' && b.setup);
    controller.current = ready?.setup
      ? new PairingController(
          ready.setup,
          realPairDeps(v => alive() && setView(v)),
          'Unpaired',
        )
      : null;
  }, []);

  useEffect(() => {
    let live = true;
    void load(() => live);
    return () => {
      live = false;
      controller.current?.cancel();
    };
  }, [load]);

  // After a pairing the store has changed: read it again to show the bridge as paired.
  useEffect(() => {
    if (view?.note === 'paired') {
      void load(() => true);
    }
  }, [view?.note, load]);

  const bridges = state?.plan.bridges ?? [];
  const target = bridges.find(b => b.state === 'Unpaired' && b.setup);
  const running =
    view !== null &&
    ['Connecting', 'ShowingCode', 'Storing'].includes(view.state);

  return (
    <View style={styles.screen}>
      <Text style={styles.title}>{t('pairTitle')}</Text>
      <View style={styles.row}>
        {target && !running ? (
          <Button
            label={t('pairStart')}
            onPress={() => controller.current?.tap()}
          />
        ) : null}
        {running ? (
          <Button
            label={t('pairCancel')}
            onPress={() => controller.current?.cancel()}
          />
        ) : null}
        <Button label={t('close')} onPress={() => void closeView()} />
      </View>
      <ScrollView style={styles.fill}>
        {state && bridges.length === 0 ? (
          <Text style={styles.text}>{t('pairNone')}</Text>
        ) : null}
        {bridges.map(b => (
          <Text key={b.id} style={styles.text}>
            {t(b.state === 'Paired' ? 'pairBridgePaired' : 'pairBridgeReady', {
              name: b.name,
            })}
          </Text>
        ))}
        {target ? (
          <Text style={styles.text}>
            {view ? pairMessage(view, target.name) : t('pairIdle')}
          </Text>
        ) : null}
        {view?.code ? (
          <View>
            <Text style={bigDigits}>{groupCode(view.code)}</Text>
            <Text style={styles.small}>
              {t('pairSeconds', {n: view.remainingSec ?? 0})}
            </Text>
            {view.qr ? <QrView text={view.qr} size={QR_SIZE} /> : null}
          </View>
        ) : null}
        {state && state.plan.problems.length > 0 ? (
          <Text style={styles.small}>
            {t('pairProblems', {
              n: state.plan.problems.length,
              ids: problemIds(state.plan.problems),
            })}
          </Text>
        ) : null}
      </ScrollView>
    </View>
  );
}

/** The ids (16 hex digits, which start every problem and every folder name) of unusable records. */
function problemIds(problems: string[]): string {
  const ids = new Set<string>();
  for (const p of problems) {
    const m = /[0-9a-f]{16}/.exec(p);
    if (m) {
      ids.add(m[0]);
    }
  }
  return [...ids].join(', ');
}

/** The QR code is the second way in; the digits come first and must be seen without scrolling. */
const QR_SIZE = 390;

const bigDigits = {
  fontSize: 72,
  fontWeight: '700' as const,
  color: '#000',
  letterSpacing: 6,
  marginVertical: 16,
};
