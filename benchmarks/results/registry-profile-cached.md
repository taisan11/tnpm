# Registry profile: cached

Measured: 2026-10-24T01:08:20.003Z

Package: hono@4.13.13; samples: 5; cache: true; binary: /Users/taisan11/Documents/tnpm/target/release/tnpm

Opt-in internal monotonic timings plus external subprocess wall time. Headers includes DNS/TCP/TLS and server waiting; it is not isolated TLS time.
Body time includes download and any automatic decompression. decoded_body_bytes is not necessarily the on-wire size.

Average subprocess wall time: **13.432 ms**

| Registry stage | avg (ms) | min (ms) | max (ms) |
| --- | ---: | ---: | ---: |
| cache_read_ms | 9.356 | 8.755 | 10.496 |
| cache_resolve_ms | 0.462 | 0.430 | 0.519 |
| total_ms | 9.821 | 9.187 | 11.018 |

Registry-to-render/output stage: 0.092 ms.

First run transport: {"build_features":"gzip=false,http2=false","cache_read_ms":10.495917,"cache_resolve_ms":0.519208,"source":"fresh-cache","section":"registry","total_ms":11.018292}
