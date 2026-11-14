"""Preserve and verify hosted macOS runner DNS around private cache login."""
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
SNAPSHOT = Path('target/cache-network.json')
REPORT = Path('target/cache-network-report.json')
PROBE_SECONDS = 4
RESTORE_SECONDS = 30


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


def service_settings():
    value = {}
    for kind in ('dnsservers', 'searchdomains'):
        text = subprocess.check_output(
            ['/usr/sbin/networksetup', '-get' + kind, 'Ethernet'],
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


def ready(probes):
    return len(probes) == len(HOSTS) and all(probe.get('addresses') for probe in probes)


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


def preserve(restore=False):
    if sys.platform != 'darwin':
        return
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('Runner DNS preservation is restricted to disposable GitHub Actions runners')
    if not restore:
        baseline = observation()
        write_report(REPORT, {'schema': 1, 'baseline': baseline})
        if 'settings' not in baseline:
            raise RuntimeError('Unable to save original macOS runner DNS settings')
        write_report(SNAPSHOT, baseline['settings'])
        print('Saved original runner DNS; native baseline:',
              'ready' if ready(baseline['probes']) else 'unavailable before private cache login')
        return
    original = json.loads(SNAPSHOT.read_text())
    report = json.loads(REPORT.read_text())
    report['before_restore'] = observation(probe=False)
    report['restoration'] = 'running'
    report['attempts'] = []
    write_report(REPORT, report)
    try:
        for kind in ('dnsservers', 'searchdomains'):
            subprocess.run(['/usr/bin/sudo', '-n', '/usr/sbin/networksetup',
                            '-set' + kind, 'Ethernet', *(original[kind] or ['Empty'])],
                           check=True, timeout=5)
        actual = service_settings()
        report['restored_settings'] = actual
        if actual != original:
            raise RuntimeError('macOS runner DNS settings did not match the saved baseline')
        report['cache_refresh'] = [
            command(['/usr/bin/sudo', '-n', '/usr/bin/dscacheutil', '-flushcache']),
            command(['/usr/bin/sudo', '-n', '/usr/bin/killall', '-HUP', 'mDNSResponder']),
        ]
        deadline = time.monotonic() + RESTORE_SECONDS
        expected = resolver_servers(report['baseline']['resolver'])
        while True:
            probes = native_probes()
            resolver = command(['/usr/sbin/scutil', '--dns'], timeout=2)
            matches = resolver.get('returncode') == 0 and (
                not expected or set(resolver_servers(resolver)) == set(expected))
            report['attempts'].append({
                'observed_at': time.time(), 'probes': probes,
                'resolver': resolver, 'resolver_matches_baseline': matches,
            })
            write_report(REPORT, report)
            if ready(probes) and matches:
                report['restoration'] = 'ready'
                print('Restored runner DNS and verified native resolution for build services.')
                break
            if time.monotonic() >= deadline:
                failed = [probe['host'] for probe in probes if not probe.get('addresses')]
                report['failed_hosts'] = failed
                report['resolver_after_failure'] = command(['/usr/sbin/scutil', '--dns'])
                report['direct_probes'] = direct_probes(report['baseline']['resolver'], failed)
                phase = ('before private cache login' if not ready(report['baseline']['probes'])
                         else 'after private cache login')
                raise RuntimeError('macOS runner native DNS unavailable ' + phase + ': ' +
                                   (', '.join(failed) if failed else 'active resolver mismatch') +
                                   '; see ' + str(REPORT))
            time.sleep(min(2, max(0, deadline - time.monotonic())))
    except Exception as error:
        report['restoration'] = 'failed'
        report['error'] = str(error)
        print('macOS runner DNS recovery failed:', json.dumps({
            'baseline_ready': ready(report['baseline']['probes']),
            'failed_hosts': report.get('failed_hosts', []),
            'cache_refresh': report.get('cache_refresh', []),
            'direct_probes': report.get('direct_probes', []),
        }), flush=True)
        raise
    finally:
        write_report(REPORT, report)
