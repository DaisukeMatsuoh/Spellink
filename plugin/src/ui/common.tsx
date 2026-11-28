import React from 'react';
import {Pressable, StyleSheet, Text, View} from 'react-native';

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

/** A titled group of settings, set off by a rule (e-ink: structure from lines and size, not colour). */
export function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <View style={styles.section}>
      <Text style={styles.sectionTitle}>{title}</Text>
      {children}
    </View>
  );
}

/** An on/off setting: its name and what it does on the left, its state in a box on the right. */
export function Switch({
  label,
  help,
  value,
  onChange,
  disabled,
}: {
  label: string;
  help: string;
  value: boolean;
  onChange: () => void;
  disabled?: boolean;
}) {
  return (
    <Pressable
      onPress={onChange}
      disabled={disabled}
      accessibilityRole="switch"
      accessibilityState={{checked: value, disabled: !!disabled}}
      style={styles.switchRow}>
      <View style={styles.switchText}>
        <Text style={styles.label}>{label}</Text>
        <Text style={styles.help}>{help}</Text>
      </View>
      <View
        style={[
          styles.switchBox,
          value && styles.switchBoxOn,
          disabled && styles.disabled,
        ]}>
        <Text style={[styles.switchState, value && styles.switchStateOn]}>
          {value ? 'ON' : 'OFF'}
        </Text>
      </View>
    </Pressable>
  );
}

export const styles = StyleSheet.create({
  section: {
    borderTopWidth: 2,
    borderTopColor: '#000',
    marginTop: 20,
    paddingTop: 12,
  },
  sectionTitle: {
    fontSize: 24,
    fontWeight: '700',
    color: '#000',
    marginBottom: 8,
  },
  label: {fontSize: 22, fontWeight: '600', color: '#000'},
  help: {fontSize: 18, color: '#333', marginTop: 4},
  switchRow: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    paddingVertical: 14,
    borderBottomWidth: 1,
    borderBottomColor: '#999',
  },
  switchText: {flex: 1, paddingRight: 16},
  switchBox: {
    minWidth: 84,
    alignItems: 'center',
    borderWidth: 2,
    borderColor: '#000',
    borderRadius: 8,
    paddingVertical: 10,
    paddingHorizontal: 12,
    backgroundColor: '#fff',
  },
  switchBoxOn: {backgroundColor: '#000'},
  switchState: {fontSize: 22, fontWeight: '700', color: '#000'},
  switchStateOn: {color: '#fff'},
  titleRow: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
  },
  statusLine: {fontSize: 22, color: '#000', marginBottom: 4},
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
  sliderBlock: {marginVertical: 12},
  sliderTrack: {height: 56, justifyContent: 'center'},
  sliderLine: {height: 4, backgroundColor: '#000'},
  sliderThumb: {
    position: 'absolute',
    width: 28,
    height: 44,
    marginLeft: -14,
    backgroundColor: '#fff',
    borderWidth: 3,
    borderColor: '#000',
    borderRadius: 6,
  },
  sliderEnds: {flexDirection: 'row', justifyContent: 'space-between'},
  disabled: {borderColor: '#999'},
  buttonText: {fontSize: 22, color: '#000'},
});
