# Memory benchmark

Source: `6473bbeb665b51526c2ff3b81f3efa5dd4ee8952`; version: `0.4.12`; profile: `dev`.

API values are extra live Rust heap, excluding the fixture/runtime baseline. Each cell is the median of independent processes; refresh=5 retains the old snapshot while replacing it.

| Workload | Mode | Refreshes | Peak MiB | Retained MiB |
| --- | --- | ---: | ---: | ---: |
| rules | streamed | 1 | 7.01 | 6.11 |
| rules | streamed | 5 | 13.13 | 6.11 |
| rules | buffered | 1 | 9.43 | 6.11 |
| rules | buffered | 5 | 15.55 | 6.11 |
| connections | streamed | 1 | 23.59 | 23.30 |
| connections | streamed | 5 | 47.34 | 23.30 |
| connections | buffered | 1 | 35.29 | 23.30 |
| connections | buffered | 5 | 58.60 | 23.30 |
| proxies | streamed | 1 | 21.23 | 21.09 |
| proxies | streamed | 5 | 42.78 | 21.09 |
| proxies | buffered | 1 | 36.84 | 21.09 |
| proxies | buffered | 5 | 57.93 | 21.09 |

GUI subscription: 2,000 nodes, 40 groups × 2,000 members, 20,000 rules. Proxy off; scheduled delay checks off; no business traffic. Xvfb/X11; snapshots show the scene used.

| Scene | GUI RSS peak MiB | Core RSS peak MiB | Total PSS peak MiB |
| --- | ---: | ---: | ---: |
| home | 65.87 | 57.79 | 116.56 |
| proxies-collapsed | 80.41 | 92.97 | 166.29 |
| proxies-search | 127.33 | 95.26 | 215.50 |
| home-after-proxies | 127.33 | 93.34 | 213.57 |

GUI peaks are observed at 250 ms intervals, not exact allocator peaks. RSS can count shared pages twice. Compare separate runs only with the same toolchain, build profile, inputs and environment. Core memory also varies independently of the Rust GUI.
