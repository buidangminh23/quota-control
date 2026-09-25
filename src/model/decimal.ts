/**
 * Swift's `Double.rounded()`: to the nearest integer, ties away from zero, on the binary value.
 * Upstream uses it for percents and for the headline's display precision; every other number goes
 * through ICU half-even rounding in `@/i18n/numbers`.
 */
export function roundHalfAwayFromZero(value: number): number {
  return value < 0 ? -Math.round(-value) : Math.round(value);
}
