# npm vs tnpm vs bun — Mitata CLI benchmark (live hono)

Measured: 2026-10-24T01:23:01.194Z

- Apple M4; darwin/arm64 27.0.0
- Node v26.12.1; npm 11.20.0; tnpm 0.1.0 (release); bun 1.4.4; Mitata 1.0.34
- 5 samples per CLI/case, concurrency 1, no sample trimming. Each sample starts a new CLI process.
- Live registry: https://registry.npmjs.org/; package: hono@4.13.13. Version resolved once by npm before timing.
- Requests target the public HTTPS registry with no benchmark-inserted server/proxy. OS system proxy settings may apply. DNS/TCP/TLS/CDN/network latency and CLI startup are included.
- Default mode may use each CLI's warmed cache. Force-network mode uses npm --prefer-online or tnpm/Bun --no-cache.
- HTTP counts, status codes, compressed byte counts, and actual cache hits are not instrumented: N/A means unknown, NOT zero requests.
- Separate temporary HOME/cache/config for each CLI; user npmrc/tokens/proxy environment variables/Bun environment are not inherited; OS system proxy settings may apply. Build/setup/warmup are outside timing.

| Case | npm avg (ms) | tnpm avg (ms) | bun avg (ms) | npm / tnpm | bun / tnpm |
| --- | ---: | ---: | ---: | ---: | ---: |
| view version / force network | 209.824 | 195.080 | 153.579 | 1.08x | 0.79x |
| view --json / force network | 220.754 | 206.024 | 236.152 | 1.07x | 1.15x |
| view summary / force network | 218.006 | 208.572 | 179.294 | 1.05x | 0.86x |

Ratios are the other tool's time divided by tnpm's time. Above 1 means tnpm is faster; below 1 means the other tool is faster.

## Commands and detailed timings

| Case | Tool | Command | p50 (ms) | p99 (ms) | HTTP / invocations* |
| --- | --- | --- | ---: | ---: | ---: |
| view version / force network | npm | `npm view hono@4.13.13 version --registry $REGISTRY/ --prefer-online` | 206.457 | 217.686 | N/A / 6 |
| view version / force network | tnpm | `tnpm view hono@4.13.13 version --registry $REGISTRY/ --no-cache` | 190.657 | 206.416 | N/A / 6 |
| view version / force network | bun | `bun info hono@4.13.13 version --registry $REGISTRY/ --cache-dir $FIXTURE/bun/.bun/install/cache --no-cache` | 146.965 | 152.523 | N/A / 6 |
| view --json / force network | npm | `npm view hono@4.13.13 --json --registry $REGISTRY/ --prefer-online` | 220.587 | 229.581 | N/A / 6 |
| view --json / force network | tnpm | `tnpm view hono@4.13.13 --json --registry $REGISTRY/ --no-cache` | 198.624 | 226.173 | N/A / 6 |
| view --json / force network | bun | `bun info hono@4.13.13 --json --registry $REGISTRY/ --cache-dir $FIXTURE/bun/.bun/install/cache --no-cache` | 220.010 | 253.225 | N/A / 6 |
| view summary / force network | npm | `npm view hono@4.13.13 --registry $REGISTRY/ --prefer-online` | 218.298 | 222.478 | N/A / 6 |
| view summary / force network | tnpm | `tnpm view hono@4.13.13 --registry $REGISTRY/ --no-cache` | 217.597 | 218.614 | N/A / 6 |
| view summary / force network | bun | `bun info hono@4.13.13 --registry $REGISTRY/ --cache-dir $FIXTURE/bun/.bun/install/cache --no-cache` | 177.598 | 187.227 | N/A / 6 |

*Live HTTP counts are unobserved; invocation counts include Mitata calibration calls but exclude explicit warmup.

## Interpretation

- These are end-to-end CLI timings, not isolated parser/cache throughput. Startup/runtime overhead is included.
- Live mode does not offer a zero-network warm-cache comparison because actual HTTP requests cannot be counted here.
- Force-network flags request fresh registry information. npm may revalidate with HTTP 304 or write its cache; payloads/compression and cache behavior can differ across CLIs.
- The same published version is pinned for all CLIs. Hono package contents/scripts are never installed or executed.
- Live measurements depend on network/CDN conditions and execution order; do not mix them with the deterministic loopback results.
- Default view timings may benefit from cached data; actual cache hits and request counts are unknown in live mode.
- Bun maps view to bun info.
- Output formats and supported functionality are not identical (notably fund/version/full metadata); timings do not imply full npm compatibility.
- OS filesystem caches are warm. This is one run, not a general claim about production registry latency or cold machine performance.
