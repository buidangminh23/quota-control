/** Confidence intervals for the rates the Benchmark tab shows. */

/** Two-sided 95%. */
export const Z95 = 1.959963984540054;
/** Two-sided 95% shared by three intervals (Bonferroni: each at 1 − 0.05/3). */
export const Z95_OF_THREE = 2.3939797998185104;

export interface Interval {
  /** Observed rate, 0..1. */
  rate: number;
  low: number;
  high: number;
  /** Sample size. */
  n: number;
}

/**
 * Wilson score interval for `successes` out of `n`. Unlike the normal approximation it stays inside
 * 0..1 and keeps a sensible width at small `n` and at rates near 0% or 100%.
 */
export function wilson(successes: number, n: number, z: number = Z95): Interval | null {
  if (!(n > 0) || successes < 0 || successes > n) return null;
  const rate = successes / n;
  const z2 = z * z;
  const denominator = 1 + z2 / n;
  const center = (rate + z2 / (2 * n)) / denominator;
  const half = (z * Math.sqrt((rate * (1 - rate)) / n + z2 / (4 * n * n))) / denominator;
  return { rate, low: Math.max(0, center - half), high: Math.min(1, center + half), n };
}

/** Whether two intervals share any value, i.e. the data cannot tell them apart at this confidence. */
export function overlaps(a: { low: number; high: number }, b: { low: number; high: number }): boolean {
  return a.low <= b.high && b.low <= a.high;
}
