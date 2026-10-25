# Registry profile: baseline

Measured: 2026-10-24T01:01:48.055Z

Package: hono@4.13.13; samples: 5; cache: false; binary: /Users/taisan11/Documents/tnpm/target/release/tnpm

Opt-in internal monotonic timings plus external subprocess wall time. Headers includes DNS/TCP/TLS and server waiting; it is not isolated TLS time.
Body time includes download and any automatic decompression. decoded_body_bytes is not necessarily the on-wire size.

Average subprocess wall time: **519.699 ms**

| Registry stage | avg (ms) | min (ms) | max (ms) |
| --- | ---: | ---: | ---: |
| cache_read_ms | 0.003 | 0.002 | 0.004 |
| cache_resolve_ms | 0.000 | 0.000 | 0.000 |
| client_build_ms | 0.053 | 0.046 | 0.064 |
| request_build_ms | 0.005 | 0.004 | 0.007 |
| headers_ms | 89.319 | 75.264 | 102.873 |
| policy_ms | 0.020 | 0.016 | 0.029 |
| body_ms | 374.773 | 222.119 | 739.769 |
| utf8_ms | 2.539 | 2.171 | 3.003 |
| json_parse_ms | 37.038 | 35.857 | 38.555 |
| resolve_ms | 0.520 | 0.463 | 0.558 |
| cache_write_ms | 0.001 | 0.001 | 0.001 |
| json_tree_drop_ms | 7.300 | 6.666 | 8.258 |
| total_ms | 511.599 | 357.378 | 886.554 |

Registry-to-render/output stage: 0.133 ms.

First run transport: {"cache_read_ms":0.003,"cache_resolve_ms":0.000375,"client_build_ms":0.0495,"request_build_ms":0.00425,"headers_ms":98.954959,"status":200,"http_version":"HTTP/1.1","content_encoding":"identity-or-decoded","content_length":6311465,"remote_addr":"104.16.8.34:443","policy_ms":0.018625,"body_ms":739.769458,"decoded_body_bytes":6311465,"utf8_ms":2.455083,"json_parse_ms":37.523375,"resolve_ms":0.5295,"cache_write_ms":0.000834,"json_tree_drop_ms":7.220625,"source":"network","section":"registry","total_ms":886.553875}
