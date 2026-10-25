import assert from 'node:assert/strict';
import { stat } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { createFixture, prepareBunFixture } from '../lib/fixtures.mjs';
import { buildCases } from '../lib/cases.mjs';
import { execute } from '../lib/process.mjs';

test('Bun integration: migrate offline, list all packages and fetch from isolated registry', { skip: process.env.TNPM_BENCH_BUN_TESTS !== '1' }, async () => {
  const fixture = await createFixture();
  try {
    await prepareBunFixture(fixture);
    await stat(join(fixture.cwd, 'bun.lock'));
    assert.equal(fixture.requests.bun, 0);
    const cases = buildCases(fixture);
    for (const name of ['ls / text', 'ls --all / text', 'view version / force network', 'view --json / force network']) {
      const command = cases.find(test => test.name === name).tools.bun;
      const before = fixture.requests.bun;
      const result = await execute('bun', command.args, { cwd: fixture.cwd, env: fixture.environments.bun });
      command.validate(result.stdout.toString());
      assert.equal(fixture.requests.bun - before, name.startsWith('view') ? 1 : 0);
    }
  } finally { await fixture.close(); }
});
