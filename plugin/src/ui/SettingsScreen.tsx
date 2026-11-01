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

  const toggle = useCallback(() => {
    if (conn && settings) {
      conn.send({
        type: 'setSettings',
        settings: {...settings, showProgress: !settings.showProgress},
      });
    }
  }, [conn, settings]);

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
          onPress={toggle}
        />
      </View>
      <Text style={styles.small}>{t('showProgressHelp')}</Text>
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
