# Test status

This file records executed results and their limits. Update it only from an
actual run; never convert an unavailable or timed-out check into PASS.

## Local offline checks

Run on 2026-10-17 in the `Clash-of-Rust` worktree:

| Check | Result | Scope |
| --- | --- | --- |
| PowerShell module parser | PASS | Syntax only |
| `tests/windows-unit.ps1` | PASS, 56 assertions | Mocked host boundaries; no WSL, firewall, DNS or route mutation |
| `tests/linux_static_test.py` | PASS, 17 tests | Portable evidence/gate logic; no kernel operations |
| `tests/server_unit.py` | PASS, 4 tests | Offline VPS profile validation |
| `scripts/test-build-tools.py` | PASS, 36 tests, 1 skipped | Build/resource validation; Windows PE version read skipped locally |
| `scripts/test-workflow-artifacts.py` | PASS, 6 tests | Immutable artifact logic |
| Docker WFP ABI layout | PASS | Structure layout only; no WFP engine opened |
| Git whitespace check | PASS | `git diff --check` |
| `cargo fmt --all -- --check` | PASS | Local Rust workspace |
| `cargo build --workspace --locked --offline` | PASS | Local Windows workspace build |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | PASS | Local Windows workspace |
| Rust tests excluding two environment-dependent Windows checks | PASS, 301; 2 skipped by name; 1 ignored by test annotation | Active GUI singleton and Task Scheduler access-denied checks cannot pass in this session |

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
  HTTP 200 and `api.okinto.com` returned HTTP 200 with certificate validation
  enabled at 2026-10-17 23:13 Asia/Taipei.
- The active distribution is Ubuntu-22.04. Read-only inspection confirmed NAT
  mode, `eth0` on the private WSL subnet, and no IPv6 default route. The distro
  was not stopped or restarted during this test run.
- `tests/windows-unit.ps1`: PASS, 56 assertions. No WSL start, firewall
  modification, route change or DNS change.
- `tests/linux_static_test.py`: PASS, 17 tests, executed inside Ubuntu-22.04.
  The tests mock operating-system calls and perform no kernel mutation.
- A subsequent rerun after the upstream merge was refused by the WSL service
  with `E_ACCESSDENIED`; that invocation is recorded as unavailable and does
  not replace the earlier executed PASS.
- `tests/server_unit.py`: PASS, 4 tests. Offline VPS profile validation only.
- `python3 -m compileall -q scripts/wsl-secure-egress`: NOT_RUN; the WSL
  command was denied by the WSL service with `E_ACCESSDENIED`.
- `cargo fmt --all -- --check`: PASS.
- `cargo build --workspace --locked`: PASS, Windows workspace.
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`:
  PASS.
- `cargo test --workspace --locked --offline`: the full run failed in two
  environment-dependent desktop tests: the existing Clash of Rust GUI owns the
  singleton mutex, and Windows denied Task Scheduler access.
- After fixing the WSL projection test fixture to request the same Tailscale
  backend as its report, its targeted rerun passed. The full workspace rerun
  excluding only those two environment-dependent tests passed 301 tests; 2 were
  skipped by name and 1 CI-only integration test remained ignored. The exact
  command was `cargo test --workspace --locked --offline -- --skip
  ordinary_user_can_initialize_desktop_control --skip
  scheduler_access_denied_falls_back_to_working_user_startup`.
- `git diff --check`: PASS.
- The disposable Linux namespace integration and Windows WFP integration were
  not run locally. Their explicit isolated-runner gates remain required; real
  WSL/Hyper-V and German VPS acceptance remain `NOT_RUN`.
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
- Upstream `upstream/main` was refreshed to commit `e2fb6f1` on 2026-10-17 and
  merged into this branch as commit `4f9b7d7`. The upstream change consolidated
  Windows CI checks and removed obsolete maintenance files; the WSL feature
  files were retained by the merge.
- Current host configuration was inspected read-only: `.wslconfig` uses
  `networkingMode=nat`. Hyper-V WSL firewall policy has not been verified as
  configured by this project on this machine. Repository support for NAT and
  Hyper-V default-deny exists, but the effective production guard has not been
  installed or accepted here.

## Acceptance rule

Positive connectivity requires a receiver-observed nonce, the expected source
and counter evidence. A send success or timeout alone is not proof. A negative
test requires a deny counter, no receiver nonce and unchanged direct-path
counters. Real production fault injection is not authorized.

Update this document after GitHub Actions and any approved VPS/WSL acceptance
with the exact commit, run URL and per-scenario result.
