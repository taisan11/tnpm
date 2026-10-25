# npm vs tnpm — Mitata CLI benchmark

Measured: 2026-10-23T11:46:55.380Z

- Apple M4; darwin/arm64 27.0.0
- Node v26.12.1; npm 11.20.0; tnpm 0.1.0 (release); Mitata 1.0.34
- 30 samples per CLI/case, concurrency 1, no sample trimming. Each sample starts a new CLI process.
- Fixture: 20 direct / 100 total installed packages; 300 registry versions served by a loopback HTTP server.
- Registry response: 259,526 bytes; HTTP 200 with max-age=3600; no TLS or internet latency.
- Separate temporary HOME/npm cache for each CLI; user npmrc/tokens/proxies are not inherited. Build/setup/warmup are outside timing.

| Case | npm avg (ms) | tnpm avg (ms) | npm p50 (ms) | tnpm p50 (ms) | npm p99 (ms) | tnpm p99 (ms) | npm / tnpm | HTTP npm / tnpm* |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| ls --json | 134.208 | 5.746 | 133.912 | 5.701 | 136.344 | 6.247 | 23.36x | 0/31 / 0/31 |
| ls --all --json | 135.110 | 5.651 | 134.954 | 5.598 | 136.590 | 5.942 | 23.91x | 0/31 / 0/31 |
| fund --json | 132.007 | 6.570 | 131.731 | 6.541 | 133.501 | 6.860 | 20.09x | 0/31 / 0/31 |
| version --json (read only) | 71.532 | 2.242 | 71.482 | 2.240 | 73.010 | 2.345 | 31.91x | 0/31 / 0/31 |
| get registry | 72.173 | 2.293 | 71.906 | 2.264 | 73.816 | 2.487 | 31.48x | 0/31 / 0/31 |
| view version / default | 121.541 | 2.942 | 121.329 | 2.925 | 122.914 | 3.071 | 41.31x | 31/31 / 0/31 |
| view version / warm cache | 116.351 | 2.964 | 116.260 | 2.919 | 117.201 | 3.317 | 39.25x | 0/31 / 0/31 |
| view version / force network | 121.651 | 3.897 | 121.575 | 3.849 | 123.148 | 4.323 | 31.22x | 31/31 / 31/31 |
| view --json / warm cache | 120.742 | 3.113 | 120.512 | 3.046 | 122.376 | 3.391 | 38.79x | 0/31 / 0/31 |
| view --json / force network | 126.250 | 3.869 | 126.096 | 3.854 | 128.334 | 4.058 | 32.63x | 31/31 / 31/31 |
| view summary / warm cache | 116.773 | 3.077 | 116.534 | 2.997 | 118.362 | 3.478 | 37.95x | 0/31 / 0/31 |

*HTTP counts include Mitata calibration calls as well as measured invocations, but exclude explicit warmup.

## Interpretation

- These are end-to-end CLI timings, not isolated parser/cache throughput. Startup/runtime overhead is included.
- Warm cache: npm uses --prefer-offline; tnpm uses its default fresh binary cache. Both are asserted to make zero HTTP requests.
- Force network: npm uses --prefer-online (may write its cache); tnpm uses --no-cache (neither reads nor writes). Every invocation must fetch the full fixture.
- Default view behavior is reported separately. HTTP counts reveal any differences in cache policy.
- Output formats and supported functionality are not identical (notably fund/version); timings do not imply full npm compatibility.
- OS filesystem caches are warm. This is one local run, not a claim about production registry latency or cold machine performance.
