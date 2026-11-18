"""Collect native DNS observations for disposable macOS installation tests."""
from concurrent.futures import ThreadPoolExecutor
import ipaddress
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time


HOSTS = ('github.com', 'api.github.com', 'static.rust-lang.org', 'pypi.org')
STATE = Path(os.environ.get('RUNNER_TEMP', 'target'))/'cor-runner-network'
PROBE_SECONDS = 4


def write_report(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix('.new')
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')
    temporary.chmod(0o600)
    temporary.replace(path)


def command(arguments, timeout=5):
    try:
        result = subprocess.run(arguments, capture_output=True, text=True, timeout=timeout)
        return {'returncode': result.returncode, 'stdout': result.stdout[:16384],
                'stderr': result.stderr[:4096]}
    except (OSError, subprocess.TimeoutExpired) as error:
        return {'returncode': None, 'error': type(error).__name__}


def service_settings(service='Ethernet'):
    value = {}
    for kind in ('dnsservers', 'searchdomains'):
        text = subprocess.check_output(
            ['/usr/sbin/networksetup', '-get' + kind, service],
            text=True, timeout=5).strip()
        value[kind] = [] if text.startswith(("There aren't any", "You haven't set")) else text.splitlines()
    for address in value['dnsservers']:
        ipaddress.ip_address(address)
    return value


def native_lookup(host):
    started = time.monotonic()
    result = command([
        sys.executable, '-c',
        'import json,socket,sys; '
        'print(json.dumps(sorted({value[4][0] for value in '
        'socket.getaddrinfo(sys.argv[1],443,type=socket.SOCK_STREAM)})))', host],
        timeout=PROBE_SECONDS)
    result['host'] = host
    result['elapsed_seconds'] = round(time.monotonic() - started, 3)
    result['addresses'] = []
    if result['returncode'] == 0:
        try:
            result['addresses'] = [str(ipaddress.ip_address(address))
                                   for address in json.loads(result['stdout'])]
        except (TypeError, ValueError):
            result['error'] = 'InvalidResolverOutput'
    return result


def native_probes():
    with ThreadPoolExecutor(max_workers=len(HOSTS)) as workers:
        return list(workers.map(native_lookup, HOSTS))


def observation(probe=True):
    value = {
        'observed_at': time.time(),
        'resolver': command(['/usr/sbin/scutil', '--dns']),
        'route': command(['/sbin/route', '-n', 'get', 'default']),
    }
    try:
        value['settings'] = service_settings()
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        value['settings_error'] = type(error).__name__
    if probe:
        value['probes'] = native_probes()
    return value


def resolver_servers(resolver):
    default = resolver.get('stdout', '').split('resolver #2', 1)[0]
    addresses = re.findall(r'nameserver\[\d+\]\s*:\s*(\S+)', default)
    servers = []
    for address in addresses:
        try:
            normalized = str(ipaddress.ip_address(address))
        except ValueError:
            continue
        if normalized not in servers:
            servers.append(normalized)
    return servers


def direct_probes(resolver, failed):
    arguments = [(server, host) for server in resolver_servers(resolver)[:2] for host in failed]

    def probe(value):
        server, host = value
        result = command(['/usr/bin/dig', '+time=2', '+tries=1', '+noall',
                          '+comments', '+answer', '@' + server, host, 'A'], timeout=3)
        return {'server': server, 'host': host, **result}

    with ThreadPoolExecutor(max_workers=4) as workers:
        return list(workers.map(probe, arguments))
