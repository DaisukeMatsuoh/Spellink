import React, {useState} from 'react';
import {Text, View, type LayoutChangeEvent} from 'react-native';
import {ratioOf, valueAt} from '../slider';
import {styles} from './common';

interface Props {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  unit?: string;
  disabled?: boolean;
  /** Called once, when the finger lifts (e-ink: no redraw storm while dragging). */
  onCommit: (value: number) => void;
}

/** A scroll-bar style slider: touch or drag the track, the value is applied on release. */
export function Slider({
  label,
  value,
  min,
  max,
  step,
  unit = '%',
  disabled,
  onCommit,
}: Props) {
  const [width, setWidth] = useState(0);
  const [drag, setDrag] = useState<number | null>(null);
  const shown = drag ?? value;

  const at = (x: number) => valueAt(x, width, min, max, step);
  return (
    <View style={styles.sliderBlock}>
      <Text style={styles.text}>{`${label}: ${shown}${unit}`}</Text>
      <View
        style={styles.sliderTrack}
        onLayout={(e: LayoutChangeEvent) =>
          setWidth(e.nativeEvent.layout.width)
        }
        onStartShouldSetResponder={() => !disabled}
        onMoveShouldSetResponder={() => !disabled}
        onResponderGrant={e => setDrag(at(e.nativeEvent.locationX))}
        onResponderMove={e => setDrag(at(e.nativeEvent.locationX))}
        onResponderRelease={e => {
          const v = at(e.nativeEvent.locationX);
          setDrag(null);
          onCommit(v);
        }}
        onResponderTerminate={() => setDrag(null)}>
        <View style={styles.sliderLine} />
        <View
          style={[
            styles.sliderThumb,
            {left: `${ratioOf(shown, min, max) * 100}%`},
          ]}
        />
      </View>
      <View style={styles.sliderEnds}>
        <Text style={styles.small}>{`${min}${unit}`}</Text>
        <Text style={styles.small}>{`${max}${unit}`}</Text>
      </View>
    </View>
  );
}
