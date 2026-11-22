import React, {useMemo} from 'react';
import {PixelRatio, View} from 'react-native';
import {qrMatrix, qrPatches, qrRects, qrScale} from '../qr';

const QUIET = 4; // modules of white around the code, as the QR standard requires

/**
 * A QR code drawn with plain views: pure black on white, no scaling filter, so e-ink keeps the
 * edges. The module is a whole number of PHYSICAL pixels (not of layout units), every block is
 * placed from the same origin, and the places where two blocks meet get a patch that crosses the
 * boundary: no hairline between rows, whatever the screen's density.
 */
export function QrView({text, size}: {text: string; size: number}) {
  const matrix = useMemo(() => qrMatrix(text), [text]);
  const n = matrix.length + 2 * QUIET;
  const {moduleDp, pixelDp} = qrScale(size, n, PixelRatio.get());
  const blocks = useMemo(() => qrRects(matrix), [matrix]);
  const patches = useMemo(() => qrPatches(matrix), [matrix]);
  const at = (units: number) => (QUIET + units) * moduleDp;
  const black = {position: 'absolute' as const, backgroundColor: '#000'};
  return (
    <View
      style={{
        width: n * moduleDp,
        height: n * moduleDp,
        backgroundColor: '#fff',
      }}>
      {blocks.map(b => (
        <View
          key={`b${b.row}:${b.col}`}
          style={{
            ...black,
            left: at(b.col),
            top: at(b.row),
            width: b.w * moduleDp,
            height: b.h * moduleDp,
          }}
        />
      ))}
      {patches.map(p => (
        <View
          key={`p${p.row}:${p.col}`}
          style={{
            ...black,
            left: at(p.col),
            top: at(p.row) - pixelDp,
            width: p.w * moduleDp,
            height: 2 * pixelDp,
          }}
        />
      ))}
    </View>
  );
}
