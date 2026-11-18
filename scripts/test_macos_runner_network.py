"""Offline regressions for native runner DNS diagnostics."""
import subprocess
import unittest
from unittest.mock import patch

import macos_runner_network as network


class MacosRunnerNetworkTests(unittest.TestCase):
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
                             'resolver #2\n nameserver[0] : 192.0.2.54\n'}
        with patch.object(network, 'command', return_value={'returncode': 0}) as command:
            result = network.direct_probes(resolver, ['github.com'])
            self.assertEqual(len(result), 1)
            self.assertIn('@192.0.2.53', command.call_args.args[0])
            self.assertNotIn('@192.0.2.54', command.call_args.args[0])
            self.assertEqual(command.call_args.kwargs['timeout'], 3)


if __name__ == '__main__':
    unittest.main()
