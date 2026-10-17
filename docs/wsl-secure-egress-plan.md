# WSL Secure Egress implementation plan

Status: accepted implementation specification, not a deployment or test report.
Baseline: upstream v0.5.11, commit `78d5f548389fe80056027c03416210f16106917e`.
Upstream and fork main were fetched and compared before implementation resumed;
all three refs matched. Software versions and live network state must be checked
again before deployment.

## Goal and accepted decisions

Add a native **WSL Secure Egress** navigation page to the Rust/Slint client. The
page manages installation, transport, application launching, distribution
coverage, diagnostics, logs, acceptance tests, rollback and uninstall.

- Keep Windows IPv4, IPv6, ordinary applications and existing proxy service
  working. Do not enable a Windows-wide VPN or Exit Node.
- Protect egress for the current user's WSL2 workloads, including terminal,
  background service and Linux forwarding paths.
- Use IPv4 for protected business traffic. Deny WSL IPv6 egress; retain Windows
  IPv6. IPv6-only destinations are intentionally unsupported.
- Applications launched through the page or dedicated command see only their
  isolated namespace's virtual interfaces, addresses and routes.
- Prefer Tailscale with one explicitly selected German Exit Node. If strict
  cold-start protection cannot be established, keep that mode blocked and use
  the separately configured fixed-endpoint WireGuard backend.
- Preserve Docker Desktop's installation, images, volumes and settings. Its
  external network remains blocked until its independent host-backend path is
  adapted and tested. Blocking is not a successful container connectivity test.
- Develop in a fork, test with GitHub Actions, build a custom Windows package,
  and submit an upstream PR after required checks pass. Do not publish a release
  or replace a production installation as an incidental build step.

The referenced external prompt attachment was unavailable during planning. This
file records the decisions confirmed in the conversation; compare any later
attachment before accepting additional requirements.

## Architecture

The native client has a per-user WSL broker and a separate privileged Windows
guard boundary. SYSTEM must never execute the user's general WSL commands.
Persistent host policy, rather than the GUI or a polling watchdog, controls the
outer egress boundary.

Use a minimal `COR-Egress` management distribution for owned Linux services and
transport state. A separate distribution is not itself a network security
boundary: WSL networking infrastructure can be shared.

Ordinary WSL traffic and Linux container forwarding enter an owned gateway.
Explicit namespace/interface/source restrictions prevent business traffic from
being forwarded directly to the underlay. A dedicated launcher additionally
creates isolated application network, mount, PID and IPC views.

Implementation responsibilities:

1. Cover shared-namespace OUTPUT, container FORWARD, and application namespaces.
2. Establish deny rules before starting transport or untrusted applications.
3. Restrict return traffic by interface, destination and connection state; no
   blanket established/related accept before the anti-bypass rules.
4. Deny gateway local proxy, DNS and control API delegation paths.
5. Preserve unrelated firewall objects. Never flush a global ruleset.
6. Disable forwarding offload for protected traffic. Record resource ownership,
   configuration generation and private recovery snapshots.
7. Drop application privileges, capabilities and inherited descriptors. Hide
   host mounts, namespace handles, Docker/D-Bus/control sockets and Windows
   interop. Do not delegate browser requests to an existing outside instance.
8. Keep management and protected traffic routes separate to avoid tunnel loops.

### Tailscale backend

Run tailscaled in a dedicated trusted gateway namespace. Pin the Exit Node's
identity; disallow automatic exit selection, unintended LAN access and other
tailnet/subnet routes. Prove source translation and return routing for forwarded
application traffic. Review Tailscale netfilter interaction with independent
project drop chains.

The strict backend is gated on a supported cold-start boundary that remains
safe when Linux guard state is absent or destroyed. Dynamic coordination,
DERP/STUN and peer transport permissions must not become plaintext application
exceptions. GUI startup ordering, a heartbeat and asynchronous event callbacks
are insufficient. If that boundary cannot be verified, report BLOCKED and do
not open business traffic.

### WireGuard backend

Use a fixed numeric IPv4 Endpoint, UDP port and peer public key. Create the
WireGuard interface in the transport namespace, then move it into the protected
application namespace, retaining its encrypted UDP socket in its birthplace.
Ordinary WSL workloads use the controlled gateway. Applications must not be able
to exploit the outer transport allowance themselves.

Backend selection is explicit. A runtime outage never changes the backend,
widens firewall permissions or restores direct internet access automatically.

### DNS, IPv6 and VPS

Use a project resolver and dedicated resolver/NSS configuration. DNS upstream
traffic follows the tunnel to a private VPS service, not host DNS tunneling or
campus/host fallback. Cover shared Unix resolver sockets and application DoH/DoT.
No publicly open recursive resolver. Pin and validate actual VPS IPv4 egress;
configuration/node identity changes invalidate previous verification.

Deny IPv6 literals, AAAA-only attempts, dual-stack racing and forwarded IPv6.
Limit the dedicated VPS workload's egress to IPv4 without changing unrelated
VPS services. Neutral test receivers are the default; no accounts, model calls
or Claude/Anthropic requests are needed for acceptance.

### Windows and Docker

Use supported WSL NAT and Hyper-V Firewall mechanisms. Migration changes shared
WSL settings and requires a coordinated window; verify the effective mode.
Disable WSL autoProxy and host DNS tunneling for the managed configuration.

Inspect ActiveStore, host-policy merge, existing broad allows and loopback.
Use default deny plus reviewed permissions, not a blanket Block rule followed
by an ineffective Allow exception. Policy conflicts stop deployment.

Docker Desktop can originate container connections from its Windows backend;
Linux nftables alone cannot prove its containment. Implement and independently
verify its host restriction, including loopback-proxy bypass. Do not delete
Docker or its data.

## Native interfaces and lifecycle

Append the navigation page without renumbering existing pages. Show transport
connection, German exit verification, effective protection and test status
separately. Show per-distribution coverage and Docker's blocked limitation.

Actions: doctor/status, installation preview/apply, connect, disconnect while
retaining protection, launch application/terminal, test, rollback and uninstall.
Closing the GUI does not remove protection. Restoring original network access
is a distinct operation.

Add independent configuration, observed-status, operation-result and event
types. Include operation identity and generation. Ignore stale results after
reconnection, reconfiguration, backend changes or rollback. Unknown,
requested-blocked and verified-blocked are different states.

State progression:

`uninstalled -> preflight -> installed/closed -> connecting -> verifying -> protected`

Failures return to a closed/requested-closed state with explicit evidence; do
not claim a verified block merely because an operation failed.

PowerShell and Linux scripts expose the same controlled operations and default
to preview for host mutations. WSL executes a fixed helper; do not concatenate
user commands into a privileged shell. Secrets never appear in argv, normal
logs, UI exports or Git. Logs are bounded and diagnostic exports are redacted.

## Implementation phases and exit gates

1. **Baseline and specification:** inspect current state, preserve user changes,
   verify mandatory Windows connectivity, establish exact object ownership,
   snapshot and targeted recovery before each network change.
2. **UI and offline controller:** implement page, state, progress, logs,
   configuration compatibility, generation and redaction tests without touching
   production network state.
3. **Linux prototype:** test isolated namespaces, routes, TCP/UDP/DNS, IPv6 denial,
   application restrictions, missing tunnel, partial initialization and cleanup
   using controlled upstream namespaces.
4. **Persistent Windows boundary:** test authorization, repeated install,
   effective policy, GUI/broker/service exit, missing Linux guard, cold start,
   and Docker host restriction. Unsupported windows remain BLOCKED.
5. **VPS and backend:** perform authorized remote inspection, back up precise
   objects, deploy dedicated identity/IPv4/DNS while preserving existing services,
   evaluate the Tailscale gate and independently validate WireGuard if needed.
6. **WSL cutover:** prepare a maintenance window, migrate owned configuration,
   establish host denial before starting Linux, enroll ordinary workloads and
   application launcher, and keep unadapted Docker Desktop externally blocked.
7. **Acceptance and recovery:** execute the full matrix below, preserving
   ordinary Windows connectivity after each step. Physical faults require an
   isolated environment or a verified independent mandatory-access path.
8. **Build and contribution:** use fork Actions for workspace checks and packages,
   preserve other platforms, isolate the custom update channel, validate the
   actual package, then open the upstream PR. Production replacement and public
   release are separate operations.

## Acceptance and evidence

Each check records version, generation, operation, timestamp, path, evidence and
separate functional/security outcomes: PASS, FAIL, BLOCKED, NOT_RUN or
NOT_SUPPORTED. A timeout, existing process or firewall rule is not proof.

Required scenarios:

- Real interface/route views, proc/sys views, privileges, FD/IPC/interop escapes.
- Proxy variables removed: TCP, UDP, nonstandard ports, long-lived connections
  and return traffic from the expected VPS IPv4.
- Uncached system DNS, TCP/UDP53, resolver outage, DoH/DoT and host delegation.
- IPv6 literals, AAAA-only, dual-stack and container forwarding denial.
- Ordinary shells, systemd services, background jobs and cross-distro coverage.
- Docker image pulls/container traffic/host backend and local-proxy bypass.
- Unreachable tunnel, cancelled exit selection, authentication expiry, daemon
  crash, interface removal, existing/new connections and rule recreation.
- GUI/broker/Linux manager exit, WSL cold boot, Windows reboot, login, sleep and
  network changes; keep unsafe/unavailable scenarios NOT_RUN or BLOCKED.
- Normal Windows DNS, TLS certificate validation and HTTP semantics for the two
  locally mandated connections, plus existing Clash regression tests.

Use randomized nonces, receiver positive controls, packet capture and counter
correlation. Keep raw captures and real endpoints private. A protected aggregate
may include a proven-blocked unsupported component, but cannot be described as
all components having working connectivity.

## Rollback and delivery

Before mutation preserve immediate snapshots and file/symlink state. Restore
only objects still matching the project's last written version. Stop owned
isolated sessions before deprotecting. Do not clear unrelated policy or use an
old full backup over newer edits. Disconnect and normal failure cleanup keep
the guard. Preserve offline recovery tooling when uninstall is incomplete.

Deliver the native page/controller, host and Linux scripts, meaningful tests,
README, security model, operating/compatibility guide, test records and recovery
instructions. Do not delete distributions, Docker data, user files or CC Switch
data. Never stop the shared production proxy to manufacture a fault test.

## Primary references

- [WSL networking](https://learn.microsoft.com/windows/wsl/networking)
- [Hyper-V Firewall](https://learn.microsoft.com/windows/security/operating-system-security/network-security/windows-firewall/hyper-v-firewall)
- [Linux network namespaces](https://man7.org/linux/man-pages/man7/network_namespaces.7.html)
- [WireGuard namespace integration](https://www.wireguard.com/netns/)
- [Tailscale transport dependencies](https://tailscale.com/docs/reference/faq/firewall-ports)
- [Docker Desktop networking](https://docs.docker.com/desktop/features/networking/)
