// Compare incompatible registry cache formats with isolated, counted loopback HTTP.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { cpus } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { createFixture } from './lib/fixtures.mjs';
import { execute } from './lib/process.mjs';
import { summarize } from './lib/stats.mjs';

const directory = dirname(fileURLToPath(import.meta.url));
const { values } = parseArgs({ options: {
  before: { type: 'string' }, after: { type: 'string', default: join(directory, '../target/release/tnpm') },
  sizes: { type: 'string', default: '300,3000' }, samples: { type: 'string', default: '15' },
  output: { type: 'string' },
} });
if (!values.before) throw new Error('Usage: node benchmarks/cache.mjs --before=BASELINE_BINARY [--after=BINARY] [--output=REPORT.json]');
const sizes = values.sizes.split(',').map(Number), samples = Number(values.samples);
assert.ok(sizes.length && sizes.every(n => Number.isSafeInteger(n) && n >= 10 && n <= 10_000));
assert.ok(Number.isSafeInteger(samples) && samples >= 5 && samples <= 1000);
const binaries = { before: resolve(values.before), after: resolve(values.after) };
const packageName = '@bench/package';

async function cacheFiles(root) {
  const result = [];
  for (const entry of await readdir(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) result.push(...await cacheFiles(path));
    else result.push({ path, bytes: (await stat(path)).size });
  }
  return result;
}

const fixture = await createFixture({ live: true }); // Only isolation; no public request is made.
const requests = { before: { get: 0, notModified: 0 }, after: { get: 0, notModified: 0 } };
let body;
const server = createServer((request, response) => {
  const tool = request.url?.split('/')[1];
  if (!Object.hasOwn(requests, tool)) { response.writeHead(404); response.end(); return; }
  const conditional = request.url.includes('/revalidate/');
  const notModified = conditional && request.headers['if-none-match'] === '"fixture-v1"';
  requests[tool][notModified ? 'notModified' : 'get']++;
  response.writeHead(notModified ? 304 : 200, {
    'Content-Type': 'application/json', 'Cache-Control': conditional ? 'no-cache' : 'max-age=3600',
    ETag: '"fixture-v1"', ...(notModified ? {} : { 'Content-Length': body.length }),
  });
  response.end(notModified ? undefined : body);
});
try {
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  const home = { before: fixture.homes.bun, after: fixture.homes.tnpm };
  const envs = { before: fixture.environments.bun, after: fixture.environments.tnpm };
  const results = [];
  for (const size of sizes) {
    const latest = `1.0.${size - 1}`;
    const versions = {}, time = {};
    for (let i = 0; i < size; i++) {
      const version = `1.0.${i}`;
      versions[version] = { name: packageName, version,
        description: 'Synthetic repeated package metadata for a local cache benchmark. '.repeat(4),
        license: 'MIT', homepage: 'https://example.test/package',
        repository: { type: 'git', url: 'git+https://example.test/package.git' },
        maintainers: [{ name: 'fixture', email: 'fixture@example.test' }],
        engines: { node: '>=20' }, keywords: ['benchmark', 'fixture'],
        exports: { '.': { import: './dist/index.js', types: './dist/index.d.ts' } },
        dist: { tarball: `http://127.0.0.1:9/p-${version}.tgz`, integrity: `sha512-${'A'.repeat(86)}==`, unpackedSize: 123456 } };
      time[version] = '2026-10-25T00:00:00.000Z';
    }
    const packument = { name: packageName, 'dist-tags': { latest }, versions, time,
      readme: 'Shared top-level metadata; install should not need to restore it.\n'.repeat(2048) };
    body = Buffer.from(JSON.stringify(packument));
    const full = { ...packument, versions: Object.keys(versions), ...versions[latest] };
    await writeFile(join(fixture.cwd, 'package.json'), JSON.stringify({ name: 'cache-bench', dependencies: { [packageName]: '^1' } }));
    for (const tool of Object.keys(binaries)) await rm(join(home[tool], '.tnpm/cache'), { recursive: true, force: true });
    const cases = [
      { name: 'warm exact view', source: 'fresh-cache', selector: latest },
      { name: 'warm tag view', source: 'fresh-cache', selector: 'latest' },
      { name: 'warm narrow range view', source: 'fresh-cache', selector: `>=1.0.${size - 3}`,
        expected: [size - 3, size - 2, size - 1].map(n => `1.0.${n}`) },
      { name: 'warm full metadata view', source: 'fresh-cache', selector: latest, full: true },
      { name: 'offline install resolution', source: 'fresh-cache', install: true },
      { name: '304 revalidation', source: 'revalidated-cache', selector: latest, conditional: true },
      { name: 'cold cache write', source: 'network', selector: latest, cold: true },
    ];
    const invoke = async (tool, test) => {
      const registry = `http://127.0.0.1:${server.address().port}/${tool}/${test.conditional ? 'revalidate/' : ''}`;
      const args = test.install
        ? ['install', '--package-lock-only', '--no-save', '--offline', '--ignore-scripts', '--json']
        : ['view', `${packageName}@${test.selector}`, ...(test.full ? [] : ['version']), '--json'];
      const env = { ...envs[tool], npm_config_registry: registry, TNPM_PROFILE: '1', TNPM_PROFILE_TRANSPORT: 'default' };
      const start = process.hrtime.bigint();
      const output = await execute(binaries[tool], args, { cwd: fixture.cwd, env, timeout: 120_000 });
      const elapsed = Number(process.hrtime.bigint() - start);
      const profiles = output.stderr.toString().split('\n').filter(line => line.startsWith('tnpm-profile: '))
        .map(line => JSON.parse(line.slice('tnpm-profile: '.length))).filter(p => p.section === 'registry');
      if (!test.install) {
        assert.equal(profiles.length, 1);
        assert.equal(profiles[0].source, test.source);
      }
      assert.deepEqual(JSON.parse(output.stdout), test.install ? { locked: 1 } : test.full ? full : test.expected ?? latest);
      return { elapsed, profiles };
    };
    const warm = { name: 'populate', source: 'network', selector: latest };
    for (const tool of Object.keys(binaries)) await invoke(tool, warm);
    const footprint = {};
    for (const tool of Object.keys(binaries)) {
      const files = await cacheFiles(join(home[tool], '.tnpm/cache'));
      assert.equal(files.length, 1);
      footprint[tool] = { bytes: files[0].bytes, format: (await readFile(files[0].path)).readUInt32LE(8) };
    }
    console.log(`${size} versions: cache ${footprint.before.bytes} -> ${footprint.after.bytes} bytes`);
    for (const test of cases) {
      const timings = { before: [], after: [] }, profiles = { before: [], after: [] };
      for (let round = -2; round < samples; round++) {
        for (const tool of round % 2 === 0 ? ['before', 'after'] : ['after', 'before']) {
          if (test.cold) await rm(join(home[tool], '.tnpm/cache'), { recursive: true, force: true });
          const before = { ...requests[tool] };
          const firstConditional = test.conditional && round === -2;
          const output = await invoke(tool, firstConditional ? { ...test, source: 'network' } : test);
          assert.equal(requests[tool].get - before.get, test.cold || firstConditional ? 1 : 0);
          assert.equal(requests[tool].notModified - before.notModified, test.conditional && !firstConditional ? 1 : 0);
          if (round >= 0) { timings[tool].push(output.elapsed); profiles[tool].push(output.profiles); }
        }
      }
      const tools = Object.fromEntries(Object.entries(timings).map(([tool, times]) => [tool, { ...summarize(times), profiles: profiles[tool] }]));
      results.push({ versions: size, name: test.name, footprint, tools, speedup: tools.before.avg / tools.after.avg });
      console.log(`  ${test.name}: ${(tools.before.avg / 1e6).toFixed(3)} -> ${(tools.after.avg / 1e6).toFixed(3)} ms (${(tools.before.avg / tools.after.avg).toFixed(2)}x)`);
    }
  }
  const hashes = Object.fromEntries(await Promise.all(Object.entries(binaries).map(async ([tool, path]) =>
    [tool, createHash('sha256').update(await readFile(path)).digest('hex')])));
  const report = { metadata: { measuredAt: new Date().toISOString(), cpu: cpus()[0]?.model,
    platform: process.platform, arch: process.arch, node: process.version, binaries, hashes,
    samples, warmup: 2, unit: 'ns', requests, network: 'counted loopback HTTP only; warm cases 0 requests, 304 case one conditional request, cold case one full response per sample',
    timing: 'fresh subprocess wall time including profiling and output draining; alternating order; no outlier removal; setup, cache clearing and validation excluded',
    fixture: 'synthetic packument with repeated strings and top-level time/readme; single dependency, no tarball I/O; HOME/npmrc/cache isolated per binary; OS file cache warm' }, results };
  if (values.output) {
    await mkdir(dirname(resolve(values.output)), { recursive: true });
    await writeFile(resolve(values.output), `${JSON.stringify(report, null, 2)}\n`);
  }
} finally {
  server.closeAllConnections();
  await new Promise(resolve => server.close(resolve));
  await fixture.close();
}
