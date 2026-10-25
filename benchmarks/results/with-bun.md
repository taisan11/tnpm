# npm vs tnpm vs bun — Mitata CLI benchmark

Measured: 2026-10-24T00:09:56.681Z

- Apple M4; darwin/arm64 27.0.0
- Node v26.12.1; npm 11.20.0; tnpm 0.1.0 (release); bun 1.4.4; Mitata 1.0.34
- 30 samples per CLI/case, concurrency 1, no sample trimming. Each sample starts a new CLI process.
- Fixture: 20 direct / 100 total installed packages; 300 registry versions served by a loopback HTTP server.
- Registry response: 259,526 bytes; HTTP 200 with max-age=3600; no TLS or internet latency.
- Separate temporary HOME/cache/config for each CLI; user npmrc/tokens/proxies/Bun environment are not inherited. Build/setup/warmup are outside timing.
- Fixture includes a generated npm lockfile; when Bun is selected, bun pm migrate generates bun.lock outside timing without HTTP requests.

| Case | npm avg (ms) | tnpm avg (ms) | bun avg (ms) | npm / tnpm | bun / tnpm |
| --- | ---: | ---: | ---: | ---: | ---: |
| ls --json | 135.864 | 5.746 | N/A | 23.64x | N/A |
| ls --all --json | 136.981 | 5.604 | N/A | 24.44x | N/A |
| ls / text | 135.696 | 5.689 | 3.396 | 23.85x | 0.60x |
| ls --all / text | 137.692 | 5.984 | 3.458 | 23.01x | 0.58x |
| fund --json | 132.849 | 6.558 | N/A | 20.26x | N/A |
| version --json (read only) | 72.481 | 2.326 | N/A | 31.17x | N/A |
| get registry | 73.908 | 2.300 | N/A | 32.13x | N/A |
| view version / default | 122.925 | 3.056 | 4.666 | 40.22x | 1.53x |
| view version / warm cache | 117.883 | 3.128 | N/A | 37.68x | N/A |
| view version / force network | 123.108 | 3.967 | 4.666 | 31.03x | 1.18x |
| view --json / default | 128.637 | 3.104 | 4.645 | 41.44x | 1.50x |
| view --json / warm cache | 122.547 | 3.091 | N/A | 39.64x | N/A |
| view --json / force network | 126.941 | 4.055 | 4.666 | 31.30x | 1.15x |
| view summary / default | 122.586 | 3.046 | 5.279 | 40.24x | 1.73x |
| view summary / warm cache | 117.604 | 3.106 | N/A | 37.86x | N/A |
| view summary / force network | 122.539 | 4.008 | 4.611 | 30.57x | 1.15x |

Ratios are the other tool's time divided by tnpm's time. Above 1 means tnpm is faster; below 1 means the other tool is faster.

## Commands and detailed timings

| Case | Tool | Command | p50 (ms) | p99 (ms) | HTTP / invocations* |
| --- | --- | --- | ---: | ---: | ---: |
| ls --json | npm | `npm ls --json` | 134.946 | 137.435 | 0/31 |
| ls --json | tnpm | `tnpm ls --json` | 5.668 | 6.311 | 0/31 |
| ls --all --json | npm | `npm ls --all --json` | 136.740 | 139.075 | 0/31 |
| ls --all --json | tnpm | `tnpm ls --all --json` | 5.577 | 5.801 | 0/31 |
| ls / text | npm | `npm ls` | 134.728 | 137.482 | 0/31 |
| ls / text | tnpm | `tnpm ls` | 5.649 | 6.079 | 0/31 |
| ls / text | bun | `bun pm ls` | 3.359 | 3.632 | 0/31 |
| ls --all / text | npm | `npm ls --all` | 137.350 | 141.182 | 0/31 |
| ls --all / text | tnpm | `tnpm ls --all` | 5.855 | 6.510 | 0/31 |
| ls --all / text | bun | `bun pm ls --all` | 3.430 | 3.776 | 0/31 |
| fund --json | npm | `npm fund --json` | 132.470 | 134.715 | 0/31 |
| fund --json | tnpm | `tnpm fund --json` | 6.509 | 6.776 | 0/31 |
| version --json (read only) | npm | `npm version --json` | 72.312 | 75.065 | 0/31 |
| version --json (read only) | tnpm | `tnpm version --json` | 2.271 | 2.628 | 0/31 |
| get registry | npm | `npm get registry` | 72.972 | 74.775 | 0/31 |
| get registry | tnpm | `tnpm get registry` | 2.299 | 2.544 | 0/31 |
| view version / default | npm | `npm view @bench/package version` | 122.559 | 124.200 | 31/31 |
| view version / default | tnpm | `tnpm view @bench/package version` | 3.024 | 3.373 | 0/31 |
| view version / default | bun | `bun info @bench/package version --registry $REGISTRY/bun/ --cache-dir $FIXTURE/bun/.bun/install/cache` | 4.647 | 4.857 | 31/31 |
| view version / warm cache | npm | `npm view @bench/package version --prefer-offline` | 117.655 | 120.423 | 0/31 |
| view version / warm cache | tnpm | `tnpm view @bench/package version` | 3.065 | 3.491 | 0/31 |
| view version / force network | npm | `npm view @bench/package version --prefer-online` | 122.800 | 124.821 | 31/31 |
| view version / force network | tnpm | `tnpm view @bench/package version --no-cache` | 3.874 | 4.396 | 31/31 |
| view version / force network | bun | `bun info @bench/package version --registry $REGISTRY/bun/ --cache-dir $FIXTURE/bun/.bun/install/cache --no-cache` | 4.631 | 4.910 | 31/31 |
| view --json / default | npm | `npm view @bench/package --json` | 126.614 | 140.827 | 31/31 |
| view --json / default | tnpm | `tnpm view @bench/package --json` | 3.071 | 3.381 | 0/31 |
| view --json / default | bun | `bun info @bench/package --json --registry $REGISTRY/bun/ --cache-dir $FIXTURE/bun/.bun/install/cache` | 4.638 | 4.885 | 31/31 |
| view --json / warm cache | npm | `npm view @bench/package --json --prefer-offline` | 121.813 | 123.514 | 0/31 |
| view --json / warm cache | tnpm | `tnpm view @bench/package --json` | 3.069 | 3.400 | 0/31 |
| view --json / force network | npm | `npm view @bench/package --json --prefer-online` | 126.757 | 128.962 | 31/31 |
| view --json / force network | tnpm | `tnpm view @bench/package --json --no-cache` | 3.959 | 4.545 | 31/31 |
| view --json / force network | bun | `bun info @bench/package --json --registry $REGISTRY/bun/ --cache-dir $FIXTURE/bun/.bun/install/cache --no-cache` | 4.654 | 4.954 | 31/31 |
| view summary / default | npm | `npm view @bench/package` | 122.250 | 124.406 | 31/31 |
| view summary / default | tnpm | `tnpm view @bench/package` | 2.999 | 3.395 | 0/31 |
| view summary / default | bun | `bun info @bench/package --registry $REGISTRY/bun/ --cache-dir $FIXTURE/bun/.bun/install/cache` | 5.222 | 5.742 | 31/31 |
| view summary / warm cache | npm | `npm view @bench/package --prefer-offline` | 117.510 | 119.337 | 0/31 |
| view summary / warm cache | tnpm | `tnpm view @bench/package` | 3.050 | 3.418 | 0/31 |
| view summary / force network | npm | `npm view @bench/package --prefer-online` | 122.482 | 124.063 | 31/31 |
| view summary / force network | tnpm | `tnpm view @bench/package --no-cache` | 3.921 | 4.700 | 31/31 |
| view summary / force network | bun | `bun info @bench/package --registry $REGISTRY/bun/ --cache-dir $FIXTURE/bun/.bun/install/cache --no-cache` | 4.583 | 4.884 | 31/31 |

*HTTP counts include Mitata calibration calls as well as measured invocations, but exclude explicit warmup.

## Not comparable / skipped

- **ls --json / bun**: bun pm ls has no JSON output; compare the text-listing cases instead.
- **ls --all --json / bun**: bun pm ls has no JSON output; compare the text-listing cases instead.
- **fund --json / bun**: No corresponding Bun funding command.
- **version --json (read only) / bun**: bun pm version prints the package version and usage, but does not support the JSON version report.
- **get registry / bun**: No corresponding registry-config getter; bun pm pkg get reads package.json instead.
- **view version / warm cache / bun**: Bun 1.4.4 info --prefer-offline still made 1 HTTP request(s) on the second warmup; not a zero-network cache comparison.
- **view --json / warm cache / bun**: Bun 1.4.4 info --prefer-offline still made 1 HTTP request(s) on the second warmup; not a zero-network cache comparison.
- **view summary / warm cache / bun**: Bun 1.4.4 info --prefer-offline still made 1 HTTP request(s) on the second warmup; not a zero-network cache comparison.

## Interpretation

- These are end-to-end CLI timings, not isolated parser/cache throughput. Startup/runtime overhead is included.
- Warm cache: npm/Bun are offered --prefer-offline; tnpm uses its default fresh binary cache. Only zero-network runs are timed; Bun is explicitly skipped if the second warmup still fetches.
- Force network: npm uses --prefer-online (may write its cache); tnpm/Bun use --no-cache. Every invocation must fetch the full fixture.
- Default view behavior is reported separately. HTTP counts reveal any differences in cache policy.
- Bun maps view to bun info, and text ls to bun pm ls. Bun listings are sourced from bun.lock; npm/tnpm inspect installed dependencies. This is not identical work.
- Bun listing JSON, funding and JSON version/config-report cases have no equivalent here and are not timed.
- Output formats and supported functionality are not identical (notably fund/version/full metadata); timings do not imply full npm compatibility.
- OS filesystem caches are warm. This is one local run, not a claim about production registry latency or cold machine performance.
