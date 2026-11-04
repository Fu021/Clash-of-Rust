"""Offline tests for resource validation and clean package staging."""
import hashlib
import json
import logging
import re
from pathlib import Path
import socket
import tempfile
import threading
import tomllib
import struct
import shutil
import subprocess
import unittest
from unittest.mock import MagicMock, patch
import build_support as build
import android as android_build
import windows_ci as windows
import workflow_storage as storage
import action_cache as local_cache
import cache_shards
import linux_apt
import linux_ci
import build_inputs
import os
import copy
import importlib.util
import io
import sys
import urllib.error
import urllib.parse
import zipfile
import tarfile
from contextlib import nullcontext

_spec = importlib.util.spec_from_file_location('publish_release', build.ROOT/'scripts/publish-release.py')
release = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(release)

_memory_spec = importlib.util.spec_from_file_location('memory_benchmark', build.ROOT/'scripts/run-memory-benchmark.py')
memory_benchmark = importlib.util.module_from_spec(_memory_spec)
_memory_spec.loader.exec_module(memory_benchmark)

_release_package_spec = importlib.util.spec_from_file_location(
    'release_package', build.ROOT/'scripts/test-release-package.py')
release_package = importlib.util.module_from_spec(_release_package_spec)
_release_package_spec.loader.exec_module(release_package)



class BuildTests(unittest.TestCase):
    def test_isolated_tls_fixture_passes_strict_certificate_validation(self):
        if os.name != 'posix' or not shutil.which('openssl'):
            self.skipTest('POSIX and OpenSSL are required for the isolated certificate fixture')
        specification = importlib.util.spec_from_file_location('linux_tun_fixture', build.ROOT/'scripts/test-linux-tun.py')
        fixture = importlib.util.module_from_spec(specification)
        specification.loader.exec_module(fixture)
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            root = fixture.create_uplink_certificates(directory, ('127.0.0.1', '::1'))
            for hostname in ('127.0.0.1', '::1', 'resolver.fixture.test'):
                identity = '-verify_hostname' if hostname.endswith('.test') else '-verify_ip'
                result = subprocess.run(['openssl', 'verify', '-x509_strict', '-CAfile', str(root),
                                         '-purpose', 'sslserver', identity, hostname, str(directory/'server.crt')],
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_composite_actions_only_use_supported_step_properties(self):
        supported = {'name', 'id', 'if', 'run', 'env', 'shell', 'working-directory',
                     'uses', 'with', 'continue-on-error'}
        for path in (build.ROOT/'.github/actions').glob('*/action.yml'):
            content = path.read_text()
            if not re.search(r'^  using: composite$', content, re.MULTILINE):
                continue
            steps = content.split('\n  steps:\n', 1)[1]
            for entry in re.finditer(r'^(?:    - |      )([a-z][a-z0-9-]*):', steps, re.MULTILINE):
                with self.subTest(action=path.parent.name, property=entry[1]):
                    self.assertIn(entry[1], supported)

    def test_windows_crash_readiness_allows_native_suite_startup_budget(self):
        ready = MagicMock()
        ready.is_file.side_effect = [False, False, True]
        kernel = MagicMock()
        kernel.WaitForSingleObject.return_value = 258
        with patch.object(windows.time, 'monotonic', side_effect=[0, 46, 120]), \
             patch.object(windows.time, 'sleep') as pause:
            windows.wait_unelevated_ready(kernel, 12, ready)
        self.assertEqual(pause.call_count, 2)
        self.assertEqual(kernel.WaitForSingleObject.call_count, 2)

    def test_windows_crash_readiness_fails_when_owner_exits_or_cannot_be_inspected(self):
        ready = MagicMock()
        ready.is_file.return_value = False
        for state, message in ((0, 'exited before'), (0xFFFFFFFF, 'wait status')):
            kernel = MagicMock()
            kernel.WaitForSingleObject.return_value = state
            with self.subTest(state=state), patch.object(windows.time, 'sleep') as pause:
                with self.assertRaisesRegex(RuntimeError, message):
                    windows.wait_unelevated_ready(kernel, 12, ready)
                pause.assert_not_called()

    def test_windows_crash_readiness_retains_bounded_timeout(self):
        ready = MagicMock()
        ready.is_file.return_value = False
        kernel = MagicMock()
        kernel.WaitForSingleObject.return_value = 258
        with patch.object(windows.time, 'monotonic', side_effect=[0, 240]), \
             patch.object(windows.time, 'sleep') as pause:
            with self.assertRaisesRegex(TimeoutError, 'did not become ready'):
                windows.wait_unelevated_ready(kernel, 12, ready)
            pause.assert_not_called()

    def test_linux_guest_package_preparation_skips_documentation_indexes(self):
        with patch.dict(os.environ, {'COR_LINUX_GUEST': 'true'}), \
             patch.object(release_package, 'run') as execute:
            release_package.prepare_guest_package_environment()
        calls = [call.args[0] for call in execute.call_args_list]
        self.assertEqual(calls, [
            ['sudo', 'debconf-set-selections'],
            ['sudo', 'tee', '/etc/dpkg/dpkg.cfg.d/99-cor-ci-no-docs'],
            ['sudo', 'rm', '-f', '/var/lib/man-db/auto-update'],
        ])
        self.assertIn('man-db man-db/auto-update boolean false\n',
                      execute.call_args_list[0].kwargs['input'])
        self.assertIn('path-exclude=/usr/share/man/*',
                      execute.call_args_list[1].kwargs['input'])

    def test_linux_guest_package_preparation_is_guest_only(self):
        with patch.dict(os.environ, {}, clear=True), \
             patch.object(release_package, 'run') as execute:
            release_package.prepare_guest_package_environment()
        execute.assert_not_called()

    def test_linux_package_install_delegates_timeout_to_root(self):
        with patch.object(linux_apt, 'run') as execute:
            release_package.run_privileged_with_timeout(['apt-get', 'install'], 900)
        execute.assert_called_once_with([
            'sudo', 'env', 'DEBIAN_FRONTEND=noninteractive', 'timeout',
            '--signal=TERM', '--kill-after=30s', '900s', 'apt-get', 'install',
        ])

    def test_linux_download_cache_identity_tracks_arch_and_upstream_content(self):
        value = {'image': 'ubuntu@sha256:' + 'c'*64, 'architectures': {
            arch: {'python': {'sha256': 'a'*64}, 'cloud': {'sha256': 'b'*64}}
            for arch in ('x64', 'arm64')}}
        keys = linux_ci.cache_keys(value, 'x64')
        self.assertIn('a'*64, keys['python'])
        self.assertIn('b'*64, keys['cloud'])
        self.assertNotEqual(keys, linux_ci.cache_keys(value, 'arm64'))
        value['architectures']['x64']['python']['sha256'] = 'not-a-checksum\ninjected=key'
        with self.assertRaises(ValueError):
            linux_ci.cache_keys(value, 'x64')

    def test_linux_download_revalidates_cached_bytes_and_uses_frozen_digest(self):
        data = b'verified upstream archive'
        record = {'url': 'https://example.invalid/archive',
                  'sha256': hashlib.sha256(data).hexdigest()}
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)/'archive'
            path.write_bytes(data)
            with patch.object(linux_ci, 'opener') as http:
                self.assertEqual(linux_ci.download(record, path), path)
                http.assert_not_called()
            path.write_bytes(b'x'*len(data))
            with patch.object(linux_ci, 'opener') as http:
                http.return_value.open.return_value = io.BytesIO(data)
                linux_ci.download(record, path)
            self.assertEqual(path.read_bytes(), data)
            path.unlink()
            with patch.object(linux_ci, 'opener') as http:
                http.return_value.open.return_value = io.BytesIO(b'bad')
                with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
                    linux_ci.download(record, path)
            self.assertFalse(path.exists())
            self.assertFalse(path.with_suffix('.partial').exists())

    def test_linux_gui_supports_gnome_336_and_newer_compositors(self):
        import linux_gui

        old = linux_gui.gnome_command('--wayland --nested', 3)
        self.assertIn('--nested', old)
        self.assertNotIn('--wayland-display=cor-wayland', old)
        self.assertNotIn('--unsafe-mode', old)
        modern = linux_gui.gnome_command('--wayland --wayland-display --headless', 46)
        self.assertIn('--wayland-display=cor-wayland', modern)
        self.assertIn('--virtual-monitor=1280x900', modern)
        self.assertIn('--unsafe-mode', modern)

    @unittest.skipUnless(hasattr(socket, 'AF_UNIX'), 'Unix sockets are unavailable')
    def test_linux_gui_finds_only_wayland_sockets(self):
        import linux_gui

        with tempfile.TemporaryDirectory() as temporary:
            runtime = Path(temporary)
            (runtime/'wayland-0.lock').write_text('')
            with socket.socket(socket.AF_UNIX) as default, socket.socket(socket.AF_UNIX) as named:
                default.bind(str(runtime/'wayland-0'))
                named.bind(str(runtime/'cor-wayland'))
                self.assertEqual(linux_gui.wayland_sockets(runtime), {'wayland-0', 'cor-wayland'})

    def test_linux_gui_waits_for_configured_window_size(self):
        import linux_gui

        small = [{'pid': 12, 'width': 0, 'height': 0}]
        usable = [{'pid': 12, 'width': 950, 'height': 700}]
        probe = MagicMock(side_effect=[[], small, usable])
        with patch.object(linux_gui.time, 'sleep'):
            self.assertEqual(linux_gui.wait_for_usable_window('wayland', probe), usable)
        self.assertEqual(probe.call_count, 3)

    def test_linux_gui_rejects_a_permanently_small_window_with_diagnostics(self):
        import linux_gui

        small = [{'pid': 12, 'width': 640, 'height': 300}]
        with (patch.object(linux_gui.time, 'sleep'),
              patch.object(linux_gui.time, 'monotonic', side_effect=[0, 0, 1])):
            with self.assertRaisesRegex(RuntimeError, r'wayland.*800x450.*640.*300'):
                linux_gui.wait_for_usable_window('wayland', lambda: small, timeout=1)

    def test_linux_gui_matches_native_wayland_windows_on_old_and_new_gnome(self):
        import linux_gui

        legacy = {'pid': -1, 'sequence': 1, 'wayland': True}
        legacy_zero = {'pid': 0, 'sequence': 5, 'wayland': True}
        modern = {'pid': 42, 'sequence': 2, 'wayland': True}
        foreign = {'pid': 99, 'sequence': 3, 'wayland': True}
        xwayland = {'pid': -1, 'sequence': 4, 'wayland': False}
        self.assertEqual(linux_gui.select_wayland_windows([legacy], [42]), [legacy])
        self.assertEqual(linux_gui.select_wayland_windows([legacy_zero], [42]), [legacy_zero])
        self.assertEqual(linux_gui.select_wayland_windows([modern, foreign], [42]), [modern])
        self.assertEqual(linux_gui.select_wayland_windows([foreign], [42]), [])
        self.assertEqual(linux_gui.select_wayland_windows([xwayland], [42]), [])
        self.assertEqual(linux_gui.select_wayland_windows([legacy], []), [])
        self.assertEqual(linux_gui.select_wayland_windows([legacy], [42, 43]), [])
        self.assertEqual(linux_gui.select_wayland_windows([legacy, xwayland], [42]), [])

    def test_linux_gui_retries_only_missing_authorization_agent(self):
        import linux_gui

        request = MagicMock()
        agent = MagicMock()
        agent.poll.return_value = None
        request.poll.return_value = 1
        self.assertEqual(
            linux_gui.authorization_dialog(request, agent, io.StringIO(
                'Error executing command as another user: No authentication agent found.\n')),
            ('retry', None))
        with self.assertRaisesRegex(RuntimeError, 'denied'):
            linux_gui.authorization_dialog(request, agent, io.StringIO('authorization denied'))
        agent.poll.return_value = 1
        with self.assertRaisesRegex(RuntimeError, 'agent exited'):
            linux_gui.authorization_dialog(request, agent, io.StringIO(''))

    def test_linux_gui_finds_authorized_core_without_reading_exe_link(self):
        import linux_gui

        with tempfile.TemporaryDirectory() as temporary:
            proc = Path(temporary)
            for pid, name, uid, parent in (
                    (101, 'mihomo', 1001, 42),
                    (102, 'mihomo', 1001, 99),
                    (103, 'mihomo', 1002, 42),
                    (104, 'unrelated', 1001, 42)):
                directory = proc/str(pid)
                directory.mkdir()
                (directory/'status').write_text(
                    f'Name:\t{name}\nPPid:\t{parent}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n')
            (proc/'105').mkdir()
            (proc/'105'/'status').write_text('Name:\tmihomo\n')

            # The two matching processes have no readable exe link or useful
            # directory ownership; both must be counted for duplicate checks.
            self.assertEqual(linux_gui.core_processes(proc, 1001), {101: 42, 102: 99})

    def test_linux_gui_session_is_staged_outside_a_private_workspace(self):
        import linux_gui

        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            directory.chmod(0o755)
            staged = linux_gui.stage_session(directory)
            self.assertEqual(staged.stat().st_mode & 0o777, 0o644)
            self.assertEqual(staged.parent.stat().st_mode & 0o777, 0o755)
            resolved_root = subprocess.check_output(
                [sys.executable, '-B', '-c', 'import build_support; print(build_support.ROOT)'],
                cwd=staged.parent, text=True).strip()
            self.assertEqual(Path(resolved_root), directory)

            report = directory/'report'
            report.mkdir()
            (report/'gui.json').write_text('{"status":"failed"}')
            (report/'desktop.log').write_text('diagnostic')
            (report/'local.yaml').write_text('private fixture')
            (report/'x11.log').symlink_to(report/'local.yaml')
            with patch.object(linux_gui, 'REPORT', directory/'saved'):
                linux_gui.preserve_report(report)
            saved = directory/'saved/desktop'
            self.assertEqual((saved/'gui.json').read_text(), '{"status":"failed"}')
            self.assertEqual((saved/'desktop.log').read_text(), 'diagnostic')
            self.assertFalse((saved/'local.yaml').exists())
            self.assertFalse((saved/'x11.log').exists())

    def test_linux_gui_session_keeps_pam_identity_but_not_runner_credentials(self):
        import linux_gui

        environment = {'XDG_SESSION_ID': 'c12', 'XDG_RUNTIME_DIR': '/run/user/1234',
                       'HOME': '/home/runner', 'XDG_CONFIG_HOME': '/home/runner/.config',
                       'COR_GUI_REPORT': '/tmp/disposable/report',
                       'COR_GUI_AUTH_PASSWORD_FILE': '/tmp/disposable/password',
                       'GITHUB_ACTIONS': 'true', 'GITHUB_TOKEN': 'secret',
                       'AWS_SECRET_ACCESS_KEY': 'secret'}
        with patch.dict(os.environ, environment, clear=True), patch.object(os, 'execve') as launch:
            linux_gui.launch_session()
        executable, command, retained = launch.call_args.args
        self.assertEqual(executable, '/usr/bin/dbus-run-session')
        self.assertIn('--session', command)
        self.assertEqual(retained['XDG_SESSION_ID'], 'c12')
        self.assertEqual(retained['XDG_RUNTIME_DIR'], '/run/user/1234')
        self.assertEqual(retained['HOME'], str(Path('/tmp/disposable/home')))
        self.assertEqual(retained['XDG_CONFIG_HOME'], str(Path('/tmp/disposable/home/.config')))
        self.assertEqual(retained['COR_GUI_AUTH_PASSWORD_FILE'], '/tmp/disposable/password')
        self.assertNotIn('GITHUB_TOKEN', retained)
        self.assertNotIn('AWS_SECRET_ACCESS_KEY', retained)

    def test_memory_mapping_categories_do_not_keep_addresses_or_paths(self):
        text = """1000-2000 rw-p 0 00:00 0
Rss: 8 kB
Pss: 6 kB
Private_Clean: 1 kB
Private_Dirty: 3 kB
2000-3000 rw-p 0 00:00 0 [heap]
Rss: 4 kB
Pss: 4 kB
Private_Dirty: 4 kB
3000-4000 r-xp 0 08:01 1 /private/path/program
Rss: 12 kB
Pss: 9 kB
Private_Clean: 2 kB
"""
        maps = memory_benchmark.memory_maps(text)
        self.assertEqual(maps['anonymous'], {'rss_bytes':8192, 'pss_bytes':6144, 'private_bytes':4096})
        self.assertEqual(maps['heap']['private_bytes'], 4096)
        self.assertEqual(maps['file-backed']['rss_bytes'], 12288)
        self.assertNotIn('/private/path', json.dumps(maps))
        classified = memory_benchmark.memory_maps(text, '/private/path/program')
        self.assertEqual(classified['executable']['rss_bytes'],12288)
        self.assertNotIn('/private/path',json.dumps(classified))

    def test_renderer_bundle_is_an_agent_before_launch_and_signed_inside_out(self):
        import macos_package as mac
        import plistlib
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload = root/'payload'
            (payload/'resources').mkdir(parents=True)
            (payload/'resources/core.json').write_text('{}')
            for name in ('clash-of-rust','clash-desktop-ui','clash-tun-launcher','cor','LICENSE','resources/mihomo'):
                (payload/name).write_bytes(name.encode())
            commands=[]
            def tool(command, **kwargs):
                commands.append([str(v) for v in command])
                if command[0] == '/usr/bin/iconutil':
                    Path(command[-1]).write_bytes(b'icon')
            with patch.object(mac,'validate_binary_arch'),patch.object(mac,'run',side_effect=tool),patch.dict(os.environ,{'MACOS_SIGNING_IDENTITY':'-'}):
                app = mac.build_app(payload,root/mac.APP_NAME,'0.6.3-dev.1','arm64')
            helper=app/mac.UI_APP
            info=plistlib.loads((helper/'Contents/Info.plist').read_bytes())
            owner=plistlib.loads((app/'Contents/Info.plist').read_bytes())
            self.assertIs(info['LSUIElement'],True)
            self.assertNotIn('LSUIElement',owner)
            self.assertNotEqual(info['CFBundleIdentifier'],owner['CFBundleIdentifier'])
            self.assertEqual((helper/'Contents/MacOS'/info['CFBundleExecutable']).read_bytes(),b'clash-desktop-ui')
            self.assertFalse((app/'Contents/MacOS/clash-desktop-ui').exists())
            signed=[command[-1] for command in commands if command[:2]==['/usr/bin/codesign','--force']]
            self.assertLess(signed.index(str(helper/'Contents/MacOS/clash-desktop-ui')),signed.index(str(helper)))
            self.assertLess(signed.index(str(helper)),signed.index(str(app)))

    def test_memory_scene_rejects_an_early_exit_missing_core_or_gui_in_headless_sample(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)/'headless.jsonl'

            def sample(roles, elapsed=9750):
                path.write_text(json.dumps({'elapsed_ms': elapsed,
                    'processes': [{'role': role} for role in roles]})+'\n')

            for roles in [('daemon',), ('gui', 'core'), ('daemon', 'gui', 'core')]:
                sample(roles)
                with self.assertRaisesRegex(ValueError, 'identity'):
                    memory_benchmark.read_scene(path, seconds=10, role='daemon')
            sample(('daemon', 'core'), elapsed=100)
            with self.assertRaisesRegex(ValueError, 'exited'):
                memory_benchmark.read_scene(path, seconds=10, role='daemon')
            path.write_text('{"type":"metadata"}\n')
            with self.assertRaisesRegex(ValueError, 'No process'):
                memory_benchmark.read_scene(path, seconds=10, role='daemon')
            sample(('daemon', 'core'))
            self.assertEqual(len(memory_benchmark.read_scene(path, seconds=10, role='daemon')), 1)

    def setUp(self):
        # Fixtures must not inherit the workflow's real dependency snapshot.
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        override = patch.dict(os.environ, {'BUILD_INPUTS_DIR': directory.name})
        override.start()
        self.addCleanup(override.stop)

    def test_local_rust_cache_invalidates_dependencies_compiler_and_build_flags(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root/'Cargo.lock').write_text('slint = 1.18.1')
            (root/'Cargo.toml').write_text('[workspace]')
            (root/'crates/ui').mkdir(parents=True)
            ui = root/'crates/ui/view.slint'
            ui.write_text('old UI')
            key, prefix = local_cache.rust_keys(root, b'compiler one', {})
            ui.write_text('new UI')
            changed, compatible = local_cache.rust_keys(root, b'compiler one', {})
            self.assertNotEqual(key, changed)
            self.assertEqual(prefix, compatible)
            icon = root/'crates/ui/icon.svg'
            icon.write_text('<svg>old</svg>')
            before, generation = local_cache.rust_keys(root, b'compiler one', {})
            icon.write_text('<svg>new</svg>')
            after, compatible = local_cache.rust_keys(root, b'compiler one', {})
            self.assertNotEqual(before, after)
            self.assertEqual(generation, compatible)
            (root/'.github/workflows').mkdir(parents=True)
            workflow = root/'.github/workflows/ci.yml'
            workflow.write_text('ndk: 28')
            before = local_cache.rust_keys(root, b'compiler one', {})[0]
            workflow.write_text('ndk: 29')
            self.assertNotEqual(before, local_cache.rust_keys(root, b'compiler one', {})[0])
            vendor = root/'vendor/backend'
            (vendor/'java').mkdir(parents=True)
            (vendor/'Cargo.toml').write_text('[package]\nname="backend"\nversion="1.0.0"\n')
            java = vendor/'java/Helper.java'
            java.write_text('old helper')
            before, generation = local_cache.rust_keys(root, b'compiler one', {})
            java.write_text('new helper')
            after, compatible = local_cache.rust_keys(root, b'compiler one', {})
            self.assertNotEqual(before, after)
            self.assertEqual(generation, compatible)
            (vendor/'Cargo.toml').write_text('[package]\nname="backend"\nversion="1.0.0"\n[features]\nnew=[]\n')
            self.assertNotEqual(generation, local_cache.rust_keys(root, b'compiler one', {})[1])
            (root/'Cargo.lock').write_text('slint = 1.19.0')
            new_prefix = local_cache.rust_keys(root, b'compiler one', {})[1]
            self.assertNotEqual(prefix, new_prefix)
            # The compiler-compatible fallback may retain unrelated libraries;
            # Cargo still resolves/builds against the new lockfile.
            self.assertTrue(prefix.startswith(local_cache.rust_environment_key(b'compiler one', {})))
            self.assertTrue(new_prefix.startswith(local_cache.rust_environment_key(b'compiler one', {})))
            self.assertNotEqual(new_prefix, local_cache.rust_keys(root, b'compiler two', {})[1])
            self.assertNotEqual(new_prefix, local_cache.rust_keys(root, b'compiler one', {'RUSTFLAGS':'-C target-cpu=native'})[1])

    def test_local_cache_archives_preserve_data_and_reject_escaping_links(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root/'source'; source.mkdir()
            (source/'module').write_bytes(b'valid cached dependency')
            archive = root/'cache.tar.gz'
            local_cache.pack([source], archive)
            restored = root/'restored'
            local_cache.unpack(archive, [restored])
            self.assertEqual((restored/'module').read_bytes(), b'valid cached dependency')
            escape = 'escape-' + root.name
            for name, kind in [('0/../../'+escape, tarfile.REGTYPE), ('0/link', tarfile.SYMTYPE)]:
                with tarfile.open(archive, 'w:gz') as output:
                    info=tarfile.TarInfo(name); info.type=kind; info.linkname='../../'+escape
                    output.addfile(info)
                with self.assertRaises(ValueError): local_cache.unpack(archive, [restored])
            self.assertFalse((root.parent/escape).exists())

    def test_rust_cache_version_bump_preserves_dependencies_but_feature_changes_do_not(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            def version(value, feature='old'):
                (root/'Cargo.toml').write_text('[workspace.package]\nversion="'+value+'"\n'
                    '[workspace.dependencies]\nslint={version="1",features=["'+feature+'"]}\n')
                (root/'Cargo.lock').write_text('[[package]]\nname="clash"\nversion="'+value+'"\n'
                    '[[package]]\nname="slint"\nversion="1.0.0"\nsource="registry+test"\n')
            version('0.6.0')
            first=local_cache.rust_keys(root,b'compiler',{})[1]
            version('0.6.1')
            self.assertEqual(first,local_cache.rust_keys(root,b'compiler',{})[1])
            version('0.6.1','new')
            self.assertNotEqual(first,local_cache.rust_keys(root,b'compiler',{})[1])

    def test_workspace_cache_verifies_contents_before_restoring_times_and_invalidates_changed_crates(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            for name in ('ui','core'):
                folder=root/'crates'/name;folder.mkdir(parents=True)
                (folder/'Cargo.toml').write_text('[package]\nname="clash-'+name+'"\nversion="1.0.0"\n')
                (folder/'lib.rs').write_bytes(b'original source')
            java=root/'apps/android/app/MainActivity.java';java.parent.mkdir(parents=True)
            java.write_bytes(b'original Java host')
            vendor=root/'vendor/slint-android';vendor.mkdir(parents=True)
            (vendor/'Cargo.toml').write_text('[package]\nname="slint-android"\nversion="1.0.0"\n')
            backend=vendor/'lib.rs';backend.write_bytes(b'original backend')
            paths=sorted(p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file())
            tracked='\0'.join(paths).encode()
            ui=root/'crates/ui/lib.rs';core=root/'crates/core/lib.rs'
            previous=core.stat().st_mtime_ns
            with patch.object(local_cache.subprocess,'check_output',return_value=tracked), \
                    patch.dict(os.environ,{'WORKSPACE_SOURCE_CACHE':str(root/'external/source.json')}):
                local_cache.workspace_sources(root,save=True)
                for name in ('ui','core'):
                    target=root/'target/release/.fingerprint'/('clash-'+name+'-hash')
                    target.mkdir(parents=True);(target/'lib-info').write_bytes(b'fingerprint')
                backend_fingerprint=root/'target/release/.fingerprint/slint-android-hash'
                backend_fingerprint.mkdir(parents=True)
                v2=root/'target/release/.fingerprint/clash-ui/another-hash'
                v2.mkdir(parents=True);(v2/'lib-info').write_bytes(b'fingerprint')
                # Checkout made unchanged sources newer. Conversely, changed
                # contents can belong to an older branch timestamp.
                os.utime(core,ns=(previous+1_000_000,previous+1_000_000))
                ui.write_bytes(b'changed source');os.utime(ui,ns=(1,1))
                java.write_bytes(b'changed Android host; unrelated to desktop Rust libraries')
                backend.write_bytes(b'changed backend');os.utime(backend,ns=(1,1))
                local_cache.workspace_sources(root)
            self.assertEqual(core.stat().st_mtime_ns,previous)
            self.assertEqual(ui.read_bytes(),b'changed source')
            self.assertFalse((root/'target/release/.fingerprint/clash-ui-hash').exists())
            self.assertFalse(v2.exists())
            self.assertFalse(backend_fingerprint.exists())
            self.assertTrue((root/'target/release/.fingerprint/clash-core-hash/lib-info').exists())

    def test_workspace_cache_invalidates_shared_slint_consumers_outside_the_ui_crate(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);ui=root/'crates/ui';mobile=root/'apps/android/native'
            ui.mkdir(parents=True);mobile.mkdir(parents=True)
            (ui/'Cargo.toml').write_text('[package]\nname="clash-ui"\nversion="1.0.0"\n')
            (mobile/'Cargo.toml').write_text('[package]\nname="clash-android"\nversion="1.0.0"\n')
            view=ui/'mobile.slint';view.write_bytes(b'old component')
            paths='\0'.join(p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file()).encode()
            with patch.object(local_cache.subprocess,'check_output',return_value=paths), \
                    patch.dict(os.environ,{'WORKSPACE_SOURCE_CACHE':str(root/'external/source.json')}):
                local_cache.workspace_sources(root,save=True)
                profile=root/'target/aarch64-linux-android/release'
                output=profile/'build/clash-android-hash/output';output.parent.mkdir(parents=True)
                output.write_text('cargo:rerun-if-changed='+str(view)+'\n')
                fingerprint=profile/'.fingerprint/clash-android-hash';fingerprint.mkdir(parents=True)
                (fingerprint/'lib-info').write_bytes(b'cached fingerprint')
                view.write_bytes(b'new component');os.utime(view,ns=(1,1))
                local_cache.workspace_sources(root)
                self.assertFalse(fingerprint.exists())

    def test_rust_shards_only_upload_changed_buckets_and_verify_before_extracting(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            source=root/'target/release/deps';source.mkdir(parents=True)
            for number in range(80): (source/f'dep{number}.rlib').write_bytes(f'dependency-{number}'.encode())
            config={'group':'rust/linux-x64','key':'generation-source-one','prefix':['generation-','compiler-'],
                    'resolved_paths':[str(root/'target')],'kind':'rust'}
            store=local_cache.Store(root/'server',budget=10*1024**2,min_free=0)
            uploads=[]
            def request(_config,path,method='GET',file=None,headers=None):
                url=urllib.parse.urlsplit(path);query=urllib.parse.parse_qs(url.query)
                if url.path == '/health':
                    reply = io.BytesIO(json.dumps({
                        'schema': local_cache.SCHEMA, 'status': 'ready',
                        'capabilities': ['validated-cache-commit-v1', 'rust-index-references-v1',
                                         'complete-cache-retention-v1'],
                    }).encode())
                    reply.status = 200
                    return nullcontext(reply)
                group=query['group'][0]
                if url.path=='/v1/lookup':
                    value=store.lookup(group,query['key'][0],query.get('prefix',[''])[0].splitlines())
                    reply=io.BytesIO(json.dumps(value).encode());reply.status=200 if value else 404
                elif method=='PUT':
                    payload=file.read();uploads.append((group,len(payload)))
                    saved=store.put(group,query['key'][0],len(payload),headers['X-Content-SHA256'],io.BytesIO(payload),
                                    int(headers['X-Cache-Sequence']))
                    reply=io.BytesIO();reply.status=201 if saved else 409
                else:
                    blob=store.path(group,query['id'][0])
                    reply=io.BytesIO(blob.read_bytes() if blob.exists() else b'');reply.status=200 if blob.exists() else 404
                return nullcontext(reply)
            with patch.object(local_cache,'response',side_effect=request),patch.object(local_cache,'cache_report'), \
                    patch.dict(os.environ,{'GITHUB_RUN_ID':'1','GITHUB_RUN_ATTEMPT':'1'}):
                cache_shards.save(config,[root/'target'])
                self.assertGreater(len(uploads),2)
                initial_bytes=sum(size for _,size in uploads)
                target=root/'restored'
                restored={**config,'resolved_paths':[str(target)]}
                self.assertTrue(cache_shards.restore(restored,False))
                self.assertEqual((target/'release/deps/dep5.rlib').read_bytes(),b'dependency-5')
                # Updating one dependency sends one bucket and the small index,
                # rather than another complete target archive.
                (source/'dep5.rlib').write_bytes(b'updated-dependency')
                uploads.clear()
                config.update(key='generation-source-two',shard_index=restored['shard_index'])
                with patch.dict(os.environ,{'GITHUB_RUN_ID':'2'}): cache_shards.save(config,[root/'target'])
                self.assertEqual(len(uploads),2)
                self.assertLess(sum(size for _,size in uploads),initial_bytes)
                restored.update(key=config['key'])
                self.assertTrue(cache_shards.restore(restored,False))
                self.assertEqual((target/'release/deps/dep5.rlib').read_bytes(),b'updated-dependency')
                # LRU eviction can remove any shard while the index and a
                # build-script success marker still exist. Cargo does not
                # validate arbitrary generated OUT_DIR files on its own.
                marker=target/'release/.fingerprint/serde_core-hash/run-build-script-build-script-build'
                marker.parent.mkdir(parents=True)
                marker.write_bytes(b'cached success')
                shard=next(s for s in restored['shard_index']['shards'] if s)
                blob=store.path(shard['group'],shard['id'])
                blob.unlink()
                self.assertTrue(cache_shards.restore(restored,False))
                self.assertFalse(restored['hit'])
                self.assertTrue(restored['mirror_required'])
                self.assertFalse(marker.parent.exists())
                github_config = {**restored, 'hit': True, 'restored': True}
                with patch.object(local_cache, 'unpack') as unpack:
                    self.assertTrue(cache_shards.restore(github_config, True))
                    unpack.assert_not_called()
                self.assertTrue(github_config['mirror_required'])
                self.assertTrue(github_config['hit'])
                uploads.clear()
                github_config.update(save=True, source=str(root), mirror_required=False)
                with patch.object(local_cache, 'cache_paths', return_value=[root/'target']), \
                        patch.dict(os.environ, {'GITHUB_RUN_ID': '3'}):
                    local_cache.save(github_config)
                self.assertCountEqual([group for group, _ in uploads],
                                      [shard['group'], config['group'] + '/index'])
                self.assertTrue(cache_shards.restore(github_config, True))
                self.assertFalse(github_config['mirror_required'])
                self.assertTrue(cache_shards.index_available(github_config, github_config['shard_index']))
                shard=next(s for s in restored['shard_index']['shards'] if s)
                store.path(shard['group'],shard['id']).write_bytes(b'corrupt')
                with self.assertRaisesRegex(ValueError,'Incomplete|checksum'):
                    cache_shards.restore(restored,False)

    def test_shard_save_uses_the_configured_budget_and_keeps_the_index_when_incomplete(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary)/'dependency.rlib'
            source.write_bytes(b'dependency')
            info = tarfile.TarInfo('0/release/deps/dependency.rlib')
            info.size = source.stat().st_size
            groups = [[(source, info, local_cache.digest(source))] for _ in range(3)]
            groups.extend([[] for _ in range(cache_shards.COUNT - len(groups))])
            for budget, committed in ((900, True), (120, False)):
                clock = [100]
                def lookup(config, group, key):
                    clock[0] += 250
                    return {'group': group, 'key': key}
                with self.subTest(budget=budget), \
                        patch.object(cache_shards.time, 'monotonic', side_effect=lambda: clock[0]), \
                        patch.object(local_cache, 'phase', return_value=nullcontext()), \
                        patch.object(local_cache, 'uploads_supported', return_value=True), \
                        patch.object(cache_shards, 'partition', return_value=groups), \
                        patch.object(cache_shards, 'lookup', side_effect=lookup), \
                        patch.object(cache_shards, 'upload', return_value={'saved': True}) as upload, \
                        patch.object(local_cache, 'cache_report') as report:
                    cache_shards.save({'group': 'rust/linux-x64', 'key': 'current',
                                       'prefix': ['compiler-'], 'save_seconds': budget}, [source.parent])
                    self.assertEqual(upload.call_count, int(committed))
                    if committed:
                        self.assertEqual(upload.call_args.args[1], 'rust/linux-x64/index')
                    self.assertIn('committed' if committed else 'previous index retained',
                                  report.call_args.args[2])

    def test_restore_budget_keeps_verified_shards_and_invalidates_partial_build_scripts(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);source=root/'source';target=root/'target'
            library=source/'release/deps/verified.rlib'
            library.parent.mkdir(parents=True);library.write_bytes(b'complete dependency')
            archive=root/'fixture.tar.gz';local_cache.pack([source],archive)
            marker=target/'release/.fingerprint/generated-hash/run-build-script-cached'
            marker.parent.mkdir(parents=True);marker.write_bytes(b'old success')
            group='rust/macos-x64'
            shards=[{'schema':local_cache.SCHEMA,'group':f'{group}/p{i:02d}',
                     'key':'generation','id':f'{i+1:064x}','sha256':'b'*64,'size':archive.stat().st_size}
                    if i<2 else None for i in range(cache_shards.COUNT)]
            record={'key':'generation','size':1000}
            config={'group':group,'key':'generation','prefix':['generation'],
                    'resolved_paths':[str(target)],'kind':'rust'}
            def download(_config,item,destination):
                if item is record:
                    destination.write_text(json.dumps({'format':'rust-shards-v1','shards':shards}));return
                if item['group'].endswith('p00'):
                    shutil.copyfile(archive,destination);return
                self.assertEqual((target/'release/deps/verified.rlib').read_bytes(),b'complete dependency')
                config['restore_deadline']=local_cache.time.monotonic()-1
                raise local_cache.RestoreBudgetExceeded('test deadline')
            with patch.object(cache_shards,'lookup',return_value=record), \
                    patch.object(cache_shards,'download',side_effect=download), \
                    patch.object(local_cache,'cache_report'):
                self.assertTrue(cache_shards.restore(config,False))
            self.assertEqual((target/'release/deps/verified.rlib').read_bytes(),b'complete dependency')
            self.assertFalse(marker.parent.exists())
            self.assertTrue(config['restored'])
            self.assertTrue(config['mirror_required'])
            self.assertFalse(config['hit'])

    def test_continuously_trickling_cache_data_cannot_extend_restore_budget(self):
        clock=[0.0]
        def tick():
            clock[0]+=0.25;return clock[0]
        reply=io.BytesIO(b'x'*100)
        # read1 returns immediately available data; a blocking read(size) would
        # permit a peer to extend the restore indefinitely by trickling bytes.
        class Trickle:
            def read1(self,_size):return reply.read(1)
        output=io.BytesIO()
        with patch.object(local_cache.time,'monotonic',side_effect=tick):
            with self.assertRaises(local_cache.RestoreBudgetExceeded):
                local_cache.download_body({'restore_deadline':2},Trickle(),output,100)
        self.assertGreater(len(output.getvalue()),0)
        self.assertLess(len(output.getvalue()),100)
        config={'restore_seconds':300}
        with patch.object(local_cache,'restore_snapshot',side_effect=local_cache.RestoreBudgetExceeded):
            with self.assertRaises(local_cache.RestoreBudgetExceeded):local_cache.restore(config)
        self.assertNotIn('restore_deadline',config,'post-job saves must have an independent deadline')

    def test_cache_action_watchdog_cleans_interrupted_restores_before_continuing(self):
        node=shutil.which('node')
        if not node:self.skipTest('Node is required for the Actions wrapper test')
        script=build.ROOT/'.github/actions/local-cache/index.cjs'
        harness=r'''
const assert=require('node:assert/strict'),vm=require('node:vm'),realfs=require('node:fs');
const script=process.argv[1],mode=process.argv[2],calls=[],outputs=[];let saved;
const fs={writeFileSync(_path,value){saved=JSON.parse(value)},
  readFileSync(){return JSON.stringify(saved)},
  appendFileSync(path,value){if(path==='output')outputs.push(value)}};
const child={spawnSync(command,args,options){
  calls.push({command,args,options});
  if(calls.length===1)return mode==='exit'?{status:1}:{status:null,error:{code:'ETIMEDOUT'}};
  if(mode==='cleanup-failure')return {status:1};
  saved={...saved,hit:false,restored:false,available:false};return {status:0};
}};
const context={__dirname:require('node:path').dirname(script),console:{log(){}},
  process:{env:{RUNNER_TEMP:'/runner/temp',GITHUB_STATE:'state',GITHUB_OUTPUT:'output'},platform:'linux'},
  require(name){if(name==='node:fs')return fs;if(name==='node:child_process')return child;return require(name)}};
const run=()=>vm.runInNewContext(realfs.readFileSync(script,'utf8'),context,{filename:script});
if(mode==='cleanup-failure'){
  assert.throws(run,/Cannot safely clean/);assert.equal(outputs.length,0);
}else{
  run();assert.equal(calls[1].args[1],'discard-restore');
  assert.ok(calls[0].options.timeout>0 && calls[0].options.timeout<=420000);
  assert.ok(calls[1].options.timeout<=120000);
  assert.match(outputs[0],/^state=\/runner\/temp\/private-cache-[a-f0-9-]+\.json\n$/);
  assert.equal(outputs.slice(1).join(''),'cache-hit=false\nrestored=false\navailable=false\n');
}
assert.equal(calls.length,2);
'''
        for mode in ('timeout','exit','cleanup-failure'):
            with self.subTest(mode=mode):
                result=subprocess.run([node,'-e',harness,str(script),mode],capture_output=True,text=True)
                self.assertEqual(result.returncode,0,result.stdout+result.stderr)

    def test_blob_requests_use_remaining_restore_budget_without_short_metadata_limit(self):
        config = {'url': 'http://100.64.0.1:8080', 'token': 'test', 'restore_deadline': 1000}
        for path, budget, expected in (('/v1/blob?id=test', 180, 180),
                                        ('/v1/blob?id=test', 800, 600),
                                        ('/v1/lookup', 180, 30)):
            with self.subTest(path=path, budget=budget), \
                    patch.object(local_cache, 'restore_remaining', return_value=budget), \
                    patch.object(local_cache.http.client, 'HTTPConnection') as connection:
                local_cache.request(config, path)
                self.assertEqual(connection.call_args.kwargs['timeout'], expected)

    def test_cache_action_save_budget_is_validated_and_applied_to_the_watchdog(self):
        node = shutil.which('node')
        if not node:
            self.skipTest('Node is required for the Actions wrapper test')
        script = build.ROOT / '.github/actions/local-cache/index.cjs'
        harness = r'''
const assert=require('node:assert/strict'),vm=require('node:vm'),realfs=require('node:fs');
const script=process.argv[1],budget=process.argv[2],post=process.argv[3]==='post',calls=[];
let saved={save:true,save_seconds:Number(budget)};
const fs={writeFileSync(_path,value){saved=JSON.parse(value)},
  readFileSync(){return JSON.stringify(saved)},appendFileSync(){},rmSync(){}};
const context={__dirname:require('node:path').dirname(script),console:{log(){}},
  process:{env:{RUNNER_TEMP:'/runner/temp',GITHUB_STATE:'state',GITHUB_OUTPUT:'output',
    'INPUT_SAVE-TIMEOUT':budget,...(post?{STATE_cache_state:'state'}:{})},platform:'linux'},
  require(name){if(name==='node:fs')return fs;if(name==='node:child_process')
    return {spawnSync(_command,_args,options){calls.push(options);return {status:0}}};return require(name)}};
const run=()=>vm.runInNewContext(realfs.readFileSync(script,'utf8'),context,{filename:script});
if(post){run();assert.equal(calls[0].timeout,Number(budget)*1000)}
else{assert.throws(run,/Invalid private cache save budget/);assert.equal(calls.length,0)}
'''
        for budget, mode in (('900', 'post'), ('1200', 'post'), ('29', 'main'),
                              ('1801', 'main'), ('invalid', 'main')):
            with self.subTest(budget=budget, mode=mode):
                result = subprocess.run([node, '-e', harness, str(script), budget, mode],
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_partial_rust_cache_invalidates_only_native_and_cross_build_script_runs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            profiles=[root/'target/release',root/'target/aarch64-linux-android/release',
                      root/'target/debug',root/'target/aarch64-linux-android/debug']
            markers=[]
            for profile in profiles:
                for layout in ['serde_core-hash', 'serde_core/another-hash']:
                    marker=profile/'.fingerprint'/layout/'run-build-script-build-script-build.json'
                    marker.parent.mkdir(parents=True)
                    marker.write_text('{"cached":"success"}')
                    markers.append(marker)
                for name in ['.fingerprint/library-hash/lib-library',
                             'build/serde_core-hash/build-script-build',
                             'deps/library.rlib']:
                    path=profile/name;path.parent.mkdir(parents=True,exist_ok=True)
                    path.write_bytes(b'reusable compilation')
            cache_shards.invalidate_build_scripts([root/'target'])
            for marker in markers:
                self.assertFalse(marker.parent.exists())
            for profile in profiles:
                for name in ['.fingerprint/library-hash/lib-library',
                             'build/serde_core-hash/build-script-build',
                             'deps/library.rlib']:
                    self.assertEqual((profile/name).read_bytes(),b'reusable compilation')

    def test_rust_shards_reject_indexes_pointing_outside_their_cache_group(self):
        record={'schema':local_cache.SCHEMA,'group':'rust/linux-x64/p00','key':'compiler-object',
                'id':'a'*64,'sha256':'b'*64,'size':1024}
        cache_shards.validate(record,'rust/linux-x64/p00')
        with self.assertRaises(ValueError): cache_shards.validate(record,'rust/windows-x64/p00')
        for field,value in (('id','../escape'),('size',-1),('sha256','bad')):
            with self.subTest(field=field),self.assertRaises(ValueError):
                cache_shards.validate({**record,field:value},record['group'])

    def test_rust_shard_index_requires_matching_remote_metadata(self):
        shard = {'schema': local_cache.SCHEMA, 'group': 'rust/linux-x64/p00',
                 'key': 'generation', 'id': 'a'*64, 'sha256': 'b'*64, 'size': 32}
        index = {'shards': [shard, *([None] * (cache_shards.COUNT - 1))]}
        for current in (None, {**shard, 'id': 'c'*64},
                        {**shard, 'sha256': 'd'*64}, {**shard, 'size': 31}, shard):
            with self.subTest(current=current), patch.object(cache_shards, 'lookup', return_value=current):
                self.assertEqual(cache_shards.index_available({}, index), current == shard)

    def test_cache_writes_require_explicit_safe_commit_capabilities(self):
        for kind in ('archives', 'rust'):
            for capabilities in (None, [], ['validated-cache-commit-v1'],
                                 ['rust-index-references-v1'],
                                 ['validated-cache-commit-v1', 'rust-index-references-v1'],
                                 ['validated-cache-commit-v1', 'complete-cache-retention-v1'],
                                 ['validated-cache-commit-v1', 'rust-index-references-v1',
                                  'complete-cache-retention-v1'], [{}]):
                with self.subTest(kind=kind, capabilities=capabilities):
                    reply = io.BytesIO(json.dumps({
                        'schema': local_cache.SCHEMA, 'status': 'ready',
                        'capabilities': capabilities,
                    }).encode())
                    reply.status = 200
                    expected = (isinstance(capabilities, list)
                                and 'validated-cache-commit-v1' in capabilities
                                and 'complete-cache-retention-v1' in capabilities
                                and (kind != 'rust' or 'rust-index-references-v1' in capabilities))
                    with patch.object(local_cache, 'response', return_value=nullcontext(reply)):
                        self.assertEqual(local_cache.uploads_supported({'kind': kind}), expected)
        reply = io.BytesIO()
        reply.status = 404
        with patch.object(local_cache, 'response', return_value=nullcontext(reply)):
            self.assertFalse(local_cache.uploads_supported({'kind': 'rust'}))

    def test_unsafe_cache_server_never_receives_archives_or_shards(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for kind in ('archives', 'rust'):
                config = {'kind': kind, 'group': 'dependencies/source' if kind == 'archives' else 'rust/linux-x64',
                          'key': 'generation', 'save': True, 'available': True}
                with self.subTest(kind=kind), \
                        patch.object(local_cache, 'cache_paths', return_value=[root]), \
                        patch.object(local_cache, 'uploads_supported', return_value=False), \
                        patch.object(local_cache, 'cache_report') as report, \
                        patch.object(local_cache, 'pack') as pack, \
                        patch.object(cache_shards, 'partition') as partition, \
                        patch.object(local_cache, 'response') as request:
                    local_cache.save(config)
                    pack.assert_not_called()
                    partition.assert_not_called()
                    request.assert_not_called()
                    self.assertIn('server upgrade required', report.call_args.args[2])

    def test_shared_android_sources_require_the_current_lockfiles_and_archive_digest(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);home=root/'home';out=root/'shared'
            (home/'go/pkg/mod/cache/download/example/@v').mkdir(parents=True)
            module=home/'go/pkg/mod/cache/download/example/@v/v1.0.0.zip';module.write_bytes(b'verified source zip')
            (home/'go/bin').mkdir(parents=True)
            for tool in ('gomobile','gobind'): (home/'go/bin'/tool).write_bytes(b'verified tool')
            (root/'apps/android/core').mkdir(parents=True)
            mod=root/'apps/android/core/go.mod';mod.write_bytes(b'module test')
            (root/'apps/android/core/go.sum').write_bytes(b'checksums')
            with patch.object(build_inputs,'ROOT',root),patch.object(build_inputs.Path,'home',return_value=home), \
                    patch.object(build_inputs,'current_commit',return_value='a'*40), \
                    patch.object(android_build,'ensure_binding_tool'),patch.object(build_inputs.subprocess,'run'):
                build_inputs.shared_go(out)
                module.unlink()
                build_inputs.shared_go(out,restore=True)
                self.assertEqual(module.read_bytes(),b'verified source zip')
                mod.write_bytes(b'changed dependency')
                with self.assertRaises(ValueError): build_inputs.shared_go(out,restore=True)
                mod.write_bytes(b'module test')
                archive=out/'go-source.tar.gz';archive.write_bytes(archive.read_bytes()+b'tampered')
                with self.assertRaises(ValueError): build_inputs.shared_go(out,restore=True)

    def test_shared_android_sources_reject_untidy_lockfiles_before_packing_or_building_tools(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(build_inputs.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, 'go')) as execute, \
                patch.object(android_build, 'ensure_binding_tool') as binding, \
                patch.object(local_cache, 'pack') as pack:
            directory = Path(temporary)
            with self.assertRaises(subprocess.CalledProcessError):
                build_inputs.shared_go(directory)
            self.assertEqual(execute.call_args.args[0][-3:], ['mod', 'tidy', '-diff'])
            binding.assert_not_called()
            pack.assert_not_called()
            self.assertFalse((directory/'receipt.json').exists())

    def test_cache_timing_report_never_copies_credentials_or_private_action_state(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            config={'group':'rust/linux-x64','key':'verified','token':'never-publish-this-token',
                    'url':'http://100.64.3.1:19081','started':10,
                    'timings':{'download':1.25,'token':'never-publish-this-token'}}
            with patch.dict(os.environ,{'GITHUB_WORKSPACE':str(root),'GITHUB_STEP_SUMMARY':str(root/'summary.txt')}), \
                    patch.object(local_cache.time,'monotonic',return_value=12), \
                    patch('sys.stdout',new_callable=io.StringIO) as logged:
                local_cache.cache_report(config,'restore','exact',1024)
            self.assertNotIn(config['token'],logged.getvalue())
            self.assertNotIn(config['url'],logged.getvalue())
            self.assertIn('restore: exact',logged.getvalue())
            report=next((root/'target/cache-report').glob('*.json'))
            self.assertEqual(json.loads(report.read_text())['timings_seconds'],{'download':1.25,'total':2})
            for path in (report,root/'summary.txt'):
                self.assertNotIn(config['token'],path.read_text())
                self.assertNotIn(config['url'],path.read_text())

    def test_interrupted_extraction_keeps_existing_files_and_compiler_cleanup_preserves_reports(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);source=root/'source';source.mkdir()
            (source/'object.rlib').write_bytes(b'new complete object')
            archive=root/'cache.tar.gz';local_cache.pack([source],archive)
            restored=root/'restored';restored.mkdir();object_file=restored/'object.rlib'
            object_file.write_bytes(b'old valid object')
            with patch.object(local_cache.shutil,'copyfileobj',side_effect=OSError('disk write interrupted')):
                with self.assertRaises(OSError):local_cache.unpack(archive,[restored])
            self.assertEqual(object_file.read_bytes(),b'old valid object')
            self.assertEqual(list(restored.glob('.cache-extract-*')),[])
            target=root/'target'
            for name in ('release/deps/bad.rlib','aarch64-linux-android/release/deps/bad.rlib',
                         'debug/deps/bad.rlib','aarch64-linux-android/debug/deps/bad.rlib',
                         'dependency-snapshot/inputs.json','android-report/logcat.txt'):
                path=target/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(b'preserve only reports/inputs')
            unpacked=root/'cargo/registry/src/crate/lib.rs'
            unpacked.parent.mkdir(parents=True);unpacked.write_bytes(b'incomplete restored source')
            with patch.dict(os.environ,{'GITHUB_ACTIONS':'true','CARGO_HOME':str(root/'cargo'),
                                       'WORKSPACE_SOURCE_CACHE':str(root/'external/source.json')}):
                local_cache.discard_restore({'kind':'rust','source':str(root)})
            self.assertFalse((target/'release').exists())
            self.assertFalse((target/'aarch64-linux-android/release').exists())
            self.assertFalse((target/'debug').exists())
            self.assertFalse((target/'aarch64-linux-android/debug').exists())
            self.assertTrue((target/'dependency-snapshot/inputs.json').exists())
            self.assertTrue((target/'android-report/logcat.txt').exists())
            self.assertFalse(unpacked.exists())

    def test_github_hit_only_checks_private_metadata_and_reuses_current_mirror(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            (root/'module.zip').write_bytes(b'GitHub restored source')
            config={'group':'dependencies/source','key':'current','base_key':'current',
                    'kind':'archives','paths':str(root),'prefix':['compatible-'],
                    'url':'http://127.0.0.1:19081','token':'test-only','save':True,'github_hit':'true'}
            generation='v'+str(local_cache.SCHEMA)+'-'+hashlib.sha256(str(root).encode()).hexdigest()[:16]+'-'
            record={'schema':local_cache.SCHEMA,'group':config['group'],'key':generation+'current','id':'a'*64}
            reply=io.BytesIO(json.dumps(record).encode()); reply.status=200
            with patch.object(local_cache,'response',return_value=nullcontext(reply)) as request, \
                    patch.object(local_cache,'cache_report'),patch.object(local_cache,'pack') as pack:
                local_cache.restore(config)
                self.assertTrue(config['restored'])
                self.assertFalse(config['mirror_required'])
                request.assert_called_once()
                self.assertTrue(request.call_args.args[1].startswith('/v1/lookup?'))
                local_cache.save(config)
                pack.assert_not_called()

    def test_github_hit_with_no_private_copy_registers_a_required_mirror(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            (root/'module.zip').write_bytes(b'GitHub restored source')
            config={'group':'dependencies/source','key':'current','base_key':'current',
                    'kind':'archives','paths':str(root),'prefix':[],
                    'url':'http://127.0.0.1:19081','token':'test-only','save':True,'github_hit':'true'}
            reply=io.BytesIO(); reply.status=404
            with patch.object(local_cache,'response',return_value=nullcontext(reply)) as request, \
                    patch.object(local_cache,'cache_report'):
                local_cache.restore(config)
                self.assertTrue(config['restored'])
                self.assertTrue(config['available'])
                self.assertTrue(config['mirror_required'])
                request.assert_called_once()

            uploaded=[]
            def upload(_config,path,**kwargs):
                self.assertEqual(kwargs['method'],'PUT')
                self.assertTrue(path.startswith('/v1/blob?'))
                uploaded.append(kwargs['file'].read())
                result=io.BytesIO(); result.status=201
                return nullcontext(result)
            with patch.object(local_cache,'response',side_effect=upload),patch.object(local_cache,'cache_report'), \
                    patch.object(local_cache, 'uploads_supported', return_value=True):
                local_cache.save(config)
                local_cache.save(config)
            self.assertEqual(len(uploaded),1)
            self.assertTrue(uploaded[0])

    def test_rust_compatible_github_hit_avoids_a_second_download_but_empty_miss_does_not(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            config={'kind':'rust','source':str(root),'github_hit':'false'}
            self.assertFalse(local_cache.github_restored(config))
            deps=root/'target/release/deps'; deps.mkdir(parents=True)
            (deps/'not-an-object.d').write_text('build metadata')
            (deps/'empty.rlib').touch()
            self.assertFalse(local_cache.github_restored(config))
            (deps/'dependency.rlib').write_bytes(b'restored dependency')
            self.assertTrue(local_cache.github_restored(config))
            config['github_hit']=''
            self.assertFalse(local_cache.github_restored(config))

    def test_rust_private_mirror_reuses_sources_but_invalidates_changed_dependencies_and_compiler(self):
        config={'kind':'rust','key':'compiler-dependencies-new-source',
                'prefix':['compiler-dependencies-','compiler-']}
        self.assertTrue(local_cache.private_current(config,{'key':'compiler-dependencies-old-source'}))
        self.assertFalse(local_cache.private_current(config,{'key':'compiler-old-dependencies-old-source'}))
        self.assertFalse(local_cache.private_current(config,{'key':'old-compiler-dependencies-old-source'}))
        self.assertFalse(local_cache.private_current(config,None))

    def test_source_only_changes_use_shards_but_exact_private_snapshots_are_not_uploaded(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary); (root/'target').mkdir()
            config={'kind':'rust','source':str(root),'group':'rust/linux-x64','key':'new-source',
                    'available':True,'save':True,'hit':False,'mirror_required':False,
                    'shard_index': {'shards': [None] * cache_shards.COUNT}}
            with patch.object(local_cache,'pack') as pack,patch.object(local_cache,'cache_report'), \
                    patch.object(cache_shards,'save') as sharded:
                local_cache.save(config)
                pack.assert_not_called()
                sharded.assert_called_once()
                config['private_exact']=True
                local_cache.save(config)
                sharded.assert_called_once()

    def test_immutable_downloads_do_not_reupload_build_script_mutations(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);(root/'source').write_bytes(b'pinned source')
            config={'kind':'archives','paths':str(root),'group':'dependencies/skia-armv7','key':'crate-checksum',
                    'available':True,'save':True,'hit':True,'immutable':True}
            with patch.object(local_cache,'pack') as pack,patch.object(local_cache,'fingerprint') as fingerprint, \
                    patch.object(local_cache,'cache_report'):
                local_cache.save(config)
                pack.assert_not_called()
                fingerprint.assert_not_called()

    def test_github_miss_restores_a_verified_private_archive(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary); source=root/'source'; source.mkdir()
            (source/'module.zip').write_bytes(b'private dependency')
            archive=root/'cache.tar.gz'; local_cache.pack([source],archive)
            destination=root/'restored'
            config={'group':'dependencies/source','key':'current','base_key':'current',
                    'kind':'archives','paths':str(destination),'prefix':[],
                    'url':'http://127.0.0.1:19081','token':'test-only','save':True,'github_hit':''}
            generation='v'+str(local_cache.SCHEMA)+'-'+hashlib.sha256(str(destination).encode()).hexdigest()[:16]+'-'
            record={'schema':local_cache.SCHEMA,'group':config['group'],'key':generation+'current',
                    'id':'a'*64,'size':archive.stat().st_size,'sha256':local_cache.digest(archive)}
            def reply(_config,path):
                result=io.BytesIO(json.dumps(record).encode() if path.startswith('/v1/lookup?') else archive.read_bytes())
                result.status=200
                return nullcontext(result)
            with patch.object(local_cache,'response',side_effect=reply) as request,patch.object(local_cache,'cache_report'):
                local_cache.restore(config)
                self.assertTrue(config['restored'])
                self.assertEqual((destination/'module.zip').read_bytes(),b'private dependency')
                self.assertEqual(request.call_count,2)

    def test_local_cache_uploads_are_atomic_checked_and_immutable(self):
        with tempfile.TemporaryDirectory() as temporary:
            store = local_cache.Store(temporary, budget=1000, min_free=0)
            payload = b'verified-cache'
            checksum = hashlib.sha256(payload).hexdigest()
            with self.assertRaises(ValueError): store.put('rust/linux-x64', 'generation-one', len(payload), checksum, io.BytesIO(b'short'))
            with self.assertRaises(ValueError): store.put('rust/linux-x64', 'generation-one', len(payload), '0'*64, io.BytesIO(payload))
            self.assertIsNone(store.lookup('rust/linux-x64', 'generation-one', []))
            self.assertEqual(list(store.incoming.iterdir()), [])
            store.put('rust/linux-x64', 'generation-one', len(payload), checksum, io.BytesIO(payload))
            record = store.lookup('rust/linux-x64', 'generation-one', [])
            with patch.object(local_cache.time,'time',return_value=9_999_999_999):
                self.assertEqual(store.fetch(record['group'],record['id']).read_bytes(),payload)
            with store.database() as database:
                self.assertEqual(database.execute('SELECT accessed FROM entries').fetchone()[0],9_999_999_999)
            other = b'another-cache'
            store.put('rust/linux-x64', 'generation-one', len(other), hashlib.sha256(other).hexdigest(), io.BytesIO(other))
            self.assertEqual(store.path(record['group'], record['id']).read_bytes(), payload)
            self.assertEqual(store.lookup('rust/linux-x64', 'generation-two', ['generation-'])['key'], 'generation-one')
            with self.assertRaises(ValueError): store.path('../../escape', record['id'])

    def test_cache_readme_rejects_bad_and_stale_uploads_without_replacing_content(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            store = local_cache.Store(root, budget=1000, min_free=0)

            def put(data, sequence, checksum=None, length=None):
                return store.put_readme(length if length is not None else len(data),
                                        checksum or hashlib.sha256(data).hexdigest(),
                                        sequence, io.BytesIO(data))

            current = b'Action-Sequence: 2001\nSource-Commit: current\n\nCurrent README\n'
            self.assertEqual(put(current, 2001), 'updated')
            self.assertEqual((root/'state/README.txt').read_bytes(), current)
            self.assertFalse((root/'README.txt').exists())
            self.assertEqual(put(current, 2001), 'unchanged')
            older = b'Action-Sequence: 1001\n\nOld README\n'
            self.assertEqual(put(older, 1001), 'superseded')
            self.assertEqual(put(b'Action-Sequence: 2001\n\nDifferent\n', 2001), 'superseded')
            with self.assertRaises(ValueError):
                put(current, 2001, checksum='0'*64)
            with self.assertRaises(ValueError):
                put(current, 3001, length=len(current)+1)
            with self.assertRaises(ValueError):
                put(b'x'*(local_cache.README_LIMIT+1), 3001)
            with self.assertRaises(ValueError):
                put(b'Action-Sequence: 3001\n\xff', 3001)
            self.assertEqual((root/'state/README.txt').read_bytes(), current)
            self.assertEqual(list((root/'state').glob('.readme-*')), [])

    def test_cache_readme_rejects_symlink_in_writable_state(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            store = local_cache.Store(root/'server', budget=1000, min_free=0)
            protected = root/'protected.txt'
            protected.write_text('keep', encoding='utf-8')
            (root/'server/state/README.txt').symlink_to(protected)
            body = b'Action-Sequence: 1001\n\nNew README\n'
            with self.assertRaises(ValueError):
                store.put_readme(len(body), hashlib.sha256(body).hexdigest(), 1001, io.BytesIO(body))
            self.assertEqual(protected.read_text(encoding='utf-8'), 'keep')

    def test_local_cache_pruning_keeps_recent_generations_and_respects_budget(self):
        with tempfile.TemporaryDirectory() as temporary:
            store = local_cache.Store(temporary, budget=100, min_free=0, versions=2)
            with patch.object(local_cache.time, 'time', return_value=100):
                for key in ('old', 'current', 'new'):
                    store.put('compiler/macos-arm64', key, 20, hashlib.sha256(b'a'*20).hexdigest(), io.BytesIO(b'a'*20))
            self.assertEqual(store.status()['groups'][0]['count'], 2)
            with self.assertRaises(OSError):
                store.put('rust/linux-x64', 'too-large', 101, hashlib.sha256(b'x'*101).hexdigest(), io.BytesIO(b'x'*101))
            self.assertEqual(store.status()['groups'][0]['count'], 2)

    def test_local_cache_plain_http_is_limited_to_private_tailnet_transport(self):
        for address in ('http://100.64.3.1:19081', 'http://127.0.0.1:19081', 'https://cache.example.com'):
            local_cache.endpoint({'url':address})
        for address in ('http://1.1.1.1', 'http://10.0.0.1', 'https://user:password@example.com', 'file:///tmp/cache'):
            with self.assertRaises(ValueError): local_cache.endpoint({'url':address})

    def test_cache_replacement_rejects_stale_jobs_and_keeps_old_data_on_failed_upload(self):
        with tempfile.TemporaryDirectory() as temporary:
            store = local_cache.Store(temporary, budget=1000, min_free=0)
            def put(key, data, sequence):
                return store.put('dependencies/source', key, len(data), hashlib.sha256(data).hexdigest(), io.BytesIO(data), sequence)
            self.assertTrue(put('snapshot-one', b'old', 100))
            with self.assertRaises(ValueError):
                store.put('dependencies/source', 'snapshot-two', 10, '0'*64, io.BytesIO(b'short'), 200)
            self.assertIsNotNone(store.lookup('dependencies/source', 'snapshot-one', []))
            self.assertTrue(put('snapshot-two', b'new', 200))
            self.assertFalse(put('snapshot-one', b'stale', 100))
            self.assertIsNone(store.lookup('dependencies/source', 'snapshot-one', []))
            self.assertTrue(put('snapshot-two', b'newer content', 300))
            record = store.lookup('dependencies/source', 'snapshot-two', [])
            self.assertEqual(store.path(record['group'], record['id']).read_bytes(), b'newer content')
            self.assertEqual(len(store.status()['entries']), 1)

    def test_dependency_snapshot_archives_keep_lockfiles(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root/'snapshot'; source.mkdir()
            (source/'Cargo.lock').write_bytes(b'locked source versions')
            archive = root/'cache.tar.gz'
            local_cache.pack([source], archive)
            local_cache.unpack(archive, [root/'restored'])
            self.assertEqual((root/'restored/Cargo.lock').read_bytes(), b'locked source versions')

    def test_unchanged_archives_ignore_temporary_locks_and_need_no_reupload(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root/'module.zip').write_bytes(b'cached source')
            fingerprint = local_cache.fingerprint([root])
            (root/'download.lock').write_bytes(b'temporary lock')
            self.assertEqual(local_cache.fingerprint([root]), fingerprint)
            config = {'group':'dependencies/source', 'key':'verified',
                      'kind':'archives', 'available':True, 'save':True, 'hit':False,
                      'fingerprint':fingerprint}
            with patch.object(local_cache, 'cache_paths', return_value=[root]), \
                 patch.object(local_cache, 'cache_report') as report, patch.object(local_cache, 'pack') as pack:
                local_cache.save(config)
                pack.assert_not_called()
                self.assertIn('unchanged', report.call_args.args[2])

    def test_rust_cache_keeps_build_scripts_without_archiving_application_or_test_binaries(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root/'target'
            names = ('release/build/tool/build-script-build.exe', 'release/deps/dep.rlib',
                     'release/clash-of-rust.exe', 'release/clash-of-rust', 'release/deps/unit-test',
                     'debug/deps/dep.rlib', 'debug/build/tool/build-script-build',
                     'debug/deps/unit-test', 'debug/clash-of-rust', 'debug/clash-desktop-ui', 'release/cor',
                     'aarch64-linux-android/debug/deps/dep.rlib',
                     'dependency-snapshot/inputs.json', 'cache-network.json', 'android-report/logcat.txt')
            for name in names:
                path = source/name; path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(b'binary')
            archive = root/'cache.tar.gz'
            local_cache.pack([source], archive, rust=True)
            local_cache.unpack(archive, [root/'restored'])
            files = sorted(p.relative_to(root/'restored').as_posix() for p in (root/'restored').rglob('*') if p.is_file())
            self.assertEqual(files, ['aarch64-linux-android/debug/deps/dep.rlib',
                                     'debug/build/tool/build-script-build', 'debug/deps/dep.rlib',
                                     'release/build/tool/build-script-build.exe', 'release/deps/dep.rlib'])

    def test_dependency_refresh_rejects_normal_ci_before_running_commands(self):
        for variables in ({}, {'GITHUB_ACTIONS': 'true', 'GITHUB_EVENT_NAME': 'push'}):
            with patch.dict(os.environ, variables, clear=True), \
                    patch.object(build_inputs.subprocess, 'run') as command:
                with self.assertRaisesRegex(ValueError, 'explicitly dispatched'):
                    build_inputs.refresh(Path('/unused'))
                command.assert_not_called()

    def test_approved_tool_check_rejects_search_path_drift_before_compiling(self):
        selected = build_inputs.tool_versions()
        for python_version, go_version, java_version, expected in (
            (selected['python'], 'go' + selected['go'], selected['java'].split('+')[0], None),
            ('3.14.0', 'go' + selected['go'], selected['java'].split('+')[0], 'Python'),
            (selected['python'], 'go1.26.10', selected['java'].split('+')[0], 'Go'),
            (selected['python'], 'go' + selected['go'], '17.0.21', 'Java'),
        ):
            with self.subTest(python=python_version, go=go_version, java=java_version), \
                    patch.object(build_inputs.sys, 'version', python_version), \
                    patch.object(build_inputs.subprocess, 'run', side_effect=[
                        subprocess.CompletedProcess([], 0, go_version + '\n', ''),
                        subprocess.CompletedProcess([], 0, '', '    java.version = ' + java_version + '\n'),
                    ]):
                if expected:
                    with self.assertRaisesRegex(ValueError, 'Active ' + expected):
                        build_inputs.check_tools()
                else:
                    build_inputs.check_tools()

    def test_dependency_refresh_does_not_export_partial_lockfiles_after_resolution_failure(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.dict(os.environ, {'GITHUB_ACTIONS': 'true', 'GITHUB_EVENT_NAME': 'workflow_dispatch'}), \
                patch.object(build_inputs.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, 'cargo')):
            output = Path(temporary) / 'updates'
            with self.assertRaises(subprocess.CalledProcessError):
                build_inputs.refresh(output)
            self.assertFalse(output.exists())

    def test_dependency_refresh_sets_approved_go_version_before_resolving_modules(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.dict(os.environ, {'GITHUB_ACTIONS': 'true', 'GITHUB_EVENT_NAME': 'workflow_dispatch'}), \
                patch.object(build_inputs.subprocess, 'run') as execute, \
                patch.object(build_inputs, 'constraints', return_value={}), \
                patch.object(build_inputs, 'current_commit', return_value='current'):
            build_inputs.refresh(Path(temporary))
        self.assertEqual([call.args[0] for call in execute.call_args_list], [
            ['cargo', 'update'], ['go', 'mod', 'edit', '-go=' + build_inputs.tool_versions()['go']],
            ['go', 'get', '-u', './...'], ['go', 'mod', 'tidy'],
        ])

    def test_resolve_uses_repository_dependencies_and_approved_tools_without_updating(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            files = {name: b'approved source' for name in build_inputs.FILES}
            files['Cargo.lock'] = b'[[package]]\nname="example"\nversion="1.0.0"\n'
            for name, content in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(content)
            snapshot = root / 'snapshot'
            snapshot.mkdir()
            (snapshot / 'inputs.json').write_text('{"upstream":{"gradle":{"version":"99.0.0"}}}')
            with patch.object(build_inputs, 'ROOT', root), \
                    patch.object(build_inputs, 'original', side_effect=files.__getitem__), \
                    patch.object(build_inputs, 'constraints', return_value={}), \
                    patch.object(build_inputs, 'current_commit', return_value='current'), \
                    patch.object(build_inputs, 'approved', return_value={'upstream': {'gradle': {'version': '9.8.2'}}}), \
                    patch.object(build_inputs, 'read_json', side_effect=AssertionError('unexpected network update')), \
                    patch.object(build_inputs.subprocess, 'run', side_effect=AssertionError('unexpected resolver')), \
                    patch.object(build_inputs, 'report'):
                build_inputs.resolve(snapshot)
                value = json.loads((snapshot / 'inputs.json').read_text())
                self.assertEqual(value['upstream']['gradle']['version'], '9.8.2')
                self.assertEqual(value['files'], {name: hashlib.sha256(content).hexdigest() for name, content in files.items()})
                self.assertTrue(all((root / name).read_bytes() == content for name, content in files.items()))
                (root / 'Cargo.lock').write_text('unapproved update')
                with self.assertRaisesRegex(ValueError, 'approved repository'):
                    build_inputs.resolve(snapshot)

    def test_version_bootstrap_does_not_require_python_311_modules(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'versions'
            script = build.ROOT / 'scripts/build_inputs.py'
            program = '''
import builtins, runpy, sys
from pathlib import Path
original_import = builtins.__import__
def bootstrap_import(name, *args, **kwargs):
    if name == 'tomllib':
        raise ModuleNotFoundError('unavailable before Python setup')
    return original_import(name, *args, **kwargs)
builtins.__import__ = bootstrap_import
script = Path(sys.argv[1])
sys.path.insert(0, str(script.parent))
sys.argv = [str(script), 'versions']
runpy.run_path(str(script), run_name='__main__')
'''
            environment = {**os.environ, 'GITHUB_OUTPUT': str(output),
                           'GITHUB_ENV': str(Path(temporary) / 'environment')}
            subprocess.run([sys.executable, '-c', program, str(script)], env=environment,
                           check=True, capture_output=True, text=True)
            self.assertIn('python=', output.read_text())

    def test_python_baseline_and_audit_are_specific_to_target_platforms(self):
        import dependency_audit

        for system, arch, expected in (('linux', 'arm64', '3.15.2'), ('windows', 'x64', '3.15.2'),
                                       ('windows', 'arm64', '3.15.2'), ('macos', 'arm64', '3.15.2')):
            with self.subTest(system=system, arch=arch), \
                    patch.object(build_inputs, 'host_system', return_value=system), \
                    patch.object(build_inputs, 'host_arch', return_value=arch):
                self.assertEqual(build_inputs.tool_versions()['python'], expected)
        releases = [
            {'version': '3.11.9', 'stable': True, 'files': [{'platform': 'win32', 'arch': 'x64'}]},
            {'version': '3.11.18', 'stable': True, 'files': [
                {'platform': 'linux', 'arch': 'x64'}, {'platform': 'linux', 'arch': 'arm64'}]},
            {'version': '3.15.2', 'stable': True, 'files': [
                {'platform': 'linux', 'arch': 'x64'}, {'platform': 'linux', 'arch': 'arm64'}]},
        ]
        with patch.object(dependency_audit, 'metadata', return_value=releases):
            self.assertEqual(dependency_audit.tool_candidate('python_windows', '3.11.9')[0], '3.11.9')
            self.assertEqual(dependency_audit.tool_candidate('python', '3.11.18')[0], '3.15.2')
        with patch.dict(os.environ, {'COR_USE_BOOTSTRAP_TAILNET': 'true'}):
            self.assertEqual(build_inputs.tool_versions()['tailscale'], build_inputs.approved()['tools']['tailscale_bootstrap'])

    def test_java_audit_follows_the_latest_supported_lts(self):
        import dependency_audit

        with patch.object(dependency_audit, 'metadata', side_effect=[
                {'available_lts_releases': [8, 11, 17, 21, 25]},
                [{'version': {'semver': '25.0.5+8.0.LTS', 'build': 8}}],
        ]) as fetch:
            self.assertEqual(dependency_audit.tool_candidate('java', '17.0.21+9')[0], '25.0.5+8')
        self.assertIn('/latest/25/', fetch.call_args.args[0])

    def test_android_image_audit_checks_revisions_and_ignores_preview_and_other_variants(self):
        import dependency_audit

        content = b'''<sdk>
<remotePackage path="system-images;android-35;google_apis;x86_64"><revision><major>99</major></revision></remotePackage>
<remotePackage path="system-images;android-36;google_apis;x86_64"><revision><major>8</major></revision></remotePackage>
<remotePackage path="system-images;android-37;google_apis;x86_64"><revision><major>1</major></revision><channelRef ref="channel-1"/></remotePackage>
<remotePackage path="system-images;android-38;google_apis;arm64-v8a"><revision><major>1</major></revision></remotePackage>
<remotePackage path="system-images;android-36-ext19;google_apis;x86_64"><revision><major>9</major></revision></remotePackage>
</sdk>'''
        with patch.object(dependency_audit, 'metadata', return_value=content):
            self.assertEqual(dependency_audit.android_candidate('system-image', '36-r7')[0], '36-r8')
        with patch.object(dependency_audit, 'metadata', return_value=content.replace(b'channel-1', b'channel-0')):
            self.assertEqual(dependency_audit.android_candidate('system-image', '36-r7')[0], '37-r1')

    def test_emulator_rejects_revisions_that_differ_from_the_cache_identity(self):
        versions = build_inputs.android_versions()
        with tempfile.TemporaryDirectory() as temporary:
            sdk = Path(temporary)
            roots = (sdk/'emulator', sdk/versions['system_image'].replace(';', '/'))
            for root, revision in zip(roots, (versions['emulator'], versions['system_image_revision'])):
                root.mkdir(parents=True)
                (root/'source.properties').write_text('Pkg.Revision = ' + revision + '\n')
            android_build.verify_emulator_versions(sdk)
            for root in roots:
                properties = root/'source.properties'
                original = properties.read_text()
                for revision in ('999', 'preview', original.split('=')[1].strip() + '0'):
                    properties.write_text('Pkg.Revision = ' + revision + '\n')
                    with self.assertRaisesRegex(ValueError, 'revision differs'):
                        android_build.verify_emulator_versions(sdk)
                properties.write_text(original)

    def test_android_preparation_uses_the_approved_sdk_and_skips_unused_package_tools(self):
        versions = build_inputs.android_versions()
        for package_only in (False, True):
            with self.subTest(package_only=package_only), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                sdk = root / 'sdk'
                output = root / 'environment'
                with patch.dict(os.environ, {'ANDROID_HOME': str(sdk), 'RUNNER_TEMP': str(root),
                                             'GITHUB_ENV': str(output)}), \
                        patch.object(android_build, 'sdkmanager_install') as install:
                    android_build.prepare_sdk(package_only=package_only)
                commands = [list(map(str, call.args[0])) for call in install.call_args_list]
                self.assertEqual(commands[0], [str(sdk / 'cmdline-tools/latest/bin/sdkmanager'),
                                               'cmdline-tools;' + versions['cmdline_tools']])
                self.assertEqual(commands[1][:3], [
                    str(sdk / 'cmdline-tools' / versions['cmdline_tools'] / 'bin/sdkmanager'),
                    'platform-tools', 'build-tools;' + versions['build_tools'],
                ])
                if package_only:
                    self.assertEqual(len(commands[1]), 3)
                    self.assertFalse(output.exists())
                else:
                    self.assertEqual(commands[1][3:], ['platforms;android-' + versions['platform'],
                                                       'ndk;' + versions['ndk']])
                    self.assertEqual(output.read_text(), 'ANDROID_NDK_ROOT=' + str(sdk / 'ndk' / versions['ndk']) + '\n')

    def test_packaging_reuses_ci_tool_metadata_without_querying_new_versions(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            path = directory / 'inputs.json'
            path.write_text(json.dumps({'commit': 'current', 'upstream': {'nsis': {'sha256': 'a'*64}},
                                        'warnings': []}))
            before = path.read_bytes()
            with patch.object(build_inputs, 'current_commit', return_value='current'), \
                    patch.object(build_inputs, 'read_json', side_effect=AssertionError('unexpected tool upgrade')), \
                    patch.object(build_inputs, 'report'):
                build_inputs.packaging(directory)
            self.assertEqual(path.read_bytes(), before)

    def test_go_audit_checks_exact_modules_and_does_not_execute_updates(self):
        import dependency_audit

        queries=[]
        def fetch(url, raw=False):
            queries.append(url)
            if '/offline/' in url:
                raise urllib.error.URLError('offline')
            if raw:
                return b'module example.org/Module\n'
            return {'Version': 'v1.3.0' if '!module' in url else 'v0.0.0-20260112195520-a5071408f32f'}
        entries = [('go', 'go4.org/netipx', 'v0.0.0-20260823151212-3075585bcbeb', dependency_audit.go_candidate),
                   ('go', 'example.org/Module', 'v1.2.0', dependency_audit.go_candidate),
                   ('go', 'example.org/offline', 'v2.0.0', dependency_audit.go_candidate)]
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(dependency_audit, 'inventory', return_value=entries), \
                patch.object(dependency_audit, 'metadata', side_effect=fetch), \
                patch.object(build_inputs.subprocess, 'run', side_effect=AssertionError('unexpected update')):
            dependency_audit.audit(Path(temporary))
            records = json.loads((Path(temporary) / 'dependency-audit.json').read_text())['results']
        self.assertEqual([entry['status'] for entry in records], ['current', 'review', 'unavailable'])
        self.assertIn('https://proxy.golang.org/go4.org/netipx/@latest', queries)
        self.assertFalse(any(url.endswith('/go4.org/@latest') for url in queries))
        self.assertGreater(build_inputs.go_version('v1.0.0'),build_inputs.go_version('v1.0.0-rc.1'))
        self.assertGreater(build_inputs.go_version('v1.0.0-rc.10'),build_inputs.go_version('v1.0.0-rc.2'))

    def test_macos_python_mirror_uses_the_installed_framework_instead_of_its_external_aliases(self):
        import runtime_cache

        with patch.object(runtime_cache, 'host_system', return_value='macos'), \
                patch.object(runtime_cache, 'host_arch', return_value='arm64'), \
                patch.object(runtime_cache, 'tool_versions', return_value={'python': '3.11.9', 'tailscale': '1.104.5'}), \
                patch.dict(os.environ, {'RUNNER_TOOL_CACHE': '/runner/toolcache'}):
            self.assertEqual(runtime_cache.roots('bootstrap'), [
                Path('/Library/Frameworks/Python.framework/Versions/3.11'),
                Path('/runner/toolcache/tailscale/1.104.5')])

    def test_runtime_snapshot_preserves_internal_links_and_rejects_parent_escapes(self):
        import runtime_cache

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            tool = root / 'tool'
            tool.mkdir()
            (tool / 'compiler').write_bytes(b'compiler')
            try:
                (tool / 'alias').symlink_to('compiler')
            except OSError:
                self.skipTest('Symbolic links are unavailable')
            cache = root / 'cache'
            runtime_cache.capture('rust', cache, [tool])
            tool.rename(root / 'original')
            runtime_cache.hydrate('rust', cache, [tool])
            self.assertTrue((tool / 'alias').is_symlink())
            self.assertEqual((tool / 'alias').read_bytes(), b'compiler')
            (tool / 'escape').symlink_to('../outside')
            with self.assertRaisesRegex(ValueError, 'escapes'):
                runtime_cache.capture('rust', cache, [tool])

    def test_runtime_restore_rejects_an_inner_link_outside_payload_before_publish(self):
        import runtime_cache

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cache = root / 'cache'
            cache.mkdir()
            destination = root / 'tool'
            archive = cache / '0.tar'
            with tarfile.open(archive, 'w') as output:
                link = tarfile.TarInfo('payload/escape')
                link.type = tarfile.SYMTYPE
                link.linkname = '../outside'
                output.addfile(link)
            (cache / 'manifest.json').write_text(json.dumps({'schema': 1, 'kind': 'rust', 'roots': [
                {'index': 0, 'root': str(destination), 'size': archive.stat().st_size,
                 'sha256': build.sha256(archive)}]}))
            with self.assertRaisesRegex(ValueError, 'escapes'):
                runtime_cache.hydrate('rust', cache, [destination])
            self.assertFalse(destination.exists())

    def test_java_cache_restores_the_official_archive_to_a_complete_tool_directory(self):
        import runtime_cache

        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            archive = directory / 'jdk.tar.gz'
            with tarfile.open(archive, 'w:gz') as output:
                executable = tarfile.TarInfo('jdk-17/bin/java')
                executable.size = len(b'approved runtime')
                executable.mode = 0o755
                output.addfile(executable, io.BytesIO(b'approved runtime'))
            destination = directory / 'toolcache/version'
            record = {'url': 'https://github.com/adoptium/example.tar.gz', 'sha256': build.sha256(archive)}
            with patch.object(runtime_cache, 'java_archive', return_value=record), \
                    patch.object(runtime_cache, 'host_arch', return_value='x64'):
                runtime_cache.hydrate('java', directory, [destination])
            self.assertEqual((destination / 'x64/bin/java').read_bytes(), b'approved runtime')
            self.assertTrue((destination / 'x64.complete').is_file())
            config = {'kind': 'archives', 'runtime': 'java', 'paths': str(directory)}
            self.assertEqual(local_cache.cache_paths(config), [archive])
            with patch.object(runtime_cache, 'java_archive', return_value=record), \
                    patch.object(runtime_cache, 'download') as download:
                runtime_cache.capture('java', directory, [destination])
            download.assert_called_once_with(record['url'], archive, digest=record['sha256'])

    def test_java_cache_rejects_invalid_archive_checksums_before_publishing(self):
        import runtime_cache

        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / 'jdk.tar.gz').write_bytes(b'invalid archive')
            destination = directory / 'toolcache/version'
            with patch.object(runtime_cache, 'java_archive', return_value={'sha256': '0' * 64}):
                with self.assertRaisesRegex(ValueError, 'checksum'):
                    runtime_cache.hydrate('java', directory, [destination])
            self.assertFalse(destination.exists())

    def test_cache_upload_preserves_early_http_rejections(self):
        config = {'url': 'http://100.64.0.1:9080', 'token': ''}
        for status in (409, 413, 507):
            for failure in (BrokenPipeError, ConnectionResetError):
                with self.subTest(status=status, failure=failure.__name__):
                    connection = MagicMock()
                    connection.request.side_effect = failure('body upload interrupted')
                    connection.getresponse.return_value.status = status
                    with patch.object(local_cache.http.client, 'HTTPConnection', return_value=connection):
                        with local_cache.response(config, '/v1/blob', method='PUT', file=io.BytesIO(b'archive')) as reply:
                            self.assertEqual(reply.status, status)
                    connection.close.assert_called_once()
                    connection.getresponse.return_value.close.assert_called_once()

    def test_cache_upload_closes_connection_when_no_rejection_can_be_read(self):
        connection = MagicMock()
        failure = BrokenPipeError('body upload interrupted')
        connection.request.side_effect = failure
        connection.getresponse.side_effect = local_cache.http.client.RemoteDisconnected('no response')
        config = {'url': 'http://100.64.0.1:9080', 'token': ''}
        with patch.object(local_cache.http.client, 'HTTPConnection', return_value=connection):
            with self.assertRaises(BrokenPipeError) as raised:
                local_cache.request(config, '/v1/blob', method='PUT', file=io.BytesIO(b'archive'))
        self.assertIs(raised.exception, failure)
        connection.close.assert_called_once()

    def test_upstream_import_sends_verified_urls_and_logical_destinations_only(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.dict(os.environ, {'RUNNER_TEMP': temporary, 'GITHUB_ACTIONS': 'true'}):
            source = Path(temporary) / 'python.tar.gz'
            source.write_bytes(b'verified upstream')
            build.record_download('https://github.com/example/tool/releases/download/v1/tool.tar.gz',
                                  source, build.sha256(source))
            config = {'kind': 'archives', 'group': 'tools/linux-python-x64', 'key': 'approved',
                      'server_capabilities': ['upstream-fetch-v1']}
            sequence = int(os.environ.get('GITHUB_RUN_ID', '0')) * 1000 + min(int(os.environ.get('GITHUB_RUN_ATTEMPT', '0')), 999)
            reply = io.BytesIO(json.dumps({'schema': 1, 'group': config['group'], 'key': config['key'],
                                           'sequence': sequence, 'status': 'queued'}).encode())
            reply.status = 202
            with patch.object(local_cache, 'response', return_value=nullcontext(reply)) as request, \
                    patch.object(local_cache, 'cache_report'):
                self.assertTrue(local_cache.upstream_import(config, [source]))
            payload = json.loads(request.call_args.kwargs['file'])
            self.assertEqual(request.call_args.args[1], '/v1/import')
            self.assertEqual(payload['files'][0]['path'], '0')
            self.assertEqual(payload['files'][0]['sha256'], build.sha256(source))
            self.assertNotIn(temporary, json.dumps(payload))
            self.assertTrue(config['import_queued'])
            config['server_capabilities'] = []
            with patch.object(local_cache, 'response') as request:
                self.assertFalse(local_cache.upstream_import(config, [source]))
                request.assert_not_called()

    def test_release_metadata_network_fallback_respects_explicit_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)/'core.json'
            cached = {'tag_name':'v1.2.3','assets':[]}
            path.write_text(json.dumps({'release':cached}))
            with patch.object(build_inputs, 'component', return_value=None), \
                 patch.object(build_inputs, 'approved', return_value={'upstream': {}}), \
                 patch.object(build_inputs, 'read_json', side_effect=urllib.error.URLError('network unavailable')):
                self.assertEqual(build_inputs.resource_release('mihomo', path), cached)
                with self.assertRaisesRegex(RuntimeError, 'no verified'):
                    build_inputs.resource_release('mihomo', path, version='v9.0.0')

    def test_network_fallback_can_migrate_legacy_resource_receipts(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)/'core.json'
            value = {'version':'v1.2.3','source':'https://github.com/MetaCubeX/mihomo/releases/download/v1.2.3/mihomo-linux-arm64-v1.2.3.gz',
                     'archive_sha256':'a'*64,'exe_sha256':'b'*64}
            path.write_text(json.dumps(value))
            with patch.object(build_inputs, 'component', return_value=None), \
                 patch.object(build_inputs, 'approved', return_value={'upstream': {}}), \
                 patch.object(build_inputs, 'read_json', side_effect=urllib.error.URLError('offline')):
                release = build_inputs.resource_release('mihomo', path)
                self.assertEqual(release['tag_name'], 'v1.2.3')
                self.assertEqual(release['assets'][0]['digest'], 'sha256:'+'a'*64)
                value['archive_sha256'] = 'invalid'; path.write_text(json.dumps(value))
                with self.assertRaisesRegex(RuntimeError, 'no verified'):
                    build_inputs.resource_release('mihomo', path)
                records = [{'name':local,'sha256':'c'*64,'size':1,'source':'https://example.test/'+remote}
                           for remote, local in build_inputs.GEO.items()]
                path.write_text(json.dumps({'version':'latest | 2026-10-16','files':records}))
                release = build_inputs.resource_release('geo', path)
                self.assertEqual({asset['name'] for asset in release['assets']}, set(build_inputs.GEO))
                path.write_text(json.dumps({'version':'latest','files':records[:1]}))
                with self.assertRaisesRegex(RuntimeError, 'no verified'):
                    build_inputs.resource_release('geo', path)

    def test_cache_login_restores_mac_dns_including_automatic_search_domains(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            original = {'dnsservers':['9.9.9.9','2620:fe::fe'], 'searchdomains':[]}
            state = copy.deepcopy(original)
            def get(command, **kwargs):
                values = state[command[1][4:]]
                return '\n'.join(values) if values else "There aren't any Search Domains set on Ethernet."
            def set_value(command, **kwargs):
                values = command[5:]
                state[command[3][4:]] = [] if values == ['Empty'] else values
            with patch.object(local_cache.sys, 'platform', 'darwin'), \
                 patch.object(local_cache, 'Path', side_effect=lambda path: root/path), \
                 patch.object(local_cache.subprocess, 'check_output', side_effect=get), \
                 patch.object(local_cache.subprocess, 'run', side_effect=set_value):
                local_cache.preserve_network()
                state.update(dnsservers=['100.100.100.100'], searchdomains=['tailnet.example'])
                local_cache.preserve_network(restore=True)
            self.assertEqual(state, original)

    def test_nsis_version_query_failure_uses_only_verified_cached_compiler(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            compiler = root/'tools/nsis/current/makensis.exe'
            compiler.parent.mkdir(parents=True)
            compiler.write_bytes(b'verified compiler')
            (root/'tools/nsis/tool.json').write_text(json.dumps({'compiler_sha256':build.sha256(compiler)}))
            with patch.object(build, 'ROOT', root), patch.object(build_inputs, 'component', return_value=None), \
                 patch.object(build_inputs, 'approved', side_effect=urllib.error.URLError('unavailable')):
                self.assertEqual(build.nsis(), compiler)
                compiler.write_bytes(b'corrupt')
                with self.assertRaises(urllib.error.URLError): build.nsis()

    def test_nsis_completed_install_survives_a_locked_temporary_download(self):
        for persistent in (False, True):
            with self.subTest(persistent=persistent), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                installer = root/'tools/nsis-installer.exe'
                compiler = root/'tools/nsis/current/makensis.exe'
                selected = {'version':'test', 'url':'https://example.test/nsis.exe', 'sha256':'1'*64}
                original_unlink = Path.unlink
                attempts = []
                def unlink(path, *args, **kwargs):
                    if path == installer:
                        attempts.append(path)
                        if persistent or len(attempts) == 1:
                            raise PermissionError('installer still open')
                    return original_unlink(path, *args, **kwargs)
                def install(*args, **kwargs):
                    candidate = root/'tools/nsis-stage/makensis.exe'
                    candidate.parent.mkdir(parents=True)
                    candidate.write_bytes(b'installed compiler')
                def download(*args, **kwargs):
                    installer.parent.mkdir(parents=True, exist_ok=True)
                    installer.write_bytes(b'installer')
                with patch.object(build, 'ROOT', root), patch.object(build_inputs, 'component', return_value=selected), \
                     patch.object(build, 'download', side_effect=download), \
                     patch.object(build.subprocess, 'run', side_effect=install), \
                     patch.object(Path, 'unlink', unlink), patch.object(build.time, 'sleep'):
                    self.assertEqual(build.nsis(), compiler)
                    self.assertEqual(build.nsis(), compiler)
                self.assertEqual(len(attempts), 3 if persistent else 2)
                self.assertEqual(compiler.read_bytes(), b'installed compiler')
                receipt = json.loads((root/'tools/nsis/tool.json').read_text())
                self.assertEqual(receipt['compiler_sha256'], build.sha256(compiler))

    def test_dependency_cli_and_readers_share_the_configured_snapshot_directory(self):
        directory = Path(os.environ['BUILD_INPUTS_DIR'])
        (directory/'inputs.json').write_text(json.dumps({'marker':'selected snapshot'}))
        self.assertEqual(build_inputs.manifest(), {'marker':'selected snapshot'})
        with patch.object(build_inputs, 'apply') as apply, \
             patch('sys.argv', ['build_inputs.py', 'apply']):
            build_inputs.main()
            apply.assert_called_once_with(directory)

    def test_export_rejects_missing_stale_or_modified_dependency_snapshots(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            snapshot = Path(os.environ['BUILD_INPUTS_DIR'])/'inputs.json'
            with patch.object(build_inputs, 'ROOT', root), \
                 patch.object(build_inputs, 'current_commit', return_value='current'):
                with self.assertRaisesRegex(ValueError, 'requires the current'):
                    build_inputs.verified_manifest()
                files = {}
                for name in build_inputs.FILES:
                    path = root/name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text('resolved '+name)
                    files[name] = build.sha256(path)
                value = {'schema':1, 'commit':'old', 'files':files}
                snapshot.write_text(json.dumps(value))
                with self.assertRaisesRegex(ValueError, 'requires the current'):
                    build_inputs.verified_manifest()
                value['commit'] = 'current'
                snapshot.write_text(json.dumps(value))
                self.assertEqual(build_inputs.verified_manifest(), value)
                (root/'Cargo.lock').write_text('changed after resolving')
                with self.assertRaisesRegex(ValueError, 'changed after resolution: Cargo.lock'):
                    build_inputs.verified_manifest()

    def test_android_dependency_retry_is_bounded_and_keeps_compiler_errors(self):
        def process(status, output):
            child = MagicMock()
            child.__enter__.return_value = child
            child.stdout = output.splitlines(keepends=True)
            child.wait.return_value = status
            return child
        command = ['cargo', 'clippy', '--locked']
        unavailable = 'fatal: requested URL returned error: 503\n'
        with patch.object(android_build.subprocess, 'Popen', side_effect=[process(101, unavailable), process(0, 'Finished\n')]) as spawn, \
             patch.object(android_build.time, 'sleep') as delay:
            android_build.cargo_build(command)
            self.assertEqual(spawn.call_count, 2)
            delay.assert_called_once_with(15)
        with patch.object(android_build.subprocess, 'Popen', side_effect=[process(101, unavailable) for _ in range(3)]) as spawn, \
             patch.object(android_build.time, 'sleep'):
            with self.assertRaises(subprocess.CalledProcessError): android_build.cargo_build(command)
            self.assertEqual(spawn.call_count, 3)
        with patch.object(android_build.subprocess, 'Popen', return_value=process(101, 'error[E0308]: mismatched types\n')) as spawn, \
             patch.object(android_build.time, 'sleep') as delay:
            with self.assertRaises(subprocess.CalledProcessError): android_build.cargo_build(command)
            spawn.assert_called_once()
            delay.assert_not_called()

    def test_android_sdk_download_retry_is_bounded_and_uses_fresh_user_caches(self):
        def process(status, output):
            child = MagicMock()
            child.__enter__.return_value = child
            child.stdout = output.splitlines(keepends=True)
            child.wait.return_value = status
            return child

        command = ['sdkmanager', 'system-images;android-35;google_apis;x86_64']
        corrupt = 'Error reading Zip content from a SeekableByteChannel\n'
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with patch.object(android_build.subprocess, 'Popen',
                              side_effect=[process(1, corrupt), process(0, 'Installed\n')]) as spawn, \
                 patch.object(android_build.time, 'sleep') as delay:
                android_build.sdkmanager_install(command, root)
                self.assertEqual(spawn.call_count, 2)
                self.assertNotEqual(spawn.call_args_list[0].kwargs['env']['ANDROID_USER_HOME'],
                                    spawn.call_args_list[1].kwargs['env']['ANDROID_USER_HOME'])
                delay.assert_called_once_with(15)
            with patch.object(android_build.subprocess, 'Popen',
                              side_effect=[process(1, corrupt) for _ in range(3)]) as spawn, \
                 patch.object(android_build.time, 'sleep'):
                with self.assertRaises(subprocess.CalledProcessError):
                    android_build.sdkmanager_install(command, root)
                self.assertEqual(spawn.call_count, 3)
            with patch.object(android_build.subprocess, 'Popen',
                              return_value=process(1, 'Package does not exist\n')) as spawn, \
                 patch.object(android_build.time, 'sleep') as delay:
                with self.assertRaises(subprocess.CalledProcessError):
                    android_build.sdkmanager_install(command, root)
                spawn.assert_called_once()
                delay.assert_not_called()

    def test_cache_budget_evicts_old_lockfiles_then_least_recently_used_entries(self):
        def entry(number, key, size, ref='refs/heads/main'):
            return {'id': number, 'key': key, 'size_in_bytes': size, 'ref': ref,
                    'last_accessed_at': f'2026-10-16T10:00:{number:02d}Z',
                    'created_at': f'2026-10-16T09:00:{number:02d}Z'}
        old = entry(1, 'native-release-linux-check-Linux-x64-aaaaaaaa-bbbbbbbb', 30)
        new = entry(2, 'native-release-linux-check-Linux-x64-aaaaaaaa-cccccccc', 30)
        go = entry(3, 'source-go-version-modules', 40)
        gradle = entry(4, 'gradle-content-unique-blob', 40)
        # Stale revisions are removed even below budget; current dependencies
        # are otherwise preserved. An 80-byte budget also evicts the oldest.
        self.assertEqual([c['id'] for c in storage.eviction_plan([old, new, go, gradle], 200)], [1])
        self.assertEqual([c['id'] for c in storage.eviction_plan([old, new, go, gradle], 80)], [1, 2])
        other_branch = entry(5, old['key'], 30, 'refs/pull/18/merge')
        self.assertEqual(storage.eviction_plan([new, other_branch], 100), [])
        compiler_old = entry(6, 'macos-compiler-arm64-' + 'a'*16 + '-' + 'b'*64, 20)
        compiler_new = entry(7, 'macos-compiler-arm64-' + 'a'*16 + '-' + 'c'*64, 20)
        self.assertEqual(storage.eviction_plan([compiler_old, compiler_new], 100), [compiler_new,compiler_old])
        legacy=entry(12,'android-go-Linux-X64-old-modules',20)
        self.assertEqual(storage.eviction_plan([legacy],100),[legacy])
        self.assertEqual(storage.eviction_plan([gradle], 10), [gradle])
        desktop=entry(8,'native-release-v2-macos-15-intel-check-Linux-x64-aaaaaaaa-bbbbbbbb',60)
        linux=entry(9,'native-release-v2-ubuntu-22.04-check-Linux-x64-aaaaaaaa-bbbbbbbb',60)
        self.assertEqual([c['id'] for c in storage.eviction_plan([desktop,linux],60)],[9])
        source_old=entry(10,'source-cargo-'+'a'*64+'-1-1',10)
        source_new=entry(11,'source-cargo-'+'b'*64+'-2-1',10)
        self.assertEqual([c['id'] for c in storage.eviction_plan([source_old,source_new],100)],[10])
        for older,newer in (
                ('linux-python-ubuntu20-v1-x64-'+'a'*64, 'linux-python-ubuntu20-v1-x64-'+'b'*64),
                ('linux-cloud-ubuntu20-v1-arm64-'+'a'*64, 'linux-cloud-ubuntu20-v1-arm64-'+'b'*64),
                ('linux-apt-ubuntu20-v1-x64-100-1', 'linux-apt-ubuntu20-v1-x64-101-1'),
                ('android-go-Linux-X64-1.26.0-'+'a'*64,'android-go-Linux-X64-1.27.0-'+'b'*64),
                ('skia-source-Linux-X64-0.153.3-'+'a'*64,'skia-source-Linux-X64-0.154.0-'+'b'*64),
                ('resources-macos-x64-'+'a'*64+'-2026-10-16','resources-macos-x64-'+'b'*64+'-'+'c'*64),
                ('tools-nsis-'+'a'*64,'tools-nsis-'+'b'*64),
                ('action-setup-tailscale/1.94.2/Linux-amd64','action-setup-tailscale/1.104.1/Linux-amd64')):
            self.assertEqual(storage.lineage(older),storage.lineage(newer))
        with self.assertRaises(ValueError): storage.eviction_plan([], 0)

    def test_cache_cleanup_rechecks_size_and_fork_report_never_deletes(self):
        entries = [{'id': 1, 'key': 'tool-one', 'ref': 'refs/heads/main', 'size_in_bytes': 30,
                    'last_accessed_at': '2026-10-15', 'created_at': '2026-10-15'},
                   {'id': 2, 'key': 'tool-two', 'ref': 'refs/heads/main', 'size_in_bytes': 50,
                    'last_accessed_at': '2026-10-16', 'created_at': '2026-10-16'}]
        writes = []
        def client(path, method='GET'):
            if method == 'DELETE':
                writes.append(path)
                entries[:] = [e for e in entries if not path.endswith('/' + str(e['id']))]
                return
            return {'actions_caches': copy.deepcopy(entries)}
        with patch.dict(build.os.environ, {}, clear=True), patch.object(storage.time, 'sleep'):
            report = storage.prune(client, 50, read_only=True)
            self.assertEqual(writes, [])
            self.assertEqual(report['after'], 80)
            report = storage.prune(client, 50)
            self.assertEqual(writes, ['/actions/caches/1'])
            self.assertEqual(report['after'], 50)
            self.assertEqual(report['removed'], 1)

    def test_cache_inventory_reads_every_page(self):
        calls = []
        def client(path):
            calls.append(path)
            return {'actions_caches': list(range(100)) if path.endswith('&page=1') else [100]}
        self.assertEqual(len(storage.caches(client)), 101)
        self.assertEqual(calls, ['/actions/caches?per_page=100&page=1', '/actions/caches?per_page=100&page=2'])

    def test_android_binding_cache_requires_the_pinned_tool_metadata(self):
        import re
        version = re.search(r'golang.org/x/mobile\s+(v\S+)', (android_build.ANDROID/'core/go.mod').read_text())[1]
        expected = '/tools/gomobile: go1.26.0\npath\tgolang.org/x/mobile/cmd/gomobile\nmod\tgolang.org/x/mobile\t' + version + '\thash\n'
        with patch.object(android_build.shutil, 'which', return_value='/tools/gomobile'), \
             patch.object(android_build.subprocess, 'check_output', return_value='go1.26.0\n'), \
             patch.object(android_build.subprocess, 'run') as inspect, patch.object(android_build, 'run') as install:
            inspect.return_value = subprocess.CompletedProcess([], 0, expected, '')
            android_build.ensure_binding_tool('gomobile')
            install.assert_not_called()
            inspect.return_value = subprocess.CompletedProcess([], 0, expected.replace(version, 'v0.0.0-outdated'), '')
            android_build.ensure_binding_tool('gomobile')
            install.assert_called_once_with(['go', 'install', 'golang.org/x/mobile/cmd/gomobile@' + version])
            install.reset_mock()
            inspect.return_value = subprocess.CompletedProcess([], 0, expected.replace('go1.26.0', 'go1.25.0'), '')
            android_build.ensure_binding_tool('gomobile')
            install.assert_called_once_with(['go', 'install', 'golang.org/x/mobile/cmd/gomobile@' + version])

    def test_android_identity_accepts_old_and_current_aapt2_but_rejects_wrong_hosts(self):
        identity="package: name='org.clashofrust.android' versionCode='1000281' versionName='0.5.12-dev.1'\n"
        for name in ('sdkVersion','minSdkVersion'):
            output=identity+name+":'26'\ntargetSdkVersion:'36'\n"
            self.assertEqual(android_build.apk_identity(output,'0.5.12-dev.1'),1000281)
            for invalid in (output.replace(":'26'",":'25'"),output.replace(":'36'",":'35'"),
                            output.replace('org.clashofrust.android','another.app'),output.replace('0.5.12-dev.1','0.5.11')):
                with self.assertRaises(ValueError): android_build.apk_identity(invalid,'0.5.12-dev.1')

    def test_shared_core_has_no_desktop_toolkit_or_host_dependency(self):
        core = tomllib.loads((build.core_directory()/'Cargo.toml').read_text())
        # Mobile hosts must be able to use all shared policies without linking
        # the desktop application, platform authorization code or GUI toolkits.
        dependencies = set(core.get('dependencies', {}))
        for target in core.get('target', {}).values():
            dependencies.update(target.get('dependencies', {}))
        forbidden = {'clash-of-rust', 'clash-client-ui', 'slint', 'iced', 'iced_graphics', 'cosmic-text',
                     'tray-icon', 'windows', 'windows-sys', 'winreg', 'zbus',
                     'objc2', 'objc2-app-kit', 'objc2-foundation', 'block2'}
        self.assertFalse(dependencies & forbidden)
        for module in (build.core_directory()/'src').rglob('*.rs'):
            text = module.read_text()
            self.assertNotIn('crate::platform', text, str(module))
            self.assertNotIn('crate::engine', text, str(module))

    def test_workspace_and_legacy_version_and_source_paths(self):
        self.assertEqual(build.manifest_version('[package]\nversion="0.4.12"\n'), '0.4.12')
        self.assertEqual(build.manifest_version('[workspace.package]\nversion="0.5.11-dev.1"\n'), '0.5.11-dev.1')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root/'Cargo.toml').write_text('[package]\nversion="0.4.12"\n')
            self.assertEqual(build.package_version(root), '0.4.12')
            self.assertEqual(build.desktop_directory(root), root)
            self.assertEqual(build.core_directory(root), root)
            (root/'apps/desktop').mkdir(parents=True)
            (root/'crates/client-core').mkdir(parents=True)
            (root/'apps/desktop/Cargo.toml').touch()
            (root/'crates/client-core/Cargo.toml').touch()
            (root/'Cargo.toml').write_text('[workspace.package]\nversion="0.5.11-dev.1"\n')
            self.assertEqual(build.package_version(root), '0.5.11-dev.1')
            self.assertEqual(build.desktop_directory(root), root/'apps/desktop')
            self.assertEqual(build.core_directory(root), root/'crates/client-core')

    def test_release_job_protocol_matches_workflow_display_names(self):
        workflow = (build.ROOT/'.github/workflows/release.yml').read_text()
        for name in release.PACKAGE_JOBS - set(release.PACKAGES):
            if name.startswith('Ubuntu 20.04-'):
                self.assertIn('    name: Ubuntu 20.04-${{ matrix.arch }} installed GUI and TUN\n', workflow)
                arch = name.removeprefix('Ubuntu 20.04-').split(' ', 1)[0]
                self.assertIn('            arch: '+arch+'\n', workflow)
            else:
                self.assertIn('    name: '+name+'\n', workflow)

    def test_failed_cargo_json_preserves_human_diagnostics(self):
        message = json.dumps({'reason': 'compiler-message', 'message': {'rendered': 'error[E0308]: expected borrowed window'}})+'\n'
        with tempfile.TemporaryDirectory() as directory, patch('build_support.subprocess.run') as runner, patch('sys.stdout', new_callable=io.StringIO) as output:
            runner.return_value = subprocess.CompletedProcess(['cargo'], 101, stdout=message)
            log = Path(directory)/'build.jsonl'
            with self.assertRaises(subprocess.CalledProcessError):
                build.cargo_messages(['cargo', 'test'], log=log)
            self.assertEqual(log.read_text(), message)
            self.assertIn('error[E0308]', output.getvalue())

    def test_asset_verification_checks_each_pe_independently(self):
        spec = importlib.util.spec_from_file_location('prepare_assets', build.ROOT/'scripts/prepare-assets.py')
        assets = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(assets)

        def executable(path, magic, group=True, complete=True):
            data = bytearray(0x400)
            pe, root = 0x80, 0x200
            struct.pack_into('<I', data, 0x3c, pe)
            data[pe:pe + 4] = b'PE\0\0'
            struct.pack_into('<H', data, pe + 6, 1)
            optional = pe + 24
            size = 240 if magic == 0x20b else 224
            struct.pack_into('<H', data, pe + 20, size)
            struct.pack_into('<H', data, optional, magic)
            directory = optional + (112 if magic == 0x20b else 96)
            struct.pack_into('<I', data, directory + 16, 0x1000)
            struct.pack_into('<IIII', data, optional + size + 8, 0x200, 0x1000, 0x200, root)
            directories = [
                (0, [(3, 0x80000020)] + ([(14, 0x80000040)] if group else [])),
                (0x20, [(1, 0x80000060), (2, 0x80000080)]),
                (0x40, [(1, 0x800000a0)]),
                (0x60, [(1033, 0xc0)]), (0x80, [(1033, 0xd0)]), (0xa0, [(1033, 0xe0)]),
            ]
            for offset, entries in directories:
                struct.pack_into('<HH', data, root + offset + 12, 0, len(entries))
                for index, (name, target) in enumerate(entries):
                    struct.pack_into('<II', data, root + offset + 16 + index * 8, name, target)
            for offset, rva, length in ((0xc0, 0x1100, 2), (0xd0, 0x1102, 3), (0xe0, 0x1105, 1)):
                struct.pack_into('<II', data, root + offset, rva, length)
            data[root + 0x100:root + 0x106] = b'abcdeg' if complete else b'abxxxg'
            path.write_bytes(data)

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            icons = root/'resources/icons'
            icons.mkdir(parents=True)
            frames = [(2, 38), (3, 40)]
            ico = struct.pack('<HHH', 0, 1, 2)
            ico += b''.join(struct.pack('<BBBBHHII', 1, 1, 0, 0, 1, 32, size, offset)
                            for size, offset in frames) + b'abcde'
            (icons/'app.ico').write_bytes(ico)
            first, second, bad = root/'app.exe', root/'installer.exe', root/'bad.exe'
            executable(first, 0x20b)
            executable(second, 0x10b)
            with patch.object(assets, 'ROOT', root), patch('sys.stdout', io.StringIO()):
                assets.verify_icons([first, second])
                executable(bad, 0x20b, group=False)
                with self.assertRaises(AssertionError):
                    assets.verify_icons([first, bad])
                executable(bad, 0x10b, complete=False)
                with self.assertRaises(AssertionError):
                    assets.verify_icons([second, bad])

    def test_disk_image_retries_only_busy_errors_and_bounds_attempts(self):
        import macos_package as mac
        busy = subprocess.CompletedProcess([], 1, '', 'hdiutil: create failed - Resource busy\n')
        ok = subprocess.CompletedProcess([], 0, 'created\n', '')
        denied = subprocess.CompletedProcess([], 1, '', 'Permission denied\n')
        with patch.object(mac.time, 'sleep'), patch.object(mac.sys, 'stdout', io.StringIO()), patch.object(mac.sys, 'stderr', io.StringIO()):
            with patch.object(mac.subprocess, 'run', side_effect=[busy, ok]) as execute:
                mac.disk_image(['create', 'path with spaces.dmg'])
                self.assertEqual(execute.call_count, 2)
                self.assertEqual(execute.call_args.args[0][-1], 'path with spaces.dmg')
            with patch.object(mac.subprocess, 'run', return_value=denied) as execute:
                with self.assertRaises(subprocess.CalledProcessError):
                    mac.disk_image(['attach', 'package.dmg'])
                self.assertEqual(execute.call_count, 1)
            with patch.object(mac.subprocess, 'run', return_value=busy) as execute:
                with self.assertRaises(subprocess.CalledProcessError):
                    mac.disk_image(['detach', '/Volumes/test'])
                self.assertEqual(execute.call_count, 5)
            with patch.object(mac.time, 'monotonic', side_effect=[0, 2]), patch.object(mac.subprocess, 'run') as execute:
                with self.assertRaises(subprocess.TimeoutExpired):
                    mac.disk_image(['create', 'package.dmg'], timeout=1)
                execute.assert_not_called()

    def test_defender_update_waits_for_busy_installation_and_refreshes_binary(self):
        busy = subprocess.CompletedProcess([], 2, 'ERROR: hr=0x80070652\n')
        ok = subprocess.CompletedProcess([], 0, 'Signature update finished\n')
        old, new = Path('old/MpCmdRun.exe'), Path('new/MpCmdRun.exe')
        with patch.object(windows, 'defender_command', side_effect=[old, new, new]), \
             patch.object(windows.time, 'monotonic', return_value=0), \
             patch.object(windows.time, 'sleep') as sleep, \
             patch.object(windows.subprocess, 'run', side_effect=[busy, ok]) as execute, \
             patch('sys.stdout', io.StringIO()):
            self.assertEqual(windows.update_signatures(), new)
            sleep.assert_called_once_with(15)
            self.assertEqual([call.args[0] for call in execute.call_args_list], [
                [str(old), '-SignatureUpdate', '-MMPC'],
                [str(new), '-SignatureUpdate', '-MMPC'],
            ])

    def test_defender_update_retries_only_exact_busy_error_and_bounds_waits(self):
        with patch.object(windows, 'defender_command', return_value=Path('MpCmdRun.exe')), \
             patch.object(windows.time, 'monotonic', return_value=0), \
             patch.object(windows.time, 'sleep') as sleep, \
             patch('sys.stdout', io.StringIO()):
            for message in ('ERROR: hr=0x80070005', 'ERROR: hr=0x800706520', ''):
                with self.subTest(message=message), \
                     patch.object(windows.subprocess, 'run', return_value=subprocess.CompletedProcess([], 2, message)) as execute:
                    with self.assertRaises(subprocess.CalledProcessError):
                        windows.update_signatures()
                    execute.assert_called_once()
            sleep.assert_not_called()
            # Some native tools redirect UTF-16 text; the HRESULT must remain recognizable.
            busy = subprocess.CompletedProcess([], 2, '\0'.join('ERROR: 0X80070652'))
            with patch.object(windows.subprocess, 'run', return_value=busy) as execute:
                with self.assertRaises(subprocess.CalledProcessError):
                    windows.update_signatures()
                self.assertEqual(execute.call_count, 5)
                self.assertEqual([call.args[0] for call in sleep.call_args_list], [15, 30, 45, 60])

    def test_defender_update_preserves_total_timeout(self):
        with patch.object(windows, 'defender_command', return_value=Path('MpCmdRun.exe')), \
             patch.object(windows.time, 'monotonic', side_effect=[0, 2]), \
             patch.object(windows.subprocess, 'run') as execute:
            with self.assertRaises(subprocess.TimeoutExpired):
                windows.update_signatures(timeout=1)
            execute.assert_not_called()
        busy = subprocess.CompletedProcess([], 2, 'ERROR: hr=0x80070652')
        with patch.object(windows, 'defender_command', return_value=Path('MpCmdRun.exe')), \
             patch.object(windows.time, 'monotonic', side_effect=[0, 0, 295]), \
             patch.object(windows.time, 'sleep') as sleep, \
             patch.object(windows.subprocess, 'run', return_value=busy) as execute, \
             patch('sys.stdout', io.StringIO()):
            with self.assertRaises(subprocess.TimeoutExpired):
                windows.update_signatures()
            self.assertEqual(execute.call_args.kwargs['timeout'], 300)
            sleep.assert_not_called()

    def test_defender_failed_update_cannot_proceed_to_scan(self):
        with tempfile.TemporaryDirectory() as directory:
            installer = Path(directory)/'installer.exe'
            installer.write_bytes(b'installer fixture')
            with patch.object(windows, 'update_signatures', side_effect=subprocess.CalledProcessError(2, ['update'])), \
                 patch.object(windows.subprocess, 'run') as execute:
                with self.assertRaises(subprocess.CalledProcessError):
                    windows.scan_windows_files([installer])
                execute.assert_not_called()

    def test_defender_scan_blocks_threats_and_changed_files(self):
        with tempfile.TemporaryDirectory() as directory:
            installer = Path(directory)/'installer.exe'
            original = b'installer fixture'
            installer.write_bytes(original)
            versions = {'AMEngineVersion': '1.1.1', 'AMProductVersion': '4.18.1',
                        'AntivirusSignatureVersion': '1.2.3'}
            status = subprocess.CompletedProcess([], 0, json.dumps(versions))
            with patch.object(windows, 'update_signatures', return_value=Path('new/MpCmdRun.exe')), \
                 patch('sys.stdout', io.StringIO()):
                with patch.object(windows.subprocess, 'run', side_effect=[
                    status, subprocess.CalledProcessError(2, ['scan']),
                ]) as execute:
                    with self.assertRaises(subprocess.CalledProcessError):
                        windows.scan_windows_files([installer])
                    self.assertEqual(execute.call_args.args[0][0], str(Path('new/MpCmdRun.exe')))
                    self.assertIn('-DisableRemediation', execute.call_args.args[0])
                    self.assertTrue(execute.call_args.kwargs['check'])
                def changed(*args, **kwargs):
                    if args[0][0] == 'powershell.exe':
                        return status
                    installer.write_bytes(b'changed fixture')
                    return subprocess.CompletedProcess(args[0], 0)
                with patch.object(windows.subprocess, 'run', side_effect=changed):
                    with self.assertRaisesRegex(RuntimeError, 'File changed'):
                        windows.scan_windows_files([installer])

    def test_defender_scan_records_unpacked_executables_and_libraries(self):
        with tempfile.TemporaryDirectory() as directory:
            files = [Path(directory)/name for name in ('setup.exe', 'clash-of-rust.exe', 'wintun.dll')]
            for path in files:
                path.write_bytes(path.name.encode('ascii'))
            versions = {'AMEngineVersion': '1.1.1', 'AMProductVersion': '4.18.1',
                        'AntivirusSignatureVersion': '1.2.3'}
            with patch.object(windows, 'update_signatures', return_value=Path('MpCmdRun.exe')), \
                 patch('sys.stdout', io.StringIO()), \
                 patch.object(windows.subprocess, 'run', side_effect=[
                     subprocess.CompletedProcess([], 0, json.dumps(versions)),
                     *[subprocess.CompletedProcess([], 0) for _ in files],
                 ]) as execute:
                report = windows.scan_windows_files(files)
            self.assertEqual(report['versions'], versions)
            self.assertEqual(report['files'], [
                {'file': path.name, 'sha256': build.sha256(path)} for path in files
            ])
            self.assertEqual(execute.call_count, 4)
            scanned = [Path(call.args[0][call.args[0].index('-File')+1])
                       for call in execute.call_args_list[1:]]
            self.assertEqual(scanned, [path.resolve() for path in files])

    def test_defender_scan_requires_version_evidence_before_scanning(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'program.exe'
            path.write_bytes(b'fixture')
            with patch.object(windows, 'update_signatures', return_value=Path('MpCmdRun.exe')), \
                 patch('sys.stdout', io.StringIO()), \
                 patch.object(windows.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, '{}')) as execute:
                with self.assertRaisesRegex(RuntimeError, 'version information'):
                    windows.scan_windows_files([path])
                self.assertEqual(execute.call_count, 1)

    def test_defender_status_timeout_retries_but_never_skips_the_scan(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'program.exe'
            path.write_bytes(b'fixture')
            versions = {'AMEngineVersion': '1.1.1', 'AMProductVersion': '4.18.1',
                        'AntivirusSignatureVersion': '1.2.3'}
            timeout = subprocess.TimeoutExpired(['powershell.exe'], 120)
            with patch.object(windows, 'update_signatures', return_value=Path('MpCmdRun.exe')), \
                 patch.object(windows.time, 'sleep') as delay, \
                 patch.object(windows.subprocess, 'run', side_effect=[
                     timeout, subprocess.CompletedProcess([], 0, json.dumps(versions)),
                     subprocess.CompletedProcess([], 0),
                 ]) as execute:
                report = windows.scan_windows_files([path])
                self.assertEqual(report['versions'], versions)
                self.assertEqual(execute.call_count, 3)
                self.assertIn('-Scan', execute.call_args.args[0])
                delay.assert_called_once_with(5)
            with patch.object(windows, 'update_signatures', return_value=Path('MpCmdRun.exe')), \
                 patch.object(windows.time, 'sleep'), \
                 patch.object(windows.subprocess, 'run', side_effect=timeout) as execute:
                with self.assertRaises(subprocess.TimeoutExpired):
                    windows.scan_windows_files([path])
                self.assertEqual(execute.call_count, 2)

    def test_windows_package_scans_payload_before_installing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            payload = root/'build/payload'
            (payload/'resources').mkdir(parents=True)
            names = ['clash-of-rust.exe', 'clash-desktop-ui.exe', 'cor.exe',
                     'clash-tun-launcher.exe', 'resources/mihomo.exe', 'resources/wintun.dll']
            for name in [*names, 'resources/manifest.json']:
                (payload/name).write_bytes(b'fixture')
            package, smoke = root/'setup.exe', root/'build/tests/core_integration.exe'
            evidence = {'versions': {}, 'files': []}
            with patch.object(windows, 'os') as platform, \
                 patch.object(windows, 'ROOT', root), \
                 patch.object(windows, 'scan_windows_files', return_value=evidence) as scan, \
                 patch.object(windows, '_verify_installed_package') as verify:
                platform.name = 'nt'
                manifest = {'tests': {'core_integration': 'tests/core_integration.exe'}}
                package.write_bytes(b'installer')
                smoke.parent.mkdir(parents=True)
                smoke.write_bytes(b'test fixture')
                self.assertEqual(windows.package_tests(package, manifest, root/'build'), evidence)
                self.assertEqual(scan.call_args.args[0], [package, *sorted(payload/name for name in names)])
                verify.assert_called_once()
                verify.reset_mock()
                scan.side_effect = subprocess.CalledProcessError(2, ['scan'])
                with self.assertRaises(subprocess.CalledProcessError):
                    windows.package_tests(package, manifest, root/'build')
                verify.assert_not_called()

    def test_linux_helpers_are_distinct_inactive_native_copies(self):
        spec = importlib.util.spec_from_file_location('build_installer',build.ROOT/'scripts/build-installer.py')
        installer = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(installer)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root/'clash-tun-launcher'
            header = bytearray(64)
            header[:6] = b'\x7fELF\x02\x01'
            struct.pack_into('<H',header,18,62)
            source.write_bytes(header)
            stage = root/'stage'
            stage.mkdir()
            installer.stage_linux_helpers(root/'clash-of-rust',stage,'x64')
            for path in (stage/'clash-tun-launcher',stage/'libexec/resolvectl'):
                self.assertEqual(path.read_bytes(),source.read_bytes())
                self.assertFalse(path.samefile(source))
                if __import__('os').name != 'nt':
                    self.assertEqual(path.stat().st_mode & 0o7777,0o755)
            self.assertFalse((stage/'clash-tun-launcher').samefile(stage/'libexec/resolvectl'))

    def test_linux_tun_authorization_requires_an_admin_in_every_session(self):
        import xml.etree.ElementTree as xml
        policy = xml.parse(build.ROOT/'resources/linux/org.clashofrust.tun.policy')
        actions = policy.findall('action')
        self.assertEqual(len(actions), 1)
        action = actions[0]
        for session in ('allow_any', 'allow_inactive', 'allow_active'):
            self.assertEqual(action.findtext('defaults/'+session), 'auth_admin')
        annotations = {item.attrib['key']: item.text for item in action.findall('annotate')}
        self.assertEqual(annotations['org.freedesktop.policykit.exec.path'], '/opt/clash-of-rust/clash-of-rust')
        self.assertEqual(annotations['org.freedesktop.policykit.exec.argv1'], '--authorize-tun')
        self.assertNotIn('org.freedesktop.policykit.exec.allow_gui', annotations)

    def test_arm64_bundles_are_separate_and_native_architecture_is_detected(self):
        self.assertEqual(build.bundle_directory('windows','x64'),build.ROOT/'bundle')
        self.assertEqual(build.bundle_directory('windows','arm64'),build.ROOT/'bundle/windows-arm64')
        self.assertEqual(build.bundle_directory('linux','arm64'),build.ROOT/'bundle/linux-arm64')
        for machine, expected in [('AMD64','x64'),('x86_64','x64'),('ARM64','arm64'),('aarch64','arm64')]:
            with patch.object(build.platform,'machine',return_value=machine):
                self.assertEqual(build.host_arch(),expected)
        with patch.object(build.platform,'machine',return_value='armv7l'):
            with self.assertRaises(ValueError):
                build.host_arch()

    def test_package_rejects_wrong_binary_architecture(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder)/'binary'
            for system, arch, machine in [('windows','x64',0x8664),('windows','arm64',0xaa64),('linux','x64',62),('linux','arm64',183),('macos','x64',0x01000007),('macos','arm64',0x0100000c)]:
                header = bytearray(64)
                if system == 'windows':
                    header[:2] = b'MZ'
                    struct.pack_into('<I',header,0x3c,64)
                    header.extend(b'PE\0\0'+struct.pack('<H',machine))
                elif system == 'macos':
                    header[:4] = b'\xcf\xfa\xed\xfe'
                    struct.pack_into('<I',header,4,machine)
                else:
                    header[:6] = b'\x7fELF\x02\x01'
                    struct.pack_into('<H',header,18,machine)
                path.write_bytes(header)
                build.validate_binary_arch(path,system,arch)
                with self.assertRaises(ValueError):
                    build.validate_binary_arch(path,system,'arm64' if arch == 'x64' else 'x64')
            path.write_bytes(b'not an executable')
            with self.assertRaises(ValueError):
                build.validate_binary_arch(path,'windows','arm64')

    def test_version_validation(self):
        self.assertEqual(build.validate_version('0.4.4'),'0.4.4')
        for invalid in ('1.2','1.2.3.4','1.2.x','1.2.65536','../0.4.4','01.2.3','0.4.12-dev.01','0.4.12-','0.4.12-dev/1',
                        '0.6.5-dev:fake-ip-sources','0.6.5-fix:android-protection'):
            with self.assertRaises(ValueError):
                build.validate_version(invalid)

    def test_prerelease_installer_versions_preserve_semver_and_upgrade_order(self):
        self.assertEqual(build.deb_version('0.4.12'),'0.4.12')
        for version, stable, previous in [('0.4.12-dev.1','0.4.12','0.4.11'),
                                          ('0.6.5-dev.1.fake-ip-sources','0.6.5','0.6.4'),
                                          ('0.6.5-fix.1.android-protection','0.6.5','0.6.4')]:
            with self.subTest(version=version):
                self.assertEqual(build.validate_version(version),version)
                self.assertEqual(build.numeric_version(version),stable+'.0')
                self.assertEqual(build.deb_version(version),version.replace('-','~',1))
                if shutil.which('dpkg'):
                    subprocess.run(['dpkg','--compare-versions',build.deb_version(version),'lt',stable],check=True)
                    subprocess.run(['dpkg','--compare-versions',build.deb_version(version),'gt',previous],check=True)

    def test_build_token_is_sent_only_to_secure_github_api_and_removed_on_redirect(self):
        import io
        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self,*_): self.close()
        class Opener:
            def open(self,request,**kwargs):
                self.request = request
                return Response(b'{"ok":true}')
        mocked = Opener()
        with patch.dict(build.os.environ, {'GH_TOKEN':'test-only-token'}), patch.object(build,'opener',return_value=mocked):
            for url, expected in [('https://api.github.com/repos/example/repo', 'Bearer test-only-token'),
                                  ('https://example.test/data', None), ('http://api.github.com/data', None)]:
                self.assertEqual(build.read_json(url), {'ok':True})
                self.assertEqual(mocked.request.get_header('Authorization'), expected)
        source = build.urllib.request.Request('https://api.github.com/data', headers={'Authorization':'Bearer test-only-token'})
        for destination in ('https://example.test/data', 'http://api.github.com/data'):
            redirected = build.SafeRedirect().redirect_request(source,None,302,'Found',{},destination)
            self.assertIsNone(redirected.get_header('Authorization'))

    def resources(self,source):
        source.mkdir()
        (source/'mihomo.exe').write_bytes(b'core')
        build.write_json(source/'core.json',{'version':'v1.20.1','exe_sha256':build.sha256(source/'mihomo.exe')})
        records = []
        for name in build.GEO_NAMES:
            path = source/name
            path.write_bytes(name.encode())
            records.append({'name':name,'size':path.stat().st_size,'sha256':build.sha256(path)})
        build.write_json(source/'geodata.json',{'files':records})

    def test_staging_excludes_old_runtime_and_checks_all_payload_hashes(self):
        with tempfile.TemporaryDirectory() as folder:
            base = Path(folder)
            source = base/'resources'
            self.resources(source)
            stale = source/'ip-check/runtime/usr/bin'
            stale.mkdir(parents=True)
            (stale/'bash.exe').write_bytes(b'stale tool')
            stage = base/'stage'
            build.stage_resources(source,stage,'windows')
            self.assertEqual({p.name for p in (stage/'resources/ip-check').iterdir()},{'LICENSE','SOURCE.md'})
            notice = (stage/'resources/THIRD-PARTY-NOTICES.txt').read_text()
            self.assertIn('Version: v1.20.1. Distributed under GPL-3.0.', notice)
            self.assertIn('https://github.com/MetaCubeX/mihomo/tree/v1.20.1', notice)
            (source/'GeoIP.dat').write_bytes(b'corruption')
            with self.assertRaises(ValueError):
                build.validate_resources(source,'windows')

    def test_android_notice_uses_resolved_go_version_and_pseudo_revision(self):
        with tempfile.TemporaryDirectory() as folder:
            module = Path(folder)/'go.mod'
            notice = Path(folder)/'notice.txt'
            module.write_text('module example.test\nrequire (\n github.com/metacubex/mihomo v1.2.4-0.20261029120000-123456789abc\n)\n')
            build.write_mihomo_notice(notice, build.mihomo_go_version(module))
            self.assertIn('Version: v1.2.4-0.20261029120000-123456789abc.', notice.read_text())
            self.assertIn('/mihomo/tree/123456789abc', notice.read_text())
            module.write_text('require github.com/metacubex/mihomo v1.2.3\nreplace github.com/metacubex/mihomo => ../local\n')
            with self.assertRaisesRegex(ValueError, 'explicit source notice'):
                build.mihomo_go_version(module)
            with self.assertRaisesRegex(ValueError, 'Invalid mihomo version'):
                build.write_mihomo_notice(notice, '../invalid')

    def test_manifest_rejects_unexpected_paths(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder)/'resources'
            self.resources(source)
            value = json.loads((source/'geodata.json').read_text())
            value['files'][0]['name'] = '../outside'
            build.write_json(source/'geodata.json',value)
            with self.assertRaises(ValueError):
                build.validate_resources(source,'windows')

    def test_verified_download_rejects_corruption_without_replacing_good_file(self):
        import io
        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self,*_): self.close()
        class Opener:
            def open(self,*args,**kwargs): return Response(b'bad')
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder)/'asset'
            path.write_bytes(b'previous')
            with patch.object(build,'opener',return_value=Opener()), patch.object(build.time,'sleep'):
                with self.assertRaises(ValueError):
                    build.download('https://example.test/asset',path,digest=hashlib.sha256(b'expected').hexdigest())
            self.assertEqual(path.read_bytes(),b'previous')
            self.assertFalse(path.with_name('asset.partial').exists())

    def test_windows_pe_version_is_read_without_executing_binary(self):
        import os
        if os.name != 'nt':
            self.skipTest('Windows version API')
        binary = build.ROOT/'target/release/clash-of-rust.exe'
        if not binary.is_file():
            self.skipTest('Application binary not prepared')
        version = build.file_version(binary)
        self.assertEqual(build.validate_version(version),version)


class ReleaseTests(unittest.TestCase):
    commit = 'a' * 40
    version = '0.4.11'
    tag = 'v0.4.11'
    notes = 'Reviewed release notes\n'

    def setUp(self):
        printing = patch('builtins.print')
        printing.start()
        self.addCleanup(printing.stop)

    def client(self, run_changes=None, jobs=None, main=None):
        run = {'id':77, 'path':'.github/workflows/ci.yml@main', 'event':'push', 'head_branch':'main',
               'head_repository':{'full_name':'example/project'}, 'head_sha':self.commit,
               'status':'completed', 'conclusion':'success'}
        run.update(run_changes or {})
        jobs = jobs if jobs is not None else [{'name':name,'status':'completed','conclusion':'success'} for name in release.CI_JOBS]
        def request(path, *args, **kwargs):
            if path.startswith('/actions/workflows/ci.yml/runs?'):return {'workflow_runs':[run]}
            if path == '/actions/runs/77':return run
            if path.startswith('/actions/runs/77/attempts/') and '/jobs?' in path:return {'jobs':jobs,'total_count':len(jobs)}
            if path.startswith('/branches/'):return {'commit':{'sha':main or self.commit}}
            self.fail('Unexpected CI request: '+path)
        request.repo = 'example/project'
        return request

    def files(self, directory):
        for suffix in release.PACKAGES.values():
            path = directory/f'Clash-of-Rust-{self.version}-{suffix}'
            path.write_bytes(('installer '+suffix).encode())
            path.with_name(path.name+'.sha256').write_text(build.sha256(path)+'\n')
        return release.validate_files(list(directory.iterdir()), self.version)

    def test_release_environment_tokens_do_not_require_git_credential_helper(self):
        for key in ('GH_TOKEN','GITHUB_TOKEN'):
            with patch.dict(build.os.environ,{key:'test-only-token'},clear=True), patch.object(release.subprocess,'run') as git:
                self.assertEqual(release.credential('example/project'),'test-only-token')
                git.assert_not_called()
        with self.assertRaises(ValueError):
            release.GitHub('example/project','test-only-token')('https://example.test/upload','POST',b'installer')

    def test_release_artifact_get_recovers_from_dns_and_interrupted_download(self):
        reply = MagicMock()
        reply.read.side_effect = release.http.client.IncompleteRead(b'partial')
        transport = MagicMock()
        transport.open.side_effect = [
            urllib.error.URLError('temporary DNS failure'),
            nullcontext(reply),
            nullcontext(io.BytesIO(b'complete verified archive')),
        ]
        with patch.object(release, 'opener', return_value=transport), patch.object(release.time, 'sleep') as sleep:
            data = release.GitHub('example/project','test-only-token')('/actions/artifacts/1/zip',raw=True)
        self.assertEqual(data,b'complete verified archive')
        self.assertEqual(transport.open.call_count,3)
        self.assertEqual([call.args[0] for call in sleep.call_args_list],[1,2])

    def test_release_metadata_does_not_reuse_a_stale_jobs_snapshot(self):
        current = {'jobs':[{'status':'in_progress','conclusion':None}]}
        cached = {}
        queries = []
        def response(request, **kwargs):
            queries.append(urllib.parse.parse_qs(urllib.parse.urlparse(request.full_url).query))
            body = cached.setdefault(request.full_url,json.dumps(current).encode())
            return nullcontext(io.BytesIO(body))
        transport = MagicMock()
        transport.open.side_effect = response
        with patch.object(release,'opener',return_value=transport), patch.object(release.time,'time_ns',side_effect=[1,2]):
            client = release.GitHub('example/project','test-only-token')
            path = '/actions/runs/77/jobs?filter=latest&per_page=100'
            self.assertEqual(client(path)['jobs'][0]['status'],'in_progress')
            current['jobs'][0].update(status='completed',conclusion='success')
            self.assertEqual(client(path)['jobs'][0]['conclusion'],'success')
        self.assertEqual(len(cached),2)
        for query in queries:
            self.assertEqual(query['filter'],['latest'])
            self.assertEqual(query['per_page'],['100'])

    def test_release_reads_attempt_jobs_and_keeps_the_newest_result_after_partial_reruns(self):
        previous = [{'name':name,'status':'completed','conclusion':'success'} for name in release.CI_JOBS]
        attempts = {1:previous, 2:[{'name':'macos-arm64','status':'completed','conclusion':'failure'}]}
        def client(path):
            match = re.fullmatch(r'/actions/runs/77/attempts/([12])/jobs\?per_page=100&page=1',path)
            self.assertIsNotNone(match, 'Must not depend on the incomplete generic jobs index')
            jobs = attempts[int(match.group(1))]
            return {'jobs':jobs,'total_count':len(jobs)}
        jobs = {j['name']:j for j in release.workflow_jobs(client,{'id':77,'run_attempt':2})}
        self.assertEqual(set(jobs),release.CI_JOBS)
        self.assertEqual(jobs['macos-arm64']['conclusion'],'failure')
        self.assertEqual(jobs['linux-x64']['conclusion'],'success')
        attempts[2][0]['conclusion']='success'
        self.assertTrue(all(j['conclusion']=='success' for j in release.workflow_jobs(client,{'id':77,'run_attempt':2})))

    def test_release_attempt_job_pages_must_be_complete_and_unique(self):
        records = [{'name':f'job-{index}'} for index in range(101)]
        def client(path):
            page = int(urllib.parse.parse_qs(urllib.parse.urlparse(path).query)['page'][0])
            return {'jobs':records[(page-1)*100:page*100],'total_count':len(records)}
        self.assertEqual(len(release.workflow_jobs(client,{'id':77})),101)
        records[-1]['name']=records[0]['name']
        with self.assertRaisesRegex(ValueError,'Duplicate'):
            release.workflow_jobs(client,{'id':77})
        with self.assertRaisesRegex(ValueError,'incomplete'):
            release.workflow_jobs(lambda _: {'jobs':[],'total_count':1},{'id':77})

    def test_release_partial_rerun_stops_before_obsolete_skipped_matrix_placeholders(self):
        records = {
            2:[{'name':name,'status':'completed','conclusion':'success'} for name in release.CI_JOBS],
            3:[{'name':'macos-arm64','status':'completed','conclusion':'success'}],
        }
        queried = []
        def client(path):
            number = int(path.split('/attempts/',1)[1].split('/',1)[0])
            queried.append(number)
            self.assertIn(number,records, 'Older failed-input attempts must not reintroduce matrix placeholders')
            jobs = records[number]
            return {'jobs':jobs,'total_count':len(jobs)}
        jobs = release.workflow_jobs(client,{'id':77,'run_attempt':3},release.CI_JOBS)
        self.assertEqual({job['name'] for job in jobs},release.CI_JOBS)
        self.assertEqual(queried,[3,2])

    def test_release_refresh_preserves_artifact_and_mutation_urls(self):
        for path,method,data,raw in [
            ('/actions/artifacts/1/zip?archive=verified','GET',None,True),
            ('https://uploads.github.com/repos/example/project/releases/1/assets?name=package','POST',b'package',False),
            ('/releases/1','PATCH',{'draft':False},False),
        ]:
            with self.subTest(path=path):
                transport = MagicMock()
                transport.open.return_value = nullcontext(io.BytesIO(b'archive' if raw else b'{}'))
                with patch.object(release,'opener',return_value=transport):
                    release.GitHub('example/project','test-only-token')(path,method,data,raw=raw)
                request = transport.open.call_args.args[0]
                expected = 'https://api.github.com/repos/example/project'+path if path.startswith('/') else path
                self.assertEqual(request.full_url,expected)
                self.assertEqual(request.get_method(),method)

    def test_release_get_retries_only_temporary_http_errors(self):
        for status in (403,404,429,503):
            with self.subTest(status=status):
                error = urllib.error.HTTPError('https://api.github.com/test',status,'HTTP failure',{},None)
                transport = MagicMock()
                transport.open.side_effect = [error,nullcontext(io.BytesIO(b'{"ok":true}'))]
                with patch.object(release, 'opener', return_value=transport), patch.object(release.time, 'sleep') as sleep:
                    client = release.GitHub('example/project','test-only-token')
                    if status in (403,404):
                        with self.assertRaises(urllib.error.HTTPError): client('/test')
                        self.assertEqual(transport.open.call_count,1)
                        sleep.assert_not_called()
                    else:
                        self.assertEqual(client('/test'),{'ok':True})
                        self.assertEqual(transport.open.call_count,2)
                        sleep.assert_called_once_with(1)

    def test_release_network_retries_are_bounded_and_never_replay_mutations(self):
        for method, attempts in [('GET',3),('POST',1),('PATCH',1),('DELETE',1)]:
            with self.subTest(method=method):
                transport = MagicMock()
                transport.open.side_effect = urllib.error.URLError('offline')
                with patch.object(release, 'opener', return_value=transport), patch.object(release.time, 'sleep'):
                    with self.assertRaises(urllib.error.URLError):
                        release.GitHub('example/project','test-only-token')('/test',method=method)
                self.assertEqual(transport.open.call_count,attempts)

    def test_release_check_mode_never_calls_publisher(self):
        with patch.object(release.sys,'argv',['publish-release.py','--check','--run-id','77','--package-run-id','88']), \
             patch.object(release,'repository',return_value='example/project'), \
             patch.object(release,'credential',return_value='test-only-token'), \
             patch.object(release,'GitHub'), patch.object(release,'git',return_value=self.commit), \
             patch.object(release,'verified_run',return_value={'id':77,'head_branch':'main','html_url':'https://example.test/ci'}), \
             patch.object(release,'release_metadata',return_value=(self.version,self.tag,self.notes)), \
             patch.object(release,'verified_package_run'), patch.object(release,'download_package_files',return_value={}), patch.object(release,'publish') as publishing:
            release.main()
            publishing.assert_not_called()

    def test_release_requires_current_main_push_ci_and_every_job(self):
        self.assertEqual(release.verified_run(self.client(),self.commit)['id'],77)
        for changes in ({'event':'pull_request'}, {'path':'.github/workflows/release.yml'}, {'head_branch':'other'},
                        {'head_sha':'b'*40}, {'head_repository':{'full_name':'other/project'}},
                        {'status':'in_progress'}, {'conclusion':'failure'}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                release.verified_run(self.client(changes),self.commit,77)
        jobs = [{'name':name,'status':'completed','conclusion':'success'} for name in release.CI_JOBS]
        with self.assertRaises(ValueError):release.verified_run(self.client(jobs=jobs[:-1]),self.commit,77)
        jobs[0]['conclusion']='skipped'
        with self.assertRaises(ValueError):release.verified_run(self.client(jobs=jobs),self.commit,77)
        with self.assertRaises(ValueError):release.verified_run(self.client(main='b'*40),self.commit,77)

    def test_preview_packaging_accepts_current_same_repository_pr_ci_only_in_check_mode(self):
        changes = {'event':'pull_request', 'head_branch':'codex/dev-0.5.2'}
        with self.assertRaises(ValueError):
            release.verified_run(self.client(changes),self.commit,77)
        self.assertEqual(release.verified_run(self.client(changes),self.commit,77,check_only=True)['id'],77)
        for invalid in ({'head_repository':{'full_name':'fork/project'}}, {'head_sha':'b'*40},
                        {'status':'in_progress'}, {'conclusion':'failure'}, {'event':'workflow_dispatch'}):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                release.verified_run(self.client(changes | invalid),self.commit,77,check_only=True)
        with self.assertRaises(ValueError):
            release.verified_run(self.client(changes, main='b'*40),self.commit,77,check_only=True)
        with self.assertRaises(ValueError):
            release.verified_run(self.client(changes,jobs=[]),self.commit,77,check_only=True)

    def test_release_version_and_notes_are_read_from_tested_commit(self):
        with patch.object(release,'git',return_value='[package]\nversion="0.4.11"'), patch.object(release.subprocess,'check_output',return_value=self.notes):
            self.assertEqual(release.release_metadata(self.commit),(self.version,self.tag,self.notes))
            with self.assertRaises(ValueError):release.release_metadata(self.commit,'v0.4.12')
            with tempfile.TemporaryDirectory() as folder:
                path=Path(folder)/'notes.md';path.write_text('Uncommitted notes')
                with self.assertRaises(ValueError):release.release_metadata(self.commit,notes_path=path)

    def test_prerelease_requires_explicit_current_same_repository_ci(self):
        changes = {'event':'pull_request', 'head_branch':'codex/fix-android'}
        self.assertEqual(release.verified_run(self.client(changes), self.commit, 77, prerelease=True)['id'], 77)
        for invalid in ({'head_repository':{'full_name':'fork/project'}},
                        {'event':'push'}, {'head_sha':'b'*40}, {'conclusion':'failure'}):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                release.verified_run(self.client(changes | invalid), self.commit, 77, prerelease=True)
        with self.assertRaises(ValueError):
            release.verified_run(self.client(changes), self.commit, prerelease=True)
        with self.assertRaises(ValueError):
            release.verified_run(self.client(changes), self.commit, 77, check_only=True, prerelease=True)
        with self.assertRaises(ValueError):
            release.verified_run(self.client(changes, main='b'*40), self.commit, 77, prerelease=True)
        with patch.object(release, 'git', return_value='[package]\nversion=\"0.6.5\"'):
            with self.assertRaises(ValueError):
                release.release_metadata(self.commit, prerelease=True)
        with patch.object(release, 'git', return_value='[package]\nversion=\"0.6.5-fix.2.android-parity\"'), \
             patch.object(release.subprocess, 'check_output', return_value=self.notes):
            self.assertEqual(release.release_metadata(self.commit, prerelease=True)[1], 'v0.6.5-fix.2.android-parity')

    def test_preview_packaging_requires_no_release_notes_and_still_checks_version(self):
        with patch.object(release,'git',return_value='[package]\nversion="0.5.2-dev.1"'), \
             patch.object(release.subprocess,'check_output') as notes:
            self.assertEqual(release.release_metadata(self.commit,check_only=True),
                             ('0.5.2-dev.1','v0.5.2-dev.1',''))
            notes.assert_not_called()
            with self.assertRaises(ValueError):
                release.release_metadata(self.commit,'v0.5.1',check_only=True)
        with patch.object(release,'git',return_value='[package]\nversion="invalid"'):
            with self.assertRaises(ValueError):
                release.release_metadata(self.commit,check_only=True)

    def test_release_rejects_missing_assets_and_bad_or_misnamed_checksums(self):
        with tempfile.TemporaryDirectory() as folder:
            directory=Path(folder);files=self.files(directory)
            with self.assertRaises(ValueError):release.validate_files(list(files.values())[:-1],self.version)
            checksum=next(p for p in files.values() if p.suffix=='.sha256')
            digest=checksum.read_text().strip()
            checksum.write_text(digest+'  unrelated.exe\n')
            with self.assertRaises(ValueError):release.validate_files(list(files.values()),self.version)
            checksum.write_text('0'*64+'\n')
            with self.assertRaises(ValueError):release.validate_files(list(files.values()),self.version)

    def artifact_client(self, bad_name=False, expired=False, bad_digest=False, duplicate=False, bad_receipt=False, bad_build=False):
        artifacts=[];archives={};builds=[]
        for number,(package,suffix) in enumerate(release.PACKAGES.items()):
            name=f'Clash-of-Rust-{self.version}-{suffix}'
            data=('installer '+suffix).encode();stream=io.BytesIO()
            system,arch=package.removeprefix('package-').split('-')
            source={'id':100+number,'name':f'build-{system}-{arch}','digest':'sha256:'+hashlib.sha256(('build '+suffix).encode()).hexdigest(),'expired':False}
            builds.append(source)
            receipt={'schema':1,'commit':self.commit,'version':self.version,'ci_run_id':77,
                     'system':system,'arch':arch,'package':name,'tests_passed':True,
                     'sha256':hashlib.sha256(data).hexdigest(),
                     'build_artifact':{key:source[key] for key in ('id','name','digest')}}
            if bad_receipt:receipt['commit']='b'*40
            if bad_build:receipt['build_artifact']['id']=999
            with zipfile.ZipFile(stream,'w') as zipped:
                zipped.writestr('../outside' if bad_name and number==0 else name,data)
                zipped.writestr(name+'.sha256',hashlib.sha256(data).hexdigest()+'\n')
                zipped.writestr('package-receipt.json',json.dumps(receipt))
            blob=stream.getvalue();archives[number]=blob
            artifacts.append({'id':number,'name':package,'expired':expired,
                              'digest':'sha256:'+('0'*64 if bad_digest else hashlib.sha256(blob).hexdigest())})
        if duplicate:artifacts.append(artifacts[0])
        def request(path, raw=False):
            if path.startswith('/actions/runs/77/artifacts?'):return {'artifacts':builds}
            if path.startswith('/actions/runs/88/artifacts?'):return {'artifacts':artifacts}
            return archives[int(path.split('/')[3])]
        return request

    def test_release_artifact_download_enforces_archive_hash_and_safe_complete_contents(self):
        with tempfile.TemporaryDirectory() as folder:
            directory=Path(folder)
            self.assertEqual(len(release.download_package_files(self.artifact_client(),88,77,self.commit,self.version,directory/'good')),len(release.PACKAGES)*2)
            for key in ('bad_name','expired','bad_digest','duplicate','bad_receipt','bad_build'):
                with self.subTest(key=key), self.assertRaises(ValueError):
                    release.download_package_files(self.artifact_client(**{key:True}),88,77,self.commit,self.version,directory/key)
            self.assertFalse((directory/'outside').exists())

    def test_release_requires_every_package_and_compatibility_job(self):
        def client(changes=None,jobs=None):
            run={'id':88,'path':'.github/workflows/release.yml@main','event':'workflow_dispatch',
                 'head_branch':'main','head_sha':self.commit,'head_repository':{'full_name':'example/project'},
                 'status':'completed','conclusion':'success'}
            run.update(changes or {})
            records=jobs if jobs is not None else [{'name':name,'status':'completed','conclusion':'success'} for name in release.PACKAGE_JOBS]
            def request(path):
                return {'jobs':records,'total_count':len(records)} if '/jobs?' in path else run
            request.repo='example/project'
            return request
        with patch.dict(build.os.environ,{},clear=True):
            self.assertEqual(release.verified_package_run(client(),88,self.commit)['id'],88)
            for change in ({'event':'push'},{'head_sha':'b'*40},{'path':'.github/workflows/ci.yml'},
                           {'status':'in_progress'},{'conclusion':'failure'},{'head_branch':'codex/dev-0.5.2'}):
                with self.subTest(change=change),self.assertRaises(ValueError):
                    release.verified_package_run(client(change),88,self.commit)
            jobs=[{'name':name,'status':'completed','conclusion':'success'} for name in release.PACKAGE_JOBS]
            with self.assertRaises(ValueError):release.verified_package_run(client(jobs=jobs[:-1]),88,self.commit)
            jobs[0]['conclusion']='skipped'
            with self.assertRaises(ValueError):release.verified_package_run(client(jobs=jobs),88,self.commit)
            self.assertEqual(release.verified_package_run(client({'head_branch':'codex/dev-0.5.2'}),
                                                         88,self.commit,check_only=True)['id'],88)
            self.assertEqual(release.verified_package_run(client({'head_branch':'codex/fix-android'}),
                             88,self.commit,prerelease=True,source_branch='codex/fix-android')['id'],88)
            with self.assertRaises(ValueError):
                release.verified_package_run(client({'head_branch':'other'}),
                    88,self.commit,prerelease=True,source_branch='codex/fix-android')
            with self.assertRaises(ValueError):
                release.verified_package_run(client({'head_branch':'codex/dev-0.5.2'},jobs=jobs),
                                             88,self.commit,check_only=True)
        with patch.dict(build.os.environ,{'GITHUB_RUN_ID':'88'},clear=True):
            release.verified_package_run(client({'status':'in_progress','conclusion':None}),88,self.commit)

    def test_resolve_emits_ci_identity_without_downloading_packages(self):
        with tempfile.TemporaryDirectory() as directory:
            output=Path(directory)/'outputs'
            with patch.object(release.sys,'argv',['publish-release.py','--resolve','--run-id','77']), \
                 patch.object(release,'repository',return_value='example/project'), \
                 patch.object(release,'credential',return_value='test-only-token'), \
                 patch.object(release,'GitHub'),patch.object(release,'git',return_value=self.commit), \
                 patch.object(release,'verified_run',return_value={'id':77}), \
                 patch.object(release,'release_metadata',return_value=(self.version,self.tag,self.notes)), \
                 patch.object(release,'build_artifacts'),patch.object(release,'download_package_files') as download, \
                 patch.object(release,'publish') as publishing,patch.dict(build.os.environ,{'GITHUB_OUTPUT':str(output)}):
                release.main()
                download.assert_not_called();publishing.assert_not_called()
            self.assertIn('commit='+self.commit,output.read_text())
            self.assertIn('ci_run_id=77',output.read_text())

    class PublisherClient:
        def __init__(self, owner, published=False, bad_upload=False, main_changed=False, wrong_tag=False, starter=False, branch='main'):
            self.owner=owner;self.bad_upload=bad_upload;self.main_changed=main_changed;self.writes=[]
            self.branch=branch
            self.tag_commit='b'*40 if wrong_tag else owner.commit if published else None
            self.release=None
            if published or starter:
                self.release={'id':1,'tag_name':owner.tag,'target_commitish':owner.commit,'body':owner.notes,
                    'draft':not published,'prerelease':False,'upload_url':'https://uploads.github.com/repos/example/project/releases/1{?name}',
                    'assets':[],'html_url':'https://github.com/example/project/releases/tag/'+owner.tag}
            if starter:
                self.release['assets']=[{'id':42,'name':sorted(release.filenames(owner.version))[0],'state':'starter','size':0}]

        def __call__(self,path,method='GET',data=None,**kwargs):
            owner=self.owner
            if method!='GET':self.writes.append((path,method,data))
            if path.startswith('/releases?'):return [copy.deepcopy(self.release)] if self.release else []
            if path=='/git/ref/tags/'+owner.tag:
                if self.tag_commit is None:raise urllib.error.HTTPError(path,404,'Not Found',{},None)
                return {'object':{'type':'commit','sha':self.tag_commit}}
            if path=='/branches/'+urllib.parse.quote(self.branch,safe=''):return {'commit':{'sha':'b'*40 if self.main_changed else owner.commit}}
            if path=='/releases' and method=='POST':
                self.release={**data,'id':1,'assets':[],
                    'upload_url':'https://uploads.github.com/repos/example/project/releases/1{?name}',
                    'html_url':'https://github.com/example/project/releases/tag/'+owner.tag}
            elif path=='/releases/1' and method=='PATCH':
                self.release.update(data)
                if not self.release['draft']:self.tag_commit=owner.commit
            elif method=='POST' and path.startswith('https://uploads.github.com/'):
                name=urllib.parse.parse_qs(urllib.parse.urlparse(path).query)['name'][0]
                asset={'id':100+len(self.release['assets']),'name':name,'size':len(data),'state':'uploaded',
                    'digest':'sha256:'+('0'*64 if self.bad_upload else hashlib.sha256(data).hexdigest())}
                self.release['assets'].append(asset);return copy.deepcopy(asset)
            elif path=='/releases/assets/42' and method=='DELETE':
                self.release['assets']=[];return None
            elif path not in ('/releases/1','/releases/tags/'+owner.tag):
                owner.fail('Unexpected publication request: '+path)
            return copy.deepcopy(self.release)

    def test_release_publishes_only_after_all_verified_uploads_and_can_resume_empty_starter(self):
        with tempfile.TemporaryDirectory() as folder:
            files=self.files(Path(folder))
            for starter in (False,True):
                client=self.PublisherClient(self,starter=starter)
                release.publish(client,self.commit,self.tag,self.notes,files)
                self.assertFalse(client.release['draft'])
                self.assertEqual(len(client.release['assets']),len(release.PACKAGES)*2)
                if starter:self.assertTrue(any(method=='DELETE' for _,method,_ in client.writes))

    def test_development_version_is_published_as_a_preview(self):
        self.version='0.4.13-dev.1';self.tag='v'+self.version
        with tempfile.TemporaryDirectory() as folder:
            files=self.files(Path(folder));client=self.PublisherClient(self)
            release.publish(client,self.commit,self.tag,self.notes,files)
            self.assertTrue(client.release['prerelease'])
            self.assertEqual(client.release['make_latest'],'false')

    def test_release_refuses_existing_public_release_or_wrong_tag_without_writes(self):
        with tempfile.TemporaryDirectory() as folder:
            files=self.files(Path(folder))
            for kwargs in ({'published':True},{'wrong_tag':True}):
                client=self.PublisherClient(self,**kwargs)
                with self.assertRaises(ValueError):release.publish(client,self.commit,self.tag,self.notes,files)
                self.assertFalse(client.writes)

    def test_fix_branch_prerelease_does_not_change_latest_and_checks_source_after_upload(self):
        self.version='0.6.5-fix.2.android-parity';self.tag='v'+self.version
        with tempfile.TemporaryDirectory() as folder:
            files=self.files(Path(folder))
            branch='codex/fix-android'
            refused=self.PublisherClient(self,branch=branch)
            with self.assertRaises(ValueError):
                release.publish(refused,self.commit,self.tag,self.notes,files,source_branch=branch)
            self.assertFalse(refused.writes)
            for changed in (False, True):
                client=self.PublisherClient(self,branch=branch,main_changed=changed)
                if changed:
                    with self.assertRaises(ValueError):
                        release.publish(client,self.commit,self.tag,self.notes,files,source_branch=branch,prerelease=True)
                    self.assertTrue(client.release['draft'])
                else:
                    release.publish(client,self.commit,self.tag,self.notes,files,source_branch=branch,prerelease=True)
                    self.assertFalse(client.release['draft'])
                    self.assertTrue(client.release['prerelease'])
                    self.assertEqual(client.release['make_latest'],'false')

    def test_release_leaves_draft_unpublished_after_bad_upload_or_main_change(self):
        with tempfile.TemporaryDirectory() as folder:
            files=self.files(Path(folder))
            for kwargs in ({'bad_upload':True},{'main_changed':True}):
                client=self.PublisherClient(self,**kwargs)
                with self.assertRaises(ValueError):release.publish(client,self.commit,self.tag,self.notes,files)
                self.assertTrue(client.release['draft'])
                self.assertFalse(any(isinstance(data,dict) and data.get('draft') is False for _,_,data in client.writes))


class AptCacheTests(unittest.TestCase):
    def test_records_require_strong_hashes_and_safe_names(self):
        checksum = 'a'*64
        valid = f"'https://example.invalid/pkg.deb' pkg_1_amd64.deb 3 SHA256:{checksum}\n"
        self.assertEqual(linux_apt.archive_records(valid), {
            'pkg_1_amd64.deb': {'size': 3, 'sha256': checksum}})
        self.assertEqual(linux_apt.archive_records(valid.replace('SHA256:', 'MD5Sum:')), {})
        self.assertEqual(linux_apt.archive_records(valid.replace('https:', 'file:')), {})
        for name in ('../pkg.deb', '/pkg.deb', 'pkg.txt', 'pkg;cmd.deb'):
            with self.subTest(name=name), self.assertRaises(ValueError):
                linux_apt.archive_records(valid.replace('pkg_1_amd64.deb', name))
        with self.assertRaises(ValueError):
            linux_apt.archive_records(valid + valid.replace(checksum, 'b'*64))

    def test_local_application_records_without_hash_do_not_disable_dependency_cache(self):
        remote = "'https://example.invalid/pkg.deb' pkg_1_amd64.deb 3 SHA256:" + 'a'*64 + '\n'
        for uri in ('file:/tmp/application.deb', '/tmp/application.deb'):
            with self.subTest(uri=uri):
                local = f"'{uri}' application.deb 100\n"
                self.assertEqual(linux_apt.archive_records(local + remote), {
                    'pkg_1_amd64.deb': {'size': 3, 'sha256': 'a'*64}})
        with self.assertRaises(ValueError):
            linux_apt.archive_records("'https://example.invalid/pkg.deb' pkg_1_amd64.deb 3\n")

    def test_install_reuses_verified_bytes_and_replaces_same_size_corruption(self):
        content = b'verified package'
        name = 'pkg_1_amd64.deb'
        query = f"'https://example.invalid/pkg.deb' {name} {len(content)} SHA256:{hashlib.sha256(content).hexdigest()}\n"
        for initial in (None, content, b'x'*len(content)):
            with self.subTest(initial=initial), tempfile.TemporaryDirectory() as temporary:
                cache = Path(temporary)/'cache'
                cache.mkdir()
                if initial is not None:
                    (cache/name).write_bytes(initial)
                directories = []

                def execute(command, timeout, **kwargs):
                    if command[0] == 'rm':
                        shutil.rmtree(command[-1])
                        return
                    directory = Path(next(item.split('=', 1)[1] for item in command
                                          if item.startswith('Dir::Cache::archives=')))
                    directories.append(directory)
                    if '--print-uris' in command:
                        self.assertFalse((directory/name).exists())
                        return subprocess.CompletedProcess(command, 0, stdout=query)
                    self.assertEqual((directory/name).exists(), initial == content)
                    (directory/name).write_bytes(content)
                    return subprocess.CompletedProcess(command, 0)

                with patch.dict(os.environ, {'GITHUB_ACTIONS': 'true',
                                            'COR_LINUX_APT_CACHE': str(cache)}), \
                     patch.object(linux_apt, 'privileged', side_effect=execute):
                    linux_apt.install(['pkg'])
                self.assertEqual((cache/name).read_bytes(), content)
                self.assertTrue(directories)
                self.assertTrue(all(not item.exists() for item in directories))

    def test_symlinks_are_never_trusted(self):
        if not hasattr(os, 'symlink'):
            self.skipTest('No symlink support')
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root/'real').write_bytes(b'valid')
            try:
                (root/'alias').symlink_to(root/'real')
            except OSError:
                self.skipTest('Symlinks unavailable')
            record = {'size': 5, 'sha256': hashlib.sha256(b'valid').hexdigest()}
            self.assertTrue(linux_apt.matches(root/'real', record))
            self.assertFalse(linux_apt.matches(root/'alias', record))

    def test_metadata_and_cleanup_failure_do_not_hide_install_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            archives = Path(temporary)/'archives'
            archives.mkdir()
            with patch.dict(os.environ, {'GITHUB_ACTIONS': 'true',
                                        'COR_LINUX_APT_CACHE': temporary}), \
                 patch.object(linux_apt.tempfile, 'mkdtemp', return_value=str(archives)), \
                 patch.object(linux_apt, 'privileged', side_effect=[
                     subprocess.CalledProcessError(100, 'metadata'),
                     subprocess.CalledProcessError(101, 'install'),
                     subprocess.CalledProcessError(102, 'cleanup')]):
                with self.assertRaises(subprocess.CalledProcessError) as failure:
                    linux_apt.install(['pkg'])
                self.assertEqual(failure.exception.returncode, 101)


if __name__ == '__main__':
    unittest.main()
