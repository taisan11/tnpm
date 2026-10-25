import assert from 'node:assert/strict';
import test from 'node:test';
import { caseFixture } from './support.mjs';
import { buildCases, parseTools } from '../lib/cases.mjs';

test('tool selection rejects unknown names and duplicates', () => {
  assert.deepEqual(parseTools('tnpm,bun'), ['tnpm', 'bun']);
  assert.deepEqual(parseTools(' npm, tnpm,bun '), ['npm', 'tnpm', 'bun']);
  for (const value of ['', 'npm,', 'npm,npm', 'pnpm']) assert.throws(() => parseTools(value));
});

test('Bun mappings use actual equivalents and never substitute npm wrappers', () => {
  const fixture = caseFixture();
  const cases = buildCases(fixture);
  const find = name => cases.find(test => test.name === name);
  assert.deepEqual(find('ls / text').tools.bun.args, ['pm', 'ls']);
  assert.deepEqual(find('ls --all / text').tools.bun.args, ['pm', 'ls', '--all']);
  for (const name of ['ls --json', 'fund --json', 'get registry', 'version --json (read only)']) {
    assert.ok(find(name).tools.bun.skipped);
  }
  const online = find('view version / force network').tools.bun.args;
  assert.equal(online[0], 'info');
  assert.ok(online.includes('--no-cache'));
  assert.ok(online.includes(fixture.registries.bun));
  assert.ok(find('view version / warm cache').tools.bun.args.includes('--prefer-offline'));
});
