"""Offline validation; importing the VPS module must never change networking."""
import base64
import contextlib
import copy
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

SOURCE = Path(__file__).resolve().parents[1]/'server/manage.py'
spec = importlib.util.spec_from_file_location('cor_vps', SOURCE)
server = importlib.util.module_from_spec(spec)
spec.loader.exec_module(server)


class ProfileTests(unittest.TestCase):
    def profile(self):
        return dict(schema=1, server_address='10.77.0.1/24', peer_address='10.77.0.2',
                    resolver_upstream='192.0.2.53', listen_port=51820,
                    peer_public_key=base64.b64encode(bytes(range(32))).decode())

    def test_validation_is_offline(self):
        with patch.object(server, 'run', side_effect=AssertionError('Unexpected process')):
            address, peer, resolver = server.validate(self.profile())
        self.assertEqual(str(peer), '10.77.0.2')
        self.assertNotEqual(peer, address.ip)
        self.assertEqual(str(resolver), '192.0.2.53')

    def test_rejects_wrong_peer_address(self):
        for peer in ('10.77.0.1', '10.78.0.2', '::1'):
            profile = self.profile()
            profile['peer_address'] = peer
            with self.subTest(peer=peer), self.assertRaises(ValueError):
                server.validate(profile)

    def test_rejects_bad_key(self):
        for key in ('', 'not-a-key', base64.b64encode(b'short').decode()):
            profile = self.profile()
            profile['peer_public_key'] = key
            with self.subTest(key=key), self.assertRaises(ValueError):
                server.validate(profile)

    def test_rejects_non_unicast_resolver_or_invalid_port(self):
        for field, value in [('resolver_upstream','127.0.0.1'), ('resolver_upstream','224.0.0.1'),
                             ('listen_port',0), ('listen_port',65536), ('listen_port',True)]:
            profile = self.profile()
            profile[field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                server.validate(profile)

    def test_forward_policy_allows_only_the_single_peer_reply_direction(self):
        source = SOURCE.read_text(encoding='utf-8')
        self.assertIn('oifname "{INTERFACE}" ip daddr {peer} counter accept;', source)
        self.assertIn('oifname "{INTERFACE}" counter drop;', source)
        self.assertNotIn('oifname "{INTERFACE}" ct state != established,related', source)


class RecoveryTests(unittest.TestCase):
    """Fake kernel/process state: no networking command or daemon can execute."""

    def setUp(self):
        self.stack = contextlib.ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        self.stack.enter_context(patch.object(server, 'ROOT', self.root))
        self.profile = ProfileTests().profile()
        files = {'profile.json': json.dumps(self.profile).encode(),
                 'wireguard.conf': b'offline fixture', 'dnsmasq.conf': b'offline fixture'}
        for name, data in files.items():
            (self.root/name).write_bytes(data)
        self.state = dict(schema=1, owner='ClashOfRust.WslEgress.VPS', phase='installed',
                          boot_id='boot-a', server_public_key='offline-public-key',
                          files={name: server.sha(data) for name, data in files.items()})
        self.table = False
        self.table_hash = 'owned-table'
        self.link = None
        self.public = '(none)'
        self.forward = '0'
        self.processes = {}
        self.next_pid = 1234
        self.dns_exits = False
        self.failure = None
        self.saved = []
        self.calls = []
        self.boot = self.stack.enter_context(patch.object(server, 'boot_id', return_value='boot-a'))
        self.stack.enter_context(patch.object(server, 'run', side_effect=self.command))
        self.stack.enter_context(patch.object(server, 'table_digest', side_effect=lambda: self.table_hash))
        self.stack.enter_context(patch.object(server, 'save', side_effect=self.persist))
        self.stack.enter_context(patch.object(server, 'dns_identity', side_effect=self.process_identity))
        self.spawn = self.stack.enter_context(patch.object(server.subprocess, 'Popen', side_effect=self.spawn_dns))
        self.kill = self.stack.enter_context(patch.object(server.os, 'kill', side_effect=self.kill_dns))
        self.stack.enter_context(patch.object(server.time, 'sleep'))
        original_read = Path.read_text

        def read_text(path, *args, **kwargs):
            if str(path).replace('\\', '/') == '/proc/sys/net/ipv4/ip_forward':
                return self.forward
            return original_read(path, *args, **kwargs)

        self.stack.enter_context(patch.object(Path, 'read_text', read_text))
        # The helper's run and daemon entry points above are simulated; also
        # reject any accidentally introduced direct subprocess invocation.
        self.stack.enter_context(patch.object(server.subprocess, 'run',
                                            side_effect=AssertionError('Real command refused')))

    def persist(self, state):
        self.saved.append(copy.deepcopy(state))

    def command(self, args, **kwargs):
        self.calls.append(args[:])
        if self.failure and args[:len(self.failure)] == self.failure:
            raise RuntimeError('Injected offline operation failure')
        output = b''
        if args == ['nft', '-j', 'list', 'tables']:
            tables = [{'table': dict(family='inet', name=server.TABLE)}] if self.table else []
            output = json.dumps({'nftables': tables}).encode()
        elif args == ['nft', '-f', '-']:
            self.table = True
        elif args == ['nft', 'delete', 'table', 'inet', server.TABLE]:
            self.table = False
        elif args == ['ip', '-j', 'link', 'show']:
            output = json.dumps([self.link] if self.link else []).encode()
        elif args == ['ip', 'link', 'show', server.INTERFACE]:
            return SimpleNamespace(returncode=0 if self.link else 1, stdout=b'')
        elif args[:5] == ['ip', '-4', '-j', 'route', 'get']:
            output = b'[{"dev":"eth0"}]'
        elif args[:3] == ['ip', 'link', 'add']:
            self.assertIsNone(self.link)
            self.link = dict(ifname=server.INTERFACE, ifindex=42, flags=[])
        elif args[:2] == ['wg', 'setconf']:
            self.public = self.state['server_public_key']
        elif args == ['wg', 'show', server.INTERFACE, 'public-key']:
            output = self.public.encode()
        elif args[:3] == ['ip', 'address', 'add']:
            pass
        elif args[:3] == ['ip', 'link', 'set']:
            self.link['flags'] = ['UP'] if args[-1] == 'up' else []
        elif args[:3] == ['ip', 'link', 'delete']:
            self.link = None
            self.public = '(none)'
        elif args[:3] == ['sysctl', '-q', '-w']:
            self.forward = args[3].split('=')[1]
        elif args == ['ss', '-H', '-lun']:
            pass
        elif args == ['wg', 'genkey']:
            output = base64.b64encode(bytes(range(32)))
        elif args == ['wg', 'pubkey']:
            output = self.state['server_public_key'].encode()
        else:
            raise AssertionError('Unexpected simulated command')
        return SimpleNamespace(returncode=0, stdout=output)

    def spawn_dns(self, *args, **kwargs):
        pid = self.next_pid
        self.next_pid += 1
        self.processes[pid] = None if self.dns_exits else str(pid * 10)
        return SimpleNamespace(pid=pid, poll=lambda: 1 if self.dns_exits else None)

    def process_identity(self, pid):
        return self.processes.get(pid)

    def kill_dns(self, pid, signal):
        self.assertEqual(signal, 15)
        self.processes.pop(pid, None)

    def assert_stopped(self):
        self.assertEqual(self.state['phase'], 'stopped')
        self.assertEqual(self.saved[-1]['phase'], 'stopped')
        self.assertIsNone(self.link)
        self.assertFalse(self.table)
        self.assertFalse(self.processes)
        self.assertNotIn('dns_pid', self.state)

    def invoke_main(self, action, state=True, raw=b''):
        fcntl = SimpleNamespace(flock=Mock(), LOCK_EX=1, LOCK_NB=2)
        with patch.object(server.sys, 'argv', ['manage.py', action, '--execute']), \
                patch.object(server.sys, 'stdin', SimpleNamespace(buffer=io.BytesIO(raw))), \
                patch.object(server.os, 'geteuid', return_value=0, create=True), \
                patch.object(server, 'private_directory'), \
                patch.object(server, 'load', return_value=self.state if state else None), \
                patch.dict(server.sys.modules, {'fcntl': fcntl}):
            return server.main()

    def test_start_checks_dns_before_recording_running_and_is_idempotent(self):
        server.start(self.state)
        self.assertEqual(self.state['phase'], 'running')
        self.assertTrue(server.dns_running(self.state))
        self.assertEqual(self.saved[-2]['phase'], 'starting')
        self.assertNotIn('running', [item['phase'] for item in self.saved[:-1]])
        count = len(self.calls)
        server.start(self.state)
        self.assertEqual(self.spawn.call_count, 1)
        self.assertFalse(any(args[:3] == ['ip', 'link', 'add'] for args in self.calls[count:]))

    def test_setconf_failure_removes_owned_unconfigured_interface_then_retry_works(self):
        self.failure = ['wg', 'setconf']
        with self.assertRaises(RuntimeError):
            server.start(self.state)
        self.assert_stopped()
        self.assertTrue(any(item.get('interface_index') == 42 for item in self.saved))
        self.failure = None
        server.start(self.state)
        self.assertEqual(self.state['phase'], 'running')

    def test_failures_at_other_start_steps_are_recoverable(self):
        for failure in (['nft', '-f', '-'], ['ip', 'address', 'add'],
                        ['ip', 'link', 'set', server.INTERFACE, 'mtu'],
                        ['sysctl', '-q', '-w']):
            with self.subTest(operation=failure):
                self.failure = failure
                with self.assertRaises(RuntimeError):
                    server.start(self.state)
                self.assert_stopped()
        self.failure = None
        server.start(self.state)
        self.assertEqual(self.state['phase'], 'running')

    def test_dns_spawn_failure_does_not_leave_running_interface(self):
        self.spawn.side_effect = OSError('Injected offline spawn failure')
        with self.assertRaises(OSError):
            server.start(self.state)
        self.assert_stopped()
        self.assertEqual(self.state['forward_before'], '0')
        self.assertEqual(self.state['forward_written'], '1')

    def test_dns_immediate_exit_never_records_running(self):
        self.dns_exits = True
        with self.assertRaisesRegex(ValueError, 'DNS failed'):
            server.start(self.state)
        self.processes.clear()  # The simulated daemon has exited.
        self.assert_stopped()
        self.assertNotIn('running', [item['phase'] for item in self.saved])

    def test_retry_recovers_interrupted_start_with_empty_interface(self):
        self.table = True
        self.link = dict(ifname=server.INTERFACE, ifindex=42, flags=[])
        self.state.update(phase='starting', table_hash=self.table_hash, interface_index=42)
        server.start(self.state)
        self.assertEqual(self.state['phase'], 'running')
        delete = self.calls.index(['ip', 'link', 'delete', server.INTERFACE])
        create = self.calls.index(['ip', 'link', 'add', server.INTERFACE, 'type', 'wireguard'])
        self.assertLess(delete, create)

    def test_reboot_recreates_runtime_and_uses_current_forwarding_baseline(self):
        self.forward = '1'
        server.start(self.state)
        self.assertEqual(self.state['forward_before'], '1')
        old_pid = self.state['dns_pid']
        self.table = False
        self.link = None
        self.public = ''
        self.forward = '0'
        self.processes[old_pid] = 'reused-pid'
        self.boot.return_value = 'boot-b'
        server.start(self.state)
        self.assertEqual(self.state['phase'], 'running')
        self.assertEqual(self.state['boot_id'], 'boot-b')
        self.assertNotEqual(self.state['dns_pid'], old_pid)
        self.kill.assert_not_called()
        self.assertEqual(self.state['forward_before'], '0')

    def test_install_start_stop_entrypoints_are_offline(self):
        def write(path, data):
            path.write_bytes(data)

        with patch.object(server, 'write', side_effect=write):
            result = self.invoke_main('install', state=False, raw=json.dumps(self.profile).encode())
        self.assertEqual(result['phase'], 'installed')
        self.state = self.saved[-1]
        self.assertEqual(self.state['boot_id'], 'boot-a')
        self.assertEqual(self.invoke_main('start')['phase'], 'running')
        self.assertEqual(self.invoke_main('stop')['phase'], 'stopped')
        self.assert_stopped()

    def test_rollback_after_sysctl_failure_accepts_unchanged_original_value(self):
        self.failure = ['sysctl', '-q', '-w']
        with self.assertRaises(RuntimeError):
            server.start(self.state)
        self.assertEqual(self.forward, '0')
        result = self.invoke_main('rollback')
        self.assertEqual(result['phase'], 'restored')
        self.assertEqual(self.forward, '0')

    def test_rollback_after_reboot_does_not_restore_previous_boot_sysctl(self):
        server.start(self.state)
        self.boot.return_value = 'boot-b'
        self.table = False
        self.link = None
        self.processes.clear()
        self.forward = '0'
        self.assertEqual(self.invoke_main('rollback')['phase'], 'restored')
        self.assertEqual(self.forward, '0')
        self.kill.assert_not_called()

    def test_rollback_restores_current_boot_forwarding_snapshot(self):
        server.start(self.state)
        self.assertEqual(self.forward, '1')
        self.assertEqual(self.invoke_main('rollback')['phase'], 'restored')
        self.assertEqual(self.forward, '0')
        self.assert_stopped_runtime()

    def assert_stopped_runtime(self):
        self.assertIsNone(self.link)
        self.assertFalse(self.table)
        self.assertFalse(self.processes)

    def test_stop_after_reboot_does_not_signal_old_pid_or_restore_old_sysctl(self):
        self.state.update(phase='running', dns_pid=1234, dns_start_time='old',
                          forward_before='1', forward_written='1', table_hash='old')
        self.boot.return_value = 'boot-b'
        server.stop(self.state)
        self.assert_stopped()
        self.kill.assert_not_called()
        self.assertNotIn('forward_before', self.state)
        self.assertNotIn('forward_written', self.state)

    def test_reboot_collision_is_preserved(self):
        self.table = True
        self.boot.return_value = 'boot-b'
        with self.assertRaisesRegex(ValueError, 'new boot'):
            server.start(self.state)
        self.assertTrue(self.table)
        self.spawn.assert_not_called()
        self.assertFalse(any(args[:2] == ['nft', 'delete'] for args in self.calls))

    def test_legacy_journal_with_no_runtime_drops_old_pid_and_snapshot(self):
        self.state.pop('boot_id')
        self.state.update(phase='running', dns_pid=1234,
                          forward_before='1', forward_written='1', table_hash='old')
        server.start(self.state)
        self.assertEqual(self.state['phase'], 'running')
        self.assertEqual(self.state['forward_before'], '0')
        self.kill.assert_not_called()

    def test_dead_dns_in_running_state_is_restarted(self):
        server.start(self.state)
        old_pid = self.state['dns_pid']
        self.processes.pop(old_pid)
        server.start(self.state)
        self.assertNotEqual(self.state['dns_pid'], old_pid)
        self.assertEqual(self.spawn.call_count, 2)
        self.assertEqual(self.state['phase'], 'running')

    def test_reused_interface_or_dns_pid_is_preserved(self):
        server.start(self.state)
        original_index = self.link['ifindex']
        self.link['ifindex'] += 1
        with self.assertRaisesRegex(ValueError, 'Interface ownership'):
            server.stop(self.state)
        self.assertTrue(self.table)
        self.assertIsNotNone(self.link)
        self.kill.assert_not_called()
        self.link['ifindex'] = original_index
        self.processes[self.state['dns_pid']] = 'reused-pid'
        with self.assertRaisesRegex(ValueError, 'DNS PID'):
            server.stop(self.state)
        self.assertTrue(self.table)
        self.assertIsNotNone(self.link)
        self.kill.assert_not_called()

    def test_changed_firewall_is_preserved(self):
        server.start(self.state)
        self.table_hash = 'foreign-table'
        with self.assertRaisesRegex(ValueError, 'policy differs'):
            server.start(self.state)
        self.assertTrue(self.table)
        self.assertIsNotNone(self.link)
        self.assertEqual(self.spawn.call_count, 1)
        self.kill.assert_not_called()

    def test_nft_listing_failure_never_means_absent(self):
        self.failure = ['nft', '-j', 'list', 'tables']
        with self.assertRaises(RuntimeError):
            server.start(self.state)
        self.spawn.assert_not_called()
        self.assertFalse(any(args[:2] == ['nft', '-f'] for args in self.calls))


class ProcessIdentityTests(unittest.TestCase):
    def test_identity_requires_exact_dns_command_and_live_start_time(self):
        fields = ['S'] + ['0'] * 18 + ['98765']
        process_stat = '123 (dnsmasq) ' + ' '.join(fields)
        args = [b'/usr/sbin/dnsmasq', b'--keep-in-foreground',
                ('--conf-file='+str(server.ROOT/'dnsmasq.conf')).encode(), b'']
        with patch.object(Path, 'read_text', return_value=process_stat), \
                patch.object(Path, 'read_bytes', return_value=b'\0'.join(args)):
            self.assertEqual(server.dns_identity(123), '98765')
        for command in (b'/bin/other\0'+args[2], b'/usr/sbin/dnsmasq\0'+args[2]+b'.foreign'):
            with self.subTest(command=command), \
                    patch.object(Path, 'read_text', return_value=process_stat), \
                    patch.object(Path, 'read_bytes', return_value=command):
                with self.assertRaisesRegex(ValueError, 'PID was reused'):
                    server.dns_identity(123)
        with patch.object(Path, 'read_text', side_effect=FileNotFoundError):
            self.assertIsNone(server.dns_identity(123))
        with patch.object(Path, 'read_text', return_value=process_stat.replace(' S ', ' Z ')), \
                patch.object(Path, 'read_bytes', side_effect=AssertionError('Zombie must not be signalled')):
            self.assertIsNone(server.dns_identity(123))


if __name__ == '__main__':
    unittest.main()
