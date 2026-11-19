import React, {useMemo} from 'react';
import {View} from 'react-native';
import {darkRuns, qrMatrix} from '../qr';

const QUIET = 4; // modules of white around the code, as the QR standard requires

/** A QR code drawn with plain views: pure black on white, no scaling filter, so e-ink keeps the edges. */
export function QrView({text, size}: {text: string; size: number}) {
  const matrix = useMemo(() => qrMatrix(text), [text]);
  const n = matrix.length + 2 * QUIET;
  const module = Math.max(1, Math.floor(size / n)); // whole pixels only: no blurred modules
  return (
    <View
      style={{
        width: n * module,
        height: n * module,
        backgroundColor: '#fff',
        paddingTop: QUIET * module,
        paddingLeft: QUIET * module,
      }}>
      {matrix.map((row, r) => (
        <View key={r} style={{height: module, width: n * module}}>
          {darkRuns(row).map(([start, len]) => (
            <View
              key={start}
              style={{
                position: 'absolute',
                left: start * module,
                width: len * module,
                height: module,
                backgroundColor: '#000',
              }}
            />
          ))}
        </View>
      ))}
    </View>
  );
}
