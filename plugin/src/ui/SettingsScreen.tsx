import React, {useCallback, useEffect, useState} from 'react';
import {ScrollView, Text, View} from 'react-native';
import {
  connectFirst,
  noBridgeMessage,
  type BridgeConnection,
} from '../bridgeClient';
import {loadBridges} from '../bridgeConfig';
import {t} from '../i18n';
import type {Settings} from '../protocol';
import {forgetThisNote} from '../forget';
import {closeView, ensurePermissions} from '../sdk';
import {show} from '../session';
import {Button, styles} from './common';
import {Slider} from './Slider';

function describeAngle(deg: number | undefined): string {
  if (!deg) {
    return t('level');
  }
  return t(deg > 0 ? 'risesRight' : 'fallsRight', {deg: Math.abs(deg)});
}

/** Opened from the plugin's config button. Settings live on the bridge. */
export function SettingsScreen() {
  const [status, setStatus] = useState(t('connecting'));
  const [settings, setSettings] = useState<Settings | null>(null);
  const [conn, setConn] = useState<BridgeConnection | null>(null);
  const [destination, setDestination] = useState('');

  useEffect(() => {
    let c: BridgeConnection | null = null;
    (async () => {
      await ensurePermissions();
      try {
        c = await connectFirst((await loadBridges()).endpoints);
      } catch (e) {
        setStatus(noBridgeMessage(e));
        return;
      }
      c.onMessage = m => {
        if (m.type === 'settings') {
          setSettings(m.settings);
          setStatus(t('saved'));
        } else if (m.type === 'historyCleared') {
          setStatus(t('forgotten', {n: m.notes}));
        }
      };
      setConn(c);
      setDestination(c.provider);
      setSettings(c.settings);
      setStatus(t('connectedTo', {name: c.name}));
    })();
    return () => c?.close();
  }, []);

  const change = useCallback(
    (patch: Partial<Settings>) => {
      if (conn && settings) {
        conn.send({type: 'setSettings', settings: {...settings, ...patch}});
      }
    },
    [conn, settings],
  );
  const toggle = useCallback(
    (
      key:
        | 'lineTilt'
        | 'thinkingAnimation'
        | 'busyNotice'
        | 'avoidOverwriting',
    ) => {
      if (settings) {
        change({[key]: !settings[key]});
      }
    },
    [change, settings],
  );

  return (
    <View style={styles.screen}>
      <Text style={styles.title}>{t('settingsTitle')}</Text>
      {/* Close and diagnostics sit at the top: the page is longer than the screen. */}
      <View style={styles.row}>
        <Button label={t('pairButton')} onPress={() => show('pair')} />
        <Button label={t('diagButton')} onPress={() => show('diagnostics')} />
        <Button label={t('close')} onPress={() => void closeView()} />
      </View>
      <ScrollView style={styles.fill}>
        <Text style={styles.text}>{status}</Text>
        {destination ? (
          <Text style={styles.small}>
            {t('destination', {name: destination})}
          </Text>
        ) : null}
        <View style={styles.row}>
          <Button
            disabled={!settings}
            label={`${t('lineTilt')}: ${settings?.lineTilt ? 'ON' : 'OFF'}`}
            onPress={() => toggle('lineTilt')}
          />
        </View>
        <Text style={styles.small}>
          {t('lineTiltHelp', {angle: describeAngle(settings?.tiltDegrees)})}
        </Text>
        <View style={styles.row}>
          <Button
            disabled={!settings}
            label={`${t('thinkingAnimation')}: ${
              settings?.thinkingAnimation ? 'ON' : 'OFF'
            }`}
            onPress={() => toggle('thinkingAnimation')}
          />
        </View>
        <Text style={styles.small}>{t('thinkingAnimationHelp')}</Text>
        <View style={styles.row}>
          <Button
            disabled={!settings}
            label={`${t('avoidOverwritingLabel')}: ${
              settings?.avoidOverwriting ? 'ON' : 'OFF'
            }`}
            onPress={() => toggle('avoidOverwriting')}
          />
        </View>
        <Text style={styles.small}>{t('avoidOverwritingHelp')}</Text>
        <View style={styles.row}>
          <Button
            disabled={!settings}
            label={`${t('busyNoticeLabel')}: ${
              settings?.busyNotice ? 'ON' : 'OFF'
            }`}
            onPress={() => toggle('busyNotice')}
          />
        </View>
        <Text style={styles.small}>{t('busyNoticeHelp')}</Text>
        <Slider
          label={t('fullWidthSpacing')}
          value={settings?.fullWidthSpacing ?? 100}
          min={100}
          max={140}
          step={5}
          disabled={!settings}
          onCommit={v => change({fullWidthSpacing: v})}
        />
        <Slider
          label={t('halfWidthSpacing')}
          value={settings?.halfWidthSpacing ?? 110}
          min={100}
          max={160}
          step={5}
          disabled={!settings}
          onCommit={v => change({halfWidthSpacing: v})}
        />
        <Text style={styles.small}>{t('spacingHelp')}</Text>
        <View style={styles.row}>
          <Button
            disabled={!conn}
            label={t('forgetThisNote')}
            onPress={() => {
              if (conn) {
                void forgetThisNote(conn).then(r => {
                  if (r === 'noNote') {
                    setStatus(t('noNoteOpen'));
                  }
                });
              }
            }}
          />
        </View>
        <Text style={styles.small}>{t('forgetThisNoteHelp')}</Text>
        <View style={styles.row}>
          <Button
            disabled={!conn}
            label={t('forgetHistory')}
            onPress={() => conn?.send({type: 'clearHistory'})}
          />
        </View>
        <Text style={styles.small}>{t('forgetHistoryHelp')}</Text>
      </ScrollView>
    </View>
  );
}
