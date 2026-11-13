"""IPv6 lifecycle checks called only inside test-linux-tun's private namespaces."""
import json
from http.client import HTTPException
import os
from pathlib import Path
import signal
import subprocess
import time


def check_protected_ipv6_route(routes, current):
    sample = current['ipv6']
    if sample['state'] == 'observed':
        assert sample.get('kind') in ('tun', 'other'), current
        bypass = sample['kind'] == 'other'
        assert ('tun_ipv6_route_bypass' in routes['warnings']) == bypass, routes
        return
    assert sample == {'state': 'unknown', 'reason': 'tcp_route_unavailable'}, current
    probe = subprocess.run([
        'ip', '-j', '-6', 'route', 'get', '2606:4700:4700::1111',
        'uid', str(current['uid']), 'ipproto', 'tcp', 'dport', '443',
    ], capture_output=True, text=True, timeout=10, env=dict(os.environ, LC_ALL='C'))
    assert (probe.returncode == 2 and not probe.stdout.strip()
            and 'Network is unreachable' in probe.stderr), (
        f'IPv6 sample remains unconfirmed: {current}; '
        f'route query exit={probe.returncode}, stdout={probe.stdout!r}, stderr={probe.stderr!r}')
    assert not routes['warnings'], routes
    print('PASS: protected IPv6 sample remains unknown; an independent route query '
          'confirms network unreachable, not an observed bypass', flush=True)


def verify(directory, invoke, api, http):
    assert Path('/run/dbus/system_bus_socket').exists()
    profile = directory/'ipv6-lifecycle.yaml'
    profile.write_text(
        'proxies: [{name: Fixture, type: socks5, server: 192.0.2.254, port: 1080}]\n'
        'proxy-groups: [{name: Main, type: select, proxies: [Fixture]}]\n'
        'rules: ["MATCH,Main"]\n'
        'dns: {enable: true, enhanced-mode: fake-ip, '
        'nameserver: ["https://192.0.2.254/dns-query#DIRECT"]}\n'
        'tun: {device: CorRootTest, dns-hijack: []}\n')

    def change(*arguments):
        subprocess.run(['ip', '-6', *arguments], check=True, timeout=10)

    def uplink(enabled):
        if enabled:
            change('addr', 'add', '2001:db8:1::1/64', 'dev', 'uplink', 'nodad')
            change('route', 'add', 'default', 'via', '2001:db8:1::fe', 'dev', 'uplink')
        else:
            change('route', 'del', 'default', 'via', '2001:db8:1::fe', 'dev', 'uplink')
            change('addr', 'del', '2001:db8:1::1/64', 'dev', 'uplink')

    def check(expected_ipv6, expect_unconfigured=False):
        status = invoke('status')
        assert status['running'] and status['proxy'] == 'tun', status
        routes = status['host_routes']
        assert routes['ipv6_uplink']['state'] == ('available' if expected_ipv6 else 'unavailable'), routes
        assert not routes['warnings'], routes
        current = next(sample for sample in routes['tcp_uid_samples']
                       if sample['uid'] == 0 and sample['protocol'] == 'tcp'
                       and sample['destination_port'] == 443)
        assert current['ipv4'].get('kind') == 'tun', current
        if expected_ipv6:
            assert current['ipv6'].get('kind') == 'tun', current
        config = api('configs')
        assert config['tun']['enable'], config['tun']
        if expected_ipv6:
            assert config['tun'].get('inet6-address'), config['tun']
        elif expect_unconfigured:
            assert not config['tun'].get('inet6-address'), config['tun']
        assert http('203.0.113.20', 80, '/auto-check') == b'protected-ok'
        if expected_ipv6:
            selected = json.loads(subprocess.check_output([
                'ip', '-j', '-6', 'route', 'get', '2001:db8:2::20',
                'uid', '0', 'ipproto', 'tcp', 'dport', '80',
            ], text=True, timeout=10))
            assert selected[0]['dev'] == 'CorRootTest', selected
            assert http('2001:db8:2::20', 80, '/auto-check') == b'protected-ok'
        return status

    def refresh_count():
        return len(invoke('logs', '--limit', '500', '--filter', '网络刷新完成'))

    def wait_for_family(expected_ipv6, previous_refreshes):
        deadline = time.monotonic()+35
        last = None
        while time.monotonic() < deadline:
            try:
                assert refresh_count() > previous_refreshes, 'The background refresh has not completed'
                return check(expected_ipv6)
            except (AssertionError, OSError, HTTPException) as error:
                last = error
                time.sleep(1)
        raise AssertionError(f'IPv6 transition did not converge: {last}')

    def protection_checks():
        return len(invoke('logs', '--limit', '500', '--filter', 'Claude 保护期间网络发生变化'))

    def wait_for_protected_family(expected_ipv6, previous_checks, locked):
        deadline = time.monotonic()+35
        last = None
        while time.monotonic() < deadline:
            try:
                assert protection_checks() > previous_checks, 'The protected coverage check has not run'
                status = invoke('status')
                assert status['running'] and status['proxy'] == 'tun' and status['protected'], status
                routes = status['host_routes']
                assert routes['ipv6_uplink']['state'] == (
                    'available' if expected_ipv6 else 'unavailable'), routes
                assert routes['coverage_expectations']['ipv6'] is expected_ipv6, routes
                current = next(sample for sample in routes['tcp_uid_samples']
                               if sample['uid'] == 0 and sample['protocol'] == 'tcp'
                               and sample['destination_port'] == 443)
                assert current['ipv4'].get('kind') == 'tun', current
                if expected_ipv6:
                    check_protected_ipv6_route(routes, current)
                else:
                    assert not routes['warnings'], routes
                assert http('203.0.113.20', 80, '/auto-check') == b'protected-ok'
                for path, content in locked.items():
                    assert path.read_bytes() == content, f'Protection rewrote {path.name}'
                return
            except (AssertionError, OSError, HTTPException) as error:
                last = error
                time.sleep(1)
        raise AssertionError(f'Protected IPv6 transition did not converge: {last}')

    def protected_roundtrip(initial_ipv6):
        locked = {
            path: path.read_bytes()
            for path in (settings_path, directory/'data/runtime/config.yaml')
        }
        for enabled in (not initial_ipv6, initial_ipv6):
            previous_checks = protection_checks()
            uplink(enabled)
            try:
                wait_for_protected_family(enabled, previous_checks, locked)
            except BaseException:
                if enabled != initial_ipv6:
                    uplink(initial_ipv6)
                raise

    def protected_restart(expected_ipv6):
        def child_pids(pid):
            children = set()
            for thread in Path(f'/proc/{pid}/task').iterdir():
                try:
                    children.update((thread/'children').read_text().split())
                except FileNotFoundError:
                    continue
            return children

        control = directory/'data/control.json'
        endpoint = json.loads(control.read_text())
        owner_pid = endpoint['pid']
        assert endpoint['owner'] == 'daemon' and owner_pid > 1 and owner_pid != os.getpid()
        environment = Path(f'/proc/{owner_pid}/environ').read_bytes().split(b'\0')
        assert os.fsencode(f'CLASH_OF_RUST_DATA_DIR={directory/"data"}') in environment
        previous = json.loads(settings_path.read_text())
        os.kill(owner_pid, signal.SIGTERM)
        deadline = time.monotonic()+35
        while True:
            try:
                process_state = Path(f'/proc/{owner_pid}/stat').read_text().rpartition(') ')[2].split()[0]
            except FileNotFoundError:
                break
            if process_state == 'Z':
                break
            assert time.monotonic() < deadline, 'The protected test owner did not exit'
            time.sleep(0.1)
        assert json.loads(settings_path.read_text())['claude_protection'] == previous['claude_protection']
        assert invoke('start')['running']
        current = json.loads(control.read_text())
        assert current['pid'] != owner_pid
        check(expected_ipv6, expect_unconfigured=not expected_ipv6)
        assert invoke('status')['protected']
        runtime = (directory/'data/runtime/config.yaml').read_bytes()
        previous_children = child_pids(current['pid'])
        assert previous_children, 'The protected owner has no live core child'
        assert invoke('start')['running']
        assert json.loads(control.read_text())['pid'] == current['pid']
        assert child_pids(current['pid']) == previous_children, 'Repeated start replaced the core'
        assert (directory/'data/runtime/config.yaml').read_bytes() == runtime
        restored = json.loads(settings_path.read_text())
        for field in ('claude_protection', 'proxy_mode', 'run_mode', 'active_profile'):
            assert restored[field] == previous[field], f'Protected restart changed {field}'
        print('PASS: protected owner exit/relaunch and idempotent explicit start preserve '
              f'protection, locked configuration and real TUN traffic; IPv6={expected_ipv6}', flush=True)

    invoke('proxy', 'off')
    invoke('profile', 'import', profile, '--name', 'ipv6-lifecycle')
    invoke('profile', 'use', 'ipv6-lifecycle')
    settings_path = directory/'data/settings.json'
    initial = json.loads(settings_path.read_text())
    assert initial['ipv6'] is True, 'The default IPv6 preference must remain enabled'
    uplink_present = True
    try:
        uplink(False)
        uplink_present = False
        invoke('stop')
        assert invoke('start')['running']
        invoke('proxy', 'tun')
        check(False, expect_unconfigured=True)
        invoke('stop')
        assert invoke('start')['running']
        check(False, expect_unconfigured=True)
        assert invoke('claude', 'protect', 'on', '--yes')['enabled']
        check(False, expect_unconfigured=True)
        rejected = invoke('stop', success=False)
        assert '请先关闭 Claude 保护' in rejected, rejected
        assert invoke('status')['protected'], 'Rejected stop disabled Claude protection'
        check(False, expect_unconfigured=True)
        protected_roundtrip(False)
        protected_restart(False)
        assert not invoke('claude', 'protect', 'off')['enabled']
        previous = settings_path.read_bytes()
        previous_refreshes = refresh_count()
        uplink(True)
        uplink_present = True
        wait_for_family(True, previous_refreshes)
        assert settings_path.read_bytes() == previous, 'IPv6 arrival changed user settings'
        previous_refreshes = refresh_count()
        uplink(False)
        uplink_present = False
        wait_for_family(False, previous_refreshes)
        assert settings_path.read_bytes() == previous, 'IPv6 loss changed user settings'
        previous_refreshes = refresh_count()
        uplink(True)
        uplink_present = True
        wait_for_family(True, previous_refreshes)
        invoke('proxy', 'off')
        invoke('proxy', 'tun')
        check(True)
        assert invoke('claude', 'protect', 'on', '--yes')['enabled']
        check(True)
        protected_roundtrip(True)
        check(True)
        protected_restart(True)
        print('PASS: default IPv6 survives an IPv4-only startup, restart, IPv6 arrival/loss '
              'and TUN reconnect; Claude protection with IPv6 absent/present, protected '
              'arrival/loss, IPv4/IPv6 HTTP and unchanged settings verified', flush=True)
    finally:
        invoke('claude', 'protect', 'off')
        invoke('proxy', 'off')
        if not uplink_present:
            uplink(True)
        invoke('profile', 'use', 'root-network')
        invoke('profile', 'remove', 'ipv6-lifecycle', '--yes')
        invoke('proxy', 'tun')
