import assert from 'node:assert/strict';
import { PACKAGE, LATEST, TOOLS } from './fixtures.mjs';

export const TOOL_NAMES = TOOLS;

export function parseTools(value) {
  const names = value.split(',').map(name => name.trim());
  if (!names.length || names.some(name => !TOOL_NAMES.includes(name)) || new Set(names).size !== names.length) {
    throw new Error('--tools must be a comma-separated selection of tnpm,bun without duplicates');
  }
  return names;
}

function json(output) { return JSON.parse(output.trim()); }
const spec = (args, validate) => ({ args, validate });
const skip = reason => ({ skipped: true, reason });

function listing(output, all, asJson) {
  const names = new Set();
  if (asJson) {
    const root = json(output);
    assert.equal(root.name, 'bench-app');
    const walk = tree => {
      for (const [name, value] of Object.entries(tree.dependencies ?? {})) { names.add(name); walk(value); }
    };
    walk(root);
  } else {
    for (const match of output.matchAll(/(@bench\/(?:root-\d{2}|leaf-\d{2}-\d))@1\.0\.0/g)) names.add(match[1]);
  }
  assert.equal(names.size, all ? 100 : 20, 'listing must include the expected installed packages');
  if (!all) assert.ok([...names].every(name => name.startsWith('@bench/root-')));
}

export function buildCases(fixture, target = { name: PACKAGE, version: LATEST, spec: PACKAGE }) {
  const local = (name, args, validate, bun) => ({ name, tools: {
    tnpm: spec(args, validate), bun,
  } });
  const cases = fixture.live ? [] : [
    local('ls --json', ['ls', '--json'], output => listing(output, false, true), skip('bun pm ls has no JSON output; compare the text-listing cases instead.')),
    local('ls --all --json', ['ls', '--all', '--json'], output => listing(output, true, true), skip('bun pm ls has no JSON output; compare the text-listing cases instead.')),
    local('ls / text', ['ls'], output => listing(output, false, false), spec(['pm', 'ls'], output => listing(output, false, false))),
    local('ls --all / text', ['ls', '--all'], output => listing(output, true, false), spec(['pm', 'ls', '--all'], output => listing(output, true, false))),
    local('fund --json', ['fund', '--json'], output => assert.equal(json(output).length, 20), skip('No corresponding Bun funding command.')),
    local('version --json (read only)', ['version', '--json'], output => assert.equal(json(output)['bench-app'], '1.0.0'), skip('bun pm version prints the package version and usage, but does not support the JSON version report.')),
    { name: 'get registry', tools: {
      tnpm: spec(['get', 'registry'], output => assert.equal(output.trim(), fixture.registries.tnpm)),
      bun: skip('No corresponding registry-config getter; bun pm pkg get reads package.json instead.'),
    } },
  ];
  for (const [label, fields, validate] of [
    ['version', ['version'], output => assert.equal(output.trim(), target.version)],
    ['--json', ['--json'], output => {
      const value = json(output);
      assert.equal((Array.isArray(value) ? value[0] : value).version, target.version);
    }],
    ['summary', [], output => assert.ok(output.trimStart().startsWith(`${target.name}@${target.version}`))],
  ]) {
    for (const mode of fixture.live ? ['default', 'force network'] : ['default', 'warm cache', 'force network']) {
      const tools = {};
      for (const tool of TOOL_NAMES) {
        const args = [tool === 'bun' ? 'info' : 'view', target.spec, ...fields, '--registry', fixture.registries[tool]];
        if (tool === 'bun') args.push('--cache-dir', fixture.environments.bun.BUN_INSTALL_CACHE_DIR);
        if (mode === 'warm cache' && tool !== 'tnpm') args.push('--prefer-offline');
        if (mode === 'force network') args.push('--no-cache');
        tools[tool] = spec(args, validate);
      }
      cases.push({ name: `view ${label} / ${mode}`, cache: mode, tools });
    }
  }
  return cases;
}
