import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { installArgs, installedPackages, isolatedEnvironment, templateFiles, TEMPLATE } from '../lib/install-fixture.mjs';

test('offline benchmark installs allow lifecycle scripts', () => {
  for (const tool of ['bun', 'tnpm']) {
    const args = installArgs(tool, true);
    assert.ok(args.includes('--offline'));
    assert.ok(!args.includes('--ignore-scripts'));
    assert.equal(args[0], 'install');
    assert.ok(!args.includes('--no-cache'));
    assert.ok(!installArgs(tool).includes('--offline'));
  }
});

test('install environment isolates caches and removes settings that skew measurements', () => {
  const env = isolatedEnvironment('/fixture/home', { PATH: '/bin', HOME: '/user', npm_config_offline: 'true',
    npm_config_omit: 'dev', BUN_INSTALL_CACHE_DIR: '/user/cache', NODE_ENV: 'production', NODE_OPTIONS: '--inspect',
    TNPM_PROFILE: '1', HTTPS_PROXY: 'https://proxy', XDG_CONFIG_HOME: '/user/config' });
  assert.equal(env.HOME, '/fixture/home');
  assert.equal(env.PATH, '/bin');
  assert.equal(env.npm_config_offline, undefined);
  assert.equal(env.npm_config_omit, undefined);
  assert.equal(env.NODE_ENV, undefined);
  assert.equal(env.NODE_OPTIONS, undefined);
  assert.equal(env.TNPM_PROFILE, undefined);
  assert.equal(env.HTTPS_PROXY, undefined);
  assert.ok(env.BUN_INSTALL_CACHE_DIR.startsWith(env.HOME));
  assert.ok(env.npm_config_cache.startsWith(env.HOME));
  assert.notEqual(env.npm_config_userconfig, env.npm_config_globalconfig);
});

test('template snapshot accepts only confined files and rejects truncated trees or symlinks', () => {
  const file = path => ({ path: `${TEMPLATE}${path}`, type: 'blob', mode: '100644', sha: 'abc' });
  const files = templateFiles({ tree: [file('package.json'), file('src/index.ts'), { path: 'unrelated', type: 'blob' }] });
  assert.deepEqual(files.map(f => f.path), ['package.json', 'src/index.ts']);
  for (const path of ['../outside', '/absolute', 'src/../../escape', 'src\\escape']) {
    assert.throws(() => templateFiles({ tree: [file(path)] }));
  }
  assert.throws(() => templateFiles({ truncated: true, tree: [] }));
  assert.throws(() => templateFiles({ tree: [{ ...file('link'), mode: '120000' }] }));
});

test('installed package validator includes scoped and nested packages', async () => {
  const root = await mkdtemp(join(tmpdir(), 'tnpm-install-validator-'));
  try {
    for (const [path, name, version] of [['a', 'a', '1'], ['@s/tool', '@s/tool', '2'], ['a/node_modules/b', 'b', '3']]) {
      const dir = join(root, 'node_modules', path);
      await mkdir(dir, { recursive: true });
      await writeFile(join(dir, 'package.json'), JSON.stringify({ name, version }));
    }
    assert.deepEqual(await installedPackages(root), ['@s/tool@2', 'a@1', 'b@3']);
  } finally { await rm(root, { recursive: true, force: true }); }
});
