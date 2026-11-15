"""Offline failure/recovery regressions; never modify the development host network."""
from contextlib import contextmanager
import copy
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import macos_test_network as guard


class MacosTestNetworkTests(unittest.TestCase):
    def configuration(self):
        return {'Ethernet': {'dns': {'dnsservers': [], 'searchdomains': []},
                             'proxies': {kind: {'enabled': False} for kind in guard.PROXIES},
                             'bypass': '*.local\n169.254/16'}}

    def probes(self, reachable=True):
        return [{'host': host, 'reachable': reachable} for host in guard.HOSTS]

    @contextmanager
    def runner(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            baseline = self.configuration()
            with patch.object(guard.sys, 'platform', 'darwin'), \
                    patch.dict(os.environ, {'GITHUB_ACTIONS': 'true', 'GITHUB_STEP_SUMMARY': str(root/'summary')}), \
                    patch.object(guard, 'SNAPSHOT', root/'network.json'), \
                    patch.object(guard, 'REPORT', root/'report.json'), \
                    patch.object(guard, 'SERVICE', root/'service.json'), \
                    patch.object(guard, 'observe', return_value={'resolver': {'stdout': ''}}), \
                    patch.object(guard, 'configuration', side_effect=lambda: copy.deepcopy(baseline)) as configuration, \
                    patch.object(guard, 'probes', return_value=self.probes()) as probes, \
                    patch.object(guard.network, 'command', return_value={'returncode': 0, 'stdout': ''}) as command, \
                    patch.object(guard, 'set_value') as change, \
                    patch.object(guard.time, 'sleep') as pause:
                yield root, configuration, probes, command, change, pause

    def test_healthy_network_is_verified_without_modifying_dns_or_proxies(self):
        with self.runner() as (_, _, probes, command, change, pause):
            with guard.guarded_tests():
                pass
            change.assert_not_called()
            pause.assert_not_called()
            probes.assert_called_once()
            self.assertFalse(any('sudo' in item.args[0] for item in command.call_args_list))
            report = json.loads(guard.REPORT.read_text())
            self.assertEqual(report['status'], 'ready')
            self.assertEqual(guard.REPORT.stat().st_mode & 0o777, 0o600)

    def test_changed_dns_proxy_and_bypass_restore_to_baseline(self):
        with self.runner() as (_, configuration, _, _, change, _):
            guard.save()
            baseline = self.configuration()
            changed = copy.deepcopy(baseline)
            changed['Ethernet']['dns']['dnsservers'] = ['127.0.0.1']
            changed['Ethernet']['proxies']['webproxy'] = {
                'enabled': True, 'server': '127.0.0.1', 'port': '7897', 'authenticated': False}
            changed['Ethernet']['bypass'] = 'example.test'
            configuration.side_effect = [changed, baseline]
            guard.recover()
            self.assertEqual([item.args for item in change.call_args_list], [
                ('Ethernet', 'dnsservers', 'Empty'),
                ('Ethernet', 'webproxystate', 'off'),
                ('Ethernet', 'proxybypassdomains', '*.local', '169.254/16')])

    def test_enabled_proxy_and_pac_restore_endpoint_then_enable_state(self):
        with self.runner() as (_, configuration, _, _, change, _):
            baseline = self.configuration()
            baseline['Ethernet']['proxies']['securewebproxy'] = {
                'enabled': True, 'server': 'proxy.test', 'port': '8443', 'authenticated': False}
            baseline['Ethernet']['proxies']['autoproxyurl'] = {'enabled': True, 'url': 'https://proxy.test/pac'}
            configuration.side_effect = [self.configuration(), baseline]
            guard.restore(baseline)
            self.assertEqual([item.args for item in change.call_args_list], [
                ('Ethernet', 'securewebproxy', 'proxy.test', '8443'),
                ('Ethernet', 'securewebproxystate', 'on'),
                ('Ethernet', 'autoproxyurl', 'https://proxy.test/pac'),
                ('Ethernet', 'autoproxystate', 'on')])

    def test_disabled_proxy_normalizes_irrelevant_old_server_values(self):
        for text in ('Enabled: No\nServer: \nPort: 0\nAuthenticated Proxy Enabled: 0',
                     'Enabled: No\nServer: 127.0.0.1\nPort: 7897\nAuthenticated Proxy Enabled: 0'):
            with self.subTest(text=text), patch.object(guard, 'settings', return_value=text):
                self.assertEqual(guard.proxy_settings('Ethernet', 'webproxy'), {'enabled': False})
        with patch.object(guard, 'settings', return_value='Auto Proxy Discovery: Off'):
            self.assertEqual(guard.proxy_settings('Ethernet', 'proxyautodiscovery'), {'enabled': False})
        with patch.object(guard, 'settings', return_value='Enabled: Maybe'):
            with self.assertRaisesRegex(RuntimeError, 'Unrecognized'):
                guard.proxy_settings('Ethernet', 'webproxy')

    def test_unknown_or_authenticated_proxy_is_not_guessed(self):
        with self.runner() as (_, configuration, _, _, change, _):
            baseline = self.configuration()
            baseline['Ethernet']['proxies']['socksfirewallproxy'] = {
                'enabled': True, 'server': 'proxy.test', 'port': '1080', 'authenticated': True}
            configuration.side_effect = [self.configuration()]
            with self.assertRaisesRegex(RuntimeError, 'credentials'):
                guard.restore(baseline)
            change.assert_not_called()

    def test_service_inventory_or_failed_readback_rejects_restoration(self):
        for current in ({}, {'Other': self.configuration()['Ethernet']}):
            with self.runner() as (_, configuration, _, _, change, _):
                configuration.side_effect = [current]
                with self.assertRaisesRegex(RuntimeError, 'inventory'):
                    guard.restore(self.configuration())
                change.assert_not_called()
        with self.runner() as (_, configuration, _, _, _, _):
            changed = self.configuration()
            changed['Ethernet']['dns']['dnsservers'] = ['127.0.0.1']
            configuration.side_effect = [changed, changed]
            with self.assertRaisesRegex(RuntimeError, 'after recovery'):
                guard.restore(self.configuration())

    def test_transient_endpoint_failure_refreshes_once_then_verifies_recovery(self):
        with self.runner() as (_, _, probes, command, change, pause):
            guard.save()
            probes.side_effect = [self.probes(False), self.probes()]
            guard.recover()
            change.assert_not_called()
            self.assertEqual(probes.call_count, 2)
            pause.assert_called_once()
            self.assertEqual(sum('dscacheutil' in ' '.join(item.args[0]) for item in command.call_args_list), 1)
            self.assertEqual(json.loads(guard.REPORT.read_text())['status'], 'ready')

    def test_persistent_dns_failure_keeps_original_resolver_evidence_and_fails(self):
        with self.runner() as (_, _, probes, _, _, pause), \
                patch.object(guard, 'RECOVERY_SECONDS', 0), \
                patch.object(guard.network, 'direct_probes', return_value=[{'server': '192.0.2.53'}]):
            guard.save()
            probes.return_value = self.probes(False)
            with self.assertRaisesRegex(RuntimeError, 'endpoints remain unreachable'):
                guard.recover()
            pause.assert_not_called()
            report = json.loads(guard.REPORT.read_text())
            self.assertEqual(report['status'], 'failed')
            self.assertEqual(report['failed_hosts'], list(guard.HOSTS))
            self.assertEqual(report['direct_probes'], [{'server': '192.0.2.53'}])
            self.assertIn('failed hosts:', Path(os.environ['GITHUB_STEP_SUMMARY']).read_text())

    def test_cached_answers_cannot_hide_wrong_default_resolver(self):
        with self.runner() as (_, _, _, command, _, _), \
                patch.object(guard, 'RECOVERY_SECONDS', 0), \
                patch.object(guard.network, 'direct_probes', return_value=[]):
            guard.save()
            report = json.loads(guard.REPORT.read_text())
            report['baseline']['resolver']['stdout'] = 'resolver #1\n nameserver[0] : 192.0.2.53'
            guard.network.write_report(guard.REPORT, report)
            command.return_value = {'returncode': 0, 'stdout': 'resolver #1\n nameserver[0] : 127.0.0.1'}
            with self.assertRaisesRegex(RuntimeError, 'active resolver mismatch'):
                guard.recover()

    def test_cleanup_failure_still_attempts_dns_restore_and_endpoint_checks(self):
        with self.runner() as (_, _, probes, _, _, _):
            guard.save()
            with patch.object(guard, 'cleanup_service', side_effect=RuntimeError('helper survived')), \
                    patch.object(guard, 'restore', return_value={}) as restore:
                with self.assertRaisesRegex(RuntimeError, 'helper survived'):
                    guard.recover()
                restore.assert_called_once()
                probes.assert_called_once()
            self.assertEqual(json.loads(guard.REPORT.read_text())['status'], 'failed')

    def test_recovery_cannot_mask_primary_test_failure_or_turn_failed_cleanup_green(self):
        for recovery_error in (None, RuntimeError('DNS failed')):
            with self.runner(), patch.object(guard, 'recover', side_effect=recovery_error) as recover:
                with self.assertRaisesRegex(ValueError, 'TUN failed'):
                    with guard.guarded_tests():
                        raise ValueError('TUN failed')
                recover.assert_called_once()
        with self.runner(), patch.object(guard, 'recover', side_effect=RuntimeError('DNS failed')):
            with self.assertRaisesRegex(RuntimeError, 'DNS failed'):
                with guard.guarded_tests():
                    pass

    def test_upload_endpoint_probe_ignores_tokens_and_non_actions_hosts(self):
        with patch.dict(os.environ, {
                'ACTIONS_RESULTS_URL': 'https://example.actions.githubusercontent.com/token?secret=not-recorded',
                'ACTIONS_CACHE_URL': 'https://example.blob.core.windows.net/private',
                'ACTIONS_RUNTIME_URL': 'https://private.test/secret'}):
            hosts = guard.endpoints()
            self.assertIn('example.actions.githubusercontent.com', hosts)
            self.assertIn('example.blob.core.windows.net', hosts)
            self.assertNotIn('private.test', hosts)
            self.assertFalse(any('token' in host or 'secret' in host for host in hosts))
        for code, status, expected in ((0, '403', True), (0, '404', True),
                                        (6, '000', False), (60, '000', False), (0, '', False)):
            with self.subTest(code=code, status=status), patch.object(
                    guard.network, 'command', return_value={'returncode': code, 'stdout': status}) as command:
                self.assertEqual(guard.https_probe('github.com')['reachable'], expected)
                self.assertIn('--noproxy', command.call_args.args[0])
                self.assertNotIn('--insecure', command.call_args.args[0])

    def test_test_service_cleanup_requires_confirmed_removal_before_deleting_files(self):
        for returncode in (113, 0, None, 1):
            with self.subTest(returncode=returncode), self.runner() as (root, _, _, command, _, _):
                label = 'org.clashofrust.network.501'
                paths = (label, root/'helper', root/'agent.plist')
                with patch.object(guard, 'service_paths', return_value=paths), \
                        patch.object(guard, 'SERVICE_STOP_SECONDS', 0), \
                        patch.object(guard.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as run:
                    guard.network.write_report(guard.SERVICE, {'label': label, 'report': str(root)})
                    command.side_effect = [
                        {'returncode': 0, 'stdout': 'running'},
                        {'returncode': 0, 'stdout': 'TUN session child started: pid=123'},
                        {'returncode': 0}, {'returncode': returncode}]
                    if returncode == 113:
                        guard.cleanup_service()
                        run.assert_called_once()
                        self.assertFalse(guard.SERVICE.exists())
                        self.assertIn('pid=123', (root/'proxy-smoke-network-service.log').read_text())
                    else:
                        with self.assertRaisesRegex(RuntimeError, 'confirm isolated'):
                            guard.cleanup_service()
                        run.assert_not_called()
                        self.assertTrue(guard.SERVICE.exists())

    def test_launchd_asynchronous_bootout_waits_for_confirmed_removal(self):
        with self.runner() as (root, _, _, command, _, pause):
            label = 'org.clashofrust.network.501'
            paths = (label, root/'helper', root/'agent.plist')
            with patch.object(guard, 'service_paths', return_value=paths), \
                    patch.object(guard.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as run:
                guard.network.write_report(guard.SERVICE, {'label': label, 'report': str(root)})
                command.side_effect = [
                    {'returncode': 0, 'stdout': 'running'},
                    {'returncode': 0, 'stdout': 'helper log'},
                    {'returncode': 0},
                    {'returncode': 0, 'stdout': 'state = exiting'},
                    {'returncode': 113, 'stderr': 'Could not find service'}]
                guard.cleanup_service()
                pause.assert_called_once()
                run.assert_called_once()
                report = json.loads((root/'proxy-smoke-network-service-cleanup.json').read_text())
                self.assertEqual([item['returncode'] for item in report['attempts']], [0, 113])
                self.assertFalse(guard.SERVICE.exists())

    def test_failed_teardown_history_survives_successful_later_recovery(self):
        with self.runner() as (root, _, _, command, _, _):
            label = 'org.clashofrust.network.501'
            paths = (label, root/'helper', root/'agent.plist')
            with patch.object(guard, 'service_paths', return_value=paths), \
                    patch.object(guard, 'SERVICE_STOP_SECONDS', 0), \
                    patch.object(guard.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as run:
                guard.network.write_report(guard.SERVICE, {'label': label, 'report': str(root)})
                command.side_effect = [
                    {'returncode': 0}, {'returncode': 0}, {'returncode': 0},
                    {'returncode': 0, 'stdout': 'state = exiting'}]
                with self.assertRaisesRegex(RuntimeError, 'confirm isolated'):
                    guard.cleanup_service()
                run.assert_not_called()
                command.side_effect = [
                    {'returncode': 0}, {'returncode': 0}, {'returncode': 3},
                    {'returncode': 113, 'stderr': 'Could not find service'}]
                guard.cleanup_service()
                report = json.loads((root/'proxy-smoke-network-service-cleanup.json').read_text())
                self.assertEqual(report['after']['returncode'], 113)
                self.assertEqual(report['history'][0]['after']['returncode'], 0)
                self.assertFalse(guard.SERVICE.exists())

    def test_existing_service_is_not_adopted_or_deleted(self):
        with self.runner() as (root, _, _, command, _, _):
            helper = root/'helper'
            helper.mkdir()
            with patch.object(guard, 'service_paths', return_value=('label', helper, root/'agent')):
                with self.assertRaisesRegex(RuntimeError, 'overwrite'):
                    with guard.network_service(root):
                        self.fail('Existing service should be refused')
            command.assert_not_called()
            self.assertFalse(guard.SERVICE.exists())

    def test_failed_service_teardown_preserves_test_exception_and_ownership(self):
        with self.runner() as (root, _, _, _, _, _), \
                patch.object(guard, 'service_paths', return_value=('label', root/'helper', root/'agent')), \
                patch.object(guard, 'cleanup_service', side_effect=RuntimeError('bootout failed')) as cleanup:
            with self.assertRaisesRegex(ValueError, 'TUN timeout'):
                with guard.network_service(root):
                    raise ValueError('TUN timeout')
            cleanup.assert_called_once()
            self.assertTrue(guard.SERVICE.exists())

    def test_non_runner_never_writes_settings_or_stops_services(self):
        for platform, actions in (('linux', 'true'), ('darwin', 'false')):
            with self.subTest(platform=platform), patch.object(guard.sys, 'platform', platform), \
                    patch.dict(os.environ, {'GITHUB_ACTIONS': actions}), \
                    patch.object(guard, 'configuration') as configuration, \
                    patch.object(guard, 'cleanup_service') as cleanup:
                with self.assertRaisesRegex(RuntimeError, 'disposable'):
                    guard.save()
                with self.assertRaisesRegex(RuntimeError, 'disposable'):
                    guard.recover()
                configuration.assert_not_called()
                cleanup.assert_not_called()


if __name__ == '__main__':
    unittest.main()
