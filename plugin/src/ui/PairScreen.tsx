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
    const ready = s.plan.bridges.find(b => b.state === 'U1' && b.setup);
    controller.current = ready?.setup
      ? new PairingController(
          ready.setup,
          realPairDeps(v => alive() && setView(v)),
          'U1',
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
  const target = bridges.find(b => b.state === 'U1' && b.setup);
  const running = view !== null && ['U2', 'U3', 'U4'].includes(view.state);

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
            {t(b.state === 'U5' ? 'pairBridgePaired' : 'pairBridgeReady', {
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
            {view.qr ? <QrView text={view.qr} size={560} /> : null}
          </View>
        ) : null}
        {state && state.plan.problems.length > 0 ? (
          <Text style={styles.small}>
            {t('pairProblems', {n: state.plan.problems.length})}
          </Text>
        ) : null}
      </ScrollView>
    </View>
  );
}

const bigDigits = {
  fontSize: 72,
  fontWeight: '700' as const,
  color: '#000',
  letterSpacing: 6,
  marginVertical: 16,
};
