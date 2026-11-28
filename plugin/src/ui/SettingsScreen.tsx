import {olderSide, PLUGIN_VERSION} from '../version';
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
import {Button, Section, styles, Switch} from './common';
import {Slider} from './Slider';

type Toggle =
  'lineTilt' | 'thinkingAnimation' | 'busyNotice' | 'avoidOverwriting';

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
  const [bridgeVersion, setBridgeVersion] = useState('');

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
      setBridgeVersion(c.bridgeVersion);
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
    (key: Toggle) => {
      if (settings) {
        change({[key]: !settings[key]});
      }
    },
    [change, settings],
  );

  const older = olderSide(PLUGIN_VERSION, bridgeVersion);
  const versionNote = older
    ? t(older === 'plugin' ? 'versionPluginOlder' : 'versionBridgeOlder')
    : '';

  const on = (key: Toggle) => () => toggle(key);
  return (
    <View style={styles.screen}>
      <View style={styles.titleRow}>
        <Text style={styles.title}>{t('settingsTitle')}</Text>
        {/* Close stays at the top: the page is longer than the screen. */}
        <Button label={t('close')} onPress={() => void closeView()} />
      </View>
      <ScrollView style={styles.fill}>
        <Section title={t('sectionConnection')}>
          <Text style={styles.statusLine}>{status}</Text>
          {versionNote ? <Text style={styles.text}>{versionNote}</Text> : null}
          {destination ? (
            <Text style={styles.help}>
              {t('destination', {name: destination})}
            </Text>
          ) : null}
          {bridgeVersion ? (
            <Text style={styles.help}>
              {t('versionLine', {
                plugin: PLUGIN_VERSION,
                bridge: bridgeVersion,
              })}
            </Text>
          ) : null}
          <View style={styles.row}>
            <Button label={t('pairButton')} onPress={() => show('pair')} />
          </View>
        </Section>

        <Section title={t('sectionWriting')}>
          <Switch
            label={t('lineTilt')}
            help={t('lineTiltHelp', {
              angle: describeAngle(settings?.tiltDegrees),
            })}
            value={!!settings?.lineTilt}
            disabled={!settings}
            onChange={on('lineTilt')}
          />
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
          <Text style={styles.help}>{t('spacingHelp')}</Text>
        </Section>

        <Section title={t('sectionBehavior')}>
          <Switch
            label={t('thinkingAnimation')}
            help={t('thinkingAnimationHelp')}
            value={!!settings?.thinkingAnimation}
            disabled={!settings}
            onChange={on('thinkingAnimation')}
          />
          <Switch
            label={t('avoidOverwritingLabel')}
            help={t('avoidOverwritingHelp')}
            value={!!settings?.avoidOverwriting}
            disabled={!settings}
            onChange={on('avoidOverwriting')}
          />
          <Switch
            label={t('busyNoticeLabel')}
            help={t('busyNoticeHelp')}
            value={!!settings?.busyNotice}
            disabled={!settings}
            onChange={on('busyNotice')}
          />
        </Section>

        <Section title={t('sectionData')}>
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
          <Text style={styles.help}>{t('forgetThisNoteHelp')}</Text>
          <View style={styles.row}>
            <Button
              disabled={!conn}
              label={t('forgetHistory')}
              onPress={() => conn?.send({type: 'clearHistory'})}
            />
          </View>
          <Text style={styles.help}>{t('forgetHistoryHelp')}</Text>
        </Section>

        <Section title={t('sectionSupport')}>
          <View style={styles.row}>
            <Button
              label={t('diagButton')}
              onPress={() => show('diagnostics')}
            />
          </View>
        </Section>
      </ScrollView>
    </View>
  );
}
