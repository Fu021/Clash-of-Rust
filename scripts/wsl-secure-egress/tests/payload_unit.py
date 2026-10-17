"""Offline payload upgrade tests; never invoke WSL, networking or the manager.

python -B scripts/wsl-secure-egress/tests/payload_unit.py

Linux root additionally exercises real ownership checks, flock and atomic
renameat2 on disposable directories under /root. Windows exercises real fixture
files with Linux-only ownership/locking/exchange adapters simulated.
"""
from contextlib import ExitStack, nullcontext, redirect_stdout
import ctypes
import errno
import importlib.util
import io
import json
import os
from pathlib import Path
import socket
import stat
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.dont_write_bytecode = True
SOURCE = Path(__file__).resolve().parents[1] / 'linux/install-payload.py'
spec = importlib.util.spec_from_file_location('cor_payload', SOURCE)
payload = importlib.util.module_from_spec(spec)
spec.loader.exec_module(payload)
NATIVE = sys.platform == 'linux' and os.geteuid() == 0


class PayloadTests(unittest.TestCase):
    native = False

    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(
            tempfile.TemporaryDirectory(prefix='cor-payload-test-', dir='/root' if self.native else None)))
        self.dest = self.root / 'lib/cor-egress'
        self.state_dir = self.root / 'state'
        self.bin = self.root / 'bin'
        self.netns = self.root / 'netns'
        for name, value in [('DEST', self.dest), ('STATE', self.state_dir),
                            ('BIN', self.bin), ('NETNS', self.netns)]:
            self.stack.enter_context(patch.object(payload, name, value))
        self.state_dir.mkdir(mode=0o700)
        self.state = dict(schema=1, owner='offline-fixture', phase='Guarded', namespaces={})
        self.write_state()
        for module, name in ((subprocess, 'run'), (subprocess, 'Popen'), (socket, 'socket')):
            self.stack.enter_context(patch.object(module, name, side_effect=AssertionError('External operation refused')))
        if not self.native:
            # Only Linux platform boundaries are simulated; fixture contents,
            # receipts, hashes, state checks and CAS comparisons are real.
            self.stack.enter_context(patch.object(payload, 'safe', side_effect=lambda p, **kw: Path(p)))
            self.stack.enter_context(patch.object(payload, 'private_file', side_effect=Path))
            self.stack.enter_context(patch.object(payload, 'write', side_effect=self.write_file))
            self.stack.enter_context(patch.object(payload, 'manager_lock', side_effect=nullcontext))
            self.stack.enter_context(patch.object(payload, 'sync_directory'))
            self.stack.enter_context(patch.object(payload, 'aliases', return_value=[]))
            self.stack.enter_context(patch.object(payload, 'exchange_directories', side_effect=self.fake_exchange))
        self.old = {name: ('old ' + name + '\n').encode() for name in payload.FILES}
        self.new = {name: ('new ' + name + '\n').encode() for name in payload.FILES}

    def write_state(self):
        path = self.state_dir / 'state.json'
        path.write_text(json.dumps(self.state), encoding='utf-8')
        path.chmod(0o600)

    @staticmethod
    def manifest(blobs):
        return {name: payload.digest(data) for name, data in blobs.items()}

    @staticmethod
    def write_file(path, data, mode):
        path.write_bytes(data)
        path.chmod(mode)

    @staticmethod
    def fake_exchange(left, right):
        # Test adapter only; production never falls back to separate renames.
        temporary = left.with_name(left.name + '-test-swap')
        os.rename(left, temporary)
        os.rename(right, left)
        os.rename(temporary, right)

    def install_old(self):
        self.assertIsNone(payload.install(self.old, self.manifest(self.old)))

    def assert_code(self, path, blobs):
        self.assertEqual({entry.name for entry in path.iterdir()}, set(payload.FILES) | {payload.RECEIPT})
        for name, expected in blobs.items():
            self.assertEqual((path / name).read_bytes(), expected)
        self.assertEqual(json.loads((path / payload.RECEIPT).read_bytes())['files'], self.manifest(blobs))

    def test_fresh_install_and_identical_reinstall(self):
        self.install_old()
        before = payload.installation(self.dest)[1]
        self.assertIsNone(payload.install(self.old, self.manifest(self.old)))
        self.assertEqual(payload.installation(self.dest)[1], before)
        self.assert_code(self.dest, self.old)

    def test_implicit_version_change_is_refused(self):
        self.install_old()
        with self.assertRaisesRegex(ValueError, 'reviewed upgrade'):
            payload.install(self.new, self.manifest(self.new))
        self.assert_code(self.dest, self.old)
        self.assertEqual(list(self.dest.parent.glob('.cor-egress-previous-*')), [])

    def test_reviewed_upgrade_retains_exact_old_code_and_receipt(self):
        self.install_old()
        previous = Path(payload.install(self.new, self.manifest(self.new), reviewed_upgrade=True))
        self.assert_code(self.dest, self.new)
        self.assert_code(previous, self.old)
        self.assertEqual(previous.parent, self.dest.parent)
        self.assertTrue(previous.name.startswith('.cor-egress-previous-'))

    def test_running_transitional_or_populated_namespaces_are_refused(self):
        self.install_old()
        for phase, namespaces in [('Running', {}), ('Starting', {}), ('Blocked', {}),
                                  ('Guarded', {'cor-egress-gw': {'inode': 123}}),
                                  ('Guarded', None)]:
            with self.subTest(phase=phase, namespaces=namespaces):
                self.state.update(phase=phase, namespaces=namespaces)
                self.write_state()
                with self.assertRaises(ValueError):
                    payload.install(self.new, self.manifest(self.new), True)
                self.assert_code(self.dest, self.old)

    def test_missing_journal_or_namespace_inventory_is_refused(self):
        self.install_old()
        (self.state_dir / 'state.json').unlink()
        with self.assertRaises(OSError):
            payload.install(self.new, self.manifest(self.new), True)
        self.state.pop('namespaces')
        self.write_state()
        with self.assertRaises(ValueError):
            payload.install(self.new, self.manifest(self.new), True)
        self.assert_code(self.dest, self.old)

    def test_unrecorded_namespace_handle_is_refused(self):
        self.install_old()
        self.netns.mkdir()
        (self.netns / 'cor-egress-gw').touch()
        with self.assertRaisesRegex(ValueError, 'namespace still exists'):
            payload.install(self.new, self.manifest(self.new), True)
        self.assert_code(self.dest, self.old)

    def test_installed_drift_and_unknown_receipt_are_refused(self):
        self.install_old()
        (self.dest / 'manager.py').write_bytes(b'changed\n')
        with self.assertRaisesRegex(ValueError, 'drift'):
            payload.install(self.new, self.manifest(self.new), True)
        (self.dest / 'manager.py').write_bytes(self.old['manager.py'])
        (self.dest / payload.RECEIPT).write_text('{"owner":"foreign","files":{}}', encoding='utf-8')
        with self.assertRaisesRegex(ValueError, 'ownership'):
            payload.install(self.new, self.manifest(self.new), True)

    def test_extra_profile_or_key_is_never_archived(self):
        self.install_old()
        for name in ('profile.json', 'wireguard.key'):
            with self.subTest(name=name):
                extra = self.dest / name
                extra.write_bytes(b'offline marker only')
                with self.assertRaisesRegex(ValueError, 'code-only'):
                    payload.install(self.new, self.manifest(self.new), True)
                extra.unlink()
        self.assert_code(self.dest, self.old)
        self.assertEqual(list(self.dest.parent.glob('.cor-egress-previous-*')), [])

    def test_cas_rejects_consistent_receipt_replacement_during_staging(self):
        self.install_old()
        original_write = payload.write
        changed = False

        def change_receipt(path, data, mode):
            nonlocal changed
            original_write(path, data, mode)
            if not changed:
                changed = True
                target = self.dest / payload.RECEIPT
                receipt = json.loads(target.read_bytes())
                receipt['review_marker'] = 'concurrent edit'
                self.write_file(target, json.dumps(receipt).encode(), 0o600)

        with patch.object(payload, 'write', side_effect=change_receipt):
            with self.assertRaisesRegex(ValueError, 'CAS'):
                payload.install(self.new, self.manifest(self.new), True)
        self.assert_code(self.dest, self.old)

    def test_cas_rejects_state_change_before_exchange(self):
        self.install_old()
        original_write = payload.write

        def change_state(path, data, mode):
            original_write(path, data, mode)
            self.state['generation'] = 999
            self.write_state()

        with patch.object(payload, 'write', side_effect=change_state):
            with self.assertRaisesRegex(ValueError, 'CAS'):
                payload.install(self.new, self.manifest(self.new), True)
        self.assert_code(self.dest, self.old)

    def test_failed_staging_or_exchange_preserves_current_directory(self):
        self.install_old()
        with patch.object(payload, 'write', side_effect=OSError('offline staged write failure')):
            with self.assertRaises(OSError):
                payload.install(self.new, self.manifest(self.new), True)
        self.assert_code(self.dest, self.old)
        with patch.object(payload, 'exchange_directories', side_effect=OSError('offline exchange failure')):
            with self.assertRaises(OSError):
                payload.install(self.new, self.manifest(self.new), True)
        self.assert_code(self.dest, self.old)

    def test_busy_manager_lock_never_stages_upgrade(self):
        self.install_old()
        with patch.object(payload, 'manager_lock', side_effect=BlockingIOError):
            with self.assertRaises(BlockingIOError):
                payload.install(self.new, self.manifest(self.new), True)
        self.assert_code(self.dest, self.old)
        self.assertEqual(list(self.dest.parent.glob('.cor-egress-previous-*')), [])

    def test_explicit_cli_checks_source_manifest_then_upgrades(self):
        self.install_old()
        source = self.root / 'source'
        source.mkdir(mode=0o700)
        for name, data in self.new.items():
            self.write_file(source / name, data, 0o755 if name != 'manager.py' else 0o644)
        manifest = source / 'manifest.json'
        self.write_file(manifest, json.dumps(self.manifest(self.new)).encode(), 0o600)
        output = io.StringIO()
        with patch.object(payload.sys, 'platform', 'linux'), \
                patch.object(payload.os, 'geteuid', return_value=0, create=True), \
                patch.object(payload.os, 'umask'), redirect_stdout(output):
            result = payload.main(['--payload', str(source), '--manifest', str(manifest), '--reviewed-upgrade'])
        self.assertEqual(result, 0)
        event = json.loads(output.getvalue())
        self.assertEqual(event['outcome'], 'succeeded')
        self.assert_code(Path(event['previous_payload']), self.old)
        self.assert_code(self.dest, self.new)


@unittest.skipUnless(NATIVE, 'Real Linux root flock/permissions/renameat2 tests require Linux root')
class NativeLinuxTests(PayloadTests):
    native = True

    def test_real_nonblocking_lock_is_held_through_atomic_exchange(self):
        import fcntl
        self.install_old()
        exchange = payload.exchange_directories

        def locked_exchange(left, right):
            with (self.state_dir / 'manager.lock').open('rb') as contender:
                with self.assertRaises(BlockingIOError):
                    fcntl.flock(contender.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            exchange(left, right)

        with patch.object(payload, 'exchange_directories', side_effect=locked_exchange):
            previous = Path(payload.install(self.new, self.manifest(self.new), True))
        self.assert_code(previous, self.old)
        self.assert_code(self.dest, self.new)

    def test_real_active_manager_lock_refuses_publication(self):
        import fcntl
        self.install_old()
        with (self.state_dir / 'manager.lock').open('rb') as holder:
            fcntl.flock(holder.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(BlockingIOError):
                payload.install(self.new, self.manifest(self.new), True)
        self.assert_code(self.dest, self.old)

    def test_real_public_receipt_and_symlink_are_refused(self):
        self.install_old()
        receipt = self.dest / payload.RECEIPT
        receipt.chmod(0o644)
        with self.assertRaises(ValueError):
            payload.install(self.new, self.manifest(self.new), True)
        receipt.chmod(0o600)
        code = self.dest / 'manager.py'
        code.unlink()
        code.symlink_to(self.dest / 'manager')
        with self.assertRaises(ValueError):
            payload.install(self.new, self.manifest(self.new), True)


class ExchangePrimitiveTests(unittest.TestCase):
    def test_unsupported_exchange_fails_without_two_rename_fallback(self):
        with patch.object(ctypes, 'CDLL', return_value=SimpleNamespace()), \
                patch.object(os, 'rename', side_effect=AssertionError('Non-atomic fallback refused')):
            with self.assertRaises(ValueError):
                payload.exchange_directories(Path('/offline/new'), Path('/offline/current'))

    def test_exchange_uses_atomic_exchange_flag_and_propagates_failure(self):
        rename = Mock(return_value=-1)
        with patch.object(ctypes, 'CDLL', return_value=SimpleNamespace(renameat2=rename)), \
                patch.object(ctypes, 'get_errno', return_value=errno.ENOSYS):
            with self.assertRaises(OSError):
                payload.exchange_directories(Path('/offline/new'), Path('/offline/current'))
        self.assertEqual(rename.call_args.args[0::2], (-100, -100, 2))

    def test_private_receipt_rejects_group_or_world_read_permissions(self):
        path = Mock()
        path.stat.return_value = SimpleNamespace(st_mode=stat.S_IFREG | 0o644)
        with patch.object(payload, 'safe', return_value=path):
            with self.assertRaisesRegex(ValueError, 'root-private'):
                payload.private_file(Path('/offline/receipt'))


if __name__ == '__main__':
    unittest.main(verbosity=2)
