// Slider arithmetic, kept apart from the component so it can be tested.

/** The value under touch position `x` on a track `width` wide, snapped to `step` and clamped. */
export function valueAt(
  x: number,
  width: number,
  min: number,
  max: number,
  step: number,
): number {
  if (width <= 0) {
    return min;
  }
  const ratio = Math.max(0, Math.min(1, x / width));
  const raw = min + ratio * (max - min);
  const snapped = min + Math.round((raw - min) / step) * step;
  return Math.max(min, Math.min(max, snapped));
}

/** Where the thumb sits, 0..1, for `value`. */
export function ratioOf(value: number, min: number, max: number): number {
  return max === min
    ? 0
    : Math.max(0, Math.min(1, (value - min) / (max - min)));
}
