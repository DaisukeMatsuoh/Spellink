import React from 'react';
import {Pressable, StyleSheet, Text} from 'react-native';

// E-ink: pure black on white, no animation, large touch targets.
export function Button({
  label,
  onPress,
  disabled,
}: {
  label: string;
  onPress: () => void;
  disabled?: boolean;
}) {
  return (
    <Pressable
      onPress={onPress}
      disabled={disabled}
      style={[styles.button, disabled && styles.disabled]}>
      <Text style={styles.buttonText}>{label}</Text>
    </Pressable>
  );
}

export const styles = StyleSheet.create({
  screen: {flex: 1, backgroundColor: '#fff', padding: 24},
  fill: {flex: 1},
  title: {fontSize: 28, fontWeight: '700', color: '#000', marginBottom: 16},
  text: {fontSize: 22, color: '#000', marginBottom: 8},
  small: {fontSize: 16, color: '#000', fontFamily: 'monospace'},
  row: {flexDirection: 'row', flexWrap: 'wrap', gap: 12, marginVertical: 12},
  button: {
    borderWidth: 2,
    borderColor: '#000',
    borderRadius: 8,
    paddingVertical: 12,
    paddingHorizontal: 20,
  },
  disabled: {borderColor: '#999'},
  buttonText: {fontSize: 22, color: '#000'},
});
