# Registry profile: default-compression

Measured: 2026-10-24T01:21:55.971Z

Package: hono@4.13.13; samples: 5; cache: false; binary: /Users/taisan11/Documents/tnpm/target/release/tnpm

Opt-in internal monotonic timings plus external subprocess wall time. Headers includes DNS/TCP/TLS and server waiting; it is not isolated TLS time.
Body time includes download and any automatic decompression. decoded_body_bytes is not necessarily the on-wire size.

Average subprocess wall time: **182.931 ms**

| Registry stage | avg (ms) | min (ms) | max (ms) |
| --- | ---: | ---: | ---: |
| cache_read_ms | 0.005 | 0.004 | 0.005 |
| cache_resolve_ms | 0.000 | 0.000 | 0.000 |
| client_build_ms | 2.495 | 2.333 | 2.798 |
| request_build_ms | 0.006 | 0.005 | 0.007 |
| headers_ms | 114.473 | 96.265 | 124.213 |
| policy_ms | 0.015 | 0.013 | 0.016 |
| body_ms | 33.300 | 20.602 | 70.050 |
| utf8_ms | 0.950 | 0.827 | 1.165 |
| json_parse_ms | 21.733 | 19.209 | 27.351 |
| resolve_ms | 0.367 | 0.343 | 0.432 |
| cache_write_ms | 0.000 | 0.000 | 0.000 |
| json_tree_drop_ms | 4.409 | 4.033 | 5.004 |
| total_ms | 177.764 | 163.351 | 203.107 |

Registry-to-render/output stage: 0.102 ms.

First run transport: {"build_features":"reqwest-defaults,gzip,brotli,deflate,zstd","transport_config":"default","accept_encoding":"gzip, br, zstd, deflate","cache_read_ms":0.004875,"cache_resolve_ms":0.000458,"client_build_ms":2.798209,"request_build_ms":0.006541,"headers_ms":96.265125,"status":200,"http_version":"HTTP/2.0","content_encoding":"identity-or-decoded","remote_addr":"104.16.9.34:443","policy_ms":0.014292,"body_ms":70.050167,"decoded_body_bytes":6311465,"utf8_ms":1.165291,"json_parse_ms":27.351042,"resolve_ms":0.432083,"cache_write_ms":0.000417,"json_tree_drop_ms":5.004458,"source":"network","section":"registry","total_ms":203.106583}
