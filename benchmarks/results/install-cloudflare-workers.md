# Cloudflare Workers template — warm-cache install benchmark

Measured: 2026-10-24T02:33:08.177Z

- Template: https://github.com/honojs/starter/tree/main/templates/cloudflare-workers
- Commit: `110fb1c4dc34e050665ca89e0a2fa1a00fde29ef`
- Apple M4; darwin/arm64; Node v26.12.1
- npm 11.20.0; tnpm 0.1.0 (release); bun 1.4.4
- 10 samples and 2 warmups per CLI/case; concurrency 1; order rotated each round; no outlier removal.
- Identical installed package/version multisets validated: 37 packages. Native layouts may differ.
- Each CLI has its own warmed package cache. Measured installs use --offline and --ignore-scripts; no audit/fund. HTTP is not externally counted.
- A single npm v3 dependency graph is shared. Bun imports it into its native lock during untimed warmup. Locks and caches remain in place.
- Clean install: remove only node_modules before every invocation, outside timing. No-op: retain node_modules.
- Setup, template fetching, cache warming, cleanup, package validation and Hono import checks are outside timing.
- End-to-end subprocess time includes startup and stdout/stderr draining. OS file caches are warm.

| Case | Tool | Mean (ms) | p50 (ms) | p99 (ms) | Min (ms) | Max (ms) |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| clean install / warm cache | npm | 667.755 | 637.931 | 745.384 | 624.015 | 745.384 |
| clean install / warm cache | tnpm | 400.762 | 401.034 | 423.462 | 385.841 | 423.462 |
| clean install / warm cache | bun | 28.864 | 29.583 | 34.315 | 22.933 | 34.315 |
| no-op install / warm cache | npm | 157.064 | 154.264 | 178.497 | 144.334 | 178.497 |
| no-op install / warm cache | tnpm | 6.154 | 6.023 | 6.688 | 5.934 | 6.688 |
| no-op install / warm cache | bun | 5.628 | 5.633 | 5.943 | 5.317 | 5.943 |

## Commands

- `npm install --ignore-scripts --no-audit --no-fund --offline`
- `tnpm install --ignore-scripts --offline`
- `bun install --ignore-scripts --offline`

## Caveats

- Bun can clone/link extracted cache contents while tnpm re-verifies and decompresses cached tarballs; these are real implementation differences, not identical I/O strategies.
- Lifecycle/native postinstall work is excluded uniformly. These timings do not measure a fully built Wrangler runtime.
- The source manifest is unchanged. The resolved graph, source commit, raw samples and binary checksum are saved in the JSON report.
- This is one machine/run, not a claim of complete npm compatibility or cold-cache/network performance.
