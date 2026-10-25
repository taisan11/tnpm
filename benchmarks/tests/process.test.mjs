import assert from 'node:assert/strict';
import test from 'node:test';
import { execute } from '../lib/process.mjs';

test('subprocess runner drains stdout and rejects failures', async () => {
  const result = await execute(process.execPath, ['-p', '42']);
  assert.equal(result.stdout.toString().trim(), '42');
  await assert.rejects(execute(process.execPath, ['-e', 'process.exit(2)']), /failed \(2\)/);
});
