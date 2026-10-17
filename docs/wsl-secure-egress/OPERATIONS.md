# Operating guide

## Before changing the host

1. Read [the security model](SECURITY_MODEL.md) and
   [compatibility guide](COMPATIBILITY.md).
2. Verify that normal Windows IPv4/IPv6 and the existing Clash/Mihomo service
   work. Configure the private mandatory-connection baseline described in the
   [project README](README.md).
3. Prepare `COR-Egress` and stop every WSL distribution. Save all Linux work.
4. Prepare the private profile and dedicated VPS service. Never put the
   profile, keys, SSH credentials or real endpoint into this repository.
5. Run the page's **Doctor** and **安装预检** actions. A preview changes no files,
   firewall objects, WSL runtime, routes or DNS.

## Install

Run **执行安装** from an administrator terminal using the reviewed command
shown by the client. Installation performs these operations in order:

- validate the private profile and fixed endpoint;
- record the immediate Hyper-V and Docker object snapshots;
- apply the persistent Windows default-deny boundary and Docker containment;
- install and verify the fixed SYSTEM audit service;
- snapshot and merge only owned `.wslconfig` fields;
- stage and hash-pin the Linux runtime payload;
- install the manager's shared deny guard;
- configure every ordinary WSL distribution and stop it to reload interop and
  resolver settings;
- audit the effective scope while the Windows boundary remains closed.

Failure leaves the outer boundary closed where it was installed. Do not rerun
installation over an existing journal. Inspect `%ProgramData%\ClashOfRust\WslEgress`
and use the reviewed recovery path.

## Connect and verify

**连接** runs the Windows scope audit, sends a short-lived attestation to the
Linux manager, creates the WireGuard namespaces and policy routes, and only then
enables the exact Windows endpoint. Enabling and verification are one
compensated transaction: a failed post-enable check closes the endpoint again.

**断开 · 留保护** closes the Windows endpoint before stopping Linux transport
objects. The shared deny guard remains. Closing the GUI does not remove
protection.

The Test action inspects local invariants only. Cold boot, sleep/resume, network
switch, actual German geolocation and real receiver-source evidence require
separate executed tests and must not be inferred from a successful connection.

## Logs and evidence

The UI displays bounded, redacted reports. Runtime state and snapshots live
under `%ProgramData%\ClashOfRust\WslEgress`; completion receipts live under
`%LOCALAPPDATA%\ClashOfRust\WslEgress\results`. Never attach private profiles,
WireGuard keys or raw packet captures to an upstream issue.

Use only disposable runners for kernel tests:

```sh
CI=true COR_EGRESS_ISOLATED_CI=1 python3 \
  scripts/wsl-secure-egress/tests/linux_integration.py --execute-isolated
```

The suite labels real Windows/WSL, Hyper-V, cold boot and VPS geography
`NOT_RUN`; its PASS applies only to the isolated Linux data-plane model.

## Recover

Default **回退 · 保留保护** closes the endpoint and leaves the guard installed.
The destructive **恢复原网络…** action requires a separate confirmation and all
WSL distributions to be stopped. It then restores per-distribution files,
removes only the owned Linux table and namespaces, restores the exact
`.wslconfig` snapshot, removes Docker containment and restores the immediate
Hyper-V settings.

Restore refuses when an object has changed since this generation wrote it.
Never clear the whole firewall, reset WSL, delete distributions or overwrite
later edits to force recovery. Preserve the journal and recovery files for
manual review.

## Uninstall

Uninstall is allowed only after the journal reaches `restored` and the fixed
audit service matches its expected path. The installer does not remove Docker
or WSL data. If the check fails, uninstall stops and retains the recovery
tooling.

## VPS service

The dedicated VPS service under
`scripts/wsl-secure-egress/server` is separate from existing proxy and
subscription services. It changes only its own interface, nftables table,
private DNS process and the IPv4 forwarding sysctl. Deploy only after reviewing
the XUAN VPS reference and taking a current backup. A server operation is not
performed by the Windows client automatically.
