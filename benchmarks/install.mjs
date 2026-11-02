import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { cpus, release } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { TOOLS } from './lib/fixtures.mjs';
import { execute } from './lib/process.mjs';
import { createInstallFixture, installArgs, installedPackages, SOURCE } from './lib/install-fixture.mjs';
import { summarize } from './lib/stats.mjs';

const directory = dirname(fileURLToPath(import.meta.url));
const { values } = parseArgs({ options: {
  samples: { type: 'string', default: '10' }, warmup: { type: 'string', default: '2' },
  ref: { type: 'string', default: 'main' }, tnpm: { type: 'string', default: join(directory, '../target/release/tnpm') },
  output: { type: 'string', default: join(directory, 'results/install-tnpm-bun-scripts') }, help: { type: 'boolean' },
} });
if (values.help) {
  console.log('node benchmarks/install.mjs [--samples=10] [--warmup=2] [--ref=COMMIT] [--tnpm=PATH] [--output=PREFIX]\nLifecycle scripts follow each tool’s policy. Warm cases are offline; the paired cold case uses the network.');
  process.exit(0);
}
if (process.platform === 'win32') throw new Error('This benchmark currently requires macOS/Linux for HOME isolation.');
const samples = Number(values.samples), warmup = Number(values.warmup);
if (!Number.isSafeInteger(samples) || samples < 5 || samples > 1000) throw new Error('--samples must be 5..1000');
if (!Number.isSafeInteger(warmup) || warmup < 2 || warmup > 100) throw new Error('--warmup must be 2..100');
const binaries = { npm: 'npm', bun: 'bun', tnpm: resolve(values.tnpm) };
const versions = Object.fromEntries(await Promise.all(TOOLS.map(async tool => [tool, (await execute(binaries[tool], ['--version'])).stdout.toString().trim()])));
console.log(`Template: ${SOURCE}\nPreparing canonical lockfile and isolated caches (outside timing)...`);
const fixture = await createInstallFixture(binaries, values.ref);
try {
  const install = (tool, offline) => execute(binaries[tool], installArgs(tool, offline),
    { cwd: fixture.projects[tool], env: fixture.environments[tool], timeout: 180_000 });
  const clean = tool => rm(join(fixture.projects[tool], 'node_modules'), { recursive: true, force: true });
  let expected;
  const validate = async tool => {
    assert.deepEqual(JSON.parse(await readFile(join(fixture.projects[tool], 'package.json'), 'utf8')), fixture.manifest, `${tool} changed package.json`);
    const installed = await installedPackages(fixture.projects[tool]);
    assert.ok(installed.some(s => s.startsWith('hono@')) && installed.some(s => s.startsWith('wrangler@')));
    if (!expected) expected = installed;
    assert.deepEqual(installed, expected, `${tool} installed a different package/version multiset`);
    // Import the actual library, not just its manifest; lifecycle scripts run during install.
    await execute(process.execPath, ['--input-type=module', '-e', 'import { Hono } from "hono"; if (typeof new Hono().fetch !== "function") process.exit(1)'],
      { cwd: fixture.projects[tool], env: fixture.environments[tool] });
  };
  const cases = [
    { name: 'clean install / warm cache', clean: true },
    { name: 'clean install / cold Bun + tnpm caches', clean: true, coldInstallCaches: true },
    { name: 'no-op install / warm cache', clean: false },
  ];
  const prepare = async (tool, test) => {
    if (test.coldInstallCaches && tool === 'bun') {
      await rm(fixture.environments.bun.BUN_INSTALL_CACHE_DIR, { recursive: true, force: true });
    }
    if (test.coldInstallCaches && tool === 'tnpm') {
      await rm(join(fixture.environments.tnpm.HOME, '.tnpm/cache'), { recursive: true, force: true });
    }
  };
  const results = [];
  for (const [caseIndex, test] of cases.entries()) {
    console.log(`\n${test.name}: ${warmup} warmups per CLI`);
    for (let round = 0; round < warmup; round++) {
      for (let n = 0; n < TOOLS.length; n++) {
        const tool = TOOLS[(round + n + caseIndex) % TOOLS.length];
        if (test.clean) await clean(tool);
        await prepare(tool, test);
        // First warmup may use the network; all subsequent warmups must work offline.
        await install(tool, !(test.coldInstallCaches && ['bun', 'tnpm'].includes(tool)) && (round > 0 || caseIndex > 0));
        await validate(tool);
        console.log(`  ${tool}: warmup ${round + 1}/${warmup} passed`);
      }
    }
    const timings = Object.fromEntries(TOOLS.map(t => [t, []]));
    for (let round = 0; round < samples; round++) {
      for (let n = 0; n < TOOLS.length; n++) {
        const tool = TOOLS[(round + n + caseIndex) % TOOLS.length];
        if (test.clean) await clean(tool); // Deliberately outside the measured interval.
        await prepare(tool, test);
        const start = process.hrtime.bigint();
        await install(tool, !(test.coldInstallCaches && ['bun', 'tnpm'].includes(tool)));
        timings[tool].push(Number(process.hrtime.bigint() - start));
        await validate(tool); // Deliberately outside the measured interval.
      }
    }
    const tools = Object.fromEntries(TOOLS.map(tool => [tool, { args: installArgs(tool, !(test.coldInstallCaches && ['bun', 'tnpm'].includes(tool))), ...summarize(timings[tool]) }]));
    results.push({ name: test.name, tools });
    for (const tool of TOOLS) console.log(`  ${tool}: avg ${(tools[tool].avg / 1e6).toFixed(3)} ms; p50 ${(tools[tool].p50 / 1e6).toFixed(3)} ms`);
  }
  const metadata = { measuredAt: new Date().toISOString(), source: SOURCE, commit: fixture.commit, files: fixture.files,
    manifest: fixture.manifest, manifestSha256: fixture.manifestSha256, canonicalLockSha256: fixture.lockSha256,
    canonicalLock: fixture.lock, installedPackages: expected, samples, warmup, concurrency: 1, unit: 'ns',
    cpu: cpus()[0]?.model, platform: process.platform, arch: process.arch, osRelease: release(), node: process.version, versions,
    tnpmBinary: binaries.tnpm, tnpmBinarySha256: createHash('sha256').update(await readFile(binaries.tnpm)).digest('hex'),
    scripts: 'enabled under each tool’s default policy; Bun blocks untrusted dependency scripts', network: 'measurement is offline except the paired cold Bun/tnpm cache case; requests not externally counted',
    timing: 'fresh subprocess wall time including startup and output draining; cleanup, validation, setup and warmup excluded',
    cache: 'separate temporary HOME and package cache per tool; paired cold Bun/tnpm case clears both install caches and allows network; OS file cache warm',
    lockfiles: 'npm-generated v3 graph used by npm and Bun; tnpm resolves independently during warmup; installed versions are validated; lockfiles retained' };
  const ms = ns => (ns / 1e6).toFixed(3);
  const markdown = [ '# Cloudflare Workers template — warm-cache install benchmark', '',
    `Measured: ${metadata.measuredAt}`, '', `- Template: ${SOURCE}`, `- Commit: \`${fixture.commit}\``,
    `- ${metadata.cpu}; ${metadata.platform}/${metadata.arch}; Node ${metadata.node}`,
    `- ${TOOLS.map(tool => `${tool} ${versions[tool]}${tool === 'tnpm' ? ' (release)' : ''}`).join('; ')}`,
    `- ${samples} samples and ${warmup} warmups per CLI/case; concurrency 1; order rotated each round; no outlier removal.`,
    `- Identical installed package/version multisets validated: ${expected.length} packages. Native layouts may differ.`,
    '- Each CLI has an isolated cache. The warm case uses --offline for measured installs; the paired cold Bun/tnpm case clears each install cache before invocation and allows registry access. Script network access is not restricted; HTTP is not externally counted.',
    '- A single npm v3 dependency graph is shared. Bun and tnpm import it into their native locks during untimed warmup. Locks and caches remain in place.',
    '- Clean install: remove only node_modules before every invocation, outside timing. No-op: retain node_modules.',
    '- The paired cold Bun/tnpm case clears both install caches; network transfer is included for both.',
    '- Setup, template fetching, cache warming, cleanup, package validation and Hono import checks are outside timing.',
    '- End-to-end subprocess time includes startup and stdout/stderr draining. OS file caches are warm.', '',
    '| Case | Tool | Mean (ms) | p50 (ms) | p99 (ms) | Min (ms) | Max (ms) |',
    '| --- | --- | ---: | ---: | ---: | ---: | ---: |',
    ...results.flatMap(row => TOOLS.map(tool => { const t = row.tools[tool]; return `| ${row.name} | ${tool} | ${[t.avg, t.p50, t.p99, t.min, t.max].map(ms).join(' | ')} |`; })), '',
    '## Commands', '', ...results.flatMap(row => TOOLS.map(tool => `- ${row.name}: \`${tool} ${row.tools[tool].args.join(' ')}\``)), '',
    '## Caveats', '', '- Bun and tnpm both reuse extracted package caches, but cache formats, validation, and clone/link implementations differ; these are real implementation differences, not identical I/O strategies.',
    '- Lifecycle scripts follow each tool’s default policy. Bun runs dependency scripts only for trusted packages, so the work performed is not guaranteed to be identical.',
    '- The source manifest is unchanged. The resolved graph, source commit, raw samples and binary checksum are saved in the JSON report.',
    '- This is one machine/run, not a claim of complete npm compatibility or cold-cache/network performance.', '' ].join('\n');
  const output = resolve(values.output);
  await mkdir(dirname(output), { recursive: true });
  await writeFile(`${output}.json`, `${JSON.stringify({ metadata, results }, null, 2)}\n`);
  await writeFile(`${output}.md`, markdown);
  console.log(`\nReports: ${output}.md and ${output}.json`);
} finally { await fixture.close(); }
