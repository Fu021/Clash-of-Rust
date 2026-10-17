# Security model

## Scope

This feature protects outbound traffic from enrolled WSL2 distributions and
native Linux container forwarding inside those distributions. It keeps the
Windows host's adapters, IPv4/IPv6 connectivity, DNS and existing Clash/Mihomo
service outside the tunnel. Protected Linux workloads intentionally have
IPv4-only Internet access. WSL has no NEU, domestic-site, campus, LAN, or
Windows-host direct exceptions: every permitted Internet flow uses the fixed
VPS path, and IPv6 is denied rather than routed locally.

## Enforcement layers

1. **Persistent Windows boundary.** Hyper-V Firewall is configured for the
   owned WSL VM creator with default inbound/outbound block, host-policy merge
   disabled and loopback disabled. The sole transport exception is an exact
   fixed IPv4/UDP endpoint tuple. This boundary is applied before the first
   Linux helper mutation.
2. **Docker containment.** A project-owned WFP provider adds persistent,
   executable-specific ALE blocks for the Docker host backend and its current
   descendant processes. Docker data is preserved. This is containment, not
   evidence that Docker has functional protected egress.
3. **Linux shared guard.** The manager creates an owned nftables table with
   default-drop output and forward chains. It opens only the encrypted endpoint
   path and the controlled business-veth path. Ordinary shared-namespace WSL
   traffic and registered native bridge sources are admitted only by source,
   interface and direction. No global established/related bypass exists.
4. **Tunnel gateway.** A dedicated transport namespace owns the encrypted
   WireGuard socket. The WireGuard interface is moved to a workload gateway
   namespace whose remaining interfaces are the business veth and loopback.
   Business traffic is source-NATed to the tunnel address. IPv6 is dropped.
   Port 53 is restricted to the fixed tunnel resolver.
5. **Application isolation.** `secure-egress run` enters the gateway network
   namespace and creates new user, mount, PID, IPC, UTS and cgroup views. It
   drops UID/GID privileges, clears supplementary groups and capabilities,
   sets `no_new_privs`, hides Windows mounts and host sockets, and refuses
   inherited directory/socket file descriptors.

The Windows boundary remains the outer fail-closed control. A Linux manager
failure, transport loss, GUI exit or audit-service failure does not enable
direct Windows networking for WSL.

## Boot and reconnect

The Windows policy is persistent. The Linux manager installs a boot hook that
recreates the owned default-deny table after a new boot but never opens the
transport. Start requires a fresh five-minute Windows attestation derived from
the current distribution set, per-distribution configuration audit, effective
Hyper-V policy, `.wslconfig` values and Docker containment.

The attestation is an orchestration assertion, not packet evidence. Cold boot,
network switching, sleep/resume, real WSL coverage and real VPS geography remain
separate acceptance gates until executed.

## Tailscale and WireGuard

Tailscale is preferred by the product requirement, but a heartbeat, GUI
ordering or asynchronous exit-node callback cannot prove that the outer
boundary exists before Linux networking starts. Both Windows and Linux
therefore reject Tailscale installation/start in this branch.

The supported backend is WireGuard with a fixed numeric IPv4 endpoint, UDP port
and peer public key. DNS bootstrap does not occur. Runtime failure does not
switch backends, broaden the endpoint rule or restore direct access.

## Trust assumptions and exclusions

- A hostile Windows administrator or Linux root can replace the enforcement
  components and is outside the protected-user threat model.
- A privileged process inside the workload can change its own networking and
  is outside the launcher boundary. The launcher is for unprivileged apps.
- Docker Desktop receives network containment only until its independent host
  backend adapter is implemented and verified.
- DoH/DoT by applications is carried through the tunnel; only port-53 resolver
  use is pinned. No claim is made that encrypted DNS is forced to the VPS
  resolver.
- IPv6-only destinations are unsupported.
- Physical fault injection on the user's live network is prohibited. Fault
  tests run only in disposable CI namespaces or with verified independent
  access.
- No component may inspect or modify `C:\Users\xuan\.cc-switch`.

## Secret handling

The private profile reaches Linux through stdin. WireGuard keys are written
atomically to root-owned `0600` files. Command arguments, stderr, raw endpoint
values, controller credentials and full subscription URLs are excluded from
public reports. A payload manifest is an integrity check, not a code signature.
