// Preserve raw observations, units, and the existing percentile convention.
export function summarize(samples) {
  if (!samples.length || samples.some(n => !Number.isFinite(n) || n < 0)) throw new Error('Invalid samples');
  const sorted = samples.toSorted((a, b) => a - b);
  const avg = samples.reduce((a, b) => a + b, 0) / samples.length;
  return { avg, p50: sorted[Math.floor((sorted.length - 1) * .5)], p99: sorted[Math.ceil((sorted.length - 1) * .99)],
    min: sorted[0], max: sorted.at(-1), samples };
}
