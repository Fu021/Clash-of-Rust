# Memory benchmark

Source: `a17796b165f5d78454308d8acc2ea4ba410c2647`; version: `0.4.13-dev.1`; profile: `dev`.

API values are extra live Rust heap, excluding the fixture/runtime baseline. Each cell is the median of independent processes; refresh=5 retains the old snapshot while replacing it.

| Workload | Mode | Refreshes | Peak MiB | Retained MiB |
| --- | --- | ---: | ---: | ---: |
| rules | streamed | 1 | 4.76 | 3.13 |
| rules | streamed | 5 | 7.90 | 3.13 |
| rules | buffered | 1 | 7.17 | 3.13 |
| rules | buffered | 5 | 10.85 | 3.13 |
| connections | streamed | 1 | 15.32 | 12.37 |
| connections | streamed | 5 | 28.21 | 12.37 |
| connections | buffered | 1 | 26.98 | 12.37 |
| connections | buffered | 5 | 39.36 | 12.37 |
| proxies | streamed | 1 | 10.30 | 8.98 |
| proxies | streamed | 5 | 19.51 | 8.98 |
| proxies | buffered | 1 | 31.59 | 8.98 |
| proxies | buffered | 5 | 41.54 | 8.98 |

GUI subscription: 2,000 nodes, 40 groups × 2,000 members, 20,000 rules. Proxy off; scheduled delay checks off; no business traffic. Xvfb/X11; snapshots show the scene used.

| Scene | GUI RSS peak MiB | Core RSS peak MiB | Total PSS peak MiB |
| --- | ---: | ---: | ---: |
| home | 66.80 | 56.61 | 116.40 |
| proxies-collapsed | 73.66 | 92.41 | 159.05 |
| proxies-search | 79.05 | 95.19 | 166.49 |
| home-after-proxies | 79.05 | 91.55 | 163.59 |

GUI peaks are observed at 250 ms intervals, not exact allocator peaks. RSS can count shared pages twice. Compare separate runs only with the same toolchain, build profile, inputs and environment. Core memory also varies independently of the Rust GUI.
