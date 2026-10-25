// Offline, end-to-end comparison of two tnpm binaries on identical lockfiles.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { cpus, tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { execute } from './lib/process.mjs';
import { summarize } from './lib/stats.mjs';

const directory = dirname(fileURLToPath(import.meta.url));
const { values } = parseArgs({ options: {
  before: { type: 'string' }, after: { type: 'string', default: join(directory, '../target/release/tnpm') },
  sizes: { type: 'string', default: '100,1000,5000' }, samples: { type: 'string', default: '15' },
  mode: { type: 'string', default: 'lock' }, output: { type: 'string' },
} });
if (!values.before) throw new Error('Usage: node benchmarks/graph.mjs --before=BASELINE_BINARY [--after=BINARY] [--output=REPORT.json]');
const sizes = values.sizes.split(',').map(Number), samples = Number(values.samples);
assert.ok(sizes.length && sizes.every(n => Number.isSafeInteger(n) && n >= 32 && n <= 40_000));
assert.ok(Number.isSafeInteger(samples) && samples >= 5 && samples <= 1000);
assert.ok(['lock', 'noop'].includes(values.mode), '--mode must be lock or noop');
const binaries = { before: resolve(values.before), after: resolve(values.after) };
const parent = join(tmpdir(), 'opencode');
await mkdir(parent, { recursive: true });
const root = await mkdtemp(join(parent, 'tnpm-graph-'));
try {
  const home = join(root, 'home');
  await mkdir(home);
  await writeFile(join(home, 'user.npmrc'), '');
  await writeFile(join(home, 'global.npmrc'), '');
  // Do not inherit personal npmrc, tokens, proxies, or profiling configuration.
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) =>
    !/^(npm_config_|tnpm_|bun_|xdg_|https?_proxy$|all_proxy$|no_proxy$)/i.test(key)));
  Object.assign(env, { HOME: home, XDG_CONFIG_HOME: home, NO_COLOR: '1',
    npm_config_userconfig: join(home, 'user.npmrc'), npm_config_globalconfig: join(home, 'global.npmrc') });
  const args = ['install', '--offline', '--ignore-scripts', '--json',
    ...(values.mode === 'lock' ? ['--package-lock-only'] : [])];
  const results = [];
  for (const size of sizes) {
    const cwd = join(root, String(size));
    await mkdir(cwd);
    const name = n => `@bench/p${n % size}`;
    const manifest = { name: 'graph-bench', version: '1.0.0', dependencies: { [name(0)]: '^1.0.0' },
      devDependencies: { [name(size / 2 | 0)]: '^1.0.0' }, optionalDependencies: { [name(size - 1)]: '^1.0.0' } };
    const packages = { '': manifest };
    for (let i = 0; i < size; i++) {
      packages[`node_modules/${name(i)}`] = { version: '1.0.0', license: 'MIT',
        resolved: `http://127.0.0.1:9/p${i}.tgz`,
        dependencies: Object.fromEntries([1, 7, 31, 127].map(step => [name(i + step), '^1.0.0'])) };
    }
    const input = JSON.stringify({ lockfileVersion: 3, packages });
    await writeFile(join(cwd, 'package.json'), JSON.stringify(manifest));
    if (values.mode === 'noop') {
      for (const [path, entry] of Object.entries(packages)) {
        if (!path) continue;
        await mkdir(join(cwd, path), { recursive: true });
        await writeFile(join(cwd, path, 'package.json'), JSON.stringify(entry));
      }
    }
    const timings = { before: [], after: [] };
    let expected;
    for (let round = -2; round < samples; round++) {
      for (const tool of round % 2 === 0 ? ['before', 'after'] : ['after', 'before']) {
        await writeFile(join(cwd, 'package-lock.json'), input); // Outside timing.
        const start = process.hrtime.bigint();
        const output = await execute(binaries[tool], args, { cwd, env, timeout: 120_000 });
        const elapsed = Number(process.hrtime.bigint() - start);
        if (round >= 0) timings[tool].push(elapsed);
        assert.equal(JSON.parse(output.stdout)[values.mode === 'lock' ? 'locked' : 'installed'],
          values.mode === 'lock' ? size : 0);
        const lock = JSON.parse(await readFile(join(cwd, 'package-lock.json'), 'utf8'));
        expected ??= lock;
        assert.deepEqual(lock, expected, `${tool} changed graph/flags for size ${size}`);
        if (values.mode === 'noop') {
          const hidden = JSON.parse(await readFile(join(cwd, 'node_modules/.package-lock.json'), 'utf8'));
          const { '': ignored, ...installed } = lock.packages;
          assert.deepEqual(hidden.packages, installed);
        }
        assert.deepEqual(JSON.parse(await readFile(join(cwd, 'package.json'), 'utf8')), manifest);
      }
    }
    const tools = Object.fromEntries(Object.entries(timings).map(([tool, times]) => [tool, summarize(times)]));
    results.push({ packages: size, tools, speedup: tools.before.avg / tools.after.avg,
      outputSha256: createHash('sha256').update(JSON.stringify(expected)).digest('hex') });
    console.log(`${size} packages: before ${(tools.before.avg / 1e6).toFixed(3)} ms; after ${(tools.after.avg / 1e6).toFixed(3)} ms; ${(tools.before.avg / tools.after.avg).toFixed(2)}x`);
  }
  const hashes = Object.fromEntries(await Promise.all(Object.entries(binaries).map(async ([tool, path]) =>
    [tool, createHash('sha256').update(await readFile(path)).digest('hex')])));
  const report = { metadata: { measuredAt: new Date().toISOString(), cpu: cpus()[0]?.model,
    platform: process.platform, arch: process.arch, node: process.version, binaries, hashes, args,
    mode: values.mode, samples, warmup: 2, unit: 'ns', network: 'offline; no downloads or extraction',
    timing: 'subprocess wall time including JSON parsing, graph traversal, lockfile serialization and output; fixture reset and validation excluded; alternating binary order; no outlier removal',
    fixture: 'synthetic scoped cyclic graph, up to four edges/package, production/dev/optional roots; all output lockfiles and manifests checked for equality' }, results };
  if (values.output) {
    await mkdir(dirname(resolve(values.output)), { recursive: true });
    await writeFile(resolve(values.output), `${JSON.stringify(report, null, 2)}\n`);
  }
} finally {
  await rm(root, { recursive: true, force: true });
}
