import assert from 'node:assert/strict';
import { stat } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { createFixture } from '../lib/fixtures.mjs';
import { buildCases } from '../lib/cases.mjs';
import { requestDelta, resolveLiveTarget } from '../lib/live.mjs';
import { caseFixture } from './support.mjs';

test('live fixture uses the public registry without a local server or synthetic dependencies', async () => {
  const fixture = await createFixture({ live: true });
  try {
    assert.equal(fixture.live, true);
    assert.equal(fixture.registry, 'https://registry.npmjs.org/');
    assert.equal(fixture.requests, null, 'unknown HTTP counts must not be reported as zero');
    assert.equal(fixture.packageBytes, null);
    for (const tool of ['tnpm', 'bun']) {
      assert.equal(fixture.registries[tool], fixture.registry);
      assert.equal(fixture.environments[tool].npm_config_registry, fixture.registry);
      assert.ok(fixture.environments[tool].HOME.startsWith(fixture.root));
    }
    await assert.rejects(stat(join(fixture.cwd, 'node_modules')), { code: 'ENOENT' });
    await assert.rejects(stat(join(fixture.cwd, 'package-lock.json')), { code: 'ENOENT' });
  } finally { await fixture.close(); }
});

test('live cases pin hono, exclude unverifiable cache comparisons and validate the response', () => {
  const fixture = caseFixture(true);
  const target = { name: 'hono', version: '4.0.0', spec: 'hono@4.0.0' };
  const cases = buildCases(fixture, target);
  assert.equal(cases.length, 6);
  assert.ok(cases.every(test => test.name.startsWith('view ') && test.cache !== 'warm cache'));
  for (const test of cases) {
    for (const tool of ['tnpm', 'bun']) {
      const command = test.tools[tool];
      assert.equal(command.args[0], tool === 'bun' ? 'info' : 'view');
      assert.equal(command.args[1], target.spec);
      assert.ok(command.args.includes('https://registry.npmjs.org/'));
      if (test.cache === 'force network') assert.ok(command.args.includes('--no-cache'));
      if (test.name.startsWith('view version')) {
        command.validate('4.0.0\n');
        assert.throws(() => command.validate('9.9.9'));
      } else if (test.name.startsWith('view --json')) {
        command.validate('{"name":"hono","version":"4.0.0"}');
        assert.throws(() => command.validate('{"version":"9.9.9"}'));
      } else { command.validate('hono@4.0.0 | MIT\n'); }
    }
  }
});

test('live discovery resolves once outside timing and handles plain/scalar/array output', async () => {
  const fixture = caseFixture(true);
  for (const [tool, output] of [['tnpm', '4.0.0\n'], ['bun', '"4.0.0"']]) {
    let calls = 0;
    const target = await resolveLiveTarget(async (invoked, args) => {
      calls++;
      assert.equal(invoked, tool);
      assert.equal(args[1], 'hono');
      assert.ok(args.includes('--registry'));
      assert.ok(args.includes('--no-cache'));
      if (tool === 'bun') assert.ok(args.includes('--cache-dir'));
      return { stdout: Buffer.from(output) };
    }, tool, fixture);
    assert.equal(calls, 1);
    assert.deepEqual(target, { name: 'hono', version: '4.0.0', spec: 'hono@4.0.0', resolvedBy: tool });
  }
  await assert.rejects(resolveLiveTarget(async () => ({ stdout: Buffer.from('not a version') }), 'tnpm', fixture), /Cannot resolve/);
});

test('unobservable live HTTP counts remain null', () => {
  assert.equal(requestDelta(null, null), null);
  assert.equal(requestDelta(null, 5), null);
  assert.equal(requestDelta(5, null), null);
  assert.equal(requestDelta(5, 8), 3);
  assert.equal(requestDelta(5, 5), 0);
});
