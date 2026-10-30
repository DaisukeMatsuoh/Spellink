import React, {useEffect, useRef, useState} from 'react';
import {ScrollView, Text, View} from 'react-native';
import {PluginManager} from 'sn-plugin-lib';
import {AskFlow, type AskState} from '../askFlow';
import {t} from '../i18n';
import {Button, styles} from './common';

/** Small popup shown while a lassoed question is answered; the note stays visible around it. */
export function AskPanel({seq}: {seq: number}) {
  const [state, setState] = useState<AskState>({message: '', finished: false});
  const flow = useRef<AskFlow | null>(null);

  useEffect(() => {
    const f = new AskFlow(setState);
    flow.current = f;
    f.run();
    return () => f.cancel();
  }, [seq]);

  const close = () => {
    if (!state.finished) {
      flow.current?.cancel();
    }
    PluginManager.closePluginView();
  };

  return (
    <View style={styles.screen}>
      <Text style={styles.text}>{state.message}</Text>
      <ScrollView style={styles.fill}>
        {state.question ? (
          <Text style={styles.text}>{`${t('question')}: ${
            state.question
          }`}</Text>
        ) : null}
        {state.answer ? (
          <Text style={styles.text}>{`${t('answer')}: ${state.answer}`}</Text>
        ) : null}
      </ScrollView>
      <View style={styles.row}>
        <Button
          label={state.finished ? t('close') : t('cancel')}
          onPress={close}
        />
      </View>
    </View>
  );
}
