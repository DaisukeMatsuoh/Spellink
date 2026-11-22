import React, {useMemo} from 'react';
import {View} from 'react-native';
import {strokeBounds, strokeSegments} from '../pairStroke';
import type {InkStroke} from '../protocol';

/** Pieces closer together than this are merged: a phrase is far fewer views that way. */
const MIN_STEP = 4;
const PEN = 4;

/**
 * Handwriting on the screen: every stroke as short straight pieces, pure black on white. Returns
 * null when the writing would need too many pieces (the caller then shows the words as text).
 */
export function StrokeView({strokes}: {strokes: InkStroke[]}) {
  const segments = useMemo(() => strokeSegments(strokes, MIN_STEP), [strokes]);
  const {width, height} = useMemo(() => strokeBounds(strokes), [strokes]);
  if (!segments) {
    return null;
  }
  return (
    <View style={{width, height, backgroundColor: '#fff'}}>
      {segments.map((s, i) => (
        <View
          key={i}
          style={{
            position: 'absolute',
            left: s.cx - s.length / 2,
            top: s.cy - PEN / 2,
            width: s.length + PEN / 2,
            height: PEN,
            borderRadius: PEN / 2,
            backgroundColor: '#000',
            transform: [{rotate: `${s.angle}rad`}],
          }}
        />
      ))}
    </View>
  );
}
