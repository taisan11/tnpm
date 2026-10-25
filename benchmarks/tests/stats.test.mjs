import assert from 'node:assert/strict';
import test from 'node:test';
import { summarize } from '../lib/stats.mjs';

test('sample summaries retain every observation without outlier removal', () => {
  const samples = [5, 1, 3, 100, 2];
  const result = summarize(samples);
  assert.equal(result.avg, 22.2);
  assert.equal(result.p50, 3);
  assert.equal(result.p99, 100);
  assert.deepEqual(result.samples, samples);
  assert.throws(() => summarize([]));
});
