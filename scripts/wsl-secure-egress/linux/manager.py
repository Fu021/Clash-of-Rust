#!/usr/bin/python3
"""Owned fail-closed WireGuard data plane; Python stdlib only.

Never import this module to activate networking. Only main() performs actions.
Root is the trusted administrator. The workload never receives root privileges.
"""
import argparse
import base64
import configparser
import fcntl
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import uuid

STATE = Path('/var/lib/cor-egress')
PROFILE = Path('/etc/cor-egress/profile.json')
TABLE = 'cor_egress_guard'
GW = 'cor-egress-gw'
TRANSPORT = 'cor-egress-transport'
ROOT_IF = 'cor-et0'
NS_IF = 'cor-ut0'
ROOT_IP = '169.254.240.1'
TRANSPORT_IP = '169.254.240.2'
BUS_IF = 'cor-eb0'
GW_BUS_IF = 'cor-bg0'
BUS_ROOT_IP = '169.254.241.1'
BUS_GW_IP = '169.254.241.2'
ROUTE_TABLE = '10077'
RULE_PREFS = (10070, 10071, 10072, 10073, 10074, 10075)
TUNNEL_MTU = 1380
WG_PORT = 51821
ENV = {'PATH': '/usr/sbin:/usr/bin:/sbin:/bin', 'LC_ALL': 'C', 'LANG': 'C'}


class Blocked(Exception):
    """An intentionally redacted, user-safe error."""


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encode(obj):
    return (json.dumps(obj, sort_keys=True, indent=2) + '\n').encode()


def sync_directory(path):
    fd = os.open(path, os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def boot_id():
    return Path('/proc/sys/kernel/random/boot_id').read_text().strip()


def private_path(path, directory=False):
    path = Path(path)
    if not path.is_absolute():
        raise Blocked('Private paths must be absolute Linux paths.')
    for part in [path, *path.parents]:
        st = part.lstat()
        if stat.S_ISLNK(st.st_mode) or st.st_uid != 0 or st.st_mode & 0o022:
            raise Blocked('Private path ownership, symlink or permissions check failed.')
    st = path.stat()
    if directory:
        if not stat.S_ISDIR(st.st_mode):
            raise Blocked('Expected a private directory.')
    elif not stat.S_ISREG(st.st_mode) or st.st_mode & 0o077 or st.st_nlink != 1:
        raise Blocked('Expected a root-owned private regular file (0600).')
    return path


def atomic(path, data, mode=0o600):
    path = Path(path)
    private_path(path.parent, directory=True)
    if path.exists() or path.is_symlink():
        if mode == 0o600:
            private_path(path)
        else:
            st = path.lstat()
            if not stat.S_ISREG(st.st_mode) or st.st_uid != 0 or st.st_nlink != 1 or st.st_mode & 0o022:
                raise Blocked('Owned public sandbox file changed.')
    fd, temp = tempfile.mkstemp(prefix='.cor-', dir=path.parent)
    try:
        os.fchmod(fd, mode)
        with os.fdopen(fd, 'wb') as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp, path)
        sync_directory(path.parent)
    finally:
        if os.path.exists(temp):
            os.unlink(temp)


def command(argv, ns=None, data=None, okay=False, timeout=25):
    if ns:
        argv = ['ip', 'netns', 'exec', ns, *argv]
    try:
        result = subprocess.run(argv, input=data, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, env=ENV, cwd='/',
                                close_fds=True, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired):
        raise Blocked('Required Linux command unavailable or timed out.') from None
    if result.returncode and not okay:
        # Command arguments/output may contain endpoint addresses or keys.
        raise Blocked(f'Linux command failed (exit {result.returncode}); '
                      'guard retained; no command output disclosed.')
    return result


def json_command(argv, ns=None):
    return json.loads(command(argv, ns=ns).stdout)


def dependencies():
    required = ['ip', 'nft', 'wg', 'bwrap', 'setpriv', 'sysctl']
    missing = [name for name in required if not shutil.which(name, path=ENV['PATH'])]
    if missing:
        raise Blocked('Missing dependencies: ' + ', '.join(missing))
    help_text = command(['bwrap', '--help']).stdout.decode()
    if any(x not in help_text for x in ['--unshare-user', '--unshare-pid', '--cap-drop', '--disable-userns']):
        raise Blocked('bubblewrap lacks required isolation options.')


def profile_validate(p):
    p = dict(p)
    if isinstance(p.get('backend'), str):
        p['backend'] = p['backend'].lower()
    if p.get('schema') != 1:
        raise Blocked('Profile schema must be 1.')
    if p.get('backend') not in ('wireguard', 'tailscale'):
        raise Blocked('Unknown backend.')
    if p['backend'] == 'tailscale':
        return p
    for key in ['endpoint_ipv4', 'resolver_ipv4']:
        ip = ipaddress.IPv4Address(p[key])
        if ip.is_unspecified or ip.is_multicast or ip.is_loopback:
            raise Blocked('Endpoint and resolver must be unicast IPv4 literals.')
    address = ipaddress.IPv4Interface(p['tunnel_address'])
    if address.ip.is_unspecified or address.ip.is_loopback or address.ip.is_multicast:
        raise Blocked('Tunnel address must be unicast IPv4.')
    for key, low, high in [('endpoint_port', 1, 65535), ('uid', 1000, 2147483646), ('gid', 1000, 2147483646)]:
        if type(p[key]) is not int or not low <= p[key] <= high:
            raise Blocked('Invalid numeric profile field: ' + key)
    for key in ['private_key', 'peer_public_key']:
        if not isinstance(p[key], str) or len(base64.b64decode(p[key], validate=True)) != 32:
            raise Blocked('WireGuard keys must encode exactly 32 bytes.')
    if not isinstance(p.get('forward_sources', []), list):
        raise Blocked('forward_sources must be a list of interface/source_cidr objects.')
    for item in p.get('forward_sources', []):
        if not re.fullmatch(r'[A-Za-z0-9_.:-]{1,15}', item['interface']):
            raise Blocked('Invalid container interface.')
        network = ipaddress.IPv4Network(item['source_cidr'], strict=True)
        if not network.is_private or network.prefixlen < 8 or network.is_loopback or network.is_link_local:
            raise Blocked('Container sources must be specific private IPv4 subnets.')
    return p


def read_profile(path):
    return profile_validate(json.loads(private_path(path).read_bytes()))


def clean_nft(obj):
    if isinstance(obj, dict):
        return {k: clean_nft(v) for k, v in obj.items()
                if k not in ('handle', 'packets', 'bytes', 'metainfo')}
    if isinstance(obj, list):
        return [clean_nft(x) for x in obj if not (isinstance(x, dict) and 'metainfo' in x)]
    return obj


def table_hash(ns=None):
    r = command(['nft', '-j', 'list', 'table', 'inet', TABLE], ns=ns, okay=True)
    if r.returncode:
        return None
    return digest(encode(clean_nft(json.loads(r.stdout))))


class Manager:
    def __init__(self, args):
        self.args = args
        self.file = STATE / 'state.json'
        self.s = json.loads(private_path(self.file).read_bytes()) if self.file.exists() else {}
        self.p = None
        self.checks = []
        self.dirty = False
        self.safe_closed = False
        self.close_requested = False

    def save(self):
        atomic(self.file, encode(self.s))

    def emergency_deny(self):
        """Close even if CAS refuses a drifted/incomplete main transaction.

        Never edit an unknown table. A separate owned table only adds denies;
        it cannot grant access or erase another owner's changes.
        """
        if not self.s or self.s.get('role') == 'distro-config' or self.s.get('phase') == 'RolledBack':
            return
        name = 'cor_egress_emergency_' + self.s['owner'].replace('-', '')[:12]
        present = command(['nft', '-j', 'list', 'table', 'inet', name], okay=True)
        if present.returncode == 0:
            # A same-name object is only acceptable if it is the exact owned
            # emergency deny. Never let a foreign collision suppress closure.
            expected = self.s.get('emergency', {}).get('hash')
            if expected and digest(encode(clean_nft(json.loads(present.stdout)))) == expected:
                return
            raise Blocked('Emergency deny name collision; Windows deny must remain active.')
        rules = f'''table inet {name} {{
          comment "cor-egress-emergency:{self.s['owner']}"
          chain output {{ type filter hook output priority -200; policy drop; counter drop; }}
          chain forward {{ type filter hook forward priority -200; policy drop; counter drop; }}
        }}\n'''
        command(['nft', '-f', '-'], data=rules.encode())
        result = command(['nft', '-j', 'list', 'table', 'inet', name])
        self.s['emergency'] = {'name': name, 'hash': digest(encode(clean_nft(json.loads(result.stdout))))}
        self.save()

    def boot_reconcile(self):
        if self.s.get('phase') in ('RolledBack', 'DistroRestored'):
            raise Blocked('Retired generation cannot be revived by a boot hook.')
        current = os.readlink('/proc/self/ns/net')
        if self.s['root_namespace'] == current and self.s.get('boot_id') == boot_id():
            return
        # A genuinely new root netns cannot contain old ifindices or sysctls.
        # Refuse all same-name objects rather than adopting foreign state.
        for name in (TRANSPORT, GW):
            if (Path('/run/netns') / name).exists() or (Path('/run/netns') / name).is_symlink():
                raise Blocked('Reboot reconciliation found a namespace collision.')
        links = json_command(['ip', '-j', 'link', 'show'])
        if any(x['ifname'] in (ROOT_IF, BUS_IF) for x in links):
            raise Blocked('Reboot reconciliation found an interface collision.')
        rules = json_command(['ip', '-4', '-j', 'rule', 'show'])
        if not isinstance(rules, list):
            rules = [rules]
        routes = self.policy_routes()
        if any(x.get('priority') in RULE_PREFS for x in rules) or routes or table_hash() is not None:
            raise Blocked('New namespace already contains reserved policy objects; no ownership assumed.')
        self.s.setdefault('previous_boots', []).append({
            'root_namespace': self.s['root_namespace'], 'phase': self.s['phase'],
            'boot_id': self.s.get('boot_id'),
            'runtime_hash': digest(encode({k: self.s.get(k) for k in ('namespaces', 'routing', 'sysctls', 'tables')}))})
        for key in ('routing', 'sysctls', 'veth_index', 'business_veth_index', 'emergency', 'attestation'):
            self.s.pop(key, None)
        self.s['namespaces'] = {}
        self.s['tables'] = {}
        self.s['root_namespace'] = current
        self.s['boot_id'] = boot_id()
        self.s['phase'] = 'Guarded'
        self.save()

    def event(self, action, outcome, message, phase=None):
        return {'schema': 1, 'schema_version': 1, 'generation': self.s.get('generation', self.args.generation or 0),
                'action': action, 'outcome': outcome.lower(),
                'protection': 'Blocked' if outcome == 'Blocked' else 'Unverified',
                'phase': phase or self.s.get('phase', 'Absent'),
                'backend': (self.p or {}).get('backend', self.s.get('backend', 'wireguard')),
                'message': message, 'checks': self.checks, 'logs': []}

    def same_generation(self):
        if not self.s:
            raise Blocked('Install the Linux helper configuration first.')
        if self.args.generation is not None and self.args.generation != self.s['generation']:
            raise Blocked('Generation mismatch; stale action refused.')

    def load(self):
        self.same_generation()
        if str(self.args.profile) != self.s['profile']:
            raise Blocked('Profile path differs from the owned installation.')
        self.p = read_profile(self.args.profile)
        if digest(Path(self.args.profile).read_bytes()) != self.s['profile_hash']:
            raise Blocked('Profile changed outside the owned transaction.')
        return self.p

    def assert_table(self, ns=None):
        key = ns or 'root'
        actual = table_hash(ns)
        expected = self.s.get('tables', {}).get(key)
        if actual != expected or expected is None:
            raise Blocked('Owned guard absent or changed; refusing to overwrite foreign rules.')

    def replace_table(self, rules, ns=None, initial=False):
        if initial:
            if table_hash(ns) is not None:
                raise Blocked('Guard table name collision; existing table preserved.')
            prefix = ''
        else:
            self.assert_table(ns)
            prefix = 'delete table inet ' + TABLE + '\n'
        self.dirty = True
        command(['nft', '-f', '-'], ns=ns, data=(prefix + rules).encode())
        self.s.setdefault('tables', {})[ns or 'root'] = table_hash(ns)
        self.save()

    def root_rules(self, opened=False):
        forward, nat, output = '', '', ''
        if opened:
            p = self.p
            up = self.s['uplink']
            endpoint, port = p['endpoint_ipv4'], p['endpoint_port']
            forward = f'''iifname "{ROOT_IF}" oifname "{up}" ip saddr {TRANSPORT_IP} ip daddr {endpoint} udp sport {WG_PORT} udp dport {port} counter accept
              iifname "{up}" oifname "{ROOT_IF}" ip saddr {endpoint} ip daddr {TRANSPORT_IP} udp sport {port} udp dport {WG_PORT} ct state established counter accept;
              iifname "{up}" oifname "{ROOT_IF}" ip daddr {TRANSPORT_IP} icmp type 3 icmp code 4 ct state related ct original ip saddr {TRANSPORT_IP} ct original ip daddr {endpoint} counter accept;'''
            sources = ', '.join(self.s['root_sources'])
            output = f'''oifname "lo" ip saddr {{ {sources} }} ip daddr {{ {sources} }} counter accept;
              oifname "{BUS_IF}" ip saddr {{ {sources} }} ip daddr != {p['resolver_ipv4']} udp dport 53 counter drop;
              oifname "{BUS_IF}" ip saddr {{ {sources} }} ip daddr != {p['resolver_ipv4']} tcp dport 53 counter drop;
              oifname "{BUS_IF}" ip saddr {{ {sources} }} counter accept;'''
            for item in self.s.get('forward_sources', []):
                iface, cidr = item['interface'], item['source_cidr']
                output += f'''\n oifname "{iface}" ip saddr {{ {sources} }} ip daddr {cidr} icmp type 3 icmp code 4 ct state related counter accept;'''
                forward += f'''
                  iifname "{iface}" oifname "{BUS_IF}" ip saddr {cidr} ip daddr != {p['resolver_ipv4']} udp dport 53 counter drop;
                  iifname "{iface}" oifname "{BUS_IF}" ip saddr {cidr} ip daddr != {p['resolver_ipv4']} tcp dport 53 counter drop;
                  iifname "{iface}" oifname "{BUS_IF}" ip saddr {cidr} counter accept;
                  iifname "{BUS_IF}" oifname "{iface}" ip daddr {cidr} ct state established,related counter accept;'''
            nat = f'''chain nat_out {{ type nat hook postrouting priority srcnat; policy accept;
              iifname "{ROOT_IF}" oifname "{up}" ip saddr {TRANSPORT_IP} ip daddr {endpoint} udp sport {WG_PORT} udp dport {port} masquerade
            }}'''
        return f'''table inet {TABLE} {{
          comment "cor-egress:{self.s['owner']}"
          chain output {{ type filter hook output priority -150; policy drop;
            meta nfproto ipv6 counter drop;
            oifname "lo" ip saddr 127.0.0.0/8 ip daddr 127.0.0.0/8 counter accept;
            {output} counter drop; }}
          chain forward {{ type filter hook forward priority -150; policy drop;
            meta nfproto ipv6 counter drop; {forward}
            counter drop; }}
          {nat}
        }}\n'''

    def snapshot(self, path, data):
        """Owned CAS restore. Keep the original object, including resolver symlinks."""
        path = Path(path)
        records = self.s.setdefault('files', {})
        key = str(path)
        if key in records:
            rec = records[key]
            if not self.written_file_matches(path, rec):
                raise Blocked('Managed distro file drift; preserving user changes.')
        else:
            rec = {'exists': path.exists() or path.is_symlink(), 'link': None, 'mode': 0o644, 'gid': 0}
            if path.is_symlink():
                if path.lstat().st_uid != 0:
                    raise Blocked('Resolver link must be owned by root.')
                rec['link'] = os.readlink(path)
                rec['gid'] = path.lstat().st_gid
            elif path.exists():
                st = path.stat()
                if not path.is_file() or st.st_nlink != 1 or st.st_uid != 0 or st.st_mode & 0o022:
                    raise Blocked('Unsupported distro configuration file type.')
                rec['mode'] = stat.S_IMODE(st.st_mode)
                rec['gid'] = st.st_gid
                backup = STATE / ('backup-' + digest(key.encode()))
                atomic(backup, path.read_bytes())
                rec['backup'] = str(backup)
                rec['original_hash'] = digest(path.read_bytes())
            records[key] = rec
            self.save()  # write-ahead original snapshot
        rec['written'] = digest(data)
        rec['pending'] = True
        self.save()
        # os.replace replaces the symlink itself, never its external target.
        fd, temp = tempfile.mkstemp(prefix='.cor-', dir=path.parent)
        try:
            os.fchmod(fd, 0o644)
            os.fchown(fd, 0, 0)
            with os.fdopen(fd, 'wb') as stream:
                stream.write(data)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(temp, path)
            sync_directory(path.parent)
        finally:
            if os.path.exists(temp):
                os.unlink(temp)
        rec['pending'] = False
        self.save()

    @staticmethod
    def written_file_matches(path, rec):
        if path.is_symlink() or not path.is_file():
            return False
        st = path.stat()
        return (st.st_uid == 0 and st.st_gid == 0 and st.st_nlink == 1 and
                stat.S_IMODE(st.st_mode) == 0o644 and digest(path.read_bytes()) == rec.get('written'))

    def distro_prepare(self):
        conf = Path('/etc/wsl.conf')
        if conf.is_symlink():
            raise Blocked('wsl.conf symlink unsupported; explicit operator review required.')
        raw = conf.read_text() if conf.exists() else ''
        parser = configparser.ConfigParser(interpolation=None, strict=True)
        parser.optionxform = str
        parser.read_string(raw)
        for section in list(parser.sections()):
            canonical = section.lower()
            if canonical in ('network', 'interop', 'boot') and canonical != section:
                if parser.has_section(canonical):
                    raise Blocked('Ambiguous case-duplicate WSL sections; original config preserved.')
                parser.add_section(canonical)
                for key, value in parser.items(section):
                    parser.set(canonical, key, value)
                parser.remove_section(section)
        for section, values in {'network': {'generateResolvConf': 'false'},
                                'interop': {'enabled': 'false', 'appendWindowsPath': 'false'}}.items():
            if not parser.has_section(section):
                parser.add_section(section)
            for key, value in values.items():
                # Remove case-insensitive aliases to avoid WSL duplicate-key ambiguity.
                for old in list(parser[section]):
                    if old.lower() == key.lower():
                        parser.remove_option(section, old)
                parser.set(section, key, value)
        if self.s.get('role') != 'distro-config':
            if not parser.has_section('boot'):
                parser.add_section('boot')
            old_values = [value.strip().strip('"') for key, value in parser.items('boot') if key.lower() == 'command']
            guard_command = '/usr/local/lib/cor-egress/manager guard'
            if any(old and old != guard_command for old in old_values):
                raise Blocked('Existing WSL boot command preserved; integrate the owned boot guard explicitly before install.')
            for key in list(parser['boot']):
                if key.lower() == 'command':
                    parser.remove_option('boot', key)
            parser.set('boot', 'command', guard_command)
        import io
        output = io.StringIO()
        parser.write(output)
        self.snapshot(conf, output.getvalue().encode())
        # Shared applications and the sandbox use the same fixed tunnel resolver.
        resolver = self.p.get('resolver_ipv4', '192.0.2.1')
        self.snapshot('/etc/resolv.conf', f'nameserver {resolver}\noptions timeout:2 attempts:1\n'.encode())

    def attestation(self, stdin=False):
        path = self.args.scope_attestation or self.s.get('attestation')
        if stdin and self.args.scope_attestation is None and not sys.stdin.isatty():
            raw = sys.stdin.buffer.read(65537)
            if raw:
                if len(raw) > 65536:
                    raise Blocked('Oversized scope attestation.')
                path = STATE / 'scope-attestation.json'
                atomic(path, raw)
        if not path:
            raise Blocked('Windows effective audit/cold-start/cross-distro attestation is required.')
        att = json.loads(private_path(path).read_bytes())
        fields = ['all_distros_managed', 'hyperv_default_deny',
                  'dns_tunneling_disabled', 'interop_disabled', 'effective_audit_passed']
        if att.get('schema') != 1 or att.get('generation') != self.s['generation'] or any(att.get(x) is not True for x in fields):
            raise Blocked('Full-WSL current effective audit or distro coverage is unproven.')
        issued = att.get('issued_at_unix')
        if type(issued) not in (int, float) or not 0 <= time.time() - issued <= 300:
            raise Blocked('Windows attestation missing or older than five minutes.')
        self.checks.append({'name': 'windows-scope-attestation', 'outcome': 'Passed',
                            'message': 'Root-supplied orchestration assertion; not end-to-end verification.'})
        self.checks.append({'name': 'cold-start-acceptance', 'outcome': 'NotRun',
                            'message': 'Cold-start acceptance is separate; connected backend remains Unverified.'})
        return str(path)

    def install(self):
        if not self.args.acknowledge_shared_guard:
            raise Blocked('install requires --acknowledge-shared-guard; shared OUTPUT/FORWARD will be denied.')
        if self.s and self.s.get('phase') != 'RolledBack':
            raise Blocked('Already installed; use the owned rollback before a new generation.')
        raw = sys.stdin.buffer.read(65537)
        if not raw or len(raw) > 65536:
            raise Blocked('install requires one stdin JSON object, at most 64 KiB.')
        p = profile_validate(json.loads(raw))
        if self.args.generation is None or self.args.generation < 1:
            raise Blocked('install requires a positive --generation.')
        dependencies()
        if self.s:
            self.archive_retired()
        profile = Path(self.args.profile)
        if not profile.is_absolute() or profile.exists() or profile.is_symlink():
            raise Blocked('Refusing to replace an existing or relative private profile.')
        profile.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        private_path(profile.parent, directory=True)
        if table_hash() is not None:
            raise Blocked('Pre-existing guard table; ownership not assumed.')
        self.s = {'schema': 1, 'owner': str(uuid.uuid4()), 'generation': self.args.generation,
                  'phase': 'Installing', 'profile': str(profile), 'backend': p['backend'],
                  'tables': {}, 'namespaces': {}, 'files': {}, 'root_namespace': os.readlink('/proc/self/ns/net'), 'boot_id': boot_id()}
        self.save()
        self.p = p
        atomic(profile, encode(p))
        self.s['profile_hash'] = digest(profile.read_bytes())
        self.save()
        self.replace_table(self.root_rules(), initial=True)  # deny before DNS/files/backend networking
        self.safe_closed = True
        self.distro_prepare()
        self.s['phase'] = 'Guarded'
        if self.args.scope_attestation:
            self.s['attestation'] = str(self.args.scope_attestation)
        self.save()
        return self.event('install', 'Succeeded', 'Installed deny guard and owned distro DNS/interop snapshots; restart orchestration and audit still required.')

    def configure(self):
        """Per-distro file setup only; Windows owns enumeration and shared guard."""
        if self.s and self.s.get('phase') != 'DistroRestored':
            raise Blocked('Existing distro ownership journal; restore-config before reconfiguring.')
        raw = sys.stdin.buffer.read(65537)
        if not raw or len(raw) > 65536 or not self.args.generation or self.args.generation < 1:
            raise Blocked('configure requires stdin profile and positive --generation.')
        p = profile_validate(json.loads(raw))
        if self.s:
            self.archive_retired()
        self.p = p
        self.s = {'schema': 1, 'owner': str(uuid.uuid4()), 'generation': self.args.generation,
                  'phase': 'ConfiguringDistro', 'backend': p['backend'], 'files': {},
                  'role': 'distro-config', 'root_namespace': os.readlink('/proc/self/ns/net'), 'boot_id': boot_id()}
        self.save()
        self.distro_prepare()
        self.s['phase'] = 'DistroConfigured'
        self.save()
        return self.event('configure', 'Succeeded', 'Owned distro DNS/interop configured; effective restart/runtime coverage must be audited by Windows.')

    def restore_config(self):
        self.same_generation()
        if self.s.get('role') != 'distro-config':
            raise Blocked('restore-config is only for a per-distro configuration journal.')
        self.restore_files()
        self.s['phase'] = 'DistroRestored'
        self.save()
        return self.event('restore-config', 'Succeeded', 'Only owned distro files restored; snapshots retained.')

    def archive_retired(self):
        if self.s['phase'] not in ('RolledBack', 'DistroRestored') or self.s.get('files') or self.s.get('tables') or self.s.get('namespaces'):
            raise Blocked('Active resources remain; generation cannot be archived.')
        archive = STATE / ('retired-' + str(self.s['generation']) + '-' + self.s['owner'])
        archive.mkdir(mode=0o700)
        if self.s.get('profile'):
            profile = private_path(self.s['profile'])
            if digest(profile.read_bytes()) != self.s['profile_hash']:
                raise Blocked('Retired profile changed; preserving it in place.')
            os.rename(profile, archive / 'profile.json')
        key = STATE / 'wireguard.key'
        if key.exists():
            private_path(key)
            os.rename(key, archive / 'wireguard.key')
        atomic(archive / 'state.json', encode(self.s))
        # Keep active state until all retirement artifacts are safely persisted.
        self.file.unlink()
        sync_directory(STATE)
        self.s = {}

    def audit_config(self):
        self.same_generation()
        matches = bool(self.s.get('files'))
        for name, rec in self.s.get('files', {}).items():
            path = Path(name)
            if not self.written_file_matches(path, rec):
                matches = False
        interop = Path('/proc/sys/fs/binfmt_misc/WSLInterop')
        interop_disabled = not interop.exists() or interop.read_text().splitlines()[0].strip() == 'disabled'
        result = self.event('audit-config', 'Succeeded', 'Current distro file hashes and Linux interop registration inspected; Windows owns all-distro and DNS-tunneling audit.')
        result['facts'] = {'owned_config_matches': matches,
                           'interop_runtime_disabled': interop_disabled,
                           'generation': self.s['generation'],
                           'shared_netns': os.readlink('/proc/self/ns/net')}
        result['checks'] = [{'name': key, 'outcome': 'Passed' if value else 'Failed'}
                            for key, value in result['facts'].items() if type(value) is bool]
        if not matches or not interop_disabled:
            result['outcome'] = 'blocked'
            result['protection'] = 'Blocked'
        return result

    def assert_namespace(self, name):
        rec = self.s['namespaces'].get(name)
        path = Path('/run/netns') / name
        if not rec or not path.exists() or path.is_symlink() or path.stat().st_ino != rec['inode']:
            raise Blocked('Namespace ownership mismatch; foreign namespace preserved.')

    def add_namespace(self, name):
        if (Path('/run/netns') / name).exists() or (Path('/run/netns') / name).is_symlink():
            raise Blocked('Namespace name collision; refusing reuse.')
        command(['ip', 'netns', 'add', name])
        self.s['namespaces'][name] = {'inode': (Path('/run/netns') / name).stat().st_ino}
        self.save()

    def collect_sources(self):
        addresses = json_command(['ip', '-4', '-j', 'addr', 'show'])
        root_sources = {BUS_ROOT_IP + '/32'}
        for link in addresses:
            if link['ifname'] in (ROOT_IF, BUS_IF, 'lo'):
                continue
            for addr in link.get('addr_info', []):
                ip = ipaddress.IPv4Address(addr['local'])
                if not ip.is_loopback and not ip.is_unspecified and not ip.is_multicast:
                    root_sources.add(str(ip) + '/32')
        forwards = list(self.p.get('forward_sources', []))
        # Existing native Docker/Linux bridge networks are discoverable without
        # trusting arbitrary uplink subnets. Future bridges require a restart.
        links = json_command(['ip', '-d', '-j', 'link', 'show'])
        bridges = {x['ifname'] for x in links if x.get('linkinfo', {}).get('info_kind') == 'bridge'}
        routes = json_command(['ip', '-4', '-j', 'route', 'show', 'table', 'main'])
        for route in routes:
            if route.get('dev') in bridges and route.get('scope') == 'link' and route.get('dst', 'default') != 'default':
                net = ipaddress.IPv4Network(route['dst'])
                if net.is_private and net.prefixlen >= 8:
                    item = {'interface': route['dev'], 'source_cidr': str(net)}
                    if item not in forwards:
                        forwards.append(item)
        available = {x['ifname'] for x in links}
        for item in forwards:
            if not re.fullmatch(r'[A-Za-z0-9_.:-]{1,15}', item['interface']):
                raise Blocked('Discovered bridge interface name is not supported.')
            if item['interface'] not in available or item['interface'] in (self.s['uplink'], ROOT_IF, BUS_IF, 'lo'):
                raise Blocked('Container ingress must be an existing dedicated bridge/veth, never the uplink.')
            ipaddress.IPv4Network(item['source_cidr'], strict=True)
        self.s['root_sources'] = sorted(root_sources)
        self.s['forward_sources'] = forwards
        self.save()

    def routing_hash(self):
        rules = json_command(['ip', '-4', '-j', 'rule', 'show'])
        owned_rules = [x for x in rules if x.get('priority') in RULE_PREFS]
        routes = self.policy_routes()
        for route in routes:
            route.pop('flags', None)  # linkdown must not prevent a safe stop
        return digest(encode({'rules': owned_rules, 'routes': routes}))

    def policy_routes(self):
        # Query table all: a nonexistent numbered FIB returns ENOENT on some
        # iproute2 versions when queried directly, which is an empty table here.
        routes = json_command(['ip', '-4', '-N', '-j', 'route', 'show', 'table', 'all'])
        return [x for x in routes if str(x.get('table')) == ROUTE_TABLE]

    def setup_routing(self):
        existing = json_command(['ip', '-4', '-j', 'rule', 'show'])
        routes = self.policy_routes()
        if any(x.get('priority') in RULE_PREFS for x in existing) or routes:
            raise Blocked('Policy-rule priority or dedicated routing table collision; foreign state preserved.')
        self.s['routing'] = {'commands': [], 'hash': self.routing_hash()}
        self.save()
        p = self.p
        commands = [
            ['route', 'add', 'unreachable', 'default', 'metric', '42760', 'table', ROUTE_TABLE],
            ['route', 'add', 'default', 'via', BUS_GW_IP, 'dev', BUS_IF, 'src', BUS_ROOT_IP, 'mtu', str(TUNNEL_MTU), 'metric', '10', 'table', ROUTE_TABLE],
            ['rule', 'add', 'pref', str(RULE_PREFS[0]), 'iif', ROOT_IF, 'from', TRANSPORT_IP + '/32', 'to', p['endpoint_ipv4'] + '/32', 'lookup', 'main'],
            ['rule', 'add', 'pref', str(RULE_PREFS[1]), 'iif', self.s['uplink'], 'from', p['endpoint_ipv4'] + '/32', 'to', TRANSPORT_IP + '/32', 'lookup', 'main'],
            ['rule', 'add', 'pref', str(RULE_PREFS[2]), 'iif', BUS_IF, 'lookup', 'main'],
            ['rule', 'add', 'pref', str(RULE_PREFS[3]), 'lookup', ROUTE_TABLE],
            ['rule', 'add', 'pref', str(RULE_PREFS[5]), 'iif', self.s['uplink'], 'to', TRANSPORT_IP + '/32', 'ipproto', 'icmp', 'lookup', 'main'],
        ]
        # These must precede the catch-all secure rule. Give the secure rule a
        # later priority, keeping PMTU exceptions precise by iif/protocol/subnet.
        commands[5] = ['rule', 'add', 'pref', str(RULE_PREFS[3]), 'iif', BUS_IF, 'lookup', 'main'] if False else commands[5]
        for cidr in sorted({x['source_cidr'] for x in self.s['forward_sources']}):
            commands.insert(-1, ['rule', 'add', 'pref', str(RULE_PREFS[4]), 'iif', 'lo', 'to', cidr, 'ipproto', 'icmp', 'lookup', 'main'])
        for args in commands:
            # Store each completed owned operation and its current kernel digest.
            command(['ip', '-4', *args])
            self.s['routing']['commands'].append(args)
            self.s['routing']['hash'] = self.routing_hash()
            self.save()

    def remove_routing(self):
        record = self.s.get('routing')
        if not record:
            return
        if self.routing_hash() != record['hash']:
            raise Blocked('Owned policy routes changed; refusing to delete foreign changes. Deny guard remains.')
        for args in reversed(record['commands'][:]):
            delete = args[:]
            delete[1] = 'del'
            command(['ip', '-4', *delete])
            record['commands'].pop()
            record['hash'] = self.routing_hash()
            self.save()
        del self.s['routing']
        self.save()

    def start(self):
        self.load()
        self.assert_table()
        if self.p['backend'] == 'tailscale':
            raise Blocked('Tailscale full-WSL cold-start boundary is unproven; backend blocked.')
        att = self.attestation(stdin=True)
        if self.s['phase'] == 'Running':
            self.health()
            return self.event('start', 'Succeeded', 'Already running with an unverified end-to-end path.')
        if self.s['namespaces']:
            raise Blocked('Stale owned namespaces; stop before retrying start.')
        route = json_command(['ip', '-4', '-j', 'route', 'get', self.p['endpoint_ipv4']])[0]
        uplink = route.get('dev', '')
        if not re.fullmatch(r'[A-Za-z0-9_.:-]{1,15}', uplink) or uplink in ('lo', ROOT_IF):
            raise Blocked('Endpoint uplink must be an existing physical/WSL interface.')
        for iface in (ROOT_IF, BUS_IF):
            if command(['ip', 'link', 'show', iface], okay=True).returncode == 0:
                raise Blocked('Transport/business veth name collision.')
        # The endpoint must use the ordinary WSL uplink. Refuse a route that
        # already enters an owned or Docker bridge so the encrypted socket
        # cannot loop through protected forwarding.
        if re.match(r'^(?:docker|br-|virbr|cor-)', uplink):
            raise Blocked('Endpoint route enters a container or owned bridge.')
        self.s['uplink'] = uplink
        self.s['attestation'] = att
        self.s['phase'] = 'Starting'
        self.save()
        self.collect_sources()
        self.replace_table(self.root_rules())  # re-deny first
        self.safe_closed = True
        self.add_namespace(TRANSPORT)
        self.add_namespace(GW)
        p = self.p
        endpoint, port = p['endpoint_ipv4'], p['endpoint_port']
        transport_rules = f'''table inet {TABLE} {{
          chain output {{ type filter hook output priority -150; policy drop;
            oifname "{NS_IF}" ip saddr {TRANSPORT_IP} ip daddr {endpoint} udp sport {WG_PORT} udp dport {port} counter accept; counter drop; }}
          chain input {{ type filter hook input priority -150; policy drop;
            iifname "{NS_IF}" ip saddr {endpoint} ip daddr {TRANSPORT_IP} udp sport {port} udp dport {WG_PORT} ct state established counter accept; counter drop; }}
          chain forward {{ type filter hook forward priority -150; policy drop; counter drop; }}
        }}\n'''
        self.replace_table(transport_rules, TRANSPORT, initial=True)
        source = str(ipaddress.IPv4Interface(p['tunnel_address']).ip)
        app_networks = [ipaddress.IPv4Network(x) for x in
                        self.s['root_sources'] + [x['source_cidr'] for x in self.s['forward_sources']]]
        app_sources = ', '.join(str(x) for x in ipaddress.collapse_addresses(app_networks))
        gateway_forward = f'''
            meta nfproto ipv6 counter drop;
            iifname "{GW_BUS_IF}" oifname "wg0" ip saddr {{ {app_sources} }} ip daddr != {p['resolver_ipv4']} udp dport 53 counter drop;
            iifname "{GW_BUS_IF}" oifname "wg0" ip saddr {{ {app_sources} }} ip daddr != {p['resolver_ipv4']} tcp dport 53 counter drop;
            iifname "{GW_BUS_IF}" oifname "wg0" ip saddr {{ {app_sources} }} counter accept;
            iifname "wg0" oifname "{GW_BUS_IF}" ip daddr {{ {app_sources} }} ct state established,related counter accept;'''
        gateway_rules = f'''table inet {TABLE} {{
          chain output {{ type filter hook output priority -150; policy drop;
            meta nfproto ipv6 counter drop;
            oifname "lo" ip saddr 127.0.0.0/8 ip daddr 127.0.0.0/8 counter accept;
            oifname "wg0" ip saddr {source} ip daddr {p['resolver_ipv4']} udp dport 53 counter accept;
            oifname "wg0" ip saddr {source} ip daddr {p['resolver_ipv4']} tcp dport 53 counter accept;
            udp dport 53 counter drop; tcp dport 53 counter drop;
            oifname "wg0" ip saddr {source} counter accept; counter drop; }}
          chain input {{ type filter hook input priority -150; policy drop;
            iifname "lo" ip saddr 127.0.0.0/8 ip daddr 127.0.0.0/8 counter accept;
            iifname "wg0" ip daddr {source} ct state established,related counter accept; counter drop; }}
          chain forward {{ type filter hook forward priority -150; policy drop; {gateway_forward} counter drop; }}
          chain nat_out {{ type nat hook postrouting priority srcnat; policy accept;
            iifname "{GW_BUS_IF}" oifname "wg0" ip saddr {{ {app_sources} }} snat ip to {source}; }}
        }}\n'''
        self.replace_table(gateway_rules, GW, initial=True)
        command(['ip', 'link', 'add', ROOT_IF, 'type', 'veth', 'peer', 'name', NS_IF, 'netns', TRANSPORT])
        self.s['veth_index'] = json_command(['ip', '-j', 'link', 'show', ROOT_IF])[0]['ifindex']
        self.save()
        command(['ip', 'addr', 'add', ROOT_IP + '/30', 'dev', ROOT_IF])
        command(['ip', 'link', 'set', ROOT_IF, 'up'])
        command(['ip', 'addr', 'add', TRANSPORT_IP + '/30', 'dev', NS_IF], ns=TRANSPORT)
        command(['ip', 'link', 'set', NS_IF, 'up'], ns=TRANSPORT)
        command(['ip', 'route', 'add', endpoint + '/32', 'via', ROOT_IP, 'dev', NS_IF], ns=TRANSPORT)
        command(['ip', 'link', 'add', BUS_IF, 'type', 'veth', 'peer', 'name', GW_BUS_IF, 'netns', GW])
        self.s['business_veth_index'] = json_command(['ip', '-j', 'link', 'show', BUS_IF])[0]['ifindex']
        self.save()
        command(['ip', 'addr', 'add', BUS_ROOT_IP + '/30', 'dev', BUS_IF])
        command(['ip', 'link', 'set', BUS_IF, 'up'])
        command(['ip', 'addr', 'add', BUS_GW_IP + '/30', 'dev', GW_BUS_IF], ns=GW)
        command(['ip', 'link', 'set', GW_BUS_IF, 'up'], ns=GW)
        for cidr in sorted(set(self.s['root_sources'] + [x['source_cidr'] for x in self.s['forward_sources']])):
            if cidr == BUS_ROOT_IP + '/32':
                continue  # the connected /30 already routes this source
            command(['ip', 'route', 'add', cidr, 'via', BUS_ROOT_IP, 'dev', GW_BUS_IF], ns=GW)
        command(['ip', 'link', 'add', 'wg0', 'type', 'wireguard'], ns=TRANSPORT)
        keyfile = STATE / 'wireguard.key'
        atomic(keyfile, (p['private_key'] + '\n').encode())
        command(['wg', 'set', 'wg0', 'private-key', str(keyfile), 'listen-port', str(WG_PORT),
                 'peer', p['peer_public_key'], 'allowed-ips', '0.0.0.0/0',
                 'endpoint', f'{endpoint}:{port}'], ns=TRANSPORT)
        command(['ip', 'link', 'set', 'wg0', 'netns', GW], ns=TRANSPORT)
        command(['ip', 'addr', 'add', p['tunnel_address'], 'dev', 'wg0'], ns=GW)
        command(['ip', 'link', 'set', 'wg0', 'mtu', '1380', 'up'], ns=GW)
        command(['ip', 'link', 'set', 'lo', 'up'], ns=GW)
        command(['ip', 'route', 'add', 'default', 'dev', 'wg0'], ns=GW)
        command(['ip', '-6', 'route', 'add', 'unreachable', 'default'], ns=GW)
        self.sysctl_forward(True)
        self.setup_routing()
        self.safe_closed = False  # a partially completed opener must get emergency deny
        self.replace_table(self.root_rules(opened=True))  # final and sole outer permit
        self.s['phase'] = 'Running'
        self.save()
        self.health()
        return self.event('start', 'Succeeded', 'WireGuard namespace ready; end-to-end egress and geographic acceptance remain unverified.')

    def sysctl_forward(self, enable):
        # Per-interface forwarding avoids net.ipv4.ip_forward's global reset of
        # host/router defaults. Existing unrelated interfaces keep their state.
        if enable:
            root_ifaces = {self.s['uplink'], ROOT_IF, BUS_IF} | {x['interface'] for x in self.s['forward_sources']}
            desired = [(None, name, key, value) for name in root_ifaces for key, value in [('forwarding', '1'), ('rp_filter', '2')]]
            desired += [(GW, name, 'forwarding', '1') for name in (GW_BUS_IF, 'wg0')]
            self.s.setdefault('sysctls', [])
            for ns, iface, key, value in desired:
                path = '/proc/sys/net/ipv4/conf/' + iface + '/' + key
                before = command(['cat', path], ns=ns).stdout.decode().strip()
                index = json_command(['ip', '-j', 'link', 'show', iface], ns=ns)[0]['ifindex']
                record = {'namespace': ns, 'interface': iface, 'ifindex': index,
                          'path': path, 'before': before, 'written': value}
                self.s['sysctls'].append(record)
                self.save()
                command(['tee', path], ns=ns, data=(value + '\n').encode())
        else:
            records = self.s.get('sysctls', [])
            restorable = []
            for rec in records:
                links = json_command(['ip', '-j', 'link', 'show'], ns=rec['namespace'])
                same = any(x['ifname'] == rec['interface'] and x['ifindex'] == rec['ifindex'] for x in links)
                if not same:
                    # The original interface is gone. Never restore settings to
                    # a replacement interface merely reusing the same name.
                    continue
                current = command(['cat', rec['path']], ns=rec['namespace'], okay=True)
                if current.returncode or current.stdout.decode().strip() != rec['written']:
                    raise Blocked('Owned interface sysctl drift; external settings preserved.')
                restorable.append(rec)
            for rec in restorable:
                command(['tee', rec['path']], ns=rec['namespace'], data=(rec['before'] + '\n').encode())
            self.s.pop('sysctls', None)
            self.save()

    def health(self):
        if self.s.get('emergency'):
            raise Blocked('An emergency deny is retained after a failed transaction; reviewed rollback is required.')
        self.assert_table()
        for name in (TRANSPORT, GW):
            self.assert_namespace(name)
            self.assert_table(name)
        links = json_command(['ip', '-j', 'link', 'show'], ns=GW)
        if {x['ifname'] for x in links} != {'lo', 'wg0', GW_BUS_IF}:
            raise Blocked('Gateway contains unexpected interfaces.')
        if not any(x['ifname'] == 'wg0' and 'UP' in x['flags'] for x in links):
            raise Blocked('WireGuard interface is missing or down.')
        if not any(x['ifname'] == GW_BUS_IF and 'UP' in x['flags'] for x in links):
            raise Blocked('Gateway business interface is down.')
        root_links = json_command(['ip', '-j', 'link', 'show'])
        for iface, key in ((ROOT_IF, 'veth_index'), (BUS_IF, 'business_veth_index')):
            if not any(x['ifname'] == iface and x['ifindex'] == self.s.get(key) and 'UP' in x['flags'] for x in root_links):
                raise Blocked('Owned root transport/business interface is missing, replaced or down.')
        transport_links = json_command(['ip', '-j', 'link', 'show'], ns=TRANSPORT)
        if {x['ifname'] for x in transport_links} != {'lo', NS_IF}:
            raise Blocked('Transport namespace contains unexpected interfaces.')
        endpoints = command(['wg', 'show', 'wg0', 'endpoints'], ns=GW).stdout.decode().split()
        allowed = command(['wg', 'show', 'wg0', 'allowed-ips'], ns=GW).stdout.decode().split()
        if endpoints != [self.p['peer_public_key'], f"{self.p['endpoint_ipv4']}:{self.p['endpoint_port']}"] or allowed != [self.p['peer_public_key'], '0.0.0.0/0']:
            raise Blocked('WireGuard peer/endpoint/allowed-ips drift; fixed backend not ready.')
        if not self.s.get('routing') or self.routing_hash() != self.s['routing']['hash']:
            raise Blocked('Shared protected policy routing absent or changed.')
        self.checks.append({'name': 'linux-guard-and-interfaces', 'outcome': 'Passed',
                            'message': 'Owned rules match; wg0 is up; no end-to-end claim.'})

    def stop(self):
        self.same_generation()
        self.close_requested = True
        self.replace_table(self.root_rules())  # withdraw outer exception BEFORE tearing down
        self.safe_closed = True
        # Refuse removal of an active workload; root deny already blocks its outer path.
        for name in list(self.s['namespaces']):
            self.assert_namespace(name)
            if command(['ip', 'netns', 'pids', name]).stdout.strip():
                self.s['phase'] = 'Blocked'
                self.save()
                raise Blocked('Workloads still running; outer tunnel denied, namespace retained. Stop workloads then retry.')
        self.remove_routing()
        self.sysctl_forward(False)
        for iface, index_key in [(ROOT_IF, 'veth_index'), (BUS_IF, 'business_veth_index')]:
            if index_key not in self.s:
                continue
            links = json_command(['ip', '-j', 'link', 'show'])
            found = [x for x in links if x['ifname'] == iface]
            if found:
                if found[0]['ifindex'] != self.s[index_key]:
                    raise Blocked('Transport veth ownership changed; interface preserved.')
                command(['ip', 'link', 'delete', iface])
            del self.s[index_key]
            self.save()
        for name in [GW, TRANSPORT]:
            if name in self.s['namespaces']:
                self.assert_namespace(name)
                command(['ip', 'netns', 'delete', name])
                del self.s['namespaces'][name]
                self.s['tables'].pop(name, None)
                self.save()
        self.s['phase'] = 'Guarded'
        self.save()
        return self.event('stop', 'Succeeded', 'Tunnel stopped; shared OUTPUT/FORWARD deny guard remains installed.')

    def boot_guard(self):
        """Reinstate deny before any user workload can use shared networking."""
        self.same_generation()
        self.p = read_profile(self.args.profile)
        self.boot_reconcile()
        # A boot hook has no user attestation and never opens the endpoint.
        if table_hash() is None:
            self.replace_table(self.root_rules(), initial=True)
        else:
            self.replace_table(self.root_rules())
        self.safe_closed = True
        self.s['root_namespace'] = os.readlink('/proc/self/ns/net')
        self.s['phase'] = 'Guarded'
        self.save()
        return self.event(
            'guard',
            'Succeeded',
            'Boot deny guard loaded; transport remains closed until a fresh Windows audit.',
        )

    def status(self):
        if not self.s:
            return self.event('status', 'Blocked', 'Not installed.')
        if self.s.get('role') == 'distro-config':
            self.same_generation()
            return self.event('status', 'Succeeded', 'Distro configuration journal only; no runtime protection claim.')
        self.load()
        self.assert_table()
        if self.s['phase'] == 'Running':
            self.health()
        return self.event('status', 'Succeeded', 'Local state inspected; full-WSL end-to-end acceptance is not verified.')

    def sandbox(self):
        self.load()
        self.attestation()
        self.health()
        if self.s['phase'] != 'Running' or not self.args.command:
            raise Blocked('run requires a running tunnel and a command after --.')
        p = self.p
        for fd in (0, 1, 2):
            mode = os.fstat(fd).st_mode
            # pipes/TTYs are explicit streams; host directories/files/sockets and
            # namespace descriptors must never become /proc/self/fd backdoors.
            if not (stat.S_ISFIFO(mode) or (stat.S_ISCHR(mode) and (os.isatty(fd) or os.path.realpath('/proc/self/fd/' + str(fd)) == '/dev/null'))):
                raise Blocked('Workload stdio must use a pipe, TTY or /dev/null; host descriptors refused.')
        # A new, allowlisted root avoids Windows mounts, host Unix sockets, netns handles,
        # /run, user homes, /sys, host /proc and inherited host control channels.
        root = Path('/run/cor-egress-sandbox')
        root.mkdir(mode=0o755, exist_ok=True)
        private_path(root, directory=True)
        root = root / self.s['owner']
        root.mkdir(mode=0o755, exist_ok=True)
        private_path(root, directory=True)
        for name, content in {
            'resolv.conf': f"nameserver {p['resolver_ipv4']}\noptions timeout:2 attempts:1\n",
            'passwd': f"sandbox:x:{p['uid']}:{p['gid']}:Sandbox:/home/sandbox:/bin/sh\n",
            'group': f"sandbox:x:{p['gid']}:\n", 'nsswitch.conf': 'hosts: files dns\npasswd: files\ngroup: files\n',
            'hosts': '127.0.0.1 localhost\n'}.items():
            atomic(root / name, content.encode(), mode=0o644)
        argv = ['ip', 'netns', 'exec', GW, 'setpriv', '--no-new-privs',
                '--reuid', str(p['uid']), '--regid', str(p['gid']), '--clear-groups',
                '--inh-caps=-all', '--ambient-caps=-all', '--bounding-set=-all',
                'bwrap', '--die-with-parent', '--new-session', '--unshare-user',
                '--unshare-pid', '--unshare-ipc', '--unshare-uts', '--unshare-cgroup-try',
                '--disable-userns', '--cap-drop', 'ALL', '--uid', str(p['uid']), '--gid', str(p['gid']),
                '--clearenv', '--setenv', 'PATH', '/usr/bin:/bin', '--setenv', 'HOME', '/home/sandbox',
                '--setenv', 'LANG', 'C.UTF-8', '--setenv', 'TZ', 'Europe/Berlin',
                '--tmpfs', '/', '--proc', '/proc', '--dev', '/dev', '--tmpfs', '/tmp',
                '--dir', '/home', '--dir', '/home/sandbox', '--dir', '/etc']
        for name in ['/usr', '/bin', '/sbin', '/lib', '/lib64']:
            if Path(name).is_symlink():
                argv += ['--symlink', os.readlink(name), name]
            elif Path(name).is_dir():
                argv += ['--ro-bind', name, name]
        # /usr/local can contain control sockets; hide it even though /usr is readable.
        argv += ['--tmpfs', '/usr/local']
        if Path('/usr/lib/wsl').exists():
            argv += ['--tmpfs', '/usr/lib/wsl']
        for name in ['resolv.conf', 'passwd', 'group', 'nsswitch.conf', 'hosts']:
            argv += ['--ro-bind', str(root / name), '/etc/' + name]
        if Path('/etc/ssl/certs').is_dir():
            argv += ['--ro-bind', '/etc/ssl/certs', '/etc/ssl/certs']
        argv += ['--chdir', '/home/sandbox', '--', *self.args.command]
        # Preserve stdio for interactive commands; never inherit controller/namespace fds.
        fcntl.flock(self.lock.fileno(), fcntl.LOCK_UN)
        result = subprocess.run(argv, env=ENV, cwd='/', close_fds=True, check=False)
        self.checks.append({'name': 'workload-exit', 'outcome': 'Passed' if result.returncode == 0 else 'Failed',
                            'exit_code': result.returncode})
        return self.event('run', 'Succeeded' if result.returncode == 0 else 'Failed',
                          'Workload completed in unprivileged mount/pid/ipc/UTS isolation; end-to-end acceptance remains unverified.')

    def restore_files(self):
        # Preflight EVERY file before restoring any file; never overwrite later edits.
        for name, rec in list(self.s.get('files', {}).items()):
            path = Path(name)
            original = (path.is_symlink() and os.readlink(path) == rec['link']) if rec['link'] is not None else (
                (not path.exists() and not path.is_symlink()) if not rec['exists'] else
                (not path.is_symlink() and path.is_file() and digest(path.read_bytes()) == rec.get('original_hash')))
            if original and (rec.get('pending') or rec.get('restoring')):
                del self.s['files'][name]
                self.save()
                continue
            if not self.written_file_matches(path, rec):
                raise Blocked('Distro configuration drift; owned snapshot restore refused.')
        for name, rec in list(self.s.get('files', {}).items()):
            path = Path(name)
            rec['restoring'] = True
            self.save()
            if rec['link'] is not None:
                temp = path.with_name('.cor-restore-' + self.s['owner'])
                os.symlink(rec['link'], temp)
                os.lchown(temp, 0, rec['gid'])
                os.replace(temp, path)
            elif rec['exists']:
                backup = private_path(rec['backup'])
                if digest(backup.read_bytes()) != rec['original_hash']:
                    raise Blocked('Snapshot digest mismatch; original preserved.')
                fd, temp = tempfile.mkstemp(prefix='.cor-', dir=path.parent)
                with os.fdopen(fd, 'wb') as stream:
                    os.fchmod(stream.fileno(), rec['mode'])
                    os.fchown(stream.fileno(), 0, rec['gid'])
                    stream.write(backup.read_bytes())
                    stream.flush()
                    os.fsync(stream.fileno())
                os.replace(temp, path)
            else:
                path.unlink()
            sync_directory(path.parent)
            del self.s['files'][name]
            self.save()

    def rollback(self):
        if not self.args.release_guard:
            raise Blocked('rollback/uninstall requires --release-guard after Windows withdrawal and stopped workloads.')
        self.same_generation()
        if self.s.get('phase') == 'RolledBack':
            return self.event(self.args.action, 'Succeeded', 'Generation already rolled back; recovery artifacts retained.')
        if self.s.get('phase') != 'ReleasingGuard':
            self.stop()
            self.restore_files()
            self.assert_table()
            self.s['phase'] = 'ReleasingGuard'
            self.save()
        emergency = self.s.get('emergency')
        if emergency:
            r = command(['nft', '-j', 'list', 'table', 'inet', emergency['name']], okay=True)
            if r.returncode == 0 and digest(encode(clean_nft(json.loads(r.stdout)))) != emergency['hash']:
                raise Blocked('Emergency deny table drift; later changes preserved.')
        if table_hash() is not None:
            self.assert_table()
            command(['nft', 'delete', 'table', 'inet', TABLE])
        self.s['tables'].pop('root', None)
        self.save()
        if emergency:
            if command(['nft', '-j', 'list', 'table', 'inet', emergency['name']], okay=True).returncode == 0:
                command(['nft', 'delete', 'table', 'inet', emergency['name']])
            self.s.pop('emergency', None)
        self.s['phase'] = 'RolledBack'
        self.save()
        return self.event(self.args.action, 'Succeeded', 'Owned distro files restored and only the owned guard removed; root-private backups retained.')

    def test(self):
        self.load()
        self.health()
        self.checks.append({'name': 'end-to-end', 'outcome': 'NotRun',
                            'message': 'Run the explicitly gated isolated CI suite; production tests are never implicit.'})
        return self.event('test', 'Blocked', 'Local invariant inspection only. No end-to-end acceptance and no external probe was performed.')


def parse(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['install', 'configure', 'audit-config', 'restore-config', 'status', 'start', 'stop', 'run', 'test', 'rollback', 'uninstall', 'guard'])
    parser.add_argument('--profile', type=Path, default=PROFILE)
    parser.add_argument('--generation', type=int)
    parser.add_argument('--scope-attestation', type=Path)
    parser.add_argument('--acknowledge-shared-guard', action='store_true')
    parser.add_argument('--release-guard', action='store_true')
    # REMAINDER on a positional would consume --profile. Split only at explicit --.
    tokens = list(sys.argv[1:] if argv is None else argv)
    command_args = []
    if '--' in tokens:
        pos = tokens.index('--')
        command_args = tokens[pos + 1:]
        tokens = tokens[:pos]
    args = parser.parse_args(tokens)
    args.command = command_args
    if command_args and args.action != 'run':
        parser.error('Only run accepts a command after --.')
    if args.action != 'guard' and args.generation is None:
        parser.error('--generation is required for every action except the fixed boot guard.')
    return args


def main(argv=None):
    args = parse(argv)
    manager = None
    try:
        if sys.platform != 'linux' or os.geteuid() != 0:
            raise Blocked('Linux root helper required; no action taken.')
        os.umask(0o077)
        os.environ.clear()
        os.environ.update(ENV)
        STATE.mkdir(mode=0o700, parents=True, exist_ok=True)
        private_path(STATE, directory=True)
        lockfile = STATE / 'manager.lock'
        fd = os.open(lockfile, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, 'w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            manager = Manager(args)
            manager.lock = lock
            if manager.s and (manager.s.get('root_namespace') != os.readlink('/proc/self/ns/net') or manager.s.get('boot_id') != boot_id()):
                # Namespace inode identity changes at reboot. Boot guard has its own narrow path.
                if args.action not in ('guard', 'restore-config', 'audit-config', 'install', 'configure'):
                    raise Blocked('Shared namespace changed since install; boot guard/reconciliation required.')
            if args.action == 'guard':
                manager.close_requested = True
                event = manager.boot_guard()
            else:
                method = {'run': 'sandbox', 'restore-config': 'restore_config', 'audit-config': 'audit_config'}.get(args.action, args.action)
                action = manager.rollback if args.action in ('rollback', 'uninstall') else getattr(manager, method)
                event = action()
            print(json.dumps(event, sort_keys=True), file=sys.stderr if args.action == 'run' else sys.stdout)
            return 0 if event['outcome'] == 'succeeded' else 2
    except (Blocked, ValueError, KeyError, TypeError, IndexError, OSError, configparser.Error) as exc:
        message = (str(exc) if isinstance(exc, Blocked)
                   else f'{type(exc).__name__}: invalid configuration or Linux '
                        'operation failed; no sensitive details disclosed.')
        if manager:
            if (manager.dirty or manager.close_requested) and not manager.safe_closed:
                try:
                    manager.emergency_deny()
                except (Blocked, ValueError, KeyError, TypeError, IndexError, OSError):
                    manager.checks.append({'name': 'emergency-deny', 'outcome': 'Failed',
                                           'message': 'Emergency deny could not be confirmed; Windows must retain its independent deny.'})
            event = manager.event(args.action, 'Blocked', message)
        else:
            event = {'schema': 1, 'schema_version': 1, 'generation': args.generation or 0, 'action': args.action,
                     'outcome': 'blocked', 'protection': 'Blocked', 'phase': 'Blocked',
                     'backend': 'wireguard', 'message': message, 'checks': [], 'logs': []}
        print(json.dumps(event, sort_keys=True), file=sys.stderr if args.action == 'run' else sys.stdout)
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
