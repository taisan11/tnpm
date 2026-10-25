# Cloudflare Workers template — warm-cache install benchmark

Measured: 2026-10-25T01:22:50.864Z

- Template: https://github.com/honojs/starter/tree/main/templates/cloudflare-workers
- Commit: `110fb1c4dc34e050665ca89e0a2fa1a00fde29ef`
- Apple M4; darwin/arm64; Node v26.12.1
- npm 11.20.0; tnpm 0.1.0 (release); bun 1.4.4
- 10 samples and 2 warmups per CLI/case; concurrency 1; order rotated each round; no outlier removal.
- Identical installed package/version multisets validated: 37 packages. Native layouts may differ.
- Each CLI has an isolated cache. The warm case uses --offline for measured installs; the paired cold Bun/tnpm case clears each install cache before invocation and allows registry access. HTTP is not externally counted.
- A single npm v3 dependency graph is shared. Bun imports it into its native lock during untimed warmup. Locks and caches remain in place.
- Clean install: remove only node_modules before every invocation, outside timing. No-op: retain node_modules.
- Cold tnpm extracted case clears only its extracted package cache while retaining verified tarballs. The paired cold Bun/tnpm case clears both package and tarball caches; network transfer is included for both.
- Setup, template fetching, cache warming, cleanup, package validation and Hono import checks are outside timing.
- End-to-end subprocess time includes startup and stdout/stderr draining. OS file caches are warm.

| Case | Tool | Mean (ms) | p50 (ms) | p99 (ms) | Min (ms) | Max (ms) |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| clean install / warm cache | npm | 645.190 | 627.206 | 769.699 | 620.341 | 769.699 |
| clean install / warm cache | tnpm | 212.631 | 211.055 | 223.991 | 205.439 | 223.991 |
| clean install / warm cache | bun | 24.955 | 23.016 | 34.636 | 22.499 | 34.636 |
| clean install / cold tnpm extracted cache | npm | 640.277 | 631.477 | 715.867 | 624.370 | 715.867 |
| clean install / cold tnpm extracted cache | tnpm | 464.599 | 458.934 | 507.534 | 452.071 | 507.534 |
| clean install / cold tnpm extracted cache | bun | 24.036 | 23.351 | 30.534 | 22.639 | 30.534 |
| clean install / cold Bun + tnpm caches | npm | 625.623 | 626.449 | 633.088 | 617.222 | 633.088 |
| clean install / cold Bun + tnpm caches | tnpm | 2092.652 | 2050.179 | 2552.187 | 1800.273 | 2552.187 |
| clean install / cold Bun + tnpm caches | bun | 2054.178 | 1844.242 | 3194.526 | 1295.163 | 3194.526 |
| no-op install / warm cache | npm | 148.880 | 151.334 | 151.999 | 143.476 | 151.999 |
| no-op install / warm cache | tnpm | 5.615 | 5.584 | 5.790 | 5.499 | 5.790 |
| no-op install / warm cache | bun | 5.632 | 5.649 | 5.826 | 5.394 | 5.826 |

## Commands

- `npm install --ignore-scripts --no-audit --no-fund --offline`
- `tnpm install --ignore-scripts --offline`
- `bun install --ignore-scripts --offline`

## Caveats

- Bun and tnpm both reuse extracted package caches, but cache formats, validation, and clone/link implementations differ; these are real implementation differences, not identical I/O strategies.
- Lifecycle/native postinstall work is excluded uniformly. These timings do not measure a fully built Wrangler runtime.
- The source manifest is unchanged. The resolved graph, source commit, raw samples and binary checksum are saved in the JSON report.
- This is one machine/run, not a claim of complete npm compatibility or cold-cache/network performance.
