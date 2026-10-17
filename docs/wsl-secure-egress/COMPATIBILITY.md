# Compatibility and limitations

## Supported baseline

| Component | Supported baseline |
| --- | --- |
| Host | Windows 11 24H2 with Hyper-V Firewall cmdlets and WSL 2.x/3.x |
| WSL networking | NAT mode; `dnsTunneling=false`, `autoProxy=false`, `firewall=true`, `localhostForwarding=false`, generated resolver and hosts disabled |
| Management distro | An existing ordinary distro named in the private profile, with Python 3, iproute2, nftables, WireGuard kernel support, wireguard-tools, util-linux and bubblewrap |
| Backend | Fixed IPv4/UDP WireGuard endpoint |
| Traffic | All permitted IPv4 TCP/UDP/ICMP and DNS port 53 through a dedicated VPS resolver; no WSL domestic/campus bypass |
| Container forwarding | Existing native Linux bridges with private IPv4 subnets |

The broker requires a positive generation and refuses unknown or changed WSL
configuration instead of adopting it. WSL installation and distro creation are
operator steps; the project does not download or import a rootfs.

## Not supported

- Tailscale strict cold-start mode.
- IPv6 Internet egress from protected workloads.
- Windows-wide VPN or changing normal Windows routes/DNS.
- Docker Desktop functional egress or proof that all Docker host traffic is
  safely adapted.
- Automatic migration of an existing `.wslconfig`, `/etc/wsl.conf` or
  `/etc/resolv.conf` when ownership cannot be proven. `/etc/hosts` is owned
  and restored by the same CAS journal.
- Coexistence with an unreviewed WSL boot command.
- A hostile root/administrator or privileged workload.
- Replacing Windows DNS, disabling DnsCache, resetting network adapters or
  clearing unrelated firewall policy.
- Production fault tests that could disconnect the user's ordinary network.

## Runtime requirements

The manager verifies that required commands exist but does not install them.
The WSL kernel must provide WireGuard and the distribution must permit mount,
PID, IPC and user namespaces. Bubblewrap must support `--seccomp`; newer
versions may additionally provide `--disable-userns`, but that optional flag
is not required because the helper's seccomp filter blocks nested namespace
creation and switching.

Unsupported kernel, WSL version or policy fields produce `BLOCKED`. A timeout
or missing output is never treated as a successful close. Windows Doctor and
Status enumerate registry names only and do not start stopped distributions.

## Update policy

Official automatic updates are disabled in this fork because the upstream
package does not carry the WSL policy owner. Upgrade only with a reviewed fork
package from the same source lineage. A guarded installation is replaced
without deleting its state or recovery resources; run an explicit restore only
after reviewing the current journal.

For a live source upgrade of an installed generation, close the transport with
`Manage-WslSecureEgress.ps1 -Action Stop -Execute`, then run
`Update-LiveWslPayload.ps1 -Execute` as administrator. The script stages the
reviewed payload, uses the installer's atomic reviewed-upgrade gate, reruns
`repair-config`, and refuses to open the endpoint. Run
`Manage-WslSecureEgress.ps1 -Action Start -Execute` afterwards.
