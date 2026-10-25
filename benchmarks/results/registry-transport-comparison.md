# Registry transport A/B — same CLI and parser

Measured: 2026-10-24T01:07:15.461Z; package: hono@4.13.13; 5 samples/variant.

All runs are HTTP 200, --no-cache, fresh process/client/connection. Variant order rotates each round; warmup excluded. Default Cargo features are unchanged.
Headers includes DNS/TCP/TLS/server waiting. Body includes transfer and automatic decompression; decoded sizes are not wire sizes.

| Variant | HTTP | Wall (ms) | Headers (ms) | Body (ms) | JSON parse (ms) | Resolve (ms) | Tree drop (ms) |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| baseline | HTTP/1.1 | 342.863 | 77.172 | 223.525 | 28.639 | 0.459 | 5.291 |
| gzip | HTTP/1.1 | 179.055 | 115.671 | 31.117 | 21.387 | 0.351 | 4.118 |
| http2 | HTTP/2.0 | 358.767 | 80.020 | 234.913 | 30.416 | 0.460 | 5.342 |
| both | HTTP/2.0 | 197.122 | 130.278 | 32.879 | 22.715 | 0.345 | 4.286 |

Decoded body sizes: baseline=6311465 B, gzip=6311465 B, http2=6311465 B, both=6311465 B.

This is one small live-network experiment. It demonstrates effects in this environment, not universal speedups or isolated TLS timings.
