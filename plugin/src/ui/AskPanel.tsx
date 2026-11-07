import React, {useSyncExternalStore} from 'react';
import {ScrollView, Text, View} from 'react-native';
import {
  cancelAsk,
  getAskState,
  panelClosed,
  subscribeAsk,
} from '../askController';
import {t} from '../i18n';
import {closeView} from '../sdk';
import {Button, styles} from './common';

/** Progress / error panel for a question being answered in the background. */
export function AskPanel() {
  const state = useSyncExternalStore(subscribeAsk, getAskState);

  const close = () => {
    if (!state.finished) {
      cancelAsk();
    }
    panelClosed();
    void closeView();
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
