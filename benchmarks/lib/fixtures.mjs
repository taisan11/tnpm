import { execute } from './process.mjs';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

export const PACKAGE = '@bench/package';
export const LATEST = '1.0.299';
export const TOOLS = ['npm', 'tnpm', 'bun'];

export function packument() {
  const versions = {};
  const time = {};
  for (let i = 0; i < 300; i++) {
    const version = `1.0.${i}`;
    time[version] = '2026-10-21T00:00:00.000Z';
    versions[version] = {
      name: PACKAGE,
      version,
      _id: `${PACKAGE}@${version}`,
      description: 'Fixed synthetic package for npm/tnpm CLI benchmarks',
      license: 'MIT',
      homepage: 'https://example.test/bench',
      keywords: ['benchmark', 'fixture'],
      dependencies: Object.fromEntries(Array.from({ length: 6 }, (_, n) => [`dep-${n}`, '^1.0.0'])),
      maintainers: [{ name: 'bench', email: 'bench@example.test' }],
      _npmUser: { name: 'bench', email: 'bench@example.test' },
      dist: {
        tarball: `https://example.test/package-${version}.tgz`,
        shasum: '0'.repeat(40),
        integrity: `sha512-${'A'.repeat(86)}==`,
        unpackedSize: 132300,
      },
      exports: { '.': { import: './dist/index.js', types: './dist/index.d.ts' } },
      engines: { node: '>=20' },
    };
  }
  return {
    _id: PACKAGE,
    name: PACKAGE,
    'dist-tags': { latest: LATEST, previous: '1.0.298' },
    versions,
    time,
    maintainers: [{ name: 'bench', email: 'bench@example.test' }],
    readme: 'Synthetic registry fixture. No production package or dependency is downloaded.',
  };
}

export async function createFixture({ live = false } = {}) {
  const parent = join(tmpdir(), 'opencode');
  await mkdir(parent, { recursive: true });
  const root = await mkdtemp(join(parent, 'tnpm-bench-'));
  let server;
  try {
    const cwd = join(root, 'project');
    const homes = Object.fromEntries(TOOLS.map(tool => [tool, join(root, tool)]));
    const writeJson = async (path, value) => {
      await mkdir(dirname(path), { recursive: true });
      await writeFile(path, JSON.stringify(value));
    };
    if (!live) {
      const dependencies = {};
      const packages = {};
      for (let direct = 0; direct < 20; direct++) {
        const name = `@bench/root-${direct.toString().padStart(2, '0')}`;
        dependencies[name] = '^1.0.0';
        const children = {};
        for (let leaf = 0; leaf < 4; leaf++) {
          const child = `@bench/leaf-${direct.toString().padStart(2, '0')}-${leaf}`;
          children[child] = '^1.0.0';
          const manifest = {
            name: child, version: '1.0.0',
            ...(leaf === 0 ? { funding: { type: 'individual', url: `https://example.test/fund/${direct}` } } : {}),
          };
          await writeJson(join(cwd, 'node_modules', child, 'package.json'), manifest);
          packages[`node_modules/${child}`] = { ...manifest, resolved: `https://example.test/${encodeURIComponent(child)}-1.0.0.tgz`, integrity: `sha512-${'A'.repeat(86)}==` };
        }
        const manifest = { name, version: '1.0.0', dependencies: children };
        await writeJson(join(cwd, 'node_modules', name, 'package.json'), manifest);
        packages[`node_modules/${name}`] = { ...manifest, resolved: `https://example.test/${encodeURIComponent(name)}-1.0.0.tgz`, integrity: `sha512-${'A'.repeat(86)}==` };
      }
      await writeJson(join(cwd, 'package.json'), { name: 'bench-app', version: '1.0.0', private: true, dependencies });
      await writeJson(join(cwd, 'package-lock.json'), {
        name: 'bench-app', version: '1.0.0', lockfileVersion: 3, requires: true,
        packages: { '': { name: 'bench-app', version: '1.0.0', dependencies }, ...packages },
      });
    } else {
      await writeJson(join(cwd, 'package.json'), { name: 'bench-live', version: '1.0.0', private: true });
    }
    for (const home of Object.values(homes)) {
      await mkdir(home, { recursive: true });
      await writeFile(join(home, 'user.npmrc'), '');
      await writeFile(join(home, 'global.npmrc'), '');
    }
    const body = live ? null : Buffer.from(JSON.stringify(packument()));
    const requests = live ? null : Object.fromEntries(TOOLS.map(tool => [tool, 0]));
    server = live ? null : createServer((request, response) => {
      let name;
      try { name = decodeURIComponent((request.url ?? '').slice(1)); } catch { name = ''; }
      const prefix = name.split('/')[0];
      const agent = request.headers['user-agent']?.toLowerCase() ?? '';
      const tool = TOOLS.includes(prefix) ? prefix : agent.includes('tnpm/') ? 'tnpm' : agent.includes('bun/') ? 'bun' : 'npm';
      if (TOOLS.includes(prefix)) name = name.slice(prefix.length + 1);
      requests[tool]++;
      if (name !== PACKAGE) {
        response.writeHead(404, { 'Content-Type': 'application/json' });
        response.end('{"error":"not found"}');
        return;
      }
      // Always 200/full payload, even for conditional or forced-online requests.
      response.writeHead(200, {
        'Content-Type': 'application/json',
        'Content-Length': body.length,
        'Cache-Control': 'public, max-age=3600',
      });
      response.end(body);
    });
    if (server) {
      await new Promise((resolve, reject) => {
        server.once('error', reject);
        server.listen(0, '127.0.0.1', resolve);
      });
    }
    const registry = live ? 'https://registry.npmjs.org/' : `http://127.0.0.1:${server.address().port}/`;
    const registries = Object.fromEntries(TOOLS.map(tool => [tool, live ? registry : `${registry}${tool}/`]));
    const environments = Object.fromEntries(Object.entries(homes).map(([tool, home]) => {
      const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => !/^(npm_|bun_)/i.test(name) && !/^(node_options|node_extra_ca_certs)$/i.test(name) && !/(^|_)proxy$/i.test(name)));
      return [tool, {
        ...env, HOME: home, USERPROFILE: home, NO_COLOR: '1', FORCE_COLOR: '0',
        NO_PROXY: '127.0.0.1,localhost',
        XDG_CONFIG_HOME: join(home, '.config'), XDG_CACHE_HOME: join(home, '.cache'),
        BUN_INSTALL: join(home, '.bun'), BUN_INSTALL_CACHE_DIR: join(home, '.bun/install/cache'),
        npm_config_userconfig: join(home, 'user.npmrc'),
        npm_config_globalconfig: join(home, 'global.npmrc'),
        npm_config_cache: join(home, '.npm'),
        npm_config_registry: registries[tool],
        npm_config_update_notifier: 'false',
        npm_config_audit: 'false',
        npm_config_fund: 'false',
      }];
    }));
    return {
      root, cwd, homes, registry, registries, environments, requests, live,
      packageBytes: body?.length ?? null,
      async close() {
        if (server) {
          server.closeAllConnections();
          await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
        }
        await rm(root, { recursive: true, force: true });
      },
    };
  } catch (error) {
    server?.close();
    await rm(root, { recursive: true, force: true });
    throw error;
  }
}

export async function prepareBunFixture(fixture, binary = 'bun') {
  const before = fixture.requests.bun;
  await execute(binary, ['pm', 'migrate'], { cwd: fixture.cwd, env: fixture.environments.bun });
  if (fixture.requests.bun !== before) throw new Error('Bun fixture migration unexpectedly contacted the registry');
}
