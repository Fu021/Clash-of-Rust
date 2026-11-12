"""IPv6 lifecycle checks called only inside test-linux-tun's private namespaces."""
import json
from http.client import HTTPException
from pathlib import Path
import subprocess
import time


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
        assert current['ipv4']['kind'] == 'tun', current
        if expected_ipv6:
            assert current['ipv6']['kind'] == 'tun', current
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
                assert current['ipv4']['kind'] == 'tun', current
                if expected_ipv6:
                    assert current['ipv6']['kind'] in ('tun', 'other'), current
                    bypass = current['ipv6']['kind'] == 'other'
                    assert ('tun_ipv6_route_bypass' in routes['warnings']) == bypass, routes
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
        invoke('stop')
        assert invoke('start')['running']
        check(False, expect_unconfigured=True)
        protected_roundtrip(False)
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
