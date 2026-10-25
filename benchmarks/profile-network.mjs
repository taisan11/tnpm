import assert from 'node:assert/strict';
import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { performance } from 'node:perf_hooks';
import { parseArgs } from 'node:util';
import { createFixture } from './lib/fixtures.mjs';
import { execute } from './lib/process.mjs';

const directory = dirname(fileURLToPath(import.meta.url));
const { values } = parseArgs({ options: {
  tnpm: { type: 'string', default: join(directory, '../target/release/tnpm') },
  samples: { type: 'string', default: '5' },
  spec: { type: 'string', default: 'hono@4.13.13' },
  label: { type: 'string', default: 'default' },
  transport: { type: 'string', default: 'default' },
  cache: { type: 'boolean', default: false },
  output: { type: 'string', default: join(directory, 'results/registry-profile') },
} });
const samples = Number(values.samples);
assert.ok(Number.isSafeInteger(samples) && samples >= 1 && samples <= 100, '--samples must be 1..100');
assert.match(values.spec, /^hono@\d+\.\d+\.\d+(?:-[\w.-]+)?$/);
assert.ok(['default', 'baseline', 'gzip', 'http2', 'both'].includes(values.transport));
const binary = resolve(values.tnpm);
const fixture = await createFixture({ live: true });
try {
  const args = ['view', values.spec, 'version', '--registry', fixture.registry, ...(values.cache ? [] : ['--no-cache'])];
  const env = { ...fixture.environments.tnpm, TNPM_PROFILE: '1', TNPM_PROFILE_TRANSPORT: values.transport };
  const invoke = () => execute(binary, args, { cwd: fixture.cwd, env });
  await invoke(); // warm up outside the recorded runs (or populate the isolated cache)
  const runs = [];
  for (let index = 0; index < samples; index++) {
    const start = performance.now();
    const result = await invoke();
    const wall_ms = performance.now() - start;
    assert.equal(result.stdout.toString().trim(), values.spec.slice('hono@'.length));
    const profiles = result.stderr.toString().split('\n').filter(line => line.startsWith('tnpm-profile: ')).map(line => JSON.parse(line.slice('tnpm-profile: '.length)));
    assert.equal(profiles.length, 2, 'expected registry and view profiles; rebuild the diagnostic binary');
    const registry = profiles.find(profile => profile.section === 'registry');
    assert.equal(registry.source, values.cache ? 'fresh-cache' : 'network');
    runs.push({ wall_ms, profiles });
    console.log(`${values.label} ${index + 1}/${samples}: wall=${wall_ms.toFixed(2)} ms, headers=${registry.headers_ms?.toFixed(2) ?? '-'} ms, body=${registry.body_ms?.toFixed(2) ?? '-'} ms, parse=${registry.json_parse_ms?.toFixed(2) ?? '-'} ms, resolve=${registry.resolve_ms?.toFixed(2) ?? '-'} ms, ${registry.http_version ?? registry.source}`);
  }
  const summarize = numbers => ({ avg: numbers.reduce((sum, n) => sum + n, 0) / numbers.length, min: Math.min(...numbers), max: Math.max(...numbers) });
  const summary = { wall_ms: summarize(runs.map(run => run.wall_ms)) };
  for (const section of ['registry', 'view']) {
    const profiles = runs.map(run => run.profiles.find(profile => profile.section === section));
    summary[section] = Object.fromEntries(Object.keys(profiles[0]).filter(key => key.endsWith('_ms')).map(key => [key, summarize(profiles.map(profile => profile[key]))]));
  }
  const report = { measuredAt: new Date().toISOString(), label: values.label, transport: values.transport, binary, args, samples, cache: values.cache, spec: values.spec, summary, runs };
  const output = resolve(values.output);
  await mkdir(dirname(output), { recursive: true });
  await writeFile(`${output}.json`, `${JSON.stringify(report, null, 2)}\n`);
  const rows = Object.entries(summary.registry).map(([stage, stats]) => `| ${stage} | ${stats.avg.toFixed(3)} | ${stats.min.toFixed(3)} | ${stats.max.toFixed(3)} |`);
  await writeFile(`${output}.md`, [
    `# Registry profile: ${values.label}`, '', `Measured: ${report.measuredAt}`, '',
    `Package: ${values.spec}; samples: ${samples}; cache: ${values.cache}; binary: ${binary}`, '',
    'Opt-in internal monotonic timings plus external subprocess wall time. Headers includes DNS/TCP/TLS and server waiting; it is not isolated TLS time.',
    'Body time includes download and any automatic decompression. decoded_body_bytes is not necessarily the on-wire size.', '',
    `Average subprocess wall time: **${summary.wall_ms.avg.toFixed(3)} ms**`, '',
    '| Registry stage | avg (ms) | min (ms) | max (ms) |', '| --- | ---: | ---: | ---: |', ...rows, '',
    `Registry-to-render/output stage: ${summary.view.render_ms?.avg.toFixed(3) ?? 'N/A'} ms.`, '',
    `First run transport: ${JSON.stringify(runs[0].profiles.find(profile => profile.section === 'registry'))}`, '',
  ].join('\n'));
  console.log(`Report: ${output}.md and ${output}.json`);
} finally { await fixture.close(); }
