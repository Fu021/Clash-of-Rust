"""Offline tests for VPS preparation and legacy retirement.

Every process, SSH, WSL, systemd, UFW, and filesystem boundary that could
touch a host is mocked or redirected into a temporary directory.
"""
import base64
import contextlib
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import types
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[3]
PREPARE_SOURCE = ROOT / 'scripts/wsl-secure-egress/prepare-vps.py'
RETIRE_SOURCE = ROOT / 'scripts/wsl-secure-egress/linux/retire-legacy.py'


def load_module(name, source):
    spec = importlib.util.spec_from_file_location(name, source)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


prepare = load_module('cor_prepare_vps', PREPARE_SOURCE)
retire = load_module('cor_retire_legacy', RETIRE_SOURCE)


class PrepareMainTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.local_app_data = self.base / 'LocalAppData'
        self.profile_dir = self.local_app_data / 'ClashOfRust' / 'WslEgress'
        self.profile = self.profile_dir / 'client.json'
        self.identity = self.base / 'id_ed25519'
        self.identity.write_text('offline identity placeholder', encoding='ascii')
        self.env = {
            'SystemRoot': str(self.base / 'Windows'),
            'LOCALAPPDATA': str(self.local_app_data),
        }
        self.calls = []

    def args(self, *extra):
        return [
            'prepare-vps.py',
            '--ssh-alias', 'offline-vps',
            '--identity-file', str(self.identity),
            '--distro', 'Ubuntu-22.04',
            '--profile', str(self.profile),
            '--port', '51820',
            *extra,
        ]

    def run_fixture(self, *args, ssh_result=None):
        ssh_result = ssh_result or {
            'schema': 1,
            'endpoint_ipv4': '8.8.8.8',
            'peer_public_key': 'server-public',
            'ssh_active': True,
            'proxy_active': True,
        }

        def fake_run(command, data=None, timeout=90, env=None):
            self.calls.append({
                'command': list(command),
                'data': data,
                'timeout': timeout,
                'env': env,
            })
            if 'powershell.exe' in str(command[0]).lower():
                return b''
            if str(command[0]).lower().endswith('wsl.exe'):
                if command[-1] == 'genkey':
                    return b'client-private-key\n'
                if command[-1] == 'pubkey':
                    return b'client-public-key\n'
                raise AssertionError('Unexpected WSL command in offline fixture')
            if 'ssh.exe' in str(command[0]).lower():
                return json.dumps(ssh_result).encode()
            raise AssertionError('Unexpected process: %r' % (command,))

        with patch.object(prepare, 'run', side_effect=fake_run), \
                patch.object(prepare.sys, 'platform', 'win32'), \
                patch.dict(prepare.os.environ, self.env, clear=False), \
                patch.object(prepare.sys, 'argv', self.args(*args)):
            return prepare.main()

    def test_remote_code_compiles_as_real_python(self):
        compile(prepare.REMOTE, '<prepare-vps-remote>', 'exec')

    def test_profile_acl_is_configured_before_secret_generation(self):
        result = self.run_fixture('--execute')
        self.assertEqual(result['outcome'], 'succeeded')
        self.assertTrue(self.profile.is_file())
        self.assertFalse(self.profile.with_suffix('.pending.json').exists())

        powershell = next(item for item in self.calls
                          if 'powershell.exe' in str(item['command'][0]).lower())
        acl_script = powershell['command'][-1]
        self.assertIn('SetAccessRuleProtection($true,$false)', acl_script)
        self.assertIn('SetOwner($user)', acl_script)
        self.assertIn('Set-Acl -LiteralPath $path -AclObject $acl', acl_script)
        self.assertEqual(
            powershell['env']['COR_EGRESS_PROFILE_DIRECTORY'],
            str(self.profile_dir),
        )
        wsl_index = next(i for i, item in enumerate(self.calls)
                         if str(item['command'][0]).lower().endswith('wsl.exe'))
        acl_index = self.calls.index(powershell)
        self.assertLess(acl_index, wsl_index)

        profile = json.loads(self.profile.read_text(encoding='utf-8'))
        self.assertEqual(profile['private_key'], 'client-private-key')
        self.assertEqual(profile['peer_public_key'], 'server-public')

    def test_failed_remote_health_check_retains_pending_secret_and_no_final_profile(self):
        with self.assertRaises(ValueError):
            self.run_fixture('--execute', ssh_result={
                'schema': 1,
                'endpoint_ipv4': '8.8.8.8',
                'peer_public_key': 'server-public',
                'ssh_active': True,
                'proxy_active': False,
            })
        self.assertFalse(self.profile.exists())
        pending = self.profile.with_suffix('.pending.json')
        self.assertTrue(pending.is_file())
        self.assertEqual(
            json.loads(pending.read_text(encoding='utf-8'))['private_key'],
            'client-private-key',
        )

    def test_rollback_server_sends_only_rollback_request(self):
        result = self.run_fixture('--execute', '--rollback-server', ssh_result={
            'outcome': 'succeeded',
            'phase': 'restored',
        })
        self.assertEqual(result, {'outcome': 'succeeded', 'phase': 'restored'})
        self.assertEqual(len(self.calls), 1)
        call = self.calls[0]
        self.assertIn('-6', call['command'])
        self.assertEqual(json.loads(call['data']), {'action': 'rollback'})

    def test_local_profile_guards_reject_repository_path_before_processes(self):
        repository_profile = ROOT / 'scripts' / 'private.json'
        with patch.object(prepare.sys, 'platform', 'win32'), \
                patch.dict(prepare.os.environ, self.env, clear=False), \
                patch.object(prepare.sys, 'argv', [
                    'prepare-vps.py',
                    '--ssh-alias', 'offline-vps',
                    '--identity-file', str(self.identity),
                    '--distro', 'Ubuntu-22.04',
                    '--profile', str(repository_profile),
                    '--execute',
                ]), \
                patch.object(prepare, 'run', side_effect=AssertionError('boundary escaped')):
            with self.assertRaisesRegex(ValueError, 'outside the repository'):
                prepare.main()


class RemoteHarness:
    """Execute REMOTE against mapped paths and a fake subprocess module."""

    def __init__(self, root):
        self.root = root
        self.calls = []
        self.ufw_added = ''
        self.fail_if_called = False
        self.virtual = {
            '/var/lib': root / 'var/lib',
            '/usr/local/lib': root / 'usr/local/lib',
            '/etc/systemd/system': root / 'etc/systemd/system',
        }
        self.unsafe_parent = None
        for path in self.virtual.values():
            path.mkdir(parents=True, exist_ok=True)

    def map_path(self, value):
        raw = os.fspath(value)
        for prefix, target in self.virtual.items():
            if raw == prefix:
                return target
            if raw.startswith(prefix + '/'):
                return target / raw[len(prefix) + 1:]
        return Path(raw)

    def mapped_path_class(self):
        harness = self

        class MappedPath:
            def __init__(self, value):
                self.real = harness.map_path(value)

            def __truediv__(self, value):
                return MappedPath(self.real / os.fspath(value))

            def __fspath__(self):
                return os.fspath(self.real)

            def __str__(self):
                return str(self.real)

            def __repr__(self):
                return 'MappedPath(%r)' % str(self.real)

            def __hash__(self):
                return hash(self.real)

            def __eq__(self, other):
                if isinstance(other, MappedPath):
                    return self.real == other.real
                return self.real == harness.map_path(other)

            def stat(self):
                current = self.real.stat()
                mode = 0o777 if harness.unsafe_parent == self.real else 0o755
                return types.SimpleNamespace(
                    st_uid=0,
                    st_mode=stat.S_IFDIR | mode,
                )

            def __getattr__(self, name):
                attribute = getattr(self.real, name)
                if not callable(attribute):
                    return attribute

                def invoke(*args, **kwargs):
                    args = [arg.real if isinstance(arg, MappedPath) else arg
                            for arg in args]
                    result = attribute(*args, **kwargs)
                    if isinstance(result, Path):
                        return MappedPath(result)
                    return result

                return invoke

        return MappedPath

    def fake_subprocess(self):
        harness = self

        class FakeSubprocess(types.ModuleType):
            def __init__(self):
                super().__init__('subprocess')

            def run(self, args, input=None, capture_output=True, env=None, timeout=45):
                harness.calls.append(list(args))
                if harness.fail_if_called:
                    raise AssertionError('remote process boundary escaped mock')
                stdout = b''
                if list(args) == ['ip', '-4', '-j', 'route', 'get', '1.1.1.1']:
                    stdout = b'[{"prefsrc":"8.8.8.8","dev":"eth0"}]'
                elif list(args) == ['ufw', 'status']:
                    stdout = b'Status: active\n'
                elif list(args) == ['ufw', 'show', 'added']:
                    stdout = harness.ufw_added.encode()
                elif args[:1] == ['ufw'] and args[1:2] not in (['show'], ['status']):
                    rule = list(args[1:])
                    harness.ufw_added += 'ufw ' + ' '.join(rule) + '\n'
                elif (args[:3] == [
                        'python3', '-I',
                        str(harness.path('/usr/local/lib/cor-wsl-egress/manage.py')),
                ] and args[3:] == ['install', '--execute']):
                    state = harness.path('/var/lib/cor-wsl-egress/state.json')
                    state.write_text(
                        json.dumps({'server_public_key': 'server-public'}),
                        encoding='utf-8',
                    )
                return types.SimpleNamespace(returncode=0, stdout=stdout)

        return FakeSubprocess()

    @contextlib.contextmanager
    def modules(self):
        fake_pathlib = types.ModuleType('pathlib')
        fake_pathlib.Path = self.mapped_path_class()
        fake_subprocess = self.fake_subprocess()
        fake_os = types.ModuleType('os')
        fake_os.__dict__.update(os.__dict__)
        fake_os.O_NOFOLLOW = 0
        fake_os.umask = lambda _mask: 0o022
        with patch.dict(sys.modules, {
            'pathlib': fake_pathlib,
            'subprocess': fake_subprocess,
            'os': fake_os,
        }):
            yield

    def execute(self, request):
        with contextlib.redirect_stdout(io.StringIO()), \
                self.modules(), \
                patch.object(sys, 'stdin', io.StringIO(json.dumps(request))):
            namespace = {'__name__': '__main__'}
            try:
                exec(compile(prepare.REMOTE, '<prepare-vps-remote>', 'exec'),
                     namespace)
            except SystemExit as error:
                if error.code != 0:
                    raise

    def path(self, virtual):
        return self.map_path(virtual)

    def install_request(self, payload=b'manage'):
        unit = b'[Unit]\n'
        return {
            'action': 'install',
            'port': 51820,
            'peer_public_key': 'client-public',
            'files': {
                'manage.py': {
                    'content': base64.b64encode(payload).decode(),
                    'sha256': hashlib.sha256(payload).hexdigest(),
                },
                'cor-wsl-egress.service': {
                    'content': base64.b64encode(unit).decode(),
                    'sha256': hashlib.sha256(unit).hexdigest(),
                },
            },
        }


class RemoteSecurityGateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.harness = RemoteHarness(Path(self.temp.name))

    def test_unsafe_parent_is_rejected_before_any_command(self):
        self.harness.unsafe_parent = self.harness.path('/var/lib')
        self.harness.fail_if_called = True
        with self.assertRaisesRegex(ValueError, 'Unsafe deployment parent'):
            self.harness.execute(self.harness.install_request())
        self.assertEqual(self.harness.calls, [])

    def test_existing_deployment_is_preserved(self):
        self.harness.path('/var/lib/cor-wsl-egress').mkdir()
        self.harness.fail_if_called = True
        with self.assertRaisesRegex(ValueError, 'Existing deployment'):
            self.harness.execute(self.harness.install_request())
        self.assertEqual(self.harness.calls, [])

    def test_payload_hash_gate_rejects_tampering_before_mutation(self):
        request = self.harness.install_request()
        request['files']['manage.py']['sha256'] = hashlib.sha256(b'changed').hexdigest()
        self.harness.fail_if_called = True
        with self.assertRaisesRegex(ValueError, 'payload hash'):
            self.harness.execute(request)
        self.assertFalse(self.harness.path('/var/lib/cor-wsl-egress').exists())
        self.assertEqual(self.harness.calls, [])

    def test_install_adds_scoped_route_rule_and_records_running_phase(self):
        self.harness.execute(self.harness.install_request())
        self.assertIn(
            ['ufw', 'route', 'allow', 'in', 'on', 'cor-wsl0', 'out', 'on',
             'eth0', 'from', '10.77.0.2', 'to', 'any', 'comment',
             'cor-wsl-egress-forward'],
            self.harness.calls,
        )
        self.assertIn(
            ['systemctl', 'enable', '--now', 'cor-wsl-egress.service'],
            self.harness.calls,
        )
        record = json.loads(
            (self.harness.path('/var/lib/cor-wsl-egress') /
             'deployment.json').read_text(encoding='utf-8'))
        self.assertEqual(record['phase'], 'running')
        self.assertEqual(len(record['ufw_added']), 4)

    def prepare_rollback_journal(self, manage=b'manage', unit=b'unit'):
        root = self.harness.path('/var/lib/cor-wsl-egress')
        library = self.harness.path('/usr/local/lib/cor-wsl-egress')
        unit_path = self.harness.path('/etc/systemd/system/cor-wsl-egress.service')
        root.mkdir(parents=True)
        library.mkdir(parents=True)
        (library / 'manage.py').write_bytes(manage)
        unit_path.write_bytes(unit)
        forward_rule = [
            'route', 'allow', 'in', 'on', 'cor-wsl0', 'out', 'on', 'eth0',
            'from', '10.77.0.2', 'to', 'any', 'comment',
            'cor-wsl-egress-forward',
        ]
        record = {
            'schema': 1,
            'owner': 'ClashOfRust.WslEgress.Deployment',
            'phase': 'running',
            'files': {
                str(library / 'manage.py'): hashlib.sha256(manage).hexdigest(),
                str(unit_path): hashlib.sha256(unit).hexdigest(),
            },
            'ufw_added': [forward_rule],
        }
        (root / 'state.json').write_text(
            json.dumps({'fixture': 'owned state'}), encoding='utf-8')
        (root / 'deployment.json').write_text(
            json.dumps(record), encoding='utf-8')
        self.harness.ufw_added = 'ufw ' + ' '.join(forward_rule) + '\n'
        return record, forward_rule

    def test_rollback_requires_unchanged_owned_files_and_removes_owned_rule(self):
        record, forward_rule = self.prepare_rollback_journal()
        self.harness.execute({'action': 'rollback'})
        self.assertEqual(self.harness.calls[:2], [
            ['systemctl', 'disable', '--now', 'cor-wsl-egress.service'],
            ['python3', '-I',
             str(self.harness.path('/usr/local/lib/cor-wsl-egress/manage.py')),
             'rollback', '--execute'],
        ])
        self.assertIn(
            ['ufw', '--force', 'route', 'delete', *forward_rule[1:]],
            self.harness.calls,
        )
        saved = json.loads(
            (self.harness.path('/var/lib/cor-wsl-egress') /
             'deployment.json').read_text(encoding='utf-8'))
        self.assertEqual(saved['phase'], 'restored')
        self.assertEqual(record['owner'], 'ClashOfRust.WslEgress.Deployment')

    def test_rollback_refuses_changed_deployed_file_before_stop(self):
        self.prepare_rollback_journal()
        manage = self.harness.path(
            '/usr/local/lib/cor-wsl-egress/manage.py')
        manage.write_bytes(b'changed')
        self.harness.fail_if_called = True
        with self.assertRaisesRegex(ValueError, 'Deployed service changed'):
            self.harness.execute({'action': 'rollback'})
        self.assertEqual(self.harness.calls, [])

    def test_rollback_refuses_unowned_ufw_rule(self):
        self.prepare_rollback_journal()
        journal = self.harness.path('/var/lib/cor-wsl-egress/deployment.json')
        record = json.loads(journal.read_text(encoding='utf-8'))
        record['ufw_added'][0][-1] = 'foreign-rule'
        journal.write_text(json.dumps(record), encoding='utf-8')
        self.harness.ufw_added = (
            'ufw route allow in on cor-wsl0 out on eth0 from 10.77.0.2 '
            'to any comment foreign-rule\n'
        )
        with self.assertRaisesRegex(ValueError, 'Unowned UFW rule'):
            self.harness.execute({'action': 'rollback'})
        self.assertEqual(
            self.harness.calls,
            [
                ['systemctl', 'disable', '--now', 'cor-wsl-egress.service'],
                ['python3', '-I',
                 str(self.harness.path(
                     '/usr/local/lib/cor-wsl-egress/manage.py')),
                 'rollback', '--execute'],
                ['ufw', 'show', 'added'],
            ],
        )


class RetireLegacyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.journal = self.base / 'retirement.json'
        self.override = self.base / '99-zz-cor-egress'
        self.runtime = self.base / 'cor-egress-retire.conf'
        self.units = [
            {
                'unit': 'xuan-docker-firewall.service',
                'sha256': 'firewall-hash',
                'enabled': 'enabled',
                'active': 'active',
            },
            {
                'unit': 'xuan-claude-de.service',
                'sha256': 'claude-hash',
                'enabled': 'disabled',
                'active': 'inactive',
            },
        ]
        self.calls = []
        self.open_modes = []

    def invoke(self, *extra, inventory=None):
        inventory = inventory if inventory is not None else copy.deepcopy(self.units)
        inventory_calls = []

        def fake_inventory():
            inventory_calls.append(True)
            result = copy.deepcopy(inventory)
            if '--rollback' not in extra and len(inventory_calls) > 1:
                for entry in result:
                    entry['enabled'] = 'disabled'
                    entry['active'] = 'inactive'
            return result

        def fake_run(args, check=True):
            self.calls.append(list(args))
            return types.SimpleNamespace(returncode=0, stdout='')

        original_open = os.open

        def tracked_open(path, flags, mode=0o777, *args, **kwargs):
            self.open_modes.append(mode)
            return original_open(path, flags, mode, *args, **kwargs)

        with patch.object(retire, 'run', side_effect=fake_run), \
                patch.object(retire, 'inventory', side_effect=fake_inventory), \
                patch.object(retire.os, 'geteuid', return_value=0, create=True), \
                patch.object(retire.os, 'O_NOFOLLOW', 0, create=True), \
                patch.object(retire.os, 'open', side_effect=tracked_open), \
                patch.object(retire, 'OVERRIDE', self.override), \
                patch.object(retire, 'RUNTIME', self.runtime), \
                patch.object(retire.sys, 'argv', [
                    'retire-legacy.py', '--journal', str(self.journal),
                    '--execute', *extra,
                ]):
            return retire.main()

    def test_root_gate_blocks_without_subprocess(self):
        with patch.object(retire.os, 'geteuid', return_value=1000, create=True), \
                patch.object(retire.sys, 'argv', [
                    'retire-legacy.py', '--journal', str(self.journal),
                    '--execute',
                ]), \
                patch.object(retire, 'run', side_effect=AssertionError('escaped')):
            with self.assertRaisesRegex(ValueError, 'Linux root'):
                retire.main()

    def test_write_owned_rejects_symlink_and_changed_existing_file(self):
        target = self.base / 'owned'
        target.write_bytes(b'original')
        with self.assertRaisesRegex(ValueError, 'already occupied'):
            retire.write_owned(target, b'changed')

        link = self.base / 'link'
        link.symlink_to(target)
        with self.assertRaisesRegex(ValueError, 'already occupied'):
            retire.write_owned(link, b'original')

    def test_retire_journals_metadata_and_keeps_original_units(self):
        result = self.invoke()
        self.assertEqual(result['phase'], 'retired')
        state = json.loads(self.journal.read_text(encoding='utf-8'))
        self.assertEqual(state['owner'], 'ClashOfRust.LegacyRetirement')
        self.assertEqual(state['phase'], 'retired')
        self.assertTrue(state['runtime_override_created'])
        self.assertTrue(state['apt_override_created'])
        self.assertEqual(self.runtime.read_bytes(), retire.RUNTIME_CONTENT)
        self.assertEqual(self.override.read_bytes(), retire.CONTENT)
        self.assertEqual(self.open_modes[0], 0o600)
        self.assertIn(
            ['systemctl', 'disable', 'xuan-docker-firewall.service'],
            self.calls,
        )
        self.assertIn(
            ['systemctl', 'stop', 'xuan-docker-firewall.service'],
            self.calls,
        )

    def make_rollback_state(self):
        state = {
            'schema': 1,
            'owner': 'ClashOfRust.LegacyRetirement',
            'phase': 'retired',
            'units': copy.deepcopy(self.units),
            'apt_override_created': True,
            'runtime_override_created': True,
        }
        self.journal.write_text(json.dumps(state), encoding='utf-8')
        self.override.write_bytes(retire.CONTENT)
        self.runtime.parent.mkdir(parents=True, exist_ok=True)
        self.runtime.write_bytes(retire.RUNTIME_CONTENT)
        return state

    def test_rollback_requires_unchanged_unit_metadata(self):
        self.make_rollback_state()
        changed = copy.deepcopy(self.units)
        changed[0]['sha256'] = 'changed'
        with self.assertRaisesRegex(ValueError, 'Legacy unit changed'):
            self.invoke('--rollback', inventory=changed)
        self.assertTrue(self.override.exists())
        self.assertTrue(self.runtime.exists())
        self.assertEqual(self.calls, [])

    def test_rollback_removes_owned_overrides_and_restores_original_active_unit(self):
        self.make_rollback_state()
        result = self.invoke('--rollback')
        self.assertEqual(result['phase'], 'restored')
        self.assertFalse(self.override.exists())
        self.assertFalse(self.runtime.exists())
        self.assertIn(
            ['systemctl', 'enable', 'xuan-docker-firewall.service'],
            self.calls,
        )
        self.assertIn(
            ['systemctl', 'start', 'xuan-docker-firewall.service'],
            self.calls,
        )
        state = json.loads(self.journal.read_text(encoding='utf-8'))
        self.assertEqual(state['phase'], 'restored')

    def test_rollback_refuses_changed_owned_override(self):
        self.make_rollback_state()
        self.override.write_bytes(b'foreign')
        with self.assertRaisesRegex(ValueError, 'APT override changed'):
            self.invoke('--rollback')
        self.assertTrue(self.override.exists())
        self.assertTrue(self.runtime.exists())
        self.assertEqual(self.calls, [])


if __name__ == '__main__':
    unittest.main()
