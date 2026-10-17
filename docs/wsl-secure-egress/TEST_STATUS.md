# Test status

This file records executed results and their limits. Update it only from an
actual run; never convert an unavailable or timed-out check into PASS.

## Local offline checks

Run on 2026-10-17 in the `Clash-of-Rust` worktree:

| Check | Result | Scope |
| --- | --- | --- |
| PowerShell module parser | PASS | Syntax only |
| `tests/windows-unit.ps1` | PASS, 50 assertions | Mocked host boundaries; no WSL, firewall, DNS or route mutation |
| `tests/linux_static_test.py` | PASS, 17 tests | Portable evidence/gate logic; no kernel operations |
| `tests/server_unit.py` | PASS, 4 tests | Offline VPS profile validation |
| `scripts/test-build-tools.py` | PASS, 36 tests, 1 skipped | Build/resource validation; Windows PE version read skipped locally |
| `scripts/test-workflow-artifacts.py` | PASS, 6 tests | Immutable artifact logic |
| Docker WFP ABI layout | PASS | Structure layout only; no WFP engine opened |
| Git whitespace check | PASS | `git diff --check` |
| `cargo fmt --all -- --check` | PASS | Local Rust workspace |
| `cargo build --workspace --locked --offline` | PASS | Local Windows workspace build |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | PASS | Local Windows workspace |
| Rust tests excluding existing GUI singleton conflict | PASS, 150 + 6 + 38 | Existing `clash-of-rust` process prevented one Windows singleton test |

## Not executed locally

| Check | Status | Required environment |
| --- | --- | --- |
| Full Rust test command including GUI singleton test | FAIL (environment) | Existing `clash-of-rust` process holds the expected single-instance mutex |
| Real WSL2/Hyper-V firewall and Windows-to-Linux calls | NOT_RUN | Disposable Windows runner and WSL environment |
| Linux namespace/nftables/WireGuard integration | NOT_RUN | Disposable root Linux runner |
| Real Docker WFP TCP/UDP loopback probe | NOT_RUN | Disposable administrator Windows runner |
| Cold boot, reboot, sleep/resume and network switch | NOT_RUN | Disposable WSL/Hyper-V environment |
| Actual German IPv4 exit and receiver-source evidence | NOT_RUN | Authorized VPS and controlled receiver |
| Docker functional protected egress | NOT_SUPPORTED | Independent adapter not implemented |
| Tailscale strict cold-start protection | BLOCKED | Supported cold-start boundary not established |

## 2026-10-17 local verification

- Windows baseline was checked before WSL-related work: `ipgw.neu.edu.cn` returned
  HTTP 302 and `api.okinto.com` returned HTTP 200 with certificate validation
  enabled.
- WSL 3.0.3 and Ubuntu-22.04 were inspected read-only. The distro has the
  required namespace tools, but `wireguard-tools` is not installed.
- The actual integration entry point refused to run because its explicit
  `CI=true` and `COR_EGRESS_ISOLATED_CI=1` gates are intentionally absent in a
  normal WSL distro. No kernel mutation was performed. An attempted package
  installation was abandoned because the distro's configured proxy
  `127.0.0.1:18001` was unavailable; no package was installed.
- The WSL integration result remains `NOT_RUN`; the GitHub disposable Linux
  runner is the authoritative environment for that test.
- Bubblewrap 0.6.1 is present in Ubuntu-22.04 and does not support
  `--disable-userns`. A disposable probe confirmed that nested user namespace
  creation succeeds without an additional restriction. The workload sandbox
  now installs a seccomp filter that denies `unshare`, `setns`, `clone3`, and
  namespace-flagged `clone`, while retaining normal process creation. The
  isolated integration suite has not yet been rerun in GitHub Actions.
- The two GitHub runs for commit `5865aaf` were inspected. The Linux disposable
  integration reached and passed its direct-egress, DNS, IPv6, and attestation
  cases, then failed when starting the inspection workload. Rust CI exposed an
  overly permissive recovery-phase check; it now accepts only explicit terminal
  status phases. These CI fixes are pending a new hosted run.
- Current host configuration was inspected read-only: `.wslconfig` still uses
  `networkingMode=mirrored`; Hyper-V WSL policy remains `NotConfigured`.
  Repository support for NAT and Hyper-V default-deny exists, but neither has
  been applied on this machine.

## Acceptance rule

Positive connectivity requires a receiver-observed nonce, the expected source
and counter evidence. A send success or timeout alone is not proof. A negative
test requires a deny counter, no receiver nonce and unchanged direct-path
counters. Real production fault injection is not authorized.

Update this document after GitHub Actions and any approved VPS/WSL acceptance
with the exact commit, run URL and per-scenario result.
