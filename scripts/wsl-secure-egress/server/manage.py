#!/usr/bin/env python3
"""Dedicated VPS WireGuard/DNS service. No SSH, package install, or global flush.

Read-only by default. Install takes a private JSON profile on stdin. Existing
proxy services and their configuration/ports are never touched.
"""
import argparse
import base64
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = Path('/var/lib/cor-wsl-egress')
INTERFACE = 'cor-wsl0'
TABLE = 'cor_wsl_vps'
ENV = {'PATH': '/usr/sbin:/usr/bin:/sbin:/bin', 'LANG': 'C', 'LC_ALL': 'C'}


def run(args, *, data=None, allow_failure=False):
    result = subprocess.run(args, input=data, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, timeout=30, env=ENV, check=False)
    if result.returncode and not allow_failure:
        raise RuntimeError('Dedicated VPS operation failed; no command or private output disclosed.')
    return result


def sha(data):
    return hashlib.sha256(data).hexdigest()


def private_directory():
    for path in [Path('/var'), Path('/var/lib'), ROOT]:
        if path.is_symlink():
            raise ValueError('Redirected state path.')
    ROOT.mkdir(mode=0o700, exist_ok=True)
    st = ROOT.stat()
    if st.st_uid != 0 or st.st_mode & 0o077:
        raise ValueError('State directory must be owned by root with mode 0700.')


def write(path, data):
    if path.is_symlink():
        raise ValueError('Redirected owned file.')
    descriptor, temp = tempfile.mkstemp(dir=ROOT, prefix='.pending-')
    try:
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, 'wb') as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp, path)
    finally:
        if os.path.exists(temp):
            os.unlink(temp)


def load():
    path = ROOT/'state.json'
    if not path.exists():
        return None
    if path.is_symlink() or path.stat().st_uid != 0 or path.stat().st_mode & 0o077:
        raise ValueError('Unsafe private journal.')
    state = json.loads(path.read_bytes())
    if state.get('owner') != 'ClashOfRust.WslEgress.VPS' or state.get('schema') != 1:
        raise ValueError('Unknown journal owner.')
    return state


def save(state):
    write(ROOT/'state.json', (json.dumps(state, indent=2) + '\n').encode())


def validate(profile):
    if profile.get('schema') != 1:
        raise ValueError('Profile schema must be 1.')
    address = ipaddress.IPv4Interface(profile['server_address'])
    peer = ipaddress.IPv4Address(profile['peer_address'])
    resolver = ipaddress.IPv4Address(profile['resolver_upstream'])
    if peer not in address.network or peer == address.ip or not address.ip.is_private:
        raise ValueError('Peer must have its own address in the private tunnel subnet.')
    if resolver.is_unspecified or resolver.is_loopback or resolver.is_multicast:
        raise ValueError('Resolver upstream must be an explicit unicast IPv4.')
    if type(profile['listen_port']) is not int or not 1 <= profile['listen_port'] <= 65535:
        raise ValueError('Invalid dedicated UDP port.')
    if len(base64.b64decode(profile['peer_public_key'], validate=True)) != 32:
        raise ValueError('Expected one WireGuard peer public key.')
    return address, peer, resolver


def table_exists():
    tables = json.loads(run(['nft', '-j', 'list', 'tables']).stdout)
    return any(item.get('table', {}).get('family') == 'inet' and
               item['table'].get('name') == TABLE for item in tables['nftables'])


def table_digest():
    raw = json.loads(run(['nft', '-j', 'list', 'table', 'inet', TABLE]).stdout)
    def clean(value):
        if isinstance(value, dict):
            return {key: clean(item) for key, item in value.items()
                    if key not in ('handle', 'packets', 'bytes', 'metainfo')}
        if isinstance(value, list):
            return [clean(item) for item in value if not (isinstance(item, dict) and 'metainfo' in item)]
        return value
    return sha(json.dumps(clean(raw), sort_keys=True).encode())


def boot_id():
    return Path('/proc/sys/kernel/random/boot_id').read_text().strip()


def interface():
    links = json.loads(run(['ip', '-j', 'link', 'show']).stdout)
    return next((link for link in links if link['ifname'] == INTERFACE), None)


def reconcile_boot(state):
    current = boot_id()
    previous = state.get('boot_id')
    if previous == current:
        return
    # Journals from before boot identity was recorded can also outlive all
    # runtime objects. Never carry their PID or forwarding snapshot into a restart.
    present_table, present_link = table_exists(), interface()
    if previous is not None and (present_table or present_link):
        raise ValueError('VPS objects exist in a new boot; ownership is unproven.')
    if not present_table and not present_link:
        for key in ('dns_pid', 'dns_start_time', 'interface_index', 'table_hash',
                    'forward_before', 'forward_written'):
            state.pop(key, None)
        if state['phase'] in ('starting', 'running'):
            state['phase'] = 'stopped'
    state['boot_id'] = current
    save(state)


def dns_identity(pid):
    process = Path('/proc')/str(pid)
    try:
        fields = (process/'stat').read_text().rsplit(')', 1)[1].split()
        if fields[0] in ('Z', 'X'):
            return None
        command = (process/'cmdline').read_bytes().split(b'\0')
    except FileNotFoundError:
        return None
    expected = ('--conf-file='+str(ROOT/'dnsmasq.conf')).encode()
    if not command or Path(os.fsdecode(command[0])).name != 'dnsmasq' or expected not in command:
        raise ValueError('DNS PID was reused; preserving process.')
    return fields[19]  # /proc/PID/stat field 22: process start time.


def dns_running(state):
    pid = state.get('dns_pid')
    if not pid:
        return False
    identity = dns_identity(pid)
    if identity is None:
        return False
    if state.get('dns_start_time') not in (None, identity):
        raise ValueError('DNS PID was reused; preserving process.')
    return True


def assert_interface(state, link):
    if state.get('interface_index') not in (None, link['ifindex']):
        raise ValueError('Interface ownership changed; preserving it.')
    public = run(['wg', 'show', INTERFACE, 'public-key']).stdout.decode().strip()
    # setconf may fail after link creation, before a public key exists. Only
    # the recorded ifindex in this boot may authorize deleting that empty link.
    pending = (state['phase'] == 'starting' and public in ('', '(none)') and
               state.get('interface_index') == link['ifindex'])
    if public != state['server_public_key'] and not pending:
        raise ValueError('Interface ownership changed; preserving it.')


def stop(state):
    reconcile_boot(state)
    # Bring down the authenticated interface BEFORE removing its restriction.
    present_table = table_exists()
    if present_table:
        if table_digest() != state.get('table_hash'):
            raise ValueError('VPS firewall changed externally; preserving it.')
    link = interface()
    if link:
        assert_interface(state, link)
    active_dns = dns_running(state)
    if link:
        run(['ip', 'link', 'set', INTERFACE, 'down'])
        run(['ip', 'link', 'delete', INTERFACE])
    if present_table:
        run(['nft', 'delete', 'table', 'inet', TABLE])
    if active_dns:
        os.kill(state['dns_pid'], 15)
        # Do not start a replacement DNS server until the old listener is gone.
        for _ in range(100):
            if not dns_running(state):
                break
            time.sleep(0.05)
        else:
            raise ValueError('Private DNS did not stop; retry after its exit.')
    for key in ('dns_pid', 'dns_start_time', 'interface_index', 'table_hash'):
        state.pop(key, None)
    state['phase'] = 'stopped'
    save(state)


def start(state):
    profile = json.loads((ROOT/'profile.json').read_bytes())
    address, peer, resolver = validate(profile)
    for name, expected in state['files'].items():
        path = ROOT/name
        if path.is_symlink() or sha(path.read_bytes()) != expected:
            raise ValueError('Private configuration changed after installation.')
    reconcile_boot(state)
    if state['phase'] in ('starting', 'running'):
        if state['phase'] == 'running' and table_exists():
            if table_digest() != state.get('table_hash'):
                raise ValueError('Running service policy differs.')
            link = interface()
            if link:
                assert_interface(state, link)
                if ('UP' in link.get('flags', []) and dns_running(state) and
                        Path('/proc/sys/net/ipv4/ip_forward').read_text().strip() == '1'):
                    return
        stop(state)
    if table_exists() or interface():
        raise ValueError('Dedicated object name collision; no overwrite.')
    route = json.loads(run(['ip', '-4', '-j', 'route', 'get', str(resolver)]).stdout)[0]
    uplink = route['dev']
    if not uplink.replace('-', '').replace('_', '').replace('.', '').isalnum() or len(uplink) > 15:
        raise ValueError('Unsupported uplink name.')
    # This table constrains only the dedicated interface; other services retain
    # their policy. A pre-existing host firewall may still reject the flow.
    rules = f'''table inet {TABLE} {{
      chain input {{ type filter hook input priority -150; policy accept;
        iifname "{INTERFACE}" ip saddr {peer} ip daddr {address.ip} udp dport 53 counter accept;
        iifname "{INTERFACE}" ip saddr {peer} ip daddr {address.ip} tcp dport 53 counter accept;
        iifname "{INTERFACE}" counter drop;
      }}
      chain forward {{ type filter hook forward priority -150; policy accept;
        iifname "{INTERFACE}" meta nfproto ipv6 counter drop;
        iifname "{INTERFACE}" ip saddr != {peer} counter drop;
        iifname "{INTERFACE}" oifname != "{uplink}" counter drop;
        iifname "{INTERFACE}" ip daddr {{ 0.0.0.0/8, 10.0.0.0/8, 127.0.0.0/8, 169.254.0.0/16, 172.16.0.0/12, 192.168.0.0/16, 224.0.0.0/4 }} counter drop;
        oifname "{INTERFACE}" ip daddr != {peer} counter drop;
        # Return traffic entering from the public uplink has not yet traversed
        # the forward hook, so conntrack can still classify its first SYN-ACK as
        # new. The separate reply-direction tuple is restricted to this peer.
        oifname "{INTERFACE}" ip daddr {peer} counter accept;
        oifname "{INTERFACE}" counter drop;
      }}
      chain nat_out {{ type nat hook postrouting priority srcnat; policy accept;
        iifname "{INTERFACE}" oifname "{uplink}" ip saddr {peer} masquerade;
      }}
    }}\n'''
    # Persist the recovery phase before the first runtime mutation. A failed
    # start must never leave a journal claiming a healthy running service.
    state['phase'] = 'starting'
    save(state)
    try:
        # Construct the restrictive firewall before bringing up the interface.
        run(['nft', '-f', '-'], data=rules.encode())
        state['table_hash'] = table_digest()
        save(state)
        run(['ip', 'link', 'add', INTERFACE, 'type', 'wireguard'])
        state['interface_index'] = interface()['ifindex']
        save(state)
        run(['wg', 'setconf', INTERFACE, str(ROOT/'wireguard.conf')])
        run(['ip', 'address', 'add', str(address), 'dev', INTERFACE])
        run(['ip', 'link', 'set', INTERFACE, 'mtu', '1380', 'up'])
        forward = Path('/proc/sys/net/ipv4/ip_forward')
        state.setdefault('forward_before', forward.read_text().strip())
        state['forward_written'] = '1'  # Write ahead of the sysctl mutation.
        save(state)
        run(['sysctl', '-q', '-w', 'net.ipv4.ip_forward=1'])
        with open(ROOT/'dns.log', 'ab') as log:
            process = subprocess.Popen(['dnsmasq', '--keep-in-foreground', '--conf-file='+str(ROOT/'dnsmasq.conf')],
                                       stdout=log, stderr=log, env=ENV, start_new_session=True)
        state['dns_pid'] = process.pid
        state['dns_start_time'] = dns_identity(process.pid)
        save(state)
        time.sleep(0.1)
        if process.poll() is not None or not dns_running(state):
            raise ValueError('Private DNS failed to start; inspect protected VPS log.')
        state['phase'] = 'running'
        save(state)
    except Exception:
        state['phase'] = 'starting'
        stop(state)  # Ownership checks retain drifted resources for recovery.
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['doctor', 'install', 'start', 'stop', 'status', 'rollback', 'uninstall'])
    parser.add_argument('--execute', action='store_true')
    args = parser.parse_args()
    if args.action == 'doctor':
        missing = [name for name in ('wg', 'ip', 'nft', 'dnsmasq', 'sysctl') if not shutil.which(name, path=ENV['PATH'])]
        return {'outcome': 'blocked' if missing else 'succeeded', 'missing_dependencies': missing,
                'runtime_acceptance': 'NOT_RUN'}
    if args.action == 'status':
        state = load()
        return {'outcome': 'succeeded', 'phase': state['phase'] if state else 'uninstalled',
                'runtime_acceptance': 'NOT_RUN'}
    if not args.execute:
        return {'outcome': 'not_run', 'message': 'Preview only. --execute is required; installation reads a private JSON from stdin.'}
    if not hasattr(os, 'geteuid') or os.geteuid() != 0:
        raise ValueError('Root is required for the dedicated VPS service.')
    private_directory()
    import fcntl
    with open(ROOT/'operation.lock', 'a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        state = load()
        if args.action == 'install':
            if state:
                raise ValueError('Existing recovery journal; no reinstall overwrite.')
            raw = sys.stdin.buffer.read(65537)
            if not raw or len(raw) > 65536:
                raise ValueError('Expected a private JSON profile, at most 64 KiB.')
            profile = json.loads(raw)
            address, peer, resolver = validate(profile)
            if table_exists() or interface():
                raise ValueError('Dedicated object collision.')
            # Existing port listeners must be reviewed before allocating a port.
            listeners = run(['ss', '-H', '-lun']).stdout.decode()
            if any(line.split()[3].rsplit(':', 1)[-1] == str(profile['listen_port'])
                   for line in listeners.splitlines() if len(line.split()) > 3):
                raise ValueError('Dedicated UDP port is occupied.')
            key = run(['wg', 'genkey']).stdout.strip()
            public = run(['wg', 'pubkey'], data=key+b'\n').stdout.decode().strip()
            config = (b'[Interface]\nPrivateKey = '+key+b'\nListenPort = '+str(profile['listen_port']).encode()+
                      b'\n[Peer]\nPublicKey = '+profile['peer_public_key'].encode()+
                      b'\nAllowedIPs = '+str(peer).encode()+b'/32\n')
            dns = (f'no-resolv\nno-hosts\nbind-interfaces\ninterface={INTERFACE}\nlisten-address={address.ip}\n'
                   f'server={resolver}\nuser=nobody\ngroup=nogroup\ncache-size=256\n').encode()
            files = {'profile.json': raw, 'wireguard.conf': config, 'dnsmasq.conf': dns}
            state = {'schema': 1, 'owner': 'ClashOfRust.WslEgress.VPS', 'phase': 'installed', 'boot_id': boot_id(),
                     'server_public_key': public, 'files': {name: sha(data) for name, data in files.items()}}
            save(state)  # preserve ownership before creating resources
            for name, data in files.items():
                write(ROOT/name, data)
        elif not state:
            raise ValueError('Not installed.')
        elif args.action == 'start':
            start(state)
        elif args.action in ('stop', 'rollback', 'uninstall'):
            stop(state)
            if args.action != 'stop' and 'forward_written' in state:
                current = Path('/proc/sys/net/ipv4/ip_forward').read_text().strip()
                if current not in (state['forward_written'], state['forward_before']):
                    raise ValueError('Forwarding changed externally; automatic restore refused.')
                if current != state['forward_before']:
                    run(['sysctl', '-q', '-w', 'net.ipv4.ip_forward='+state['forward_before']])
            if args.action != 'stop':
                state['phase'] = 'restored'
                save(state)
        return {'outcome': 'succeeded', 'phase': state['phase'], 'runtime_acceptance': 'NOT_RUN',
                'message': 'Owned VPS operation complete; no exit geography or end-to-end claim. Private recovery files retained.'}


if __name__ == '__main__':
    try:
        print(json.dumps(main(), sort_keys=True))
    except (OSError, ValueError, RuntimeError, KeyError, subprocess.SubprocessError):
        print(json.dumps({'outcome': 'blocked', 'message': 'VPS operation failed. Inspect private recovery files; unrelated services were not restored or reset.'}))
        sys.exit(2)
