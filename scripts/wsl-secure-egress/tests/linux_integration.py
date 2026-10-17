#!/usr/bin/env python3
"""Destructive Linux tests, exclusively inside a disposable CI namespace.

Entry point (root, disposable Linux CI runner only)::

    CI=true COR_EGRESS_ISOLATED_CI=1 python3 tests/linux_integration.py --execute-isolated

No dependencies are installed and no kernel modules are loaded by this script.
Requires iproute2, nftables, wireguard-tools, util-linux, bubblewrap and Python.
The launcher performs no firewall operations. A fresh net/mount/PID namespace
and private runtime/configuration mounts are mandatory before fixture setup.
All IPs, DNS answers and WireGuard keys are disposable fixture data; no Internet,
WSL, SSH, production daemon or Windows configuration participates in these tests.
Missing capabilities/dependencies are failures, never successful skips.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import time


HERE = Path(__file__).resolve()
MANAGER = HERE.parents[1] / "linux" / "manager.py"
VPS = "cor-egress-fixture-vps"
GATEWAY = "cor-egress-gw"
TRANSPORT = "cor-egress-transport"
GUARD = "cor_egress_guard"
ENDPOINT = "192.0.2.2"
TARGET = "198.51.100.10"
TARGET6 = "2001:db8:77::10"
RESOLVER = "10.77.0.1"
UID = GID = 10000
WORK = Path("/run/cor-egress-integration")
TIMEOUT = 2.0
GENERATION = 1
FIXTURE_SOURCE = "cor-egress-fixture-source"
PAYLOAD_PYTHON = "/usr/bin/python3"


# Kept inline so the managed UID need not read a root-private fixture directory.
# The receiver journals actual application receipt, never just successful send().
RECEIVER = r'''
import json, pathlib, socket, socketserver, struct, sys, threading
folder = pathlib.Path(sys.argv[1])
lock = threading.Lock()
def receipt(nonce, kind, source):
    with lock, (folder / "receipts.jsonl").open("a") as stream:
        stream.write(json.dumps(dict(nonce=nonce, kind=kind, source=source)) + "\n")
        stream.flush()
def dns(data, kind, source):
    if len(data) < 17 or struct.unpack("!H", data[4:6])[0] != 1:
        raise ValueError("invalid DNS question")
    labels, pos = [], 12
    while data[pos]:
        size = data[pos]; pos += 1
        if size > 63:
            raise ValueError("compressed/invalid fixture question")
        labels.append(data[pos:pos+size].decode("ascii")); pos += size
    pos += 1
    qtype, qclass = struct.unpack("!HH", data[pos:pos+4]); pos += 4
    if len(labels) != 2 or labels[1] != "test" or qclass != 1:
        raise ValueError("not a fixture query")
    receipt(labels[0], kind, source)
    if qtype == 1:
        answer = socket.inet_pton(socket.AF_INET, "198.51.100.10")
    elif qtype == 28:
        answer = socket.inet_pton(socket.AF_INET6, "2001:db8:77::10")
    else:
        raise ValueError("unsupported fixture query")
    return (data[:2] + struct.pack("!HHHHH", 0x8180, 1, 1, 0, 0)
            + data[12:pos] + b"\xc0\x0c"
            + struct.pack("!HHIH", qtype, 1, 30, len(answer)) + answer)
class TCP(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.settimeout(30)
        with self.request.makefile("rb") as stream:
            while True:
                if self.server.server_address[1] == 53:
                    prefix = stream.read(2)
                    if not prefix: return
                    if len(prefix) != 2: return
                    size = struct.unpack("!H", prefix)[0]
                    data = stream.read(size)
                    if len(data) != size: return
                    reply = dns(data, "dns-tcp", self.client_address[0])
                    self.request.sendall(struct.pack("!H", len(reply)) + reply)
                else:
                    data = stream.readline(4096)
                    if not data: return
                    nonce = data.rstrip(b"\n").decode("ascii")
                    receipt(nonce, "tcp", self.client_address[0])
                    self.request.sendall(data)
class UDP(socketserver.BaseRequestHandler):
    def handle(self):
        data, sock = self.request
        if self.server.server_address[1] == 53:
            reply = dns(data, "dns-udp", self.client_address[0])
        else:
            receipt(data.decode("ascii"), "udp", self.client_address[0])
            reply = data
        sock.sendto(reply, self.client_address)
class TCP4(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True
class UDP4(socketserver.ThreadingUDPServer):
    allow_reuse_address = True
    daemon_threads = True
class TCP6(TCP4): address_family = socket.AF_INET6
class UDP6(UDP4): address_family = socket.AF_INET6
servers = []
for cls, handler, address, port in [
    (TCP4, TCP, "0.0.0.0", 18080), (UDP4, UDP, "198.51.100.10", 18080),
    (TCP4, TCP, "0.0.0.0", 53), (UDP4, UDP, "10.77.0.1", 53),
    (TCP4, TCP, "192.0.2.2", 51820), (UDP4, UDP, "192.0.2.2", 51821),
    (TCP6, TCP, "2001:db8:77::10", 18080),
    (UDP6, UDP, "2001:db8:77::10", 18080),
]:
    server = cls((address, port), handler)
    servers.append(server)
    threading.Thread(target=server.serve_forever, daemon=True).start()
(folder / "ready").write_text("ready")
threading.Event().wait()
'''


CLIENT = r'''
import json, socket, struct, sys
config = json.loads(sys.argv[1])
def configure():
    global mode, host, port, dns, udp, sock
    mode, host, port = config["mode"], config["host"], config["port"]
    family = socket.AF_INET6 if ":" in host else socket.AF_INET
    dns = mode.startswith("dns-")
    udp = mode in ("udp", "dns-udp")
    sock = socket.socket(family, socket.SOCK_DGRAM if udp else socket.SOCK_STREAM)
    sock.settimeout(config.get("timeout", 2))
    if config.get("bind_host"):
        sock.bind((config["bind_host"], 0))
def read_exact(size):
    result = b""
    while len(result) < size:
        data = sock.recv(size-len(result))
        if not data: raise OSError("unexpected EOF")
        result += data
    return result
def probe(nonce):
    result = dict(nonce=nonce, sent=False, reply=False)
    try:
        if dns:
            ident = bytes.fromhex(nonce[:4])
            name = bytes([len(nonce)]) + nonce.encode() + b"\x04test\0"
            packet = (ident + struct.pack("!HHHHH", 0x100, 1, 0, 0, 0)
                      + name + struct.pack("!HH", config.get("qtype", 1), 1))
        else:
            packet = nonce.encode() + (b"" if udp else b"\n")
        if udp:
            sock.sendto(packet, (host, port)); result["sent"] = True
            reply, peer = sock.recvfrom(4096)
            if peer[0] != host or peer[1] != port:
                raise OSError("reply source mismatch")
        else:
            sock.sendall(struct.pack("!H", len(packet)) + packet if dns else packet)
            result["sent"] = True
            reply = read_exact(struct.unpack("!H", read_exact(2))[0]) if dns else read_exact(len(packet))
        if dns:
            expected = socket.inet_pton(socket.AF_INET6, "2001:db8:77::10") if config.get("qtype") == 28 else socket.inet_aton("198.51.100.10")
            result["reply"] = (len(reply) > 12 and reply[:2] == ident
                               and reply[2:4] == b"\x81\x80"
                               and reply[6:8] == b"\0\x01" and reply.endswith(expected))
        else:
            result["reply"] = reply == packet
    except OSError as error:
        result["error"] = type(error).__name__
    return result
def once():
    try:
        if not udp: sock.connect((host, port))
        result = probe(config["nonce"])
    except OSError as error:
        result = dict(nonce=config["nonce"], sent=False, reply=False, error=type(error).__name__)
    return result
if config.get("worker"):
    print(json.dumps(dict(ready=True)), flush=True)
    for line in sys.stdin:
        config = json.loads(line)
        configure()
        print(json.dumps(once()), flush=True)
        sock.close()
elif config.get("persistent"):
    configure()
    sock.connect((host, port))
    print(json.dumps(dict(ready=True)), flush=True)
    for line in sys.stdin:
        print(json.dumps(probe(line.strip())), flush=True)
else:
    configure()
    print(json.dumps(once()), flush=True)
if not config.get("worker"):
    sock.close()
'''


SANDBOX_AUDIT = r'''
import json, os, pathlib, socket, subprocess
status = {}
for line in pathlib.Path("/proc/self/status").read_text().splitlines():
    key, _, value = line.partition(":")
    status[key] = value.strip()
hidden = ["/run/netns", "/run/cor-egress-integration", "/mnt/cor-egress-ci-sentinel",
          "/etc/cor-egress-ci-sentinel", "/var/lib/cor-egress", "/sys/class/net",
          "/root", "/media", "/proc/1/root/run/cor-egress-integration"]
fds = []
for entry in pathlib.Path("/proc/self/fd").iterdir():
    if int(entry.name) > 2:
        try: fds.append(os.readlink(entry))
        except FileNotFoundError: pass
write_usr = False
try:
    with open("/usr/cor-egress-ci-write-test", "x") as stream: stream.write("bad")
    write_usr = True
except OSError: pass
with open("/tmp/cor-egress-ci-write-test", "x") as stream: stream.write("private")
unshare = subprocess.run(["/usr/bin/unshare", "--user", "--map-root-user", "/bin/true"],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
print(json.dumps(dict(
    uid=os.getuid(), gid=os.getgid(), groups=os.getgroups(), status=status,
    namespaces={kind: os.stat("/proc/self/ns/" + kind).st_ino
                for kind in ("net", "mnt", "pid", "user", "ipc", "uts")},
    hidden={name: not pathlib.Path(name).exists() for name in hidden},
    extra_fds=fds, usr_writable=write_usr, nested_userns_exit=unshare.returncode,
    env=dict(os.environ), cwd=os.getcwd(),
    resolver=pathlib.Path("/etc/resolv.conf").read_text(),
    mounts=pathlib.Path("/proc/self/mountinfo").read_text(),
)))
'''


class Failure(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    # Never use assert for safety gates: python -O must preserve them.
    if not condition:
        raise Failure(message)


def execution_gate(execute: bool) -> None:
    require(execute, "refusing kernel tests: --execute-isolated is required")
    require(os.environ.get("CI") == "true", "refusing kernel tests: CI=true is required")
    require(os.environ.get("COR_EGRESS_ISOLATED_CI") == "1",
            "refusing kernel tests: COR_EGRESS_ISOLATED_CI=1 is required")
    require(sys.platform == "linux", "kernel tests require a disposable Linux CI runner")
    require(os.geteuid() == 0, "isolated CI kernel tests require root")


def namespace_ids() -> dict[str, int]:
    return {kind: os.stat(f"/proc/self/ns/{kind}").st_ino for kind in ("net", "mnt", "pid")}


def verify_child(parent: dict[str, int]) -> None:
    require(os.getpid() == 1, "fixture must be PID 1 in its fresh PID namespace")
    current = namespace_ids()
    require(set(parent) == set(current), "missing parent namespace identities")
    require(all(current[kind] != parent[kind] for kind in current),
            "refusing fixture: net, mount and PID namespaces must all differ from launcher")
    # sysfs can reflect its mounting netns; rtnetlink is the authority here.
    links = json.loads(command(["ip", "-j", "link", "show"]).stdout)
    require([link["ifname"] for link in links] == ["lo"],
            "fixture parent must start with loopback only, no inherited uplink")


def command(args, *, namespace=None, check=True, input=None, timeout=30):
    prefix = ["ip", "netns", "exec", namespace] if namespace else []
    result = subprocess.run([*prefix, *map(str, args)], input=input, text=True,
                            capture_output=True, timeout=timeout)
    if check and result.returncode:
        # Never echo command arguments or manager output: either could contain keys.
        raise Failure(f"{Path(str(args[0])).name} failed (exit {result.returncode})")
    return result


def private_mounts() -> None:
    command(["mount", "--make-rprivate", "/"])
    # Do not let manager writes reach runner /etc, /run, persistent state or logs.
    files = {}
    for name in ("passwd", "group", "nsswitch.conf", "hosts", "os-release"):
        path = Path("/etc") / name
        if path.is_file():
            files[name] = path.read_bytes()
    for target in ("/run", "/etc", "/var/lib", "/var/log", "/mnt"):
        command(["mount", "-t", "tmpfs", "-o", "mode=0755,nosuid,nodev", "tmpfs", target])
    for name, content in files.items():
        (Path("/etc") / name).write_bytes(content)
    Path("/etc/resolv.conf").write_text(f"nameserver {RESOLVER}\noptions timeout:1 attempts:1\n")
    WORK.mkdir(mode=0o700)
    rootprivate(Path("/mnt/cor-egress-ci-sentinel"), "must not enter workload\n")
    rootprivate(Path("/etc/cor-egress-ci-sentinel"), "must not enter workload\n")


def rootprivate(path: Path, content: str) -> None:
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as stream:
        stream.write(content)


def nonce() -> str:
    return secrets.token_hex(16)


class Fixture:
    def __init__(self):
        self.processes: list[subprocess.Popen] = []
        self.profile = WORK / "profile.json"
        self.attestation = WORK / "scope.json"
        self.evidence: list[dict] = []
        self.install_input = None
        self.original_resolver = Path("/etc/resolv.conf").read_bytes()
        self.original_wsl = b"[boot]\nsystemd=true\n[user]\ndefault=fixture\n"
        rootprivate(Path("/etc/wsl.conf"), self.original_wsl.decode())

    def manager_argv(self, action):
        return [sys.executable, "-I", MANAGER, action, "--profile", self.profile,
                "--generation", str(GENERATION)]

    def attest(self, **overrides):
        attestation = dict(schema=1, generation=GENERATION, issued_at_unix=time.time(),
                           all_distros_managed=True, hyperv_default_deny=True,
                           dns_tunneling_disabled=True, interop_disabled=True,
                           isolated_ci_fixture=True, effective_audit_passed=True)
        attestation.update(overrides)
        if self.attestation.exists():
            self.attestation.unlink()
        rootprivate(self.attestation, json.dumps(attestation))
        require(self.attestation.stat().st_mode & 0o777 == 0o600,
                "fixture attestation must be restored to mode 0600")

    def manager(self, action, *extra, outcome="succeeded", input=None, stdin_attestation=False):
        args = self.manager_argv(action)
        if action in ("start", "run"):
            self.attest()
            if stdin_attestation:
                require(action == "start" and input is None, "only start supports stdin attestation")
                input = self.attestation.read_text()
            else:
                args += ["--scope-attestation", self.attestation]
        result = command([*args, *extra], check=False, input=input, timeout=40)
        try:
            event = json.loads(result.stdout)
        except ValueError:
            detail = ""
            if os.environ.get("COR_EGRESS_TEST_DEBUG") == "1":
                # Opt-in only: fixture keys and paths are synthetic, but workflow
                # logs remain redacted by default.
                detail = (f" stdout={result.stdout[-2000:]!r}"
                          f" stderr={result.stderr[-2000:]!r}")
            raise Failure(f"manager {action}: missing JSON event{detail}") from None
        require(event.get("outcome") == outcome,
                f"manager {action}: unexpected outcome {event.get('outcome')!r} "
                f"phase={event.get('phase')!r} message={event.get('message')!r}")
        require(event.get("schema_version", event.get("schema")) == 1,
                f"manager {action}: unexpected schema")
        require(result.returncode == (0 if outcome == "succeeded" else 2),
                f"manager {action}: inconsistent exit code")
        return event

    def managed_prefix(self):
        self.attest()
        return [*self.manager_argv("run"), "--scope-attestation", self.attestation, "--"]

    def launch(self, prefix, code, config, *, ordinary=False, inherit_fds=(),
               capture_stderr=False):
        identity = dict(user=UID, group=GID, extra_groups=()) if ordinary else {}
        # /usr/local and /opt are intentionally absent inside manager's bwrap.
        process = subprocess.Popen([*map(str, prefix), PAYLOAD_PYTHON, "-I", "-u", "-c", code,
                                    json.dumps(config)], stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE if capture_stderr else subprocess.DEVNULL,
                                   text=True, pass_fds=inherit_fds, **identity)
        self.processes.append(process)
        return process

    def setup(self):
        command(["ip", "link", "set", "lo", "up"])
        command(["ip", "netns", "add", VPS])
        command(["ip", "link", "add", "fixture-uplink", "type", "veth",
                 "peer", "name", "vps-uplink"])
        command(["ip", "link", "set", "vps-uplink", "netns", VPS])
        for ns, iface, v4, v6 in (
            (None, "fixture-uplink", "192.0.2.1/24", "2001:db8:1::1/64"),
            (VPS, "vps-uplink", f"{ENDPOINT}/24", "2001:db8:1::2/64"),
        ):
            for args in (["ip", "link", "set", "lo", "up"],
                         ["ip", "addr", "add", v4, "dev", iface],
                         ["ip", "-6", "addr", "add", v6, "dev", iface, "nodad"],
                         ["ip", "link", "set", iface, "up"]):
                command(args, namespace=ns)
        command(["ip", "addr", "add", f"{TARGET}/32", "dev", "lo"], namespace=VPS)
        command(["ip", "-6", "addr", "add", f"{TARGET6}/128", "dev", "lo", "nodad"], namespace=VPS)
        command(["ip", "route", "add", "default", "via", ENDPOINT])
        command(["ip", "-6", "route", "add", "default", "via", "2001:db8:1::2"])
        command(["ip", "netns", "add", FIXTURE_SOURCE])
        command(["ip", "link", "add", "fixture-source", "type", "veth",
                 "peer", "name", "fixture-client", "netns", FIXTURE_SOURCE])
        command(["ip", "addr", "add", "172.29.77.1/30", "dev", "fixture-source"])
        command(["ip", "link", "set", "fixture-source", "up"])
        for args in (["ip", "link", "set", "lo", "up"],
                     ["ip", "addr", "add", "172.29.77.2/30", "dev", "fixture-client"],
                     ["ip", "link", "set", "fixture-client", "up"],
                     ["ip", "route", "add", "default", "via", "172.29.77.1"]):
            command(args, namespace=FIXTURE_SOURCE)
        command(["ip", "route", "add", "172.29.77.0/30", "via", "192.0.2.1"], namespace=VPS)
        command(["sysctl", "-q", "-w", "net.ipv4.ip_forward=1"])
        client_key = command(["wg", "genkey"]).stdout.strip()
        server_key = command(["wg", "genkey"]).stdout.strip()
        client_public = command(["wg", "pubkey"], input=client_key + "\n").stdout.strip()
        server_public = command(["wg", "pubkey"], input=server_key + "\n").stdout.strip()
        rootprivate(WORK / "server.key", server_key + "\n")
        command(["ip", "link", "add", "wg-vps", "type", "wireguard"], namespace=VPS)
        command(["wg", "set", "wg-vps", "private-key", WORK / "server.key",
                 "listen-port", "51820", "peer", client_public,
                 "allowed-ips", "10.77.0.2/32"], namespace=VPS)
        command(["ip", "addr", "add", "10.77.0.1/24", "dev", "wg-vps"], namespace=VPS)
        command(["ip", "link", "set", "wg-vps", "up"], namespace=VPS)
        # install owns profile creation; the key is supplied via stdin, not argv.
        self.install_input = json.dumps(dict(
            schema=1, backend="wireguard", endpoint_ipv4=ENDPOINT, endpoint_port=51820,
            private_key=client_key, peer_public_key=server_public,
            tunnel_address="10.77.0.2/32", resolver_ipv4=RESOLVER, uid=UID, gid=GID,
            forward_sources=[dict(interface="fixture-source", source_cidr="172.29.77.0/30")],
        ))
        # These are synthetic CI assertions, not an audit of Windows/Hyper-V.
        self.attest()
        command(["nft", "-f", "-"], namespace=VPS, input='''
table inet cor_fixture_observe {
    counter direct { }
    counter tunnel { }
    counter wireguard { }
    chain input {
        type filter hook input priority -300; policy accept;
        iifname "vps-uplink" udp dport 51820 counter name wireguard
        iifname "vps-uplink" tcp dport { 53, 18080, 51820 } counter name direct
        iifname "vps-uplink" udp dport { 53, 18080, 51821 } counter name direct
        iifname "wg-vps" tcp dport { 53, 18080 } counter name tunnel
        iifname "wg-vps" udp dport { 53, 18080 } counter name tunnel
    }
}
''')
        log = (WORK / "receiver.log").open("w")
        try:
            process = subprocess.Popen(["ip", "netns", "exec", VPS, sys.executable,
                                        "-u", "-c", RECEIVER, str(WORK)],
                                       stdout=log, stderr=log)
        finally:
            log.close()
        self.processes.append(process)
        deadline = time.monotonic() + 8
        while not (WORK / "ready").exists():
            require(process.poll() is None, "fixture receiver failed to start")
            require(time.monotonic() < deadline, "fixture receiver readiness timeout")
            time.sleep(0.05)

    def receipts(self, token):
        path = WORK / "receipts.jsonl"
        records = [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
        # UDP receiver threads and the filesystem may expose the append after
        # the echo has arrived. Poll for a bounded interval, while the caller
        # still rejects a missing nonce and performs a positive control.
        deadline = time.monotonic() + 2
        while True:
            matched = [record for record in records if record["nonce"] == token]
            if matched or time.monotonic() >= deadline:
                return matched
            time.sleep(0.02)
            records = ([json.loads(line) for line in path.read_text().splitlines()]
                       if path.exists() else [])

    def observe(self):
        objects = json.loads(command(["nft", "-j", "list", "table", "inet", "cor_fixture_observe"],
                                     namespace=VPS).stdout)["nftables"]
        return {obj["counter"]["name"]: obj["counter"]["packets"]
                for obj in objects if "counter" in obj}

    def deny_counters(self, namespace=None):
        objects = json.loads(command(["nft", "-j", "list", "ruleset"], namespace=namespace).stdout)["nftables"]
        packets = 0
        count = 0
        for obj in objects:
            rule = obj.get("rule", {})
            if not rule.get("table", "").startswith("cor_egress"):
                continue
            exprs = rule.get("expr", [])
            if not any("drop" in expr or "reject" in expr for expr in exprs):
                continue
            for expr in exprs:
                counter = expr.get("counter")
                if isinstance(counter, dict) and "packets" in counter:
                    packets += counter["packets"]
                    count += 1
        require(count > 0, "deny rules need explicit nft counter drop/reject evidence")
        return packets

    def probe(self, mode="tcp", host=TARGET, port=18080, *, managed=False,
              namespace=None, token=None, qtype=1, worker=None, bind_host=None):
        config = dict(mode=mode, host=host, port=port, nonce=token or nonce(),
                      timeout=TIMEOUT, qtype=qtype, bind_host=bind_host)
        if worker is not None:
            worker.stdin.write(json.dumps(config) + "\n")
            worker.stdin.flush()
            result = self.read_line(worker)
            require(result.get("nonce") == config["nonce"], "worker probe nonce mismatch")
            return result
        prefix = (self.managed_prefix() if managed
                  else ["ip", "netns", "exec", namespace] if namespace else [])
        if namespace and namespace != VPS:
            prefix += ["setpriv", "--reuid", str(UID), "--regid", str(GID), "--clear-groups"]
        process = self.launch(prefix, CLIENT, config, ordinary=not managed and namespace is None)
        try:
            out, _ = process.communicate(timeout=12)
        except subprocess.TimeoutExpired:
            process.kill()
            process.communicate()
            raise Failure("probe process timed out instead of producing evidence") from None
        require(process.returncode == 0, "probe/manager run failed before evidence collection")
        try:
            result = json.loads(out)
        except ValueError:
            raise Failure("probe did not produce a single JSON result") from None
        require(result.get("nonce") == config["nonce"], "probe nonce mismatch")
        return result

    def control(self, mode="tcp", host=TARGET, port=18080, qtype=1):
        result = self.probe(mode, host, port, namespace=VPS, qtype=qtype)
        require(result["reply"] and self.receipts(result["nonce"]), "receiver positive control failed")

    def positive(self, label, mode="tcp", host=TARGET, port=18080, *, managed=False,
                 qtype=1, namespace=None, worker=None, via_tunnel=None, bind_host=None):
        before = self.observe()
        result = self.probe(mode, host, port, managed=managed, qtype=qtype,
                            namespace=namespace, worker=worker, bind_host=bind_host)
        records = self.receipts(result["nonce"])
        after = self.observe()
        tunnel = managed if via_tunnel is None else via_tunnel
        path = "tunnel" if tunnel else "direct"
        require(result["reply"] and len(records) == 1,
                f"{label}: no unique nonce echo/receipt "
                f"(reply={bool(result.get('reply'))}, receipts={len(records)}, "
                f"error={result.get('error', 'none')})")
        require(records[0].get("kind") == mode, f"{label}: receiver protocol mismatch")
        require(after[path] > before[path], f"{label}: no receiving-interface counter increase")
        if tunnel:
            require(records[0]["source"] == "10.77.0.2", f"{label}: wrong tunnel source")
            require(after["wireguard"] > before["wireguard"], f"{label}: no encrypted transport counter increase")
            require(after["direct"] == before["direct"], f"{label}: direct path observed")
        self.evidence.append(dict(case=label, result="PASS", nonce=result["nonce"],
                                  receipt=records[0], before=before, after=after))

    def negative(self, label, mode="tcp", host=TARGET, port=18080, *, managed=False,
                 namespace=None, persistent=None, worker=None, probe_namespace=None,
                 route_refusal=False, control_host=None, control_port=None):
        # A blocked destination cannot also be the positive control endpoint.
        # DNS UDP, for example, is intentionally bound only on the managed
        # resolver address rather than on every address in the fixture, so both
        # the pre-probe and the post-probe control must use the declared
        # control endpoint instead of the blocked destination.
        control = (control_host or host, control_port or port)
        self.control(mode, *control)
        before = self.observe()
        denied_before = self.deny_counters(namespace)
        route_evidence = None
        if route_refusal:
            family = "-6" if ":" in host else "-4"
            route = command(["ip", family, "-j", "route", "get", host], namespace=namespace, check=False)
            routes = json.loads(command(["ip", family, "-j", "route", "show", "table", "all"],
                                        namespace=namespace).stdout)
            require(route.returncode != 0 and
                    any(r.get("type") == "unreachable" and r.get("dst") == "default" for r in routes),
                    f"{label}: no explicit unreachable route evidence")
            route_evidence = dict(unreachableDefault=True, lookupExit=route.returncode)
        token = nonce()
        if persistent is None:
            result = self.probe(mode, host, port, managed=managed, token=token,
                                namespace=probe_namespace, worker=worker)
        else:
            result = self.session_probe(persistent, token)
        # Allow already queued receiver work to become visible before rejecting.
        time.sleep(0.15)
        after = self.observe()
        denied_after = self.deny_counters(namespace)
        self.control(mode, *control)
        require(not result["reply"] and not self.receipts(token), f"{label}: denied nonce reached receiver")
        require(after["direct"] == before["direct"], f"{label}: direct packet escaped")
        if not route_refusal:
            require(denied_after > denied_before, f"{label}: timeout without a deny counter is not proof")
        self.evidence.append(dict(case=label, result="PASS", nonce=token, received=0,
                                  before=before, after=after,
                                  denyBefore=denied_before, denyAfter=denied_after,
                                  route=route_evidence))

    def session(self, *, managed=False, bind_host=None):
        prefix = self.managed_prefix() if managed else []
        process = self.launch(prefix, CLIENT, dict(mode="tcp", host=TARGET, port=18080,
                                                 persistent=True, timeout=TIMEOUT,
                                                 bind_host=bind_host), ordinary=not managed)
        ready = self.read_line(process)
        require(ready.get("ready") is True, "persistent TCP setup failed")
        result = self.session_probe(process, nonce())
        require(result["reply"] and self.receipts(result["nonce"]), "persistent TCP positive control failed")
        return process

    def worker(self):
        process = self.launch(self.managed_prefix(), CLIENT, dict(worker=True))
        require(self.read_line(process).get("ready") is True, "managed worker failed to start")
        self.positive("resident-worker-positive-control", managed=True, worker=process)
        return process

    @staticmethod
    def finish(process):
        if process.poll() is None:
            process.stdin.close()
            process.stdin = None
            process.communicate(timeout=12)
        require(process.returncode == 0, "probe did not exit cleanly")

    def sandbox_audit(self):
        links = json.loads(command(["ip", "-j", "link", "show"], namespace=GATEWAY).stdout)
        require({link["ifname"] for link in links} == {"lo", "wg0", "cor-bg0"},
                "gateway interface set differs from current business-veth contract")
        expected_namespaces = {kind: os.stat(f"/proc/self/ns/{kind}").st_ino
                               for kind in ("net", "mnt", "pid", "user", "ipc", "uts")}
        expected_namespaces["net"] = Path(f"/run/netns/{GATEWAY}").stat().st_ino
        handle = os.open(f"/run/netns/{VPS}", os.O_RDONLY)
        try:
            process = self.launch(self.managed_prefix(), SANDBOX_AUDIT, {},
                                  inherit_fds=(handle,), capture_stderr=True)
        finally:
            os.close(handle)
        out, error = process.communicate(timeout=20)
        require(process.returncode == 0,
                f"sandbox inspection workload failed (exit={process.returncode}): "
                f"{(error or '').strip()[:1500]}")
        try:
            result = json.loads(out)
        except ValueError:
            raise Failure("sandbox inspection did not produce JSON") from None
        require(result["uid"] == UID and result["gid"] == GID,
                "sandbox workload has unexpected UID/GID")
        require(set(result["groups"]) <= {GID}, "sandbox retained supplementary groups")
        for name in ("CapEff", "CapPrm", "CapAmb", "CapBnd"):
            require(int(result["status"][name], 16) == 0, f"sandbox retained {name}")
        require(result["status"].get("NoNewPrivs") == "1", "sandbox lacks no_new_privs")
        require(result["namespaces"]["net"] == expected_namespaces["net"],
                "sandbox is outside the managed network namespace")
        for kind in ("mnt", "pid", "user", "ipc", "uts"):
            require(result["namespaces"][kind] != expected_namespaces[kind],
                    f"sandbox inherited {kind} namespace")
        require(all(result["hidden"].values()), "host mount or control handle visible inside sandbox")
        require(not result["extra_fds"], "sandbox inherited extra file descriptors")
        require(not result["usr_writable"], "sandbox /usr must be read-only")
        require(result["nested_userns_exit"] != 0, "sandbox allows nested user namespaces")
        require(result["cwd"] == "/home/sandbox", "unexpected sandbox cwd")
        require(result["env"].get("HOME") == "/home/sandbox" and
                result["env"].get("TZ") == "Europe/Berlin", "sandbox identity environment differs")
        require(not any(key.lower().endswith("proxy") or key.startswith("WSL")
                        for key in result["env"]), "sandbox inherited host proxy/interop environment")
        require(result["resolver"].splitlines()[0] == f"nameserver {RESOLVER}",
                "sandbox DNS resolver differs from profile")
        require("cor-egress-integration" not in result["mounts"], "fixture mount leaked into sandbox")
        self.evidence.append(dict(case="bwrap-permissions-and-mounts", result="PASS",
                                  uid=UID, gid=GID, capabilities=0, noNewPrivs=True))

    def stdio_control(self):
        # A root-opened host file as stdin is a /proc/self/fd backdoor. The
        # manager must reject it; normal probe stdio uses pipes or /dev/null.
        path = WORK / "stdio-marker"
        rootprivate(path, "fixture-only host marker\n")
        with path.open("r") as stream:
            result = subprocess.run([*map(str, self.managed_prefix()), "/bin/true"],
                                    stdin=stream, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                    text=True, timeout=20)
        try:
            event = json.loads(result.stderr)
        except ValueError:
            raise Failure("host-stdio rejection did not emit a helper JSON event on stderr") from None
        require(result.returncode == 2 and event.get("outcome") == "blocked" and not result.stdout,
                "run accepted a host file as workload stdio")
        self.evidence.append(dict(case="bwrap-host-stdio-refused", result="PASS"))

    def attestation_controls(self):
        for label, overrides in (
            ("expired", dict(issued_at_unix=time.time() - 301)),
            ("future", dict(issued_at_unix=time.time() + 60)),
            ("generation", dict(generation=GENERATION + 1)),
            ("unmanaged-distro", dict(all_distros_managed=False)),
            ("effective-audit", dict(effective_audit_passed=False)),
        ):
            self.attest(**overrides)
            result = command([*self.manager_argv("start"), "--scope-attestation", self.attestation], check=False)
            event = json.loads(result.stdout)
            require(result.returncode == 2 and event.get("outcome") == "blocked",
                    f"{label}: invalid attestation accepted")
            self.guard_present()
            require(not Path(f"/run/netns/{GATEWAY}").exists(), "bad attestation created gateway")
            self.evidence.append(dict(case=f"attestation-{label}", result="PASS"))
        self.attest()
        self.attestation.chmod(0o644)
        result = command([*self.manager_argv("start"), "--scope-attestation", self.attestation], check=False)
        require(result.returncode == 2 and json.loads(result.stdout).get("outcome") == "blocked",
                "non-private attestation accepted")
        self.attest()
        self.evidence.append(dict(case="attestation-permissions", result="PASS"))

    @staticmethod
    def read_line(process):
        # A bound on readline also covers a stuck bwrap/manager before Python starts.
        import select
        ready, _, _ = select.select([process.stdout], [], [], 12)
        require(bool(ready), "persistent probe readiness/result timeout")
        line = process.stdout.readline()
        require(bool(line), "persistent probe exited without evidence")
        try:
            return json.loads(line)
        except ValueError:
            raise Failure("persistent probe returned invalid JSON") from None

    def session_probe(self, process, token):
        require(process.poll() is None, "persistent TCP probe unexpectedly exited")
        process.stdin.write(token + "\n")
        process.stdin.flush()
        result = self.read_line(process)
        require(result.get("nonce") == token, "persistent probe nonce mismatch")
        return result

    def guard_present(self):
        result = command(["nft", "-j", "list", "table", "inet", GUARD])
        chains = [obj["chain"] for obj in json.loads(result.stdout)["nftables"] if "chain" in obj]
        for hook in ("output", "forward"):
            require(any(chain.get("hook") == hook and chain.get("policy") == "drop" for chain in chains),
                    f"shared guard missing default-drop {hook} chain")

    def run(self):
        # Real pre-policy controls ensure IPv6, UDP and DNS paths actually work.
        for mode in ("tcp", "udp"):
            self.positive("pre-install-" + mode, mode)
            self.positive("pre-install-ipv6-" + mode, mode, TARGET6)
            self.positive("pre-install-forward-" + mode, mode, namespace=FIXTURE_SOURCE)
            self.positive("pre-install-legacy-bound-" + mode, mode, bind_host="192.0.2.1")
        for mode in ("dns-tcp", "dns-udp"):
            self.positive("pre-install-" + mode, mode, RESOLVER, 53)
        existing_root = self.session()
        self.manager("install", input=self.install_input, outcome="blocked")
        require(not self.profile.exists(), "unacknowledged install wrote profile")
        self.manager("install", "--acknowledge-shared-guard", input=self.install_input)
        self.install_input = None
        self.guard_present()
        self.manager("status")
        self.negative("install-blocks-existing-tcp", persistent=existing_root)
        self.finish(existing_root)
        for mode in ("tcp", "udp"):
            self.negative("installed-new-" + mode, mode)
            self.negative("installed-ipv6-" + mode, mode, TARGET6)
            self.negative("installed-forward-" + mode, mode, probe_namespace=FIXTURE_SOURCE)
        for mode in ("dns-tcp", "dns-udp"):
            self.negative("installed-direct-" + mode, mode, RESOLVER, 53)
        self.attestation_controls()
        self.manager("start")
        self.guard_present()
        self.manager("status")
        event = self.manager("test", outcome="blocked")
        require(any(check.get("name") == "end-to-end" and check.get("outcome") == "NotRun"
                    for check in event.get("checks", [])), "manager test must report end-to-end NotRun")
        self.sandbox_audit()
        self.stdio_control()
        for mode in ("tcp", "udp"):
            self.positive("managed-" + mode, mode, managed=True)
            self.positive("running-shared-" + mode, mode, via_tunnel=True)
            self.positive("running-legacy-bound-" + mode, mode, via_tunnel=True, bind_host="192.0.2.1")
            self.positive("running-container-forward-" + mode, mode, via_tunnel=True, namespace=FIXTURE_SOURCE)
        for mode in ("dns-tcp", "dns-udp"):
            self.positive("managed-" + mode, mode, RESOLVER, 53, managed=True)
            self.positive("managed-aaaa-" + mode, mode, RESOLVER, 53, managed=True, qtype=28)
            self.positive("running-shared-" + mode, mode, RESOLVER, 53, via_tunnel=True)
            self.positive("running-container-" + mode, mode, RESOLVER, 53,
                          via_tunnel=True, namespace=FIXTURE_SOURCE)
            self.negative("managed-unauthorized-" + mode, mode, TARGET, 53,
                          managed=True, namespace=GATEWAY, control_host=RESOLVER)
            self.negative("shared-unauthorized-" + mode, mode, TARGET, 53,
                          control_host=RESOLVER)
            self.negative("container-unauthorized-" + mode, mode, TARGET, 53,
                          probe_namespace=FIXTURE_SOURCE, control_host=RESOLVER)
        for mode in ("tcp", "udp"):
            self.negative("managed-ipv6-" + mode, mode, TARGET6, managed=True,
                          namespace=GATEWAY, route_refusal=True)
            self.negative("running-shared-ipv6-" + mode, mode, TARGET6)
        # Untrusted raw sockets in the transport namespace get no general path.
        self.negative("transport-endpoint-tcp", "tcp", ENDPOINT, 51820,
                      namespace=TRANSPORT, probe_namespace=TRANSPORT)
        self.negative("transport-endpoint-wrong-udp", "udp", ENDPOINT, 51821,
                      namespace=TRANSPORT, probe_namespace=TRANSPORT)
        existing_managed = self.session(managed=True)
        existing_shared = self.session()
        resident = self.worker()
        # With active workloads stop must first close the outer permit, then
        # return blocked rather than delete namespaces that are still in use.
        event = self.manager("stop", outcome="blocked")
        require(event.get("phase") == "Blocked", "active-workload stop did not withdraw the outer permit")
        self.guard_present()
        self.negative("stop-blocks-existing-managed-tcp", persistent=existing_managed)
        self.negative("stop-blocks-existing-shared-tcp", persistent=existing_shared)
        for mode in ("tcp", "udp"):
            self.negative("stop-blocks-new-shared-" + mode, mode)
            self.negative("stop-blocks-new-managed-" + mode, mode, managed=True, worker=resident)
            self.negative("stop-blocks-container-forward-" + mode, mode, probe_namespace=FIXTURE_SOURCE)
        for process in (existing_managed, existing_shared, resident):
            self.finish(process)
        self.manager("stop")
        self.guard_present()
        for mode in ("tcp", "udp"):
            self.negative("stopped-new-shared-" + mode, mode)
        self.manager("start", stdin_attestation=True)
        self.positive("restart-managed-tcp", managed=True)
        self.positive("restart-managed-udp", "udp", managed=True)
        # Discover by type, never assume the manager's private interface name.
        links = json.loads(command(["ip", "-j", "-d", "link", "show"], namespace=GATEWAY).stdout)
        wg_links = [link["ifname"] for link in links if link.get("linkinfo", {}).get("info_kind") == "wireguard"]
        require(len(wg_links) == 1, "expected one managed WireGuard interface in gateway")
        established = self.session(managed=True)
        resident = self.worker()
        shared_established = self.session()
        command(["ip", "link", "delete", wg_links[0]], namespace=GATEWAY)
        self.guard_present()
        # Force a real fallback route after tunnel loss, so these negative cases
        # exercise the guard instead of passing because no route exists. The
        # additional veth is fixture-only and deliberately outside manager health.
        command(["ip", "link", "add", "fixture-fail", "type", "veth", "peer", "name", "vps-fallback"], namespace=GATEWAY)
        command(["ip", "link", "set", "vps-fallback", "netns", VPS], namespace=GATEWAY)
        for ns, iface, address in ((GATEWAY, "fixture-fail", "203.0.113.1/30"),
                                   (VPS, "vps-fallback", "203.0.113.2/30")):
            command(["ip", "addr", "add", address, "dev", iface], namespace=ns)
            command(["ip", "link", "set", iface, "up"], namespace=ns)
        command(["ip", "route", "add", "default", "via", "203.0.113.2"], namespace=GATEWAY)
        command(["nft", "-f", "-"], namespace=VPS, input=
                'add rule inet cor_fixture_observe input iifname "vps-fallback" counter name direct\n')
        for mode in ("tcp", "udp"):
            self.negative("missing-tunnel-new-" + mode, mode, managed=True, namespace=GATEWAY, worker=resident)
            self.negative("missing-tunnel-shared-" + mode, mode, namespace=GATEWAY)
            self.negative("missing-tunnel-container-" + mode, mode,
                          namespace=GATEWAY, probe_namespace=FIXTURE_SOURCE)
        self.negative("missing-tunnel-existing-tcp", managed=True, namespace=GATEWAY, persistent=established)
        self.negative("missing-tunnel-existing-shared-tcp", namespace=GATEWAY, persistent=shared_established)
        for process in (established, resident, shared_established):
            self.finish(process)
        self.manager("stop")
        self.guard_present()
        self.negative("stop-after-tunnel-loss-shared-tcp")
        self.manager("rollback", outcome="blocked")
        self.guard_present()
        self.manager("rollback", "--release-guard")
        require(command(["nft", "list", "table", "inet", GUARD], check=False).returncode != 0,
                "explicit rollback left guard installed")
        require(Path("/etc/resolv.conf").read_bytes() == self.original_resolver,
                "rollback did not restore immediate resolver snapshot")
        require(Path("/etc/wsl.conf").read_bytes() == self.original_wsl,
                "rollback did not restore immediate wsl.conf snapshot")
        for mode in ("tcp", "udp"):
            self.positive("post-rollback-direct-" + mode, mode)
            self.positive("post-rollback-forward-" + mode, mode, namespace=FIXTURE_SOURCE)
            self.positive("post-rollback-ipv6-" + mode, mode, TARGET6)
        for mode in ("dns-tcp", "dns-udp"):
            self.positive("post-rollback-" + mode, mode, RESOLVER, 53)

    def close(self):
        for process in reversed(self.processes):
            if process.poll() is None:
                process.terminate()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=3)
        # No emergency flush/rollback: exiting PID 1 destroys the disposable tree.


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute-isolated", action="store_true")
    parser.add_argument("--_isolated-parent", help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    execution_gate(args.execute_isolated)
    require(MANAGER.is_file(), "linux/manager.py is required")
    require(Path(PAYLOAD_PYTHON).is_file(), "CI requires /usr/bin/python3 for sandbox workloads")
    for tool in ("unshare", "mount", "ip", "nft", "wg", "bwrap", "setpriv", "sysctl"):
        require(shutil.which(tool) is not None, f"missing CI dependency: {tool}")
    if args._isolated_parent is None:
        # First and only kernel operation in the launcher: enter fresh isolation.
        return subprocess.run([
            "unshare", "--net", "--mount", "--pid", "--fork", "--mount-proc",
            "--kill-child=SIGKILL", sys.executable, str(HERE), "--execute-isolated",
            "--_isolated-parent", json.dumps(namespace_ids()),
        ], check=False, timeout=420).returncode
    verify_child(json.loads(args._isolated_parent))
    private_mounts()
    fixture = Fixture()
    complete = False
    try:
        fixture.setup()
        fixture.run()
        complete = True
    finally:
        fixture.close()
        print(json.dumps(dict(suite="linux-isolated-integration", outcome="passed" if complete else "failed",
                              cases=fixture.evidence,
                              notRun=["Windows/WSL/Hyper-V effective audit", "cold boot", "real VPS/geography"],
                              scope="disposable Linux netns only; not WSL/Windows/real VPS acceptance")), flush=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (Failure, OSError, subprocess.SubprocessError, ValueError) as error:
        # Do not dump subprocess args, profiles, keys or receiver logs on failure.
        print(f"FAIL: {type(error).__name__}: "
              + (str(error) if isinstance(error, Failure) else "isolated harness failed"), file=sys.stderr)
        raise SystemExit(1)
