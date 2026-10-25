# Cloudflare Workers template — warm-cache install benchmark

Measured: 2026-10-25T01:16:42.676Z

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
| clean install / warm cache | npm | 642.671 | 631.849 | 694.977 | 617.829 | 694.977 |
| clean install / warm cache | tnpm | 215.046 | 211.115 | 228.699 | 207.346 | 228.699 |
| clean install / warm cache | bun | 31.525 | 33.132 | 40.088 | 23.069 | 40.088 |
| clean install / cold tnpm extracted cache | npm | 640.636 | 629.906 | 685.681 | 623.490 | 685.681 |
| clean install / cold tnpm extracted cache | tnpm | 465.498 | 460.859 | 511.012 | 455.648 | 511.012 |
| clean install / cold tnpm extracted cache | bun | 24.948 | 23.701 | 31.663 | 22.732 | 31.663 |
| clean install / cold Bun + tnpm caches | npm | 633.024 | 628.345 | 696.604 | 619.484 | 696.604 |
| clean install / cold Bun + tnpm caches | tnpm | 1993.840 | 1830.554 | 3504.568 | 1585.794 | 3504.568 |
| clean install / cold Bun + tnpm caches | bun | 1869.475 | 1615.396 | 2832.791 | 1444.732 | 2832.791 |
| no-op install / warm cache | npm | 148.670 | 150.335 | 153.026 | 143.749 | 153.026 |
| no-op install / warm cache | tnpm | 5.612 | 5.602 | 5.858 | 5.364 | 5.858 |
| no-op install / warm cache | bun | 5.606 | 5.652 | 5.789 | 5.337 | 5.789 |

## Commands

- `npm install --ignore-scripts --no-audit --no-fund --offline`
- `tnpm install --ignore-scripts --offline`
- `bun install --ignore-scripts --offline`

## Caveats

- Bun and tnpm both reuse extracted package caches, but cache formats, validation, and clone/link implementations differ; these are real implementation differences, not identical I/O strategies.
- Lifecycle/native postinstall work is excluded uniformly. These timings do not measure a fully built Wrangler runtime.
- The source manifest is unchanged. The resolved graph, source commit, raw samples and binary checksum are saved in the JSON report.
- This is one machine/run, not a claim of complete npm compatibility or cold-cache/network performance.
