"""Restore disposable macOS test networking before Actions report uploads."""
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from urllib.parse import urlsplit

import macos_runner_network as network


SNAPSHOT = network.STATE/'test-network.json'
REPORT = network.STATE/'test-network-report.json'
SERVICE = network.STATE/'test-service.json'
HOSTS = ('github.com', 'api.github.com', 'codeload.github.com',
         'objects.githubusercontent.com', 'release-assets.githubusercontent.com',
         'results-receiver.actions.githubusercontent.com', 'static.rust-lang.org', 'pypi.org')
PROXIES = ('webproxy', 'securewebproxy', 'socksfirewallproxy',
           'autoproxyurl', 'proxyautodiscovery')
RECOVERY_SECONDS = 30


def require_runner():
    if sys.platform != 'darwin' or os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('Network recovery is restricted to disposable macOS Actions runners')


def endpoints():
    hosts = list(HOSTS)
    for key in ('ACTIONS_RESULTS_URL', 'ACTIONS_CACHE_URL', 'ACTIONS_RUNTIME_URL'):
        value = urlsplit(os.environ.get(key, ''))
        host = value.hostname
        if (value.scheme == 'https' and host and
                host.endswith(('.actions.githubusercontent.com', '.blob.core.windows.net')) and
                host not in hosts):
            hosts.append(host)
    return hosts


def https_probe(host):
    result = network.command([
        '/usr/bin/curl', '--head', '--silent', '--show-error', '--noproxy', '*',
        '--connect-timeout', '3', '--max-time', '5', '--output', '/dev/null',
        '--write-out', '%{http_code}', 'https://' + host + '/'], timeout=6)
    status = result.get('stdout', '').strip()
    return {'host': host, **result,
            'reachable': result.get('returncode') == 0 and status.isdigit() and
                         100 <= int(status) <= 599}


def probes():
    hosts = endpoints()
    with ThreadPoolExecutor(max_workers=4) as workers:
        return list(workers.map(https_probe, hosts))


def settings(service, kind):
    return subprocess.check_output(
        ['/usr/sbin/networksetup', '-get' + kind, service], text=True, timeout=5).strip()


def proxy_settings(service, kind):
    text = settings(service, kind)
    if kind == 'proxyautodiscovery':
        if text.rsplit(' ', 1)[-1] not in ('On', 'Off'):
            raise RuntimeError('Unrecognized proxy discovery state')
        return {'enabled': text.endswith('On')}
    value = properties(text)
    if value.get('Enabled') not in ('Yes', 'No'):
        raise RuntimeError('Unrecognized ' + kind + ' state')
    result = {'enabled': value['Enabled'] == 'Yes'}
    if result['enabled']:
        if kind == 'autoproxyurl':
            result['url'] = value['URL']
        else:
            result.update(server=value['Server'], port=value['Port'],
                          authenticated=any(key.startswith('Authenticated') and item in ('1', 'Yes')
                                            for key, item in value.items()))
    return result


def configuration():
    services = subprocess.check_output(
        ['/usr/sbin/networksetup', '-listallnetworkservices'], text=True, timeout=5).splitlines()[1:]
    return {service: {'dns': network.service_settings(service),
                      'proxies': {kind: proxy_settings(service, kind) for kind in PROXIES},
                      'bypass': settings(service, 'proxybypassdomains')}
            for service in services if service and not service.startswith('*')}


def properties(text):
    return dict(line.split(': ', 1) for line in text.splitlines() if ': ' in line)


def set_value(service, kind, *values):
    subprocess.run(['/usr/bin/sudo', '-n', '/usr/sbin/networksetup',
                    '-set' + kind, service, *values], check=True, timeout=5)


def restore(original):
    current = configuration()
    if current.keys() != original.keys():
        raise RuntimeError('Network service inventory changed during the isolated test')
    for service, baseline in original.items():
        actual = current[service]
        for kind, values in baseline['dns'].items():
            if actual['dns'][kind] != values:
                set_value(service, kind, *(values or ['Empty']))
        for kind, value in baseline['proxies'].items():
            if actual['proxies'][kind] == value:
                continue
            enabled = value['enabled']
            if enabled and kind != 'proxyautodiscovery':
                if kind == 'autoproxyurl':
                    set_value(service, kind, value['url'])
                else:
                    if value['authenticated']:
                        raise RuntimeError('Cannot restore authenticated runner proxy without credentials')
                    set_value(service, kind, value['server'], value['port'])
            state_kind = 'autoproxystate' if kind == 'autoproxyurl' else kind + 'state'
            if kind == 'proxyautodiscovery':
                state_kind = kind
            set_value(service, state_kind, 'on' if enabled else 'off')
        if actual['bypass'] != baseline['bypass']:
            domains = baseline['bypass']
            set_value(service, 'proxybypassdomains',
                      *(['Empty'] if domains.startswith("There aren't any") else domains.splitlines()))
    actual = configuration()
    if actual != original:
        raise RuntimeError('Runner DNS/proxy settings did not match the test baseline after recovery')
    return actual


def observe():
    value = network.observation(probe=False)
    value['proxy'] = network.command(['/usr/sbin/scutil', '--proxy'])
    value['interfaces'] = network.command(['/sbin/ifconfig'])
    value['routes'] = network.command(['/usr/sbin/netstat', '-rn'])
    return value


def save():
    require_runner()
    network.write_report(SNAPSHOT, configuration())
    network.write_report(REPORT, {'schema': 1, 'baseline': observe(), 'attempts': []})


def emit(report):
    value = {'status': report['status'], 'error': report.get('error'),
             'failed_hosts': report.get('failed_hosts', []),
             'attempts': report.get('attempts', []), 'report': str(REPORT)}
    print('macOS post-test network: ' + json.dumps(value), flush=True)
    summary = os.environ.get('GITHUB_STEP_SUMMARY')
    if summary:
        with open(summary, 'a') as output:
            output.write('\n### macOS post-test network\n\n' +
                         report['status'] + '; attempts: ' + str(len(report.get('attempts', []))) +
                         '; failed hosts: ' + ', '.join(report.get('failed_hosts', [])) + '\n')


def recover():
    require_runner()
    report = json.loads(REPORT.read_text()) if REPORT.exists() else {'schema': 1}
    report['before_recovery'] = observe()
    report['attempts'] = []
    report['status'] = 'running'
    network.write_report(REPORT, report)
    try:
        failures = []
        for name, operation in (
                ('service_cleanup', cleanup_service),
                ('settings_restore', lambda: restore(json.loads(SNAPSHOT.read_text())) if SNAPSHOT.exists() else None)):
            try:
                report[name] = operation()
            except Exception as error:
                report[name] = {'error': str(error)}
                failures.append(name + ': ' + str(error))
        deadline = time.monotonic() + RECOVERY_SECONDS
        refreshed = False
        expected = network.resolver_servers(report.get('baseline', {}).get('resolver', {}))
        while True:
            results = probes()
            resolver = network.command(['/usr/sbin/scutil', '--dns'], timeout=2)
            matches = resolver.get('returncode') == 0 and (
                not expected or set(network.resolver_servers(resolver)) == set(expected))
            report['attempts'].append({'observed_at': time.time(), 'probes': results,
                                       'resolver_matches_baseline': matches})
            report['failed_hosts'] = [value['host'] for value in results if not value['reachable']]
            network.write_report(REPORT, report)
            if not report['failed_hosts'] and matches:
                if failures:
                    raise RuntimeError('; '.join(failures))
                report['status'] = 'ready'
                break
            if not refreshed:
                report['cache_refresh'] = [
                    network.command(['/usr/bin/sudo', '-n', '/usr/bin/dscacheutil', '-flushcache']),
                    network.command(['/usr/bin/sudo', '-n', '/usr/bin/killall', '-HUP', 'mDNSResponder']),
                ]
                refreshed = True
            if time.monotonic() >= deadline:
                report['after_failure'] = observe()
                baseline = report.get('baseline', {}).get('resolver', {})
                report['direct_probes'] = network.direct_probes(baseline, report['failed_hosts'])
                reason = ', '.join(report['failed_hosts']) or 'active resolver mismatch'
                raise RuntimeError('; '.join([*failures, 'Runner endpoints remain unreachable: ' + reason]))
            time.sleep(min(2, max(0, deadline - time.monotonic())))
    except BaseException as error:
        report['status'] = 'failed'
        report['error'] = str(error)
        raise
    finally:
        network.write_report(REPORT, report)
        emit(report)


@contextmanager
def guarded_tests():
    save()
    primary = None
    try:
        yield
    except BaseException as error:
        primary = error
        raise
    finally:
        try:
            recover()
        except Exception:
            if primary is None:
                raise


def service_paths():
    label = f'org.clashofrust.network.{os.getuid()}'
    return (label, Path('/Library/PrivilegedHelperTools')/label,
            Path('/Library/LaunchDaemons')/(label + '.plist'))


def cleanup_service():
    if not SERVICE.exists():
        return
    require_runner()
    label, directory, agent = service_paths()
    marker = json.loads(SERVICE.read_text())
    if marker != {'label': label, 'report': str(Path(marker.get('report', '')).resolve())}:
        raise RuntimeError('Invalid test service ownership marker')
    output = Path(marker['report'])
    output.mkdir(parents=True, exist_ok=True)
    network.write_report(output/'proxy-smoke-network-before-cleanup.json', observe())
    status = network.command(['/usr/bin/sudo', '-n', '/bin/launchctl', 'print', 'system/' + label], timeout=10)
    network.write_report(output/'proxy-smoke-network-service-state.json', status)
    log = network.command(['/usr/bin/sudo', '-n', '/usr/bin/tail', '-c', '32768',
                           str(directory/'network-service.log')], timeout=10)
    (output/'proxy-smoke-network-service.log').write_text(log.get('stdout', '') + log.get('stderr', ''))
    print('macOS test helper before cleanup:\n' + log.get('stdout', '') + log.get('stderr', ''), flush=True)
    stopped = network.command(['/usr/bin/sudo', '-n', '/bin/launchctl', 'bootout', 'system/' + label], timeout=20)
    after = network.command(['/usr/bin/sudo', '-n', '/bin/launchctl', 'print', 'system/' + label], timeout=5)
    network.write_report(output/'proxy-smoke-network-service-cleanup.json',
                         {'bootout': stopped, 'after': after})
    if after['returncode'] != 113:
        raise RuntimeError('Unable to confirm isolated launchd network service was removed')
    subprocess.run(['/usr/bin/sudo', '-n', '/bin/rm', '-rf', str(directory), str(agent)],
                   check=True, timeout=10)
    SERVICE.unlink()


@contextmanager
def network_service(report):
    require_runner()
    label, directory, agent = service_paths()
    if directory.exists() or agent.exists() or SERVICE.exists():
        raise RuntimeError('Refusing to overwrite an existing network service')
    network.write_report(SERVICE, {'label': label, 'report': str(report.resolve())})
    primary = None
    try:
        yield
    except BaseException as error:
        primary = error
        raise
    finally:
        try:
            cleanup_service()
        except Exception as error:
            print('macOS test service cleanup failed: ' + str(error), flush=True)
            if primary is None:
                raise
