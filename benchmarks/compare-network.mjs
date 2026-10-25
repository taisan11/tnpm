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
  samples: { type: 'string', default: '5' },
  spec: { type: 'string', default: 'hono@4.13.13' },
  output: { type: 'string', default: join(directory, 'results/registry-transport-comparison-enabled') },
} });
const samples = Number(values.samples);
assert.ok(Number.isSafeInteger(samples) && samples >= 1 && samples <= 100);
assert.match(values.spec, /^hono@\d+\.\d+\.\d+(?:-[\w.-]+)?$/);
const labels = ['baseline', 'gzip', 'http2', 'both', 'default'];
const binary = resolve(directory, '../target/release/tnpm');
const binaries = Object.fromEntries(labels.map(label => [label, binary]));
const fixture = await createFixture({ live: true });
try {
  const args = ['view', values.spec, 'version', '--registry', fixture.registry, '--no-cache'];
  const invoke = async label => {
    const env = { ...fixture.environments.tnpm, TNPM_PROFILE: '1', TNPM_PROFILE_TRANSPORT: label };
    const start = performance.now();
    const result = await execute(binaries[label], args, { cwd: fixture.cwd, env });
    const wall_ms = performance.now() - start;
    assert.equal(result.stdout.toString().trim(), values.spec.slice('hono@'.length));
    const profiles = result.stderr.toString().split('\n').filter(line => line.startsWith('tnpm-profile: ')).map(line => JSON.parse(line.slice('tnpm-profile: '.length)));
    assert.equal(profiles.length, 2);
    const registry = profiles.find(p => p.section === 'registry');
    assert.equal(registry.source, 'network');
    assert.equal(registry.status, 200);
    assert.equal(registry.transport_config, label, 'binary does not support the requested diagnostic mode; rebuild it');
    return { wall_ms, profiles };
  };
  for (const label of labels) await invoke(label);
  const runs = Object.fromEntries(labels.map(label => [label, []]));
  for (let round = 0; round < samples; round++) {
    // Rotate the order every round rather than measuring four sequential blocks.
    for (let offset = 0; offset < labels.length; offset++) {
      const label = labels[(round + offset) % labels.length];
      const result = await invoke(label);
      runs[label].push(result);
      const p = result.profiles.find(p => p.section === 'registry');
      console.log(`${round + 1}/${samples} ${label}: wall=${result.wall_ms.toFixed(2)}, headers=${p.headers_ms.toFixed(2)}, body=${p.body_ms.toFixed(2)}, parse=${p.json_parse_ms.toFixed(2)} ms, ${p.http_version}`);
    }
  }
  const mean = values => values.reduce((sum, value) => sum + value, 0) / values.length;
  const summary = Object.fromEntries(labels.map(label => {
    const profiles = runs[label].map(run => run.profiles.find(p => p.section === 'registry'));
    return [label, {
      wall_ms: mean(runs[label].map(run => run.wall_ms)),
      ...Object.fromEntries(['headers_ms', 'body_ms', 'json_parse_ms', 'resolve_ms', 'json_tree_drop_ms', 'decoded_body_bytes'].map(key => [key, mean(profiles.map(p => p[key]))])),
      protocols: [...new Set(profiles.map(p => p.http_version))],
      build_features: profiles[0].build_features,
      transport_config: profiles[0].transport_config,
    }];
  }));
  const report = { measuredAt: new Date().toISOString(), spec: values.spec, samples, order: 'rotated every round', binaries, summary, runs };
  const output = resolve(values.output);
  await mkdir(dirname(output), { recursive: true });
  await writeFile(`${output}.json`, `${JSON.stringify(report, null, 2)}\n`);
  await writeFile(`${output}.md`, [
    '# Registry transport A/B — same CLI and parser', '', `Measured: ${report.measuredAt}; package: ${values.spec}; ${samples} samples/variant.`, '',
    'All runs use the same binary: HTTP 200, --no-cache, fresh process/client/connection. Diagnostic runtime settings choose identity/gzip and HTTP/1.1/HTTP2. The default variant uses reqwest defaults plus all four compression codecs. Order rotates each round; warmup excluded.',
    'Headers includes DNS/TCP/TLS/server waiting. Body includes transfer and automatic decompression; decoded sizes are not wire sizes.', '',
    '| Variant | HTTP | Wall (ms) | Headers (ms) | Body (ms) | JSON parse (ms) | Resolve (ms) | Tree drop (ms) |',
    '| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |',
    ...labels.map(label => { const s = summary[label]; return `| ${label} | ${s.protocols.join(', ')} | ${s.wall_ms.toFixed(3)} | ${s.headers_ms.toFixed(3)} | ${s.body_ms.toFixed(3)} | ${s.json_parse_ms.toFixed(3)} | ${s.resolve_ms.toFixed(3)} | ${s.json_tree_drop_ms.toFixed(3)} |`; }), '',
    `Decoded body sizes: ${labels.map(label => `${label}=${summary[label].decoded_body_bytes} B`).join(', ')}.`, '',
    'This is one small live-network experiment. It demonstrates effects in this environment, not universal speedups or isolated TLS timings.', '',
  ].join('\n'));
  console.log(`Report: ${output}.md and ${output}.json`);
} finally { await fixture.close(); }
