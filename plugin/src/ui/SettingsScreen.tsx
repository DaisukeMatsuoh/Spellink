import React, {useCallback, useEffect, useState} from 'react';
import {Text, View} from 'react-native';
import {PluginManager} from 'sn-plugin-lib';
import {connectFirst, type BridgeConnection} from '../bridgeClient';
import {loadBridges} from '../bridgeConfig';
import {t} from '../i18n';
import type {Settings} from '../protocol';
import {ensurePermissions} from '../sdk';
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

  useEffect(() => {
    let c: BridgeConnection | null = null;
    (async () => {
      await ensurePermissions();
      try {
        c = await connectFirst((await loadBridges()).endpoints);
      } catch {
        setStatus(t('noBridge'));
        return;
      }
      c.onMessage = m => {
        if (m.type === 'settings') {
          setSettings(m.settings);
          setStatus(t('saved'));
        }
      };
      setConn(c);
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
    (key: 'showProgress' | 'lineTilt' | 'thinkingAnimation') => {
      if (settings) {
        change({[key]: !settings[key]});
      }
    },
    [change, settings],
  );

  return (
    <View style={styles.screen}>
      <Text style={styles.title}>{t('settingsTitle')}</Text>
      <Text style={styles.text}>{status}</Text>
      <View style={styles.row}>
        <Button
          disabled={!settings}
          label={`${t('showProgress')}: ${
            settings?.showProgress ? 'ON' : 'OFF'
          }`}
          onPress={() => toggle('showProgress')}
        />
      </View>
      <Text style={styles.small}>{t('showProgressHelp')}</Text>
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
        <Button label={t('diagButton')} onPress={() => show('diagnostics')} />
        <Button
          label={t('close')}
          onPress={() => PluginManager.closePluginView()}
        />
      </View>
    </View>
  );
}
