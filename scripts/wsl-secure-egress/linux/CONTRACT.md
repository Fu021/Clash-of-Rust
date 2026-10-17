# Linux helper contract (schema 1)

This is a deployable prototype, not a production acceptance report. No local WSL,
SSH, network change, package installation or kernel test is authorized by reading
this file. Linux kernel integration is explicitly gated to disposable CI.

## Payload preparation

The user supplies the existing `COR-Egress` rootfs. No automatic download/import.
It must provide Python 3, iproute2, nftables, kernel WireGuard support,
wireguard-tools, util-linux (`setpriv`) and bubblewrap supporting `--disable-userns`.
The distro must permit mount/PID/IPC/user namespaces. No packages are fetched by
the helper. WireGuard kernel support must already be available.

Trusted Windows code stages LF-normalized `manager`, `manager.py`,
`secure-egress`, `cor-egress-manager` and `install-payload.py` under a root-owned
Linux directory with no group/world write. A separately trusted root-owned JSON
manifest maps exactly the four runtime filenames (excluding the installer) to
SHA256 digests. Invoke:

```text
/usr/bin/python3 -I /root/private-stage/install-payload.py --payload /root/private-stage --manifest /root/private-stage/sha256.json
```

Windows must independently verify the installer hash before executing it; a
manifest bundled by an untrusted source is not a trust anchor. Existing owned
identical payload is idempotent; foreign files or changed versions are refused.
The installer does not start services, import a distro or change networking.

## Private profile

Windows delivers this object to **stdin**, never argv. Values below are
documentation fixtures; `private_key` and `peer_public_key` must be real valid
base64-encoded 32-byte keys in a deployment. `manager_distro` and other Windows
orchestration fields may accompany the Linux fields.

```json
{
  "schema": 1,
  "backend": "WireGuard",
  "manager_distro": "COR-Egress",
  "endpoint_ipv4": "192.0.2.2",
  "endpoint_port": 51820,
  "private_key": null,
  "peer_public_key": "BASE64_PUBLIC_KEY_32_BYTES",
  "tunnel_address": "10.77.0.2/32",
  "resolver_ipv4": "10.77.0.1",
  "uid": 10000,
  "gid": 10000
}
```

Backend is case-insensitive `WireGuard` or `Tailscale`. Tailscale start is always
Blocked pending a proved strict cold-start design. Endpoint is an IPv4 literal;
no bootstrap DNS. Resolver is the fixed IPv4 VPS tunnel resolver. IPv6 is denied.
UID/GID must be integer >=1000; workload commands never root-execute.
Optional `forward_sources` is a list of `{ "interface": "docker0",
"source_cidr": "172.17.0.0/16" }` objects for existing dedicated container
ingress interfaces. Existing native bridge connected subnets are also discovered
automatically. New bridges require a stop/start refresh; Docker Desktop's separate
VM remains outside this Linux coverage.

## Calls and order

The fixed executable is `/usr/local/lib/cor-egress/manager`. Windows uses
`wsl.exe -d COR-Egress -u root --exec <helper> <action> ...`. All actions accept
`--profile /etc/cor-egress/profile.json --generation N`. **Keep installation
generation N stable across start/stop**. A mismatch blocks the action.

1. Windows gets critical host connectivity baselines/backups and installs the
   persistent Hyper-V default deny before starting any distro. Rootfs preparation
   is explicit. Existing boot command integration must be reviewed in advance.
2. `install --generation N --acknowledge-shared-guard` reads profile stdin;
   creates root-private profile/key/journal and installs shared OUTPUT/FORWARD
   default deny **before** touching DNS or configuring network interfaces.
   Existing profile/table/namespace ownership is never assumed. No transport
   exception is created at installation. Shared `eth0` is never moved.
3. In **each other distro**, install the same trusted payload then invoke
   `configure --generation N` with the same profile stdin. It owns only local
   `/etc/wsl.conf` (disable generated resolver and Windows interop) and
   `/etc/resolv.conf` (fixed tunnel resolver), with snapshots and CAS restoration.
   It creates no WireGuard device and no shared firewall. Windows owns distro
   enumeration, shutdown/restart, `.wslconfig` DNS tunneling, and effective audit.
4. Manager install sets `[boot] command=/usr/local/lib/cor-egress/manager guard`.
   An existing different boot command is refused, not overwritten. Boot `guard`
   only installs/reinstates deny policy and never opens an endpoint. WSL boot
   hooks alone are **not** evidence of no pre-hook traffic: Windows persistence
   and effective policy remain external prerequisites. Cold-start acceptance
   stays NOT_RUN until separately tested; the prototype may prepare a connected
   but Unverified backend before that acceptance is complete.
5. `start --generation N` consumes fresh root-supplied scope attestation stdin,
   or accepts `--scope-attestation /root/private-scope.json`. Schema:

```json
{
  "schema": 1,
  "generation": 123,
  "issued_at_unix": 0,
  "all_distros_managed": true,
  "hyperv_default_deny": true,
  "dns_tunneling_disabled": true,
  "interop_disabled": true,
  "effective_audit_passed": true
}
```

   Use the installation generation and actual UTC Unix seconds; maximum age
   five minutes. These are privileged Windows orchestrator assertions, not Linux
   discoveries or end-to-end proof. Unknown/missing/false current coverage is
   Blocked. `cold_start_guarded` may accompany the object but is not an artificial
   requirement to claim an unperformed cold-start test.
   `start` creates a dedicated transport namespace/veth, gives it only a fixed
   IPv4/UDP endpoint route, creates wg0 there and moves wg0 into the workload
   gateway namespace. A second business pair `cor-eb0`/`cor-bg0` carries ordinary
   shared WSL workloads to the gateway, whose real interfaces are `lo`, `wg0`,
   `cor-bg0`. Reserved policy table `10077` and priorities `10070`–`10073` route
   IPv4 business traffic into this pair; only precisely matched transport and
   business-return ingress consult main. The table has an unreachable default
   plus the preferred gateway route, preventing fallback to main after link loss.
   Root OUTPUT permits necessary loopback and the business veth with owned local
   sources. Root FORWARD permits registered container sources and the encrypted
   endpoint tuple with scoped returns. Gateway FORWARD permits business to wg0
   only, restricts TCP/UDP53 to the fixed resolver, and SNATs to tunnel IPv4;
   returns match wg0/business interfaces, destinations and connection state.
   There is no global established allowance. Per-interface forwarding avoids
   changing global ip_forward host/router defaults. Shared eth0 stays in place,
   and legacy bound sources receive explicit gateway return routes. These
   narrow paths are opened only after all backend setup succeeds.
   **No handshake is required before Windows opens the endpoint**. Windows
   enables the exact endpoint only after successful Linux preparation and its
   own effective audit. Neither side reports verified from this preparation.
6. `run --generation N [--scope-attestation /root/private-scope.json] -- /usr/bin/program ...`
   uses bwrap and a new allowlisted filesystem, PID/IPC/UTS/user namespace,
   dropped capabilities and no_new_privs. No `/mnt`, home/workspace bind,
   `/run`, host `/proc`, host sockets or netns handles. Resolver is readable and
   fixed. Existing Windows mounts remain hidden. Supplementary groups are
   cleared. Actual host UID/GID drop happens before bwrap creates a user namespace;
   a namespace-local nonzero UID therefore cannot map to host root. All
   capabilities are dropped before execution. Stdio must be a pipe, TTY or
   `/dev/null`, preventing host descriptor paths via `/proc/self/fd`. Workload
   stdout stays stdout; the final status JSON is on stderr for this action only.
7. `stop --generation N` first closes the root transport exception, then deletes
   only owned namespaces/veth. If workloads remain, it retains namespaces and
   reports Blocked; business and outer paths are already denied. The management
   lock is released while workloads execute so stop can withdraw their path.
   It **retains the shared
   deny guard** and never flushes global firewall rules. Windows must close its
   endpoint first as well.
8. `audit-config --generation N` returns current `facts.owned_config_matches`,
   `facts.interop_runtime_disabled`, `facts.shared_netns` and generation. Windows
   can combine these per-distro facts with effective host policy, NAT and DNS
   tunneling checks; these facts do not fabricate cold-start acceptance.
   `status` inspects local state; `test` inspects invariants and reports Blocked
   because no external/target probe is implicit. Neither means end-to-end PASS.
9. `restore-config --generation N` in each nonmanager distro restores owned
   files only if the current bytes still match the written hashes; original
   resolver symlinks and regular file modes are preserved. In the manager,
   `rollback|uninstall --generation N --release-guard` stop, restore owned distro
   files and remove only its own table. Root-private recovery backups, installed
   payload and secrets are preserved for deliberate review/cleanup. Windows
   closes its transport first and restores host policy only after all Linux
   restoration and workload withdrawal are complete. No automatic broad reset.
   Repeated rollback is idempotent. A subsequent install privately archives the
   retired generation/profile/key before accepting a new profile. A retired
   generation's boot hook cannot revive it.

## Result and limitations

One JSON object on stdout, except `run` uses stderr. Fields:
`schema:1, schema_version:1, generation, action, outcome, protection, phase,
backend, message, checks, logs`. Outcomes: `succeeded`, `blocked`, `failed`.
Protection is `Unverified` or `Blocked`; **never verified**. Exit 0 means the
requested local action succeeded, exit 2 means Blocked/failed. stderr and command
arguments never disclose keys or endpoint values. `run` workload output belongs
to the caller and is not included in status logs.

Only trusted root can modify namespaces/firewalls/attestations; hostile root,
privileged Docker and Windows-side arbitrary execution are outside the workload
boundary. Ordinary unwrapped shared-namespace IPv4 tasks and registered native
container sources route through the business veth and wg0, without launcher or
proxy variables. The launcher adds interface visibility and filesystem/process
isolation; a full developer home, GUI, Docker Desktop, workspace mounts and host
socket bridges are absent from that launcher. IPv6 is a documented
functional limitation. Production Windows cold-start, cross-distro behavior,
receiver-observed source/geolocation and critical host connectivity are separate
acceptance gates; Linux CI cannot prove them.

Failed mutations attempt an independent emergency OUTPUT/FORWARD deny while
preserving drifted tables. Interrupted nft/namespace/policy mutations may require
deliberate reconciliation when ownership cannot be proved. They are never
silently adopted, globally flushed or reported restored. Backups are preserved.
Boot reconciliation archives vanished previous-boot runtime records only when
all reserved object names are absent; collisions remain Blocked.

## Isolated CI entry points

Portable evidence/gate checks (14 tests, no subprocess or socket is executed):

```text
python -B scripts/wsl-secure-egress/tests/linux_static_test.py
```

Kernel integration requires an explicitly disposable Linux runner with dependencies
already installed, kernel WireGuard and unprivileged user namespace support:

```text
sudo env CI=true COR_EGRESS_ISOLATED_CI=1 python3 -B scripts/wsl-secure-egress/tests/linux_integration.py --execute-isolated
```

All three gates are mandatory. The launcher enters fresh net/mount/PID namespaces
before fixture work and verifies namespace identity and a loopback-only initial
link set. `/etc`, `/run`, `/var/lib`, `/var/log`, `/mnt` are private tmpfs mounts.
It never changes the runner's default namespace firewall. The fixture supplies
its own VPS WireGuard peer, TCP/UDP echo and DNS receivers. Random nonce receipt,
receiving-interface counters, encrypted transport counters and positive controls
are required: send success or timeout alone cannot pass a negative case.

Coverage includes ordinary shared TCP/UDP/DNS, fixed legacy source bindings,
registered container forwarding, restricted launcher isolation, existing/new
connections, unauthorized DNS, IPv6, missing tunnel with a deliberately usable
fallback route, stop with live workloads, retained guard and explicit rollback.
The JSON test report labels Windows/WSL/Hyper-V, cold boot and real VPS/geography
NOT_RUN. CI must not turn those labels into production verified status.
