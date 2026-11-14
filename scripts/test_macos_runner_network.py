"""Offline regressions for native runner DNS readiness and bounded recovery."""
from contextlib import contextmanager
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import macos_ci
import macos_runner_network as network


class MacosRunnerNetworkTests(unittest.TestCase):
    def probes(self, success=True):
        return [{'host': host, 'addresses': ['192.0.2.10'] if success else [],
                 'returncode': 0 if success else 1} for host in network.HOSTS]

    @contextmanager
    def runner(self, original=None):
        original = original or {'dnsservers': ['192.0.2.53', '2001:db8::53'],
                                'searchdomains': []}
        state = copy.deepcopy(original)
        commands = []

        def settings(command, **kwargs):
            values = state[command[1][4:]]
            return '\n'.join(values) if values else "There aren't any Search Domains set on Ethernet."

        def update(command, **kwargs):
            commands.append(command)
            values = command[5:]
            state[command[3][4:]] = [] if values == ['Empty'] else values
            return subprocess.CompletedProcess(command, 0)

        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(network.sys, 'platform', 'darwin'), \
                patch.dict(os.environ, {'GITHUB_ACTIONS': 'true'}), \
                patch.object(network, 'SNAPSHOT', Path(temporary)/'network.json'), \
                patch.object(network, 'REPORT', Path(temporary)/'report.json'), \
                patch.object(network.subprocess, 'check_output', side_effect=settings), \
                patch.object(network.subprocess, 'run', side_effect=update), \
                patch.object(network, 'command', return_value={'returncode': 0, 'stdout': ''}), \
                patch.object(network, 'native_probes', return_value=self.probes()) as probes:
            yield state, commands, probes

    def test_restores_explicit_or_automatic_dns_and_search_settings(self):
        for original in (
                {'dnsservers': ['192.0.2.53', '2001:db8::53'], 'searchdomains': []},
                {'dnsservers': [], 'searchdomains': ['example.test']}):
            with self.subTest(original=original), self.runner(original) as (state, commands, _):
                network.preserve()
                state.update(dnsservers=['100.100.100.100'], searchdomains=['tailnet.test'])
                network.preserve(restore=True)
                self.assertEqual(state, original)
                self.assertEqual(len(commands), 2)
                report = json.loads(network.REPORT.read_text())
                self.assertEqual(report['restoration'], 'ready')
                self.assertEqual(report['before_restore']['settings']['dnsservers'], ['100.100.100.100'])
                self.assertEqual(report['restored_settings'], original)
                self.assertEqual(network.REPORT.stat().st_mode & 0o777, 0o600)

    def test_native_resolution_is_required_after_a_successful_settings_write(self):
        with self.runner() as (_, _, probes), \
                patch.object(network.time, 'sleep') as pause:
            network.preserve()
            probes.side_effect = [self.probes(False), self.probes()]
            network.preserve(restore=True)
            report = json.loads(network.REPORT.read_text())
            self.assertEqual([network.ready(value['probes']) for value in report['attempts']],
                             [False, True])
            pause.assert_called_once()
            self.assertEqual(report['restoration'], 'ready')

    def test_persistent_dns_failure_keeps_evidence_and_fails_before_toolchain_setup(self):
        with self.runner() as (_, _, probes), \
                patch.object(network, 'RESTORE_SECONDS', 0), \
                patch.object(network, 'direct_probes', return_value=[{'returncode': 0}]) as direct:
            network.preserve()
            probes.return_value = self.probes(False)
            with self.assertRaisesRegex(RuntimeError, 'after private cache login'):
                network.preserve(restore=True)
            report = json.loads(network.REPORT.read_text())
            self.assertEqual(report['restoration'], 'failed')
            self.assertEqual(report['failed_hosts'], list(network.HOSTS))
            self.assertEqual(report['direct_probes'], [{'returncode': 0}])
            self.assertEqual(direct.call_count, 1)

    def test_baseline_failure_is_not_attributed_to_private_cache_login(self):
        with self.runner() as (_, _, probes), \
                patch.object(network, 'RESTORE_SECONDS', 0), \
                patch.object(network, 'direct_probes', return_value=[]):
            probes.return_value = self.probes(False)
            network.preserve()
            with self.assertRaisesRegex(RuntimeError, 'before private cache login'):
                network.preserve(restore=True)

    def test_cached_answers_cannot_hide_an_unrestored_active_resolver(self):
        with self.runner(), patch.object(network, 'RESTORE_SECONDS', 0), \
                patch.object(network, 'direct_probes', return_value=[]):
            with patch.object(network, 'command', return_value={
                    'returncode': 0, 'stdout': 'resolver #1\n nameserver[0] : 192.0.2.53\n'}):
                network.preserve()
            with patch.object(network, 'command', return_value={
                    'returncode': 0, 'stdout': 'resolver #1\n nameserver[0] : 100.100.100.100\n'}):
                with self.assertRaisesRegex(RuntimeError, 'active resolver mismatch'):
                    network.preserve(restore=True)
            report = json.loads(network.REPORT.read_text())
            self.assertTrue(network.ready(report['attempts'][0]['probes']))
            self.assertFalse(report['attempts'][0]['resolver_matches_baseline'])
            self.assertEqual(report['restoration'], 'failed')

    def test_settings_mismatch_is_not_reported_as_success(self):
        with self.runner(), patch.object(
                network, 'command', return_value={'returncode': 0, 'stdout': ''}) as command:
            network.preserve()
            with patch.object(network.subprocess, 'run'), \
                    patch.object(network, 'service_settings', return_value={
                        'dnsservers': ['100.100.100.100'], 'searchdomains': []}):
                with self.assertRaisesRegex(RuntimeError, 'did not match'):
                    network.preserve(restore=True)
            self.assertEqual(json.loads(network.REPORT.read_text())['restoration'], 'failed')
            self.assertFalse(any('killall' in str(call) for call in command.call_args_list))

    def test_only_disposable_macos_runners_may_restore_dns(self):
        with patch.object(network.sys, 'platform', 'linux'), \
                patch.object(network, 'observation') as observe:
            network.preserve()
            network.preserve(restore=True)
            observe.assert_not_called()
        with patch.object(network.sys, 'platform', 'darwin'), \
                patch.dict(os.environ, {'GITHUB_ACTIONS': 'false'}), \
                patch.object(network.subprocess, 'run') as run:
            with self.assertRaisesRegex(RuntimeError, 'disposable'):
                network.preserve(restore=True)
            run.assert_not_called()

    def test_native_probe_is_bounded_and_requires_actual_ip_addresses(self):
        with patch.object(network.subprocess, 'run', side_effect=subprocess.TimeoutExpired('lookup', 4)):
            self.assertEqual(network.native_lookup('github.com')['error'], 'TimeoutExpired')
        for stdout, valid in (('["192.0.2.1", "2001:db8::1"]', True),
                              ('[]', False), ('["not-an-address"]', False), ('null', False)):
            with self.subTest(stdout=stdout), patch.object(
                    network, 'command', return_value={'returncode': 0, 'stdout': stdout}):
                result = network.native_lookup('github.com')
                self.assertEqual(bool(result['addresses']), valid)

    def test_direct_diagnostics_use_only_original_default_resolvers(self):
        resolver = {'stdout': 'resolver #1\n nameserver[0] : 192.0.2.53\n'
                             'resolver #2\n nameserver[0] : 100.100.100.100\n'}
        with patch.object(network, 'command', return_value={'returncode': 0}) as command:
            result = network.direct_probes(resolver, ['github.com'])
            self.assertEqual(len(result), 1)
            self.assertIn('@192.0.2.53', command.call_args.args[0])
            self.assertNotIn('@100.100.100.100', command.call_args.args[0])
            self.assertEqual(command.call_args.kwargs['timeout'], 3)

    def test_failed_restore_command_preserves_first_failure(self):
        with self.runner():
            network.preserve()
            with patch.object(network.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, 'restore')):
                with self.assertRaises(subprocess.CalledProcessError):
                    network.preserve(restore=True)
            self.assertEqual(json.loads(network.REPORT.read_text())['restoration'], 'failed')

    def test_failed_pre_restore_inspection_does_not_skip_restoring_dns(self):
        with self.runner() as (state, _, _):
            network.preserve()
            actual = copy.deepcopy(state)
            state.update(dnsservers=['100.100.100.100'], searchdomains=['tailnet.test'])
            with patch.object(network, 'service_settings', side_effect=[
                    subprocess.TimeoutExpired('inspect', 5), actual]):
                network.preserve(restore=True)
            report = json.loads(network.REPORT.read_text())
            self.assertEqual(report['restoration'], 'ready')
            self.assertEqual(report['before_restore']['settings_error'], 'TimeoutExpired')
            self.assertEqual(state, actual)

    def test_runner_network_reports_survive_discarding_interrupted_compiler_cache(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = root/'target'
            report = target/'macos-report'
            target.mkdir()
            source = root/'runner-temp/cache-network-report.json'
            source.parent.mkdir()
            source.write_text('{"schema":1,"restoration":"ready"}')
            shutil.rmtree(target)
            report.mkdir(parents=True)
            with patch.object(macos_ci, 'REPORT', report), \
                    patch.object(network, 'REPORT', source), \
                    patch.object(network, 'SNAPSHOT', root/'runner-temp/cache-network.json'):
                macos_ci.copy_runner_network_report()
            self.assertEqual((report/source.name).read_bytes(), source.read_bytes())


if __name__ == '__main__':
    unittest.main()
