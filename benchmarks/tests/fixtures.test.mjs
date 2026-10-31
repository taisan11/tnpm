import assert from 'node:assert/strict';
import { readFile, stat } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { createFixture, PACKAGE, LATEST } from '../lib/fixtures.mjs';

test('fixture creates isolated homes, dependencies and a counted local registry', async () => {
  const fixture = await createFixture();
  try {
    const root = JSON.parse(await readFile(join(fixture.cwd, 'package.json'), 'utf8'));
    assert.equal(Object.keys(root.dependencies).length, 20);
    assert.notEqual(fixture.environments.bun.HOME, fixture.environments.tnpm.HOME);
    assert.ok(fixture.environments.bun.BUN_INSTALL_CACHE_DIR.startsWith(fixture.homes.bun));
    assert.equal(Object.keys(JSON.parse(await readFile(join(fixture.cwd, 'package-lock.json'), 'utf8')).packages).length, 101);
    const response = await fetch(`${fixture.registry}${encodeURIComponent(PACKAGE)}`, { headers: { 'User-Agent': 'tnpm/test' } });
    assert.equal(response.status, 200);
    assert.equal(response.headers.get('cache-control'), 'public, max-age=3600');
    const data = await response.json();
    assert.equal(Object.keys(data.versions).length, 300);
    assert.equal(data['dist-tags'].latest, LATEST);
    assert.equal(data.versions[LATEST].name, PACKAGE);
    assert.equal(fixture.requests.tnpm, 1);
    const missing = await fetch(`${fixture.registry}missing`);
    assert.equal(missing.status, 404);
    await missing.arrayBuffer();
    assert.equal(fixture.requests.tnpm, 2);
    const before = { ...fixture.requests };
    const bun = await fetch(`${fixture.registries.bun}${encodeURIComponent(PACKAGE)}`, { headers: { 'User-Agent': 'npm/test' } });
    assert.equal(bun.status, 200);
    await bun.arrayBuffer();
    assert.deepEqual(fixture.requests, { ...before, bun: before.bun + 1 }, 'endpoint must override a misleading user agent');
  } finally {
    await fixture.close();
  }
  await assert.rejects(stat(fixture.root), { code: 'ENOENT' });
});
