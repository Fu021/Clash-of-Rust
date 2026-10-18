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
| Linux namespace/nftables/WireGuard integration | PASS, 86 cases | Real Ubuntu-22.04 WSL2 root; see the 2026-10-18 local WSL section below |
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

## 2026-10-18 live WSL acceptance (Windows host, Ubuntu-22.04)

Admin-level Start and Stop transactions were executed through the reviewed
PowerShell UAC path. Addresses and packet payloads are omitted; raw reports stay
in the ignored local backup directory.

| Check | Result | Evidence and limits |
| --- | --- | --- |
| Windows baseline before start | PASS | `ipgw.neu.edu.cn` HTTP 200 and `api.okinto.com` HTTP 200, TLS validated, 02:05:25 Asia/Taipei |
| WSL host NAT readback | PASS | `eth0` on the WSL private `/20`, default route to the WSL gateway only, no global IPv6, `/etc/resolv.conf` nameserver is the tunnel resolver |
| `.wslconfig` readback | PASS | `networkingMode=nat`, `dnsTunneling=false`, `autoProxy=false`, `firewall=true`, `localhostForwarding=false` |
| Admin `Start -Execute` | PASS | 02:15:24; generation `1792256171034`; endpoint enabled with host policy audit PASS |
| Controlled live acceptance, ordinary WSL | PASS | DNS nonce observed on the tunnel and upstream, HTTPS 200 via IPv4, VPS-observed source match, IPv6 connect failed, no clear uplink nonce |
| Controlled live acceptance, isolated launcher | PASS | Interfaces exactly `lo`, `cor-bg0`, `wg0`; UID 1000; all capability sets zero; `NoNewPrivs=1`; `/mnt/c` and `ipconfig.exe` hidden; same DNS/HTTPS/IPv6/VPS evidence |
| Windows baseline after stop and restart | PASS | both mandatory Windows URLs HTTP 200 with TLS validation at 02:14:02 |
| Isolated CI data-plane and Windows WFP fixture runs | NOT_RUN locally | Tracked separately in GitHub Actions after the branch is pushed |

The live acceptance report is
`ClashOfRust-network-backups/wsl-host-20261018/live-acceptance-20261018-021627.json`.
An earlier run at 02:13:02 is retained as FAIL: the ordinary path passed, but
the isolated launcher correctly refused a stale Windows attestation and the
aggregate result was failed. It was not silently replaced.

### Hosts isolation upgrade, fail-closed and recovery

The user explicitly authorized administrator elevation. The upgrade was then
executed end to end:

- `Stop -Execute` closed the endpoint and removed the owned namespaces; the
  mandatory Windows URL baseline returned HTTP 200 with TLS validation at
  02:34:27 Asia/Taipei.
- `Update-LiveWslPayload.ps1 -Execute` completed with the atomic reviewed
  upgrade and wrote a protected receipt. `owned_config_matches=true`,
  `interop_runtime_disabled=true`, `endpoint_enabled=false`.
- Readback inside Ubuntu-22.04: `generateResolvConf=false`,
  `generateHosts=false`, interop `enabled=false`,
  `appendWindowsPath=false`, `/etc/resolv.conf` nameserver is the fixed tunnel
  resolver, and `/etc/hosts` contains only `127.0.0.1 localhost`,
  `127.0.1.1 Xuan` and `::1 localhost ip6-localhost ip6-loopback`. The
  installed `manager.py` SHA256 matched the repository file.
- `Start -Execute` recreated the namespaces and enabled the fixed endpoint.
  The post-upgrade acceptance at 02:36:27 returned PASS for both
  `ordinary-wsl` and `isolated-launcher`, including `hosts_leak_free=true`,
  `interop_disabled=true`, IPv6 denial and VPS receiver evidence.
- Controlled fail-closed test: the owned VPS receiver reported `READY`, the
  administrator `Stop -Execute` closed the endpoint, the ordinary WSL client
  failed with `TimeoutError` and `connected=false`, and the receiver reported
  `received=false` with `NO_RECEIPT`. Suite outcome: passed.
- Recovery: `Start -Execute` succeeded, both mandatory Windows URLs returned
  HTTP 200 with TLS validation at 02:38:47, host policy audit was PASS, and a
  final acceptance at 02:39:00 returned PASS for both scopes again.
- Offline suites after the change: `windows-unit.ps1` PASS, 79 assertions;
  `linux_static_test.py` PASS, 21 tests; payload unit PASS, 32 tests;
  provision unit PASS, 18 tests; server unit PASS, 24 tests; Python
  compilation PASS; acceptance-harness endpoint validation PASS, 3 tests.
- The acceptance scripts previously contained a hard-coded probe address that
  could make a fail-closed result meaningless. They now require the fixed
  public IPv4 endpoint to be supplied from the private profile and reject
  missing, non-numeric or private values. Real endpoint values are not written
  to reports or Git.

Still `NOT_RUN` on this host: Windows cold boot, sleep/resume, physical network
switch and independent geolocation corroboration.

## 2026-10-18 local WSL isolated integration run

Executed inside the real distribution, not a container: Ubuntu-22.04, kernel
`6.18.54.1-microsoft-standard-WSL2`, root, repository copied to a Linux-native
temporary directory first (the suite mounts tmpfs over `/mnt`, so a copy under
`/mnt/e` cannot be the run directory).

| Check | Result | Scope |
| --- | --- | --- |
| `tests/linux_integration.py --execute-isolated` | PASS, 86 cases | Disposable net/mount/PID namespaces with the fixture VPS, WireGuard transport and nftables guard; no Windows, WSL host or real VPS configuration participated |
| `python3 -m compileall -q scripts/wsl-secure-egress` | PASS | Syntax only |
| `tests/linux_static_test.py` | PASS, 21 tests | Portable evidence/gate logic |
| `tests/acceptance_harness_unit.py` | PASS, 3 tests | Endpoint validation |
| `tests/server_unit.py` | PASS, 24 tests | Offline VPS profile validation |
| `tests/payload_unit.py` | PASS, 32 tests | Real Linux root ownership, flock and atomic exchange on disposable directories under `/root` |
| `tests/provision_unit.py` | PASS, 18 tests | Mocked host boundaries only |
| `find scripts/wsl-secure-egress/linux ... -exec sh -n` | PASS | Shell syntax of the managed launchers |
| `cargo build --workspace --locked --offline` and `cargo build -p clash-of-rust --bins --release --locked --offline` | PASS | Produced `target/release/clash-of-rust.exe`, `clash-tun-launcher.exe`, `clash-wsl-guardian.exe` |

Evidence: `ClashOfRust-network-backups/wsl-local-20261018/linux-egress-integration-20261017-214237.json`
(`outcome=passed`, 86 cases). The suite reports the same limits as CI: the
Windows/WSL/Hyper-V effective audit, cold boot and real VPS/geography checks are
`notRun` inside it.

Two failures happened before that PASS and are recorded rather than replaced:

1. `manager install` returned `blocked` with `Private path ownership, symlink or
   permissions check failed.` The cause was the host, not the manager: an earlier
   automation defect ran `cp -a <repo>/. /` inside WSL, which copies the source
   directory attributes onto an existing destination and therefore left `/`
   owned by the desktop user with mode `0777`. The guard correctly refuses to
   install private state in that condition. The duplicated tree was moved to
   `/root/quarantine-root-repo-copy-20261018` and `/` was restored to
   `root:root 0755`; `/usr`, `/etc`, `/var`, `/root`, `/home`, `/mnt` and the
   `init`/`.package-cache-mutate` files were not modified, and `/.local-tools`
   and `/.cargo` were left in place.
2. `managed-unauthorized-dns-udp` then failed with `receiver positive control
   failed`, the same defect that failed CI. The negative-case helper re-probed
   the intentionally blocked destination as its trailing positive control; the
   fixture DNS receiver binds only the managed resolver address for UDP, so the
   control could never observe a receipt. Both the pre-probe and the post-probe
   control now use the declared control endpoint, and `COR_EGRESS_TEST_DEBUG=1`
   was added as an opt-in for redacted fixture stdout/stderr when a manager JSON
   event is missing. Default CI output stays redacted.

## 2026-10-18 CI failure, root cause and deterministic fixture fix

GitHub Actions run
[39063692775](https://github.com/Sonder4/Clash-of-Rust/actions/runs/39063692775)
for commit `f6e2542` failed the isolated suite at
`missing-tunnel-existing-tcp: timeout without a deny counter is not proof`.
The Windows/Rust workflow for the same commit passed. The first local rerun
afterwards failed on the same case with `sent=True errno=None
error=TimeoutError`.

Root cause, taken from the fixture rather than from the product: the case
deleted the managed WireGuard link and then added a fixture fallback route, but
the pre-established socket had selected the tunnel address `10.77.0.2/32` as
its source. Once that address disappeared with the device, the kernel refused
the flow during route lookup, so nftables never saw a packet and the guard
counter could not move. The case was proving that a deleted source address
blackholes a socket, not that the guard drops it.

A first attempt to accept a named local `errno` as refusal evidence was
reverted before commit: it weakened a real proof into a weaker one and still
did not hold, because the kernel retransmit path swallows route lookup
failures instead of reporting them to the application.

The fixture now reassigns `10.77.0.2/32` to the fallback link, so the
pre-existing flow stays routable and its packets must reach the guard. The
negative rule is unchanged and strict: every negative case still requires a
deny-counter increase, no receiver nonce and unchanged direct-path counters.
The same change makes the case deterministic for both scopes, because the
ordinary (root-namespace) session never lost its source address and already
produced counter evidence.

Executed after the fix on this host:

| Check | Result | Scope |
| --- | --- | --- |
| Full isolated suite in Ubuntu-22.04 | PASS, 86 cases | Disposable net/PID namespaces with the fixture VPS; no Windows, WSL host or real VPS configuration participated |
| `missing-tunnel-existing-tcp` evidence | PASS, deny counter 30 -> 35 | Pre-established managed flow genuinely dropped by the guard after tunnel loss |
| `missing-tunnel-existing-shared-tcp` evidence | PASS, deny counter 37 -> 42 | Same for the ordinary-namespace session |
| `python3 -m compileall` and `sh -n` launchers | PASS | Syntax only |
| `linux_static_test.py` | PASS, 21 tests | Portable evidence/gate logic |
| `acceptance_harness_unit.py` | PASS, 3 tests | Endpoint validation |
| `server_unit.py` | PASS, 24 tests | Offline VPS profile validation |
| `payload_unit.py` | PASS, 32 tests | Real Linux root ownership, flock and atomic exchange |
| `provision_unit.py` | PASS, 18 tests | Mocked host boundaries only |
| `windows-unit.ps1` (Windows PowerShell 5.1) | PASS, 86 assertions | Mocked host boundaries; no WSL, firewall, DNS or route mutation |

Evidence: `ClashOfRust-network-backups/wsl-local-20261018/linux-egress-integration-20261017-221209.json`
(`outcome=passed`, 86 cases). Run `39063692775` is recorded as the failure it
was; it is not replaced or reinterpreted.

The same change set fixes an installer defect found by the independent
read-only Windows review: the uninstall gate compared the live
`ClashOfRust.WslEgress` image path against the copy next to the script, while
the reviewed helper package is what installs the service under
`%ProgramFiles%\Clash of Rust WSL`. Any uninstall or ordinary reinstall on this
host would therefore have stopped with exit code 2 forever. The gate now
accepts exactly the two protected package locations and still requires a
`LocalSystem` account, an exact `--service` command line and a journal that
reached `restored`. The live service on this host
(`"C:\Program Files\Clash of Rust WSL\clash-wsl-guardian.exe" --service`,
`LocalSystem`) is accepted by the new check; the deletion itself was not
executed, because the active generation must keep its guardian.

The review also raised two follow-ups that are **not** fixed here and are
tracked for a later change: update mode (`/S /UPDATE`) does not run the WSL
guard pre-check that the ordinary path runs and restores only the main
executable if extraction fails, and several accepted-but-unused broker
parameters (`-Distro`, `-EndpointIPv4`, `-EndpointPort`) are forwarded from the
GUI without being read.

## 2026-10-18 independent reviews and install-path fixes

Two read-only review agents inspected commit `8b7fe19`: one for the Windows
side (broker, firewall, installer, GUI) and one for the Linux side (manager,
harness, contract). Neither found a P0 or an exploitable fail-open path. The
Windows review raised two P1 items; the uninstall half was already fixed in
`8b7fe19`, and this change set fixes the install half.

Fixes in this change set:

| Item | Fix |
| --- | --- |
| Guardian service identity at install time | A service already registered by either canonical package (`%ProgramFiles%\Clash of Rust WSL` or `%ProgramFiles%\Clash of Rust`) is adopted after the account, image path and automatic start mode are verified, instead of aborting the installation on a path comparison against the running copy |
| Alternate data stream profile path | `New-WslEgressUnixPath` rejected a second colon only when it appeared at index 1, so `C:\Private\egress.json:ads` was accepted; any colon after the drive colon is now refused |
| Stale documentation | `linux/CONTRACT.md` said 14 portable tests (now 21); `SECURITY_MODEL.md` claimed Linux rejects Tailscale install and start (Linux installs a closed deny guard and only `start` refuses the backend) |

Executed after the changes on this host (Windows PowerShell 5.1):

| Check | Result | Scope |
| --- | --- | --- |
| `tests/windows-unit.ps1` | PASS, 100 assertions (was 86) | Mocked host boundaries; the guardian service boundary is replaced by fixtures, no service, WSL, firewall, DNS or route was touched |
| New install-adoption assertions | PASS | A `LocalSystem` service from either canonical package is adopted without recreating it, is started once, and a foreign image is refused and never started |
| Extended-service-path assertion | PASS | The command line the installer writes is accepted by the ownership predicate that uninstall uses |
| Alternate-stream and network path assertions | PASS | `C:\Private\egress.json:ads` and `\\server\share\egress.json` are refused (`GetFullPath` returns the stream path unchanged, so the check, not the framework, is what refuses it) |
| PowerShell parser (module, broker, uninstall gate, guardian audit, helper install, unit suite) | PASS | Syntax only |

Not executed and not claimed: the live guardian service was not re-registered,
no WSL distribution was started, and no firewall, DNS, route or Hyper-V object
was modified by these tests.

## 2026-10-18 CI, package verification and the local install gate

Head commit `19ea903` on the `Sonder4` fork was verified by GitHub Actions, and
the same commit was packaged into an installer that was downloaded and
hash-checked on this host.

| Check | Result | Evidence |
| --- | --- | --- |
| `WSL Secure Egress` workflow, push | PASS | run 39067005355 on `19ea903` |
| `CI` workflow, push | PASS | run 39067005271 on `19ea903` |
| `CI` workflow, pull request | PASS | run 39068106473 on `19ea903`, then run 39082984419 after the documentation commit `0003cd6`; both events are `pull_request` with all ten `CI_JOBS` successful |
| `WSL Secure Egress` workflow, pull request | PASS | run 39082984425 on the branch head `0003cd6` |
| `Publish Release` test-only packaging | PARTIAL | run 39070338528: `package-windows-x64` PASS; `package-windows-arm64` cancelled and `Publish verified release` skipped because `check_only` was set |
| `Clash-of-Rust-0.6.0-windows-x64-setup.exe` | PASS | SHA-256 `5e282cdc36e4d8e966c6b812a38aa964a431842d47d2d6ae0fcc728d80e3e84a` recomputed locally and identical to the published `.sha256`; `package-receipt.json` names commit `19ea903…` and CI run 39068106473 |

Local update attempt on this host, recorded from the commands that ran:

| Step | Result |
| --- | --- |
| Read-only baseline before the update | PASS. Installed package was 0.5.11 in `D:\Clash of Rust` with no `resources\wsl-secure-egress`; `ipgw.neu.edu.cn` and `api.okinto.com` both returned HTTP 200; guardian audit `policy_matches: true`; broker status `host_policy PASS`, phase `connected_unverified`, backend WireGuard. Evidence: `ClashOfRust-network-backups\install-20261018\pre-install-verification.json` |
| User-data snapshot before the update | PASS. `%LOCALAPPDATA%\clash-of-rust\ClashOfRust` copied to `ClashOfRust-network-backups\install-20261018\userdata-pre-update` (13 files, 37,529,812 bytes). The EFS-encrypted `%LOCALAPPDATA%\ClashOfRust\WslEgress` profile was left untouched and is not part of the copy |
| Graceful client exit through `Local\ClashOfRust.Exit` | PASS. The GUI exited in 6.5 s and stopped its core; both mandatory URLs still returned HTTP 200 with the client stopped |
| Elevated uninstall and install of the verified package | BLOCKED, environment. `Start-Process -Verb RunAs` returned `ERROR_CANCELLED` four times; `consent.exe` was observed running, so the consent dialog was raised and then dismissed without an answer. Creating a `Highest` run-level task is refused for the filtered token (`Access is denied`). Completing a per-machine install therefore needs one interactive consent click |
| Fork build running locally, WSL page visible, copied admin command | NOT_RUN — depends on the blocked step |
| Reboot, sleep/resume and physical network switch | NOT_RUN |

The package, hash and CI conclusions above were read back from the GitHub API;
the local rows are the recorded output of the commands that actually ran.

The packaged installer corresponds to `19ea903`. The commits listed after it on
this branch edit this test record only, so they do not change the installed
binary; any further commit that touches this document alone has the same
property.

## Acceptance rule

Positive connectivity requires a receiver-observed nonce, the expected source
and counter evidence. A send success or timeout alone is not proof. A negative
test requires a deny counter, no receiver nonce and unchanged direct-path
counters. When a fixture would otherwise let the kernel refuse a flow before
nftables sees it, the fixture must be extended until the traffic really reaches
the guard; weakening the rule instead is not an acceptable fix. Real production
fault injection is not authorized.

Update this document after GitHub Actions and any approved VPS/WSL acceptance
with the exact commit, run URL and per-scenario result.
