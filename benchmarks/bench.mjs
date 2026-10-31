import assert from 'node:assert/strict';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { cpus, totalmem, release } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { measure } from 'mitata';
import { createFixture, prepareBunFixture } from './lib/fixtures.mjs';
import { execute } from './lib/process.mjs';
import { buildCases, parseTools } from './lib/cases.mjs';
import { requestDelta, resolveLiveTarget } from './lib/live.mjs';

const directory = dirname(fileURLToPath(import.meta.url));
const { values } = parseArgs({ options: {
  samples: { type: 'string' },
  live: { type: 'boolean', default: false },
  filter: { type: 'string', default: '' },
  tnpm: { type: 'string', default: join(directory, '../target/release/tnpm') },
  tools: { type: 'string', default: 'tnpm,bun' },
  output: { type: 'string' },
  help: { type: 'boolean', default: false },
} });
if (values.help) {
  console.log('node bench.mjs [--live] [--tools=tnpm,bun] [--samples=N] [--filter=REGEX] [--tnpm=PATH] [--output=PREFIX]\n--live: benchmark hono against https://registry.npmjs.org/ (default: 5 samples; local: 30)');
  process.exit(0);
}
if (process.platform === 'win32') throw new Error('This benchmark isolates dirs::home_dir through HOME and currently requires macOS/Linux.');
const samples = Number(values.samples ?? (values.live ? '5' : '30'));
if (!Number.isSafeInteger(samples) || samples < 5 || samples > 10_000) throw new Error('--samples must be an integer between 5 and 10000');
const filter = new RegExp(values.filter);
const selectedTools = parseTools(values.tools);
const binaries = { tnpm: resolve(values.tnpm), bun: 'bun' };
const fixture = await createFixture({ live: values.live });

try {
  const invoke = (tool, args) => execute(binaries[tool], args, { cwd: fixture.cwd, env: fixture.environments[tool] });
  const tools = Object.fromEntries(await Promise.all(selectedTools.map(async tool => [tool, (await invoke(tool, ['--version'])).stdout.toString().trim()])));
  if (selectedTools.includes('bun') && !fixture.live) await prepareBunFixture(fixture, binaries.bun);
  const target = fixture.live ? await resolveLiveTarget(invoke, selectedTools[0], fixture) : undefined;
  if (target) console.log(`Live registry: ${fixture.registry}; package: ${target.spec} (resolved outside timing)`);
  const mitata = JSON.parse(await readFile(join(directory, 'node_modules/mitata/package.json'), 'utf8')).version;
  const metadata = {
    measuredAt: new Date().toISOString(),
    platform: process.platform, arch: process.arch, osRelease: release(),
    cpu: cpus()[0]?.model, logicalCpus: cpus().length, memoryBytes: totalmem(),
    node: process.version, mitata, ...tools, selectedTools,
    tnpmBinary: binaries.tnpm, build: 'cargo build --release (must be built before running)',
    samplesRequested: samples, concurrency: 1,
    mode: fixture.live ? 'live' : 'local',
    fixture: fixture.live
      ? { registry: fixture.registry, package: target, httpRequestsObservable: false, npmLockfile: false, bunLockfile: false }
      : { directDependencies: 20, totalDependencies: 100, registryVersions: 300, registryBytes: fixture.packageBytes, npmLockfile: true, bunLockfile: selectedTools.includes('bun'), httpRequestsObservable: true },
    timing: 'Subprocess wall time, including process startup, CLI work and draining stdout/stderr; no shell. Setup/build/cache warmup excluded.',
  };
  const cases = buildCases(fixture, target).filter(test => filter.test(test.name));
  if (!cases.length) throw new Error('No benchmark matches --filter');
  const results = [];
  for (const [index, test] of cases.entries()) {
    const pair = { name: test.name, tools: {} };
    // Rotate the starting tool so there is no fixed first/last CLI.
    const start = index % selectedTools.length;
    const order = [...selectedTools.slice(start), ...selectedTools.slice(0, start)];
    for (const tool of order) {
      const command = test.tools[tool];
      if (command.skipped) {
        pair.tools[tool] = command;
        console.log(`${test.name}: ${tool} SKIP — ${command.reason}`);
        continue;
      }
      const { args } = command;
      const call = () => invoke(tool, args);
      // Validate and warm up outside timing. Errors abort rather than becoming fast samples.
      let warmupRequests = null;
      for (let n = 0; n < 2; n++) {
        const before = fixture.requests?.[tool] ?? null;
        const output = (await call()).stdout.toString().trim();
        command.validate(output);
        if (n === 1) warmupRequests = requestDelta(before, fixture.requests?.[tool] ?? null);
      }
      if (tool === 'bun' && test.cache === 'warm cache' && warmupRequests > 0) {
        const reason = `Bun ${tools.bun} info --prefer-offline still made ${warmupRequests} HTTP request(s) on the second warmup; not a zero-network cache comparison.`;
        pair.tools[tool] = { skipped: true, reason, args, probeHttpRequests: warmupRequests };
        console.log(`${test.name}: ${tool} SKIP — ${reason}`);
        continue;
      }
      const before = fixture.requests?.[tool] ?? null;
      let invocations = 0;
      const stats = await measure(async () => { invocations++; await call(); }, {
        gc: false, concurrency: 1,
        min_samples: samples, max_samples: samples, min_cpu_time: 0,
        batch_threshold: 0, warmup_threshold: 0,
        samples_threshold: Number.MAX_SAFE_INTEGER,
      });
      const httpRequests = requestDelta(before, fixture.requests?.[tool] ?? null);
      if (test.cache === 'warm cache' && httpRequests !== null) assert.equal(httpRequests, 0, `${tool}: warm-cache benchmark made HTTP requests`);
      if (test.cache === 'force network' && httpRequests !== null) assert.equal(httpRequests, invocations, `${tool}: force-network benchmark did not fetch for every invocation`);
      const { debug, ...measurements } = stats;
      pair.tools[tool] = { args, ...measurements, invocations, httpRequests };
      console.log(`${test.name}: ${tool} ${(stats.avg / 1e6).toFixed(3)} ms (${stats.samples.length} samples, HTTP ${httpRequests === null ? 'unobserved' : `${httpRequests}/${invocations}`})`);
    }
    if (pair.tools.tnpm && !pair.tools.tnpm.skipped) {
      pair.relativeToTnpm = Object.fromEntries(selectedTools.filter(tool => tool !== 'tnpm' && !pair.tools[tool].skipped).map(tool => [tool, pair.tools[tool].avg / pair.tools.tnpm.avg]));
    }
    results.push(pair);
  }
  const report = { metadata, results };
  const ms = ns => (ns / 1e6).toFixed(3);
  const ratioTools = selectedTools.includes('tnpm') ? selectedTools.filter(tool => tool !== 'tnpm') : [];
  const headings = ['Case', ...selectedTools.map(tool => `${tool} avg (ms)`), ...ratioTools.map(tool => `${tool} / tnpm`)];
  const commandText = (tool, args) => [tool, ...args].map(arg => arg.replace(fixture.root, '$FIXTURE').replace(fixture.registry, '$REGISTRY/')).join(' ');
  const skipped = results.flatMap(row => selectedTools.filter(tool => row.tools[tool].skipped).map(tool => `- **${row.name} / ${tool}**: ${row.tools[tool].reason}`));
  const markdown = [
    `# ${selectedTools.join(' vs ')} — Mitata CLI benchmark${fixture.live ? ' (live hono)' : ''}`, '',
    `Measured: ${metadata.measuredAt}`, '',
    `- ${metadata.cpu}; ${metadata.platform}/${metadata.arch} ${metadata.osRelease}`,
    `- Node ${metadata.node}; ${selectedTools.map(tool => `${tool} ${tools[tool]}${tool === 'tnpm' ? ' (release)' : ''}`).join('; ')}; Mitata ${mitata}`,
    `- ${samples} samples per CLI/case, concurrency 1, no sample trimming. Each sample starts a new CLI process.`,
    ...(fixture.live ? [
      `- Live registry: ${fixture.registry}; package: ${target.spec}. Version resolved once by ${target.resolvedBy} before timing.`,
      '- Requests target the public HTTPS registry with no benchmark-inserted server/proxy. OS system proxy settings may apply. DNS/TCP/TLS/CDN/network latency and CLI startup are included.',
      '- Default mode may use each CLI\'s warmed cache. Force-network mode uses --no-cache.',
      '- HTTP counts, status codes, compressed byte counts, and actual cache hits are not instrumented: N/A means unknown, NOT zero requests.',
    ] : [
      '- Fixture: 20 direct / 100 total installed packages; 300 registry versions served by a loopback HTTP server.',
      `- Registry response: ${fixture.packageBytes.toLocaleString('en-US')} bytes; HTTP 200 with max-age=3600; no TLS or internet latency.`,
    ]),
    '- Separate temporary HOME/cache/config for each CLI; user npmrc/tokens/proxy environment variables/Bun environment are not inherited; OS system proxy settings may apply. Build/setup/warmup are outside timing.',
    ...(fixture.live ? [] : ['- Fixture includes a generated npm lockfile; when Bun is selected, bun pm migrate generates bun.lock outside timing without HTTP requests.']), '',
    `| ${headings.join(' | ')} |`,
    `| --- | ${headings.slice(1).map(() => '---:').join(' | ')} |`,
    ...results.map(row => {
      const averages = selectedTools.map(tool => row.tools[tool].skipped ? 'N/A' : ms(row.tools[tool].avg));
      const ratios = ratioTools.map(tool => row.relativeToTnpm?.[tool] === undefined ? 'N/A' : `${row.relativeToTnpm[tool].toFixed(2)}x`);
      return `| ${[row.name, ...averages, ...ratios].join(' | ')} |`;
    }), '',
    'Ratios are the other tool\'s time divided by tnpm\'s time. Above 1 means tnpm is faster; below 1 means the other tool is faster.', '',
    '## Commands and detailed timings', '',
    '| Case | Tool | Command | p50 (ms) | p99 (ms) | HTTP / invocations* |',
    '| --- | --- | --- | ---: | ---: | ---: |',
    ...results.flatMap(row => selectedTools.filter(tool => !row.tools[tool].skipped).map(tool => {
      const stats = row.tools[tool];
      return `| ${row.name} | ${tool} | \`${commandText(tool, stats.args)}\` | ${ms(stats.p50)} | ${ms(stats.p99)} | ${stats.httpRequests === null ? `N/A / ${stats.invocations}` : `${stats.httpRequests}/${stats.invocations}`} |`;
    })), '',
    fixture.live ? '*Live HTTP counts are unobserved; invocation counts include Mitata calibration calls but exclude explicit warmup.' : '*HTTP counts include Mitata calibration calls as well as measured invocations, but exclude explicit warmup.', '',
    ...(skipped.length ? ['## Not comparable / skipped', '', ...skipped, ''] : []),
    '## Interpretation', '',
    '- These are end-to-end CLI timings, not isolated parser/cache throughput. Startup/runtime overhead is included.',
    ...(fixture.live ? [
      '- Live mode does not offer a zero-network warm-cache comparison because actual HTTP requests cannot be counted here.',
      '- Force-network flags request fresh registry information; payloads/compression and cache behavior can differ across CLIs.',
      '- The same published version is pinned for all CLIs. Hono package contents/scripts are never installed or executed.',
      '- Live measurements depend on network/CDN conditions and execution order; do not mix them with the deterministic loopback results.',
    ] : [
      '- Warm cache: Bun is offered --prefer-offline; tnpm uses its default fresh binary cache. Only zero-network runs are timed; Bun is explicitly skipped if the second warmup still fetches.',
      '- Force network: tnpm/Bun use --no-cache. Every invocation must fetch the full fixture.',
    ]),
    fixture.live ? '- Default view timings may benefit from cached data; actual cache hits and request counts are unknown in live mode.' : '- Default view behavior is reported separately. HTTP counts reveal any differences in cache policy.',
    '- Bun maps view to bun info.',
    ...(fixture.live ? [] : [
      '- Bun text listings map to bun pm ls and are sourced from bun.lock; tnpm inspects installed dependencies. This is not identical work.',
      '- Bun listing JSON, funding and JSON version/config-report cases have no equivalent here and are not timed.',
    ]),
    '- Output formats and supported functionality are not identical (notably fund/version/full metadata); timings do not imply full npm compatibility.',
    '- OS filesystem caches are warm. This is one run, not a general claim about production registry latency or cold machine performance.', '',
  ].join('\n');
  const output = resolve(values.output ?? join(directory, fixture.live ? 'results/live-tnpm-bun' : 'results/tnpm-bun'));
  await mkdir(dirname(output), { recursive: true });
  await writeFile(`${output}.json`, `${JSON.stringify(report, null, 2)}\n`);
  await writeFile(`${output}.md`, markdown);
  console.log(`\nReports: ${output}.md and ${output}.json`);
} finally {
  await fixture.close();
}
