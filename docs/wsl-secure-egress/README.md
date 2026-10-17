# WSL Secure Egress

WSL Secure Egress is a Windows-only, opt-in management surface in this fork of
Clash of Rust. It keeps the Windows host on its existing IPv4 and IPv6 network
while placing protected WSL2 traffic on a fixed-endpoint IPv4 WireGuard path to
a dedicated German VPS workload. IPv6 egress from protected WSL traffic is
denied. A missing or unhealthy tunnel remains closed; it does not restore a
Windows or WSL direct route.

The page is available as the eleventh sidebar item. The command-line broker and
Linux helper expose the same lifecycle. All mutation commands are previews
unless `-Execute` or `--execute` is explicitly supplied, except fixed
root-owned service entry points documented in the Linux contract.

## Status of this branch

The repository implementation and offline tests can be reviewed without
changing the local network. This is not a deployment or production acceptance
record.

- Fixed-endpoint WireGuard is the only installable backend.
- Tailscale remains `BLOCKED` until a supported Windows-side cold-start
  boundary has been implemented and verified with real WSL/Hyper-V restart and
  fault tests.
- Docker Desktop is preserved. Its external networking is not claimed as
  functional until an independently verified adapter exists.
- Windows-wide VPN, global DNS takeover, WSL distribution deletion and CC
  Switch data changes are outside this feature.
- Actual results for CI and local checks are recorded separately in
  [TEST_STATUS.md](TEST_STATUS.md). A test that has not executed is `NOT_RUN`.

## Documentation

- [Security model](SECURITY_MODEL.md) describes trust boundaries, enforcement
  layers and explicit non-goals.
- [Operating guide](OPERATIONS.md) covers prerequisites, install, connect,
  status, tests, recovery and uninstall.
- [Compatibility](COMPATIBILITY.md) records supported Windows/WSL versions,
  network modes and unsupported workloads.
- [Linux helper contract](../../scripts/wsl-secure-egress/linux/CONTRACT.md)
  defines the fixed Windows-to-Linux API.
- [Implementation plan](../wsl-secure-egress-plan.md) preserves the accepted
  design and acceptance gates.

## Trusted resources

The Windows package includes the broker, SYSTEM audit entry point, WFP helper
source, Linux runtime payload and VPS service under
`resources/wsl-secure-egress`. The desktop verifies this complete resource set
against bytes compiled into the client before invoking it. The Windows broker
stages only the five reviewed Linux runtime files into a new root-private WSL
directory, checks their SHA256 values and invokes the separate hash-pinned
payload installer.

Private WireGuard keys and endpoint profiles are supplied through stdin and
stored in root-private files. They must never be committed, pasted into logs or
included in support reports.

## Quick start

Install a reviewed Windows package from this fork, open **WSL Secure Egress**,
and run **Doctor** and **安装预检** first. The full sequence requires an existing
`COR-Egress` WSL distribution with the dependencies listed in the compatibility
guide, a private profile and a dedicated VPS WireGuard service. See the
[operating guide](OPERATIONS.md) before any host mutation.

The mandatory Windows connectivity baseline is deliberately private and is not
part of the upstream repository. Configure it at:

```text
%LOCALAPPDATA%\ClashOfRust\WslEgress\protected-connections.json
```

It must contain a JSON array of credential-free HTTPS URLs and accepted status
codes. No network mutation starts when that baseline is absent or unhealthy.
