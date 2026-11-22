"""Offline checks for published Linux update fixtures."""
import base64
from contextlib import ExitStack, contextmanager, nullcontext, redirect_stdout
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, mock_open, patch

import linux_ci
import linux_gui
import linux_update_compatibility as update


class LinuxPublishedUpdateTests(unittest.TestCase):
    def setUp(self):
        temporary = self.enterContext(tempfile.TemporaryDirectory())
        self.enterContext(patch.multiple(update, REPORT=Path(temporary)/'cli.json',
                                         GUI_REPORT=Path(temporary)/'gui.json'))
        self.tag = 'v0.7.3-dev.2.live-language'
        self.version = self.tag[1:]
        self.name = f'Clash-of-Rust-{self.version}-linux-amd64.deb'
        self.url = f'https://github.com/example/project/releases/download/{self.tag}/{self.name}'
        self.release = {
            'tag_name': self.tag, 'draft': False, 'prerelease': True,
            'assets': [
                {'name': self.name, 'state': 'uploaded', 'browser_download_url': self.url,
                 'digest': 'sha256:' + 'a'*64, 'size': 100},
                {'name': self.name + '.sha256', 'state': 'uploaded',
                 'browser_download_url': self.url + '.sha256',
                 'digest': 'sha256:' + 'c'*64, 'size': 65},
            ],
        }

    def client(self, release=None):
        class Client:
            repo = 'example/project'

            def __call__(self, path):
                if path.startswith('/releases/tags/'):
                    return release if release is not None else self_release
                if path.startswith('/git/ref/tags/'):
                    return {'object': {'type': 'commit', 'sha': 'b'*40}}
                raise AssertionError(path)

        self_release = self.release
        return Client()

    def test_release_requires_the_exact_uploaded_package_and_checksum(self):
        result = update.release_asset(self.client(), self.tag, 'deb', 'x64')
        self.assertEqual((result['version'], result['package'], result['sha256'], result['commit']),
                         (self.version, self.name, 'a'*64, 'b'*40))
        cases = []
        for field, value in (('draft', True), ('tag_name', 'v0.7.3'), ('prerelease', False)):
            release = copy.deepcopy(self.release)
            release[field] = value
            cases.append(release)
        for field, value in (('digest', ''), ('browser_download_url', 'https://example.invalid/package'),
                             ('state', 'new'), ('size', 0)):
            release = copy.deepcopy(self.release)
            release['assets'][0][field] = value
            cases.append(release)
        for field, value in (('digest', ''), ('size', 0), ('size', 1025)):
            release = copy.deepcopy(self.release)
            release['assets'][1][field] = value
            cases.append(release)
        release = copy.deepcopy(self.release)
        release['assets'].pop()
        cases.append(release)
        release = copy.deepcopy(self.release)
        release['assets'].append(copy.deepcopy(release['assets'][0]))
        cases.append(release)
        for release in cases:
            with self.subTest(release=release), self.assertRaises(ValueError):
                update.release_asset(self.client(release), self.tag, 'deb', 'x64')
        for tag in ('v../other', 'v0.7.3/other', '0.7.3'):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                update.release_asset(self.client(), tag, 'deb', 'x64')

    def test_previous_manifest_uses_the_selected_commit_and_validates_version(self):
        content = b'[workspace.package]\nversion = "0.7.3-dev.1.old"\n'
        response = {'type': 'file', 'encoding': 'base64', 'size': len(content),
                    'content': base64.b64encode(content).decode()}
        seen = []

        def client(path):
            seen.append(path)
            return response

        self.assertEqual(update.previous_version(client, 'a'*40), '0.7.3-dev.1.old')
        self.assertEqual(seen, ['/contents/Cargo.toml?ref=' + 'a'*40])
        for changes in ({'size': 1}, {'encoding': 'utf-8'}, {'content': '***'}):
            with self.subTest(changes=changes):
                response.update(changes)
                with self.assertRaises((ValueError, KeyError)):
                    update.previous_version(client, 'a'*40)
                response = {'type': 'file', 'encoding': 'base64', 'size': len(content),
                            'content': base64.b64encode(content).decode()}

    def test_prepare_rejects_downgrades_before_fetching_artifacts(self):
        for previous, published in (('2.0.0', '1.0.0'), ('2.0.0', '2.0.0-rc.1'),
                                    ('2.0.0-rc.10', '2.0.0-rc.2')):
            with self.subTest(previous=previous, published=published), \
                    tempfile.TemporaryDirectory() as temporary, \
                    patch.object(update.compatibility, 'validate_run'), \
                    patch.object(update, 'previous_version', return_value=previous), \
                    patch.object(update, 'release_asset', return_value={'version': published}), \
                    patch.object(update, 'fetch_package_inputs') as fetch:
                client = Mock(repo='example/project', return_value={
                    'head_sha': 'c'*40, 'id': 1, 'event': 'push'})
                directory = Path(temporary)/'fixture'
                with self.assertRaisesRegex(ValueError, 'newer'):
                    update.prepare(client, 1, 'v'+published, 'deb', 'x64', directory)
                fetch.assert_not_called()
                self.assertFalse(directory.exists())

    def test_version_precedence_follows_semver_prerelease_rules(self):
        ordered = ('1.0.0-alpha', '1.0.0-alpha.1', '1.0.0-alpha.beta',
                   '1.0.0-beta', '1.0.0-beta.2', '1.0.0-beta.10', '1.0.0-rc.1',
                   '1.0.0', '1.0.1-alpha', '2.0.0')
        self.assertEqual(sorted(reversed(ordered), key=update.version_precedence), list(ordered))
        with self.assertRaises(ValueError):
            update.version_precedence('1.0.0-rc.01')

    @unittest.skipUnless(shutil.which('dpkg'), 'requires the read-only DEB version comparator')
    def test_deb_upgrade_rejects_native_downgrades_and_equal_versions(self):
        for previous, published, strict in (('1.2.3-rc.1', '1.2.3-rc1', False),
                                            ('1.2.3-rc', '1.2.3-rc0', False),
                                            ('1.2.3-rc.9', '1.2.3', True)):
            with self.subTest(previous=previous, published=published):
                self.assertGreater(update.version_precedence(published), update.version_precedence(previous))
                fixture = {'format': 'deb', 'previous': {'version': previous},
                           'published': {'version': published}}
                expectation = nullcontext() if strict else self.assertRaisesRegex(ValueError, 'DEB package manager')
                with expectation:
                    update.verify_upgrade_order(fixture)

    def test_arch_and_rpm_use_the_native_comparator(self):
        for package_format, expected in (('pkg', '1.2.3pre.rc.1-1'), ('rpm', '1.2.3~rc.1-1')):
            fixture = {'format': package_format, 'previous': {'version': '1.2.3-rc.1'},
                       'published': {'version': '1.2.3'}}
            for native_order in (-1, 0, 1):
                with self.subTest(package_format=package_format, native_order=native_order), \
                        patch('linux_native_guest.compare_versions', return_value=native_order) as compare:
                    expectation = (self.assertRaisesRegex(RuntimeError, 'Native package manager')
                                   if native_order >= 0 else nullcontext())
                    with expectation:
                        update.verify_upgrade_order(fixture)
                    compare.assert_called_once_with(expected, '1.2.3-1')

    def test_installed_rpm_version_checks_package_identity_and_epoch(self):
        for name, epoch in (('clash-of-rust', '0'), ('clash-of-rust', '2'), ('other-package', '0')):
            installed = subprocess.CompletedProcess([], 0, stdout=f'{name}\n{epoch}\n2.0.0-1\n')
            with self.subTest(name=name, epoch=epoch), patch.object(update, 'invoke', return_value=installed) as run:
                if (name, epoch) == ('clash-of-rust', '0'):
                    self.assertEqual(update.installed_version('rpm'), '2.0.0-1')
                else:
                    with self.assertRaisesRegex(ValueError, 'identity or epoch'):
                        update.installed_version('rpm')
                self.assertIn('%{EPOCHNUM}', run.call_args.args[0][3])

    @unittest.skipUnless(shutil.which('dpkg'), 'requires the read-only DEB version comparator')
    def test_native_downgrade_never_starts_cli_or_gui_acceptance(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(update, 'require_disposable_update_runner'), \
                patch.object(update, 'host_arch', return_value='x64'), \
                patch.object(update, 'exercise_cli') as cli, patch.object(update, 'exercise_gui') as gui:
            directory = Path(temporary)
            package = directory/'old.deb'
            package.write_bytes(b'synthetic package')
            fixture = {'schema': 1, 'arch': 'x64', 'format': 'deb',
                       'previous': {'version': '1.2.3-rc.1', 'package': package.name,
                                    'sha256': update.sha256(package)},
                       'published': {'version': '1.2.3-rc1'}}
            update.write_json(directory/'fixture.json', fixture)
            with self.assertRaisesRegex(ValueError, 'DEB package manager'):
                update.exercise(directory)
            cli.assert_not_called()
            gui.assert_not_called()
            for path in (update.GUI_REPORT, update.REPORT):
                self.assertEqual(linux_gui.read_object(path)['status'], 'failed')

    def test_fixture_rejects_changed_bytes_and_paths_outside_the_fixture(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)/'fixture'
            root.mkdir()
            package = root/'old.deb'
            package.write_bytes(b'verified old package')
            fixture = {'previous': {'package': 'old.deb', 'sha256': update.sha256(package)}}
            self.assertEqual(update.fixture_package(root, fixture), package)
            package.write_bytes(b'different bytes')
            with self.assertRaisesRegex(ValueError, 'differs'):
                update.fixture_package(root, fixture)
            outside = Path(temporary)/'outside.deb'
            outside.write_bytes(b'verified old package')
            (root/'link.deb').symlink_to(outside)
            for name in ('../outside.deb', str(outside), 'link.deb'):
                fixture['previous']['package'] = name
                with self.subTest(name=name), self.assertRaises(ValueError):
                    update.fixture_package(root, fixture)

    def test_install_checks_refuse_a_local_development_environment(self):
        with patch.dict(os.environ, {}, clear=True), patch.object(update, 'install_previous') as install:
            with self.assertRaisesRegex(RuntimeError, 'disposable Actions'):
                update.exercise(Path('/not-a-fixture'))
            with self.assertRaisesRegex(RuntimeError, 'disposable Actions'):
                update.exercise_gui({}, Path('/not-a-package'))
        install.assert_not_called()

    @unittest.skipUnless(sys.platform == 'linux', 'requires Linux process groups')
    def test_cleanup_does_not_signal_a_reaped_child_or_foreign_group(self):
        with patch.object(os, 'waitid', side_effect=[None, ChildProcessError]), \
                patch.object(os, 'getpgid', return_value=1200), \
                patch.object(os, 'getsid', return_value=1300), \
                patch.object(os, 'killpg') as kill:
            linux_gui.stop_process_group(Mock(pid=1200, returncode=0))
            with self.assertRaisesRegex(RuntimeError, 'fixture child'):
                linux_gui.stop_process_group(Mock(pid=1200, returncode=None))
            with self.assertRaisesRegex(RuntimeError, 'reaped before cleanup'):
                linux_gui.stop_process_group(Mock(pid=1200, returncode=None))
        kill.assert_not_called()

    def test_native_fixture_handles_present_and_absent_installed_packages(self):
        package = Path('/tmp/previous-package')
        for package_format, query in (('pkg', ['pacman', '-Qq']), ('rpm', ['rpm', '-q', '--quiet'])):
            for result_code in (0, 1, 2):
                result = subprocess.CompletedProcess(query, result_code)
                with self.subTest(package_format=package_format, result_code=result_code), \
                        patch.object(update, 'invoke', return_value=result) as invoke, \
                        patch('linux_native_guest.remove_package') as remove, \
                        patch('linux_native_guest.install_package') as install:
                    if result_code == 2:
                        with self.assertRaisesRegex(ValueError, 'Could not check'):
                            update.install_previous(package_format, package)
                        install.assert_not_called()
                    else:
                        update.install_previous(package_format, package)
                        install.assert_called_once_with(package)
                    invoke.assert_called_once_with([*query, 'clash-of-rust'], check=False)
                    self.assertEqual(remove.call_count, int(result_code == 0))

    def test_deb_fixture_waits_for_competing_apt_transactions(self):
        import linux_apt
        package = Path('/tmp/previous.deb')
        with patch.dict(os.environ, {'COR_LINUX_APT_CACHE': ''}), \
                patch.object(linux_apt, 'privileged') as privileged:
            update.install_previous('deb', package)
        self.assertEqual(len(privileged.call_args_list), 2)
        for request in privileged.call_args_list:
            command = request.args[0]
            self.assertIn('DPkg::Lock::Timeout=120', command)
            self.assertIn(command[-3], ('purge', 'install'))

    @unittest.skipUnless(hasattr(os, 'geteuid'), 'sudo authorization is Linux-only')
    def test_terminal_authorization_is_scoped_and_always_removed(self):
        commands = []

        def invoke(command, **_kwargs):
            commands.append(list(map(str, command)))
            if command[:4] == ['sudo', '-n', 'visudo', '-cf']:
                self.assertEqual(Path(command[4]).read_text(), 'Defaults:corci verifypw=never\n')

        with patch('pwd.getpwuid', return_value=SimpleNamespace(pw_name='corci')), \
                patch.object(update.secrets, 'token_hex', return_value='testtoken'), \
                patch.object(update, 'invoke', side_effect=invoke):
            with self.assertRaisesRegex(RuntimeError, 'installation failed'):
                with update.terminal_authorization():
                    raise RuntimeError('installation failed')
        self.assertEqual(commands[1][:5], ['sudo', '-n', 'install', '-m', '0440'])
        self.assertEqual(commands[1][-1], '/etc/sudoers.d/cor-update-testtoken')
        self.assertEqual(commands[2], ['sudo', '-n', '-v'])
        self.assertEqual(commands[3], ['sudo', '-n', 'rm', '-f', '--',
                                       '/etc/sudoers.d/cor-update-testtoken'])

    @unittest.skipUnless(hasattr(os, 'geteuid'), 'sudo authorization is Linux-only')
    def test_terminal_authorization_removes_rule_if_validation_fails(self):
        commands = []

        def invoke(command, **_kwargs):
            commands.append(list(map(str, command)))
            if command == ['sudo', '-n', '-v']:
                raise RuntimeError('authorization failed')

        with patch('pwd.getpwuid', return_value=SimpleNamespace(pw_name='corci')), \
                patch.object(update.secrets, 'token_hex', return_value='testtoken'), \
                patch.object(update, 'invoke', side_effect=invoke):
            with self.assertRaisesRegex(RuntimeError, 'authorization failed'):
                with update.terminal_authorization():
                    self.fail('must not run without validation')
        self.assertEqual(commands[-1], ['sudo', '-n', 'rm', '-f', '--',
                                        '/etc/sudoers.d/cor-update-testtoken'])

    def test_published_update_checks_channel_versions_and_saved_data(self):
        content = b'published-package'
        digest = hashlib.sha256(content).hexdigest()
        for scenario in ('prerelease', 'stable', 'changed-settings', 'removed-setting',
                         'stale-client', 'stale-backend'):
            with self.subTest(scenario=scenario), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                old_package = root/'old.deb'
                old_package.write_bytes(b'old-package')
                prerelease = scenario != 'stable'
                version = '0.7.3-dev.2.test' if prerelease else '0.7.2'
                published = {'version': version, 'prerelease': prerelease, 'package': 'new.deb',
                             'sha256': digest, 'checksum_url': 'https://example.test/new.deb.sha256',
                             'checksum_sha256': 'a'*64}
                fixture = {'schema': 1, 'format': 'deb', 'arch': 'x64',
                           'previous': {'version': '0.7.3-dev.1.test' if prerelease else '0.7.1', 'package': 'old.deb',
                                        'sha256': update.sha256(old_package)},
                           'published': published}
                (root/'fixture.json').write_text(json.dumps(fixture))
                channel = str(prerelease).lower()

                def fake_cli(data, *arguments, **_kwargs):
                    if arguments == ('config', 'set', 'update_prereleases', channel):
                        (data/'profiles').mkdir(parents=True)
                        (data/'settings.json').write_text(json.dumps(
                            {'update_prereleases': prerelease, 'text_scale': 125, 'theme_mode': None}))
                    elif arguments[:2] == ('profile', 'add'):
                        (data/'profiles'/'fixture.yaml').write_text('rules: []\n')
                    elif arguments[:2] == ('profile', 'list'):
                        return [{'id': 'fixture'}]
                    elif arguments == ('update', 'check'):
                        return {'available': True, 'version': version, 'preview': prerelease,
                                'package': 'new.deb'}
                    elif arguments[:2] == ('update', 'download'):
                        Path(arguments[-1]).write_bytes(content)
                        return {'sha256': digest}
                    elif arguments[:2] == ('update', 'install'):
                        if scenario == 'changed-settings':
                            (data/'settings.json').write_text(json.dumps(
                                {'update_prereleases': prerelease, 'text_scale': 100, 'theme_mode': None}))
                        elif scenario == 'removed-setting':
                            (data/'settings.json').write_text(json.dumps(
                                {'update_prereleases': prerelease, 'text_scale': 125}))
                    elif arguments == ('update', 'status'):
                        return {'state': 'succeeded', 'error': False}
                    elif arguments == ('version',):
                        return {'version': 'old' if scenario == 'stale-client' else version}
                    elif arguments == ('config', 'get', 'update_prereleases'):
                        return {'update_prereleases': prerelease}
                    elif arguments == ('status',):
                        return {'version': 'old' if scenario == 'stale-backend' else version,
                                'running': False}
                    else:
                        raise AssertionError(arguments)

                def fake_download(_url, destination, **_kwargs):
                    Path(destination).write_text(digest + '\n')

                daemon = Mock()
                daemon.poll.return_value = 0
                report = root/'report.json'
                environment = {'GITHUB_ACTIONS': 'true', 'COR_LINUX_GUEST': 'true'}
                with patch.dict(os.environ, environment, clear=True), patch.object(update.sys, 'platform', 'linux'), \
                        patch.object(update.os, 'geteuid', return_value=1000, create=True), \
                        patch.object(update, 'host_arch', return_value='x64'), \
                        patch.object(update, 'REPORT', report), \
                        patch.object(update, 'install_previous'), \
                        patch.object(update, 'verify_upgrade_order'), \
                        patch.object(update, 'installed_version', side_effect=[
                            update.expected_version('deb', fixture['previous']['version']),
                            update.expected_version('deb', version)]), \
                        patch.object(update, 'wait_for', side_effect=[1234, 5678]), \
                        patch.object(update, 'terminal_authorization', return_value=nullcontext()), \
                        patch.object(update.subprocess, 'Popen', return_value=daemon), \
                        patch.object(update, 'download', side_effect=fake_download), \
                        patch.object(update, 'cli', side_effect=fake_cli):
                    if scenario in ('prerelease', 'stable'):
                        update.exercise(root)
                        self.assertEqual(json.loads(report.read_text())['status'], 'passed')
                    else:
                        with self.assertRaises(ValueError):
                            update.exercise(root)
                        self.assertEqual(json.loads(report.read_text())['status'], 'failed')

    def test_installed_versions_follow_each_native_package_manager(self):
        version = '0.7.3-dev.2.live-language'
        self.assertEqual(update.expected_version('deb', version), '0.7.3~dev.2.live-language')
        self.assertEqual(update.expected_version('rpm', version), '0.7.3~dev.2.live+language-1')
        self.assertEqual(update.expected_version('pkg', version), '0.7.3pre.dev.2.live+language-1')

    def test_graphical_update_uses_only_a_representative_hosted_deb_and_arch_guest(self):
        for distro, version, package, arch, environment, expected in (
                ('ubuntu', '24.04', 'deb', 'x64', {'RUNNER_ENVIRONMENT': 'github-hosted'}, True),
                ('ubuntu', '24.04', 'deb', 'arm64', {'RUNNER_ENVIRONMENT': 'github-hosted'}, False),
                ('ubuntu', '24.04', 'deb', 'x64', {'COR_LINUX_GUEST': 'true'}, False),
                ('ubuntu', '24.04', 'deb', 'x64', {}, False),
                ('ubuntu', '26.04', 'deb', 'x64', {'RUNNER_ENVIRONMENT': 'github-hosted'}, False),
                ('arch', '', 'pkg', 'x64', {'COR_LINUX_GUEST': 'true'}, True),
                ('arch', '', 'pkg', 'x64', {'RUNNER_ENVIRONMENT': 'github-hosted'}, False),
                ('fedora', '44', 'rpm', 'x64', {'COR_LINUX_GUEST': 'true'}, False)):
            with self.subTest(distro=distro, version=version, arch=arch, environment=environment), \
                    patch.dict(os.environ, environment, clear=True), \
                    patch.object(update.platform, 'freedesktop_os_release',
                                 return_value={'ID': distro, 'VERSION_ID': version}):
                self.assertIs(update.gui_target(package, arch), expected)

    def test_graphical_update_runs_only_after_successful_cli_on_the_hosted_target(self):
        fixture = {'schema': 1, 'format': 'deb', 'arch': 'x64', 'previous': {}, 'published': {}}
        failure = RuntimeError('CLI failed')
        for hosted, error in ((True, None), (False, None), (True, failure)):
            with self.subTest(hosted=hosted, error=error), \
                    tempfile.TemporaryDirectory() as temporary, \
                    patch.dict(os.environ, {'RUNNER_ENVIRONMENT': 'github-hosted'} if hosted else {}, clear=True), \
                    patch.object(update.platform, 'freedesktop_os_release',
                                 return_value={'ID': 'ubuntu', 'VERSION_ID': '24.04'}), \
                    patch.object(update, 'require_disposable_update_runner'), \
                    patch.object(update, 'host_arch', return_value='x64'), \
                    patch.object(update, 'verify_upgrade_order'), \
                    patch.object(update, 'exercise_cli', side_effect=error) as cli, \
                    patch.object(update, 'exercise_gui') as gui:
                root = Path(temporary)
                package = root/'old.deb'
                update.write_json(root/'fixture.json', fixture)
                expectation = self.assertRaisesRegex(RuntimeError, 'CLI failed') if error else nullcontext()
                with patch.object(update, 'GUI_REPORT', root/'published-gui-update.json'), \
                        patch.object(update, 'fixture_package', return_value=package), expectation:
                    update.exercise(root)
                check_interrupted = cli.call_args.kwargs['check_interrupted']
                identity = cli.call_args.kwargs['run_identity']
                self.assertEqual(identity['fixture_sha256'], update.sha256(root/'fixture.json'))
                cli.assert_called_once_with(fixture, package, check_interrupted=check_interrupted,
                                            run_identity=identity)
                if hosted and not error:
                    gui.assert_called_once_with(fixture, package, check_interrupted=check_interrupted,
                                                run_identity=identity)
                else:
                    gui.assert_not_called()

    def test_hosted_and_vm_published_update_deadlines_leave_time_for_cleanup(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root/'package.tar.gz'
            archive.touch()
            target = SimpleNamespace(family='arch', release='', arch='x64', distribution='Arch',
                                     os_release_id='arch', os_release_version='', full_desktop=False,
                                     matrix_entry=lambda: {'os': 'ubuntu-24.04'})
            guest = SimpleNamespace(target=target, kvm=True, architecture='x86_64',
                                    remote=Mock(), environment='cd ~/cor-source && COR_LINUX_GUEST=true ')
            fixture = root/'fixture directory'
            package_future = Mock()
            package_future.result.return_value = archive
            with patch.multiple(linux_ci, REPORT=root, fetch_compatibility_package=Mock(),
                                collect_arch_package=Mock(), run=Mock()), \
                    patch.object(linux_ci.os, 'uname', return_value=SimpleNamespace(release='test'), create=True):
                linux_ci.hosted_test(1, target, fixture)
                local = linux_ci.run.call_args
                linux_ci.verify_vm_package(guest, root, 1, package_future=package_future,
                                           fixture=fixture)
            remote = guest.remote.call_args
            remote_argv = shlex.split(remote.args[0].removeprefix(guest.environment))
            for args, outer_timeout in ((local.args[0], local.kwargs['timeout']),
                                        (remote_argv, remote.kwargs['timeout'])):
                self.assertEqual(args[:2], ['/usr/bin/timeout', '--signal=TERM'])
                grace = int(args[2].removeprefix('--kill-after='))
                deadline = int(args[3])
                self.assertEqual(deadline, 1800)
                self.assertGreaterEqual(grace, 180)
                self.assertGreater(outer_timeout, deadline + grace)
            self.assertEqual(local.args[0][-1], fixture)
            self.assertEqual(remote_argv[-1], 'target/linux-update-fixture')

    @unittest.skipUnless(sys.platform == 'linux' and Path('/usr/bin/timeout').is_file(),
                         'requires GNU timeout')
    def test_published_update_watchdog_allows_signal_delayed_cleanup(self):
        command = linux_ci.published_update_command(sys.executable, 'fixture', 1)
        code = """
import time
from linux_gui import fixture_signals

with fixture_signals() as check_interrupted:
    try:
        while True:
            check_interrupted()
            time.sleep(.02)
    finally:
        print('fixture cleaned', flush=True)
"""
        result = subprocess.run([*command[:4], sys.executable, '-B', '-c', code],
                                cwd=update.ROOT/'scripts', capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 124, result.stderr)
        self.assertEqual(result.stdout.strip(), 'fixture cleaned')

    @unittest.skipUnless(sys.platform == 'linux' and Path('/usr/bin/timeout').is_file(),
                         'requires GNU timeout')
    def test_published_update_watchdog_can_interrupt_a_stalled_installer(self):
        command = linux_ci.published_update_command(sys.executable, 'fixture', 1)
        code = """
import sys
from linux_gui import fixture_signals
from linux_update_compatibility import invoke

installer = 'import signal, time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(60)'
with fixture_signals() as check_interrupted:
    try:
        invoke([sys.executable, '-c', installer], timeout=60, check_interrupted=check_interrupted)
    finally:
        print('fixture cleaned', flush=True)
"""
        result = subprocess.run([*command[:2], '--kill-after=3', command[3], sys.executable, '-B', '-c', code],
                                cwd=update.ROOT/'scripts', capture_output=True, text=True, timeout=8)
        self.assertEqual(result.returncode, 124, result.stderr)
        self.assertEqual(result.stdout.strip(), 'fixture cleaned')

    @unittest.skipUnless(sys.platform == 'linux', 'requires Linux process groups')
    def test_command_timeout_terminates_descendants_even_after_leader_exits(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            ready, terminated = root/'ready', root/'terminated'
            descendant = """
import pathlib, signal, sys, time
ready, terminated = map(pathlib.Path, sys.argv[1:])
def stop(*_args):
    terminated.touch()
    raise SystemExit
signal.signal(signal.SIGTERM, stop)
ready.touch()
time.sleep(4)
"""
            leader = """
import pathlib, subprocess, sys, time
subprocess.Popen([sys.executable, '-B', '-c', *sys.argv[1:]])
while not pathlib.Path(sys.argv[2]).exists():
    time.sleep(.02)
"""
            with self.assertRaises(subprocess.TimeoutExpired):
                update.invoke([sys.executable, '-B', '-c', leader, descendant, ready, terminated],
                              timeout=1, check_interrupted=lambda: None)
            self.assertTrue(ready.exists(), 'descendant started')
            self.assertTrue(terminated.exists(), 'descendant received the fixture shutdown signal')

    @unittest.skipUnless(sys.platform == 'linux', 'requires Linux process groups')
    def test_exited_leader_stays_pinned_until_its_group_is_stopped(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            ready, terminated = root/'ready', root/'terminated'
            code = """
import os, pathlib, signal, sys, time
ready, terminated = map(pathlib.Path, sys.argv[1:])
if os.fork() == 0:
    def stop(*_args):
        terminated.touch()
        raise SystemExit
    signal.signal(signal.SIGTERM, stop)
    ready.touch()
    time.sleep(5)
else:
    while not ready.exists():
        time.sleep(.02)
    sys.exit(3)
"""
            with linux_gui.process([sys.executable, '-B', '-c', code, ready, terminated],
                                   subprocess.DEVNULL) as leader:
                self.assertEqual(leader.wait(timeout=2), 3)
                self.assertEqual(leader.poll(), 3)
                self.assertIsNone(leader.returncode)
                self.assertEqual(os.getsid(leader.pid), leader.pid)
            self.assertTrue(terminated.exists())
            self.assertEqual(leader.returncode, 3)

    @unittest.skipUnless(sys.platform == 'linux', 'requires Linux signal handling')
    def test_dependency_worker_interrupts_promptly_and_cleans_its_scope(self):
        with tempfile.TemporaryDirectory() as temporary:
            started = Path(temporary)/'started'
            events = []
            code = ('from pathlib import Path\nimport time\n'
                    'Path(sys.argv[1]).touch()\ntime.sleep(20)')

            original_confirm = update.confirm_installation_scope

            def confirm(child, unit, checkpoint):
                self.assertFalse(started.exists(), 'installation started before isolation')
                return original_confirm(child, unit, checkpoint)

            with patch.object(update, 'installation_command', side_effect=self.fake_installation_command), \
                    patch.object(update, 'confirm_installation_scope', side_effect=confirm), \
                    patch.object(linux_gui, 'stop_fixture_unit',
                                 side_effect=lambda _unit, **_properties: events.append('isolated install stopped')), \
                    self.assertRaisesRegex(KeyboardInterrupt, 'Interrupted by SIGTERM'), \
                    linux_gui.fixture_signals() as check_signal:
                def checkpoint():
                    if started.exists():
                        os.kill(os.getpid(), signal.SIGTERM)
                    check_signal()

                update.run_installation(code, started, check_interrupted=checkpoint)
            self.assertEqual(events, ['isolated install stopped'])

    @staticmethod
    def fake_installation_command(command, unit, _description, _timeout):
        return [sys.executable, '-B', '-c',
                'import json, os, sys\n'
                'print("/sys/fs/cgroup/system.slice/" + sys.argv[1], flush=True)\n'
                'payload = sys.stdin.read()\n'
                'if payload: os.execvpe(sys.argv[2], sys.argv[2:], json.loads(payload))',
                unit, *command]

    @unittest.skipUnless(sys.platform == 'linux', 'requires Linux process groups')
    def test_installation_never_runs_when_scope_attachment_fails(self):
        with tempfile.TemporaryDirectory() as temporary:
            started = Path(temporary)/'started'
            code = 'from pathlib import Path\nPath(sys.argv[1]).touch()'
            with patch.object(update, 'installation_command', side_effect=self.fake_installation_command), \
                    patch.object(update, 'confirm_installation_scope',
                              side_effect=TimeoutError('not attached')), \
                    patch.object(linux_gui, 'stop_fixture_unit') as stop, \
                    self.assertRaisesRegex(TimeoutError, 'not attached'):
                update.run_installation(code, started, check_interrupted=lambda: None)
            self.assertFalse(started.exists())
            self.assertRegex(stop.call_args.args[0], r'^cor-install-[a-f0-9]{32}\.scope$')
            self.assertRegex(stop.call_args.kwargs['Description'], r' [a-f0-9]{32}$')

    @unittest.skipUnless(sys.platform == 'linux', 'requires Linux process groups')
    def test_installation_scope_stops_only_its_verified_unit_and_checks_for_survivors(self):
        unit = 'cor-install-test.scope'
        events = Path('/sys/fs/cgroup/system.slice')/unit/'cgroup.events'

        def read(path):
            self.assertEqual(path, events)
            return 'populated 1\n'

        with patch('linux_gui.stop_fixture_unit') as stop, \
                patch('linux_gui.stop_process_group') as stop_group, \
                patch.object(update.secrets, 'token_hex', return_value='test'), \
                patch.object(update, 'installation_command', side_effect=self.fake_installation_command), \
                patch.object(Path, 'read_text', read), \
                self.assertRaisesRegex(RuntimeError, 'still contains processes'), ExitStack() as scope:
            update.invoke([sys.executable, '-B', '-c', 'pass'], installation_scope=scope)
            stop.assert_not_called()
        stop.assert_called_once_with(unit, Description='Clash of Rust installation fixture test')
        stop_group.assert_not_called()

    @unittest.skipUnless(sys.platform == 'linux', 'requires Linux cgroup paths')
    def test_installation_gate_recognizes_unified_hybrid_and_legacy_cgroups(self):
        unit = 'cor-install-test.scope'
        group = '/system.slice/'+unit
        command = update.installation_command(['unused'], unit, 'Test scope', 60)
        code = command[command.index('-c')+1]
        for membership, mount in (('0::', ''), ('0::', 'unified'), ('1:name=systemd:', 'systemd')):
            directory = Path('/sys/fs/cgroup')/mount/group.lstrip('/')
            output_stream = io.StringIO()
            with self.subTest(mount=mount), \
                    patch('builtins.open', mock_open(read_data=membership+group+'\n')), \
                    patch.object(Path, 'is_file', autospec=True,
                                 side_effect=lambda path: path == directory/'cgroup.procs'), \
                    patch.object(Path, 'read_text', return_value=str(os.getpid())+'\n'), \
                    patch.object(sys, 'argv', ['-c', unit, 'unused']), \
                    patch.object(sys, 'stdin', io.StringIO()), redirect_stdout(output_stream), \
                    self.assertRaises(SystemExit) as stopped:
                exec(code, {})
            self.assertEqual(stopped.exception.code, 125)
            self.assertEqual(output_stream.getvalue().strip(), str(directory))

    @unittest.skipUnless(sys.platform == 'linux' and Path('/usr/bin/timeout').is_file(),
                         'requires GNU timeout')
    def test_published_update_watchdog_leaves_in_flight_cleanup_running(self):
        command = linux_ci.published_update_command(sys.executable, 'fixture', 1)
        code = """
import subprocess, sys
from linux_gui import cleanup_steps, fixture_signals

def cleanup_rule():
    subprocess.run([sys.executable, '-c', 'import time; time.sleep(1.5); print("rule removed")'],
                   start_new_session=True, check=True, timeout=3)

with fixture_signals():
    cleanup_steps(('remove rule', cleanup_rule),
                  ('remove account', lambda: print('account removed', flush=True)))
"""
        result = subprocess.run([*command[:2], '--kill-after=3', command[3], sys.executable, '-B', '-c', code],
                                cwd=update.ROOT/'scripts', capture_output=True, text=True, timeout=8)
        self.assertEqual(result.returncode, 124, result.stderr)
        self.assertEqual(result.stdout.splitlines(), ['rule removed', 'account removed'])

    @unittest.skipUnless(sys.platform == 'linux', 'requires Linux signal handling')
    def test_interrupted_cli_cannot_begin_graphical_installation(self):
        def interrupt(*_args, **_kwargs):
            os.kill(os.getpid(), signal.SIGTERM)

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixture = {'schema': 1, 'format': 'deb', 'arch': 'x64', 'previous': {}, 'published': {}}
            update.write_json(root/'fixture.json', fixture)
            with patch.object(update, 'exercise_cli', side_effect=interrupt), \
                    patch.object(update, 'require_disposable_update_runner'), \
                    patch.object(update, 'host_arch', return_value='x64'), \
                    patch.object(update, 'fixture_package', return_value=root/'previous.deb'), \
                    patch.object(update, 'verify_upgrade_order'), \
                    patch.object(update, 'GUI_REPORT', root/'published-gui-update.json'), \
                    patch.object(update, 'gui_target', return_value=True), \
                    patch.object(update, 'exercise_gui') as gui, \
                    self.assertRaisesRegex(KeyboardInterrupt, 'Interrupted by SIGTERM'):
                update.exercise(root)
        gui.assert_not_called()

    def test_gui_update_language_overrides_runner_messages_and_preserves_utf8(self):
        with patch.dict(os.environ, {'LANG': 'C.UTF-8', 'LC_ALL': 'en_US.UTF-8', 'LANGUAGE': 'en'}, clear=True):
            env = linux_gui.update_gui_environment(Path('/isolated-downloads'))
        self.assertEqual((env['LANG'], env['LC_ALL'], env['LC_MESSAGES'], env['LANGUAGE']),
                         ('C.UTF-8', '', 'zh_CN.UTF-8', 'zh_CN'))
        self.assertEqual(env['TMPDIR'], '/isolated-downloads')
        self.assertEqual(env['SLINT_SCALE_FACTOR'], '1')

    def test_gui_labels_combine_word_geometry_and_match_the_published_version(self):
        lines = linux_gui.recognized_lines(
            'level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n'
            '5\t1\t1\t1\t1\t1\t10\t30\t24\t14\t90\t检查\n'
            '5\t1\t1\t1\t1\t2\t36\t30\t24\t14\t90\t更新\n'
            '5\t1\t2\t1\t1\t1\t50\t100\t26\t14\t90\t更新\n'
            '5\t1\t3\t1\t1\t1\t60\t130\t24\t14\t90\t发现\n'
            '5\t1\t3\t1\t1\t2\t86\t130\t200\t14\t90\t0.7.3–dev.2.live-language\n')
        labels = linux_gui.UPDATE_LABELS
        self.assertEqual(linux_gui.screen_label(lines, labels['check']), (35, 37))
        self.assertEqual(linux_gui.screen_label(lines, labels['updates']), (63, 107))
        self.assertIs(linux_gui.screen_label(lines, (self.version,), contains=True), True)
        self.assertIsNone(linux_gui.screen_label(lines, labels['download']))

    def test_gui_ocr_uses_control_geometry_and_keeps_confirmation_countdown_distinct(self):
        labels = linux_gui.UPDATE_LABELS
        lines = [[('外观', (280, 61, 26, 14)), ('更', (341, 61, 12, 14)),
                  ('新', (354, 61, 13, 14)), ('计划任务', (394, 61, 54, 14))],
                 [('Check', (990, 630, 36, 14)), ('for', (1030, 630, 18, 14)),
                  ('updates', (1053, 630, 51, 14))]]
        self.assertEqual(linux_gui.screen_label(lines, labels['updates']), (354, 68))
        self.assertEqual(linux_gui.screen_label(lines, labels['check']), (1047, 637))
        for countdown in ([('继续安装', (700, 520, 46, 14)), ('（2秒）', (750, 520, 36, 14))],
                          [('Continue', (700, 520, 52, 14)), ('(2s)', (755, 520, 22, 14))]):
            self.assertIsNone(linux_gui.screen_label([countdown], labels['continue']))

    def test_gui_discovery_matches_the_complete_version_with_ocr_spacing(self):
        for words, version, matches in (
                (['发现预发布版', '0.7.3', '–', 'dev.2.live-language'], self.version, True),
                (['Version', 'v'+self.version, '·', 'Preview'], self.version, True),
                (['Version', self.version+'.1'], self.version, False),
                (['Version', self.version, '. 1'], self.version, False),
                (['Version', self.version+'-extra'], self.version, False),
                (['Version', self.version+'+build.01'], self.version, False),
                (['Version', self.version+'+build.01'], self.version+'+build.01', True),
                (['Version', '10.7.3-dev.2.live-language'], self.version, False),
                (['Version', '1', '.', self.version], self.version, False),
                (['Version', 'Version', '–', self.version], self.version, True),
                (['Version', '0.7.3-Dev.2.live-language'], self.version, False),
                (['Version', self.version], '0.7.3', False)):
            with self.subTest(words=words, version=version):
                lines = [[(word, (index*50, 60, 45, 14)) for index, word in enumerate(words)]]
                self.assertIs(linux_gui.screen_version(lines, version), matches)

    def test_gui_navigation_resets_a_scrolled_legacy_page_when_the_update_tab_is_unreadable(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            navigation = linux_gui.UpdateNavigation(directory, self.version)
            navigation.window = 42
            navigation.geometry = (0, 0, 1280, 900)
            frames = iter([
                [[('设置', (240, 61, 30, 14))]],
                [[('更新', (600, 290, 28, 14))]],
                [[('常规', (700, 280, 25, 14))]],
                [[('检查', (1160, 730, 24, 14)), ('更新', (1185, 730, 24, 14))]],
            ])

            def capture():
                navigation.lines = next(frames)
                return True

            with patch.object(navigation, 'capture', side_effect=capture), \
                    patch.object(navigation, 'focus', return_value=42), \
                    patch.object(linux_gui, 'run') as invoke, patch.object(linux_gui.time, 'sleep'):
                navigation.check()

            events = json.loads((directory/'update-navigation.json').read_text())
            self.assertEqual([(entry['step'], entry['result']) for entry in events],
                             [('open-settings', 'sent'), ('settings-page', 'visible'),
                              ('reset-settings-scroll', 'sent'),
                              ('scroll-settings', 'sent'), ('check-for-updates', 'sent')])
            self.assertEqual(events[0]['point'], [56, 838])
            self.assertEqual(events[2]['button'], 4)
            self.assertEqual(events[3]['button'], 5)
            self.assertEqual(events[-1]['point'], [1184, 737])
            self.assertEqual(events[-1]['focused'], 42)
            self.assertEqual(invoke.call_count, 4)

    def test_gui_navigation_falls_back_to_the_compact_sidebar(self):
        with tempfile.TemporaryDirectory() as temporary:
            navigation = linux_gui.UpdateNavigation(Path(temporary), self.version)
            navigation.geometry = (0, 0, 1280, 900)
            frame = [[('设置', (84, 61, 28, 14)), ('更新', (275, 61, 28, 14))],
                     [('检查更新', (1140, 730, 58, 14))]]
            compact = False

            def click(step, _point):
                nonlocal compact
                if step == 'open-compact-settings':
                    compact = True

            def capture():
                navigation.lines = frame if compact else []
                return True

            def poll(description, probe, timeout=45):
                result = probe()
                if not result:
                    raise TimeoutError(description)
                return result

            with patch.object(navigation, 'capture', side_effect=capture), \
                    patch.object(navigation, 'input', side_effect=click) as send, \
                    patch.object(linux_gui, 'wait_for', side_effect=poll):
                navigation.check()

            self.assertEqual([entry.args for entry in send.call_args_list],
                             [('open-settings', (56, 838)), ('open-compact-settings', (30, 865)),
                              ('open-updates', (289, 68)), ('check-for-updates', (1169, 737))])

    def test_gui_reopen_verifies_two_page_headers_with_navigation_only(self):
        navigation = linux_gui.UpdateNavigation(Path('unused'), self.version)
        navigation.geometry = (0, 0, 1280, 900)
        for label, home in (((65, 124), (65, 124)), (None, (30, 124))):
            with self.subTest(home=home), patch.multiple(
                    navigation, input=Mock(), wait=Mock(), locate=Mock(return_value=label)):
                navigation.verify_responsive()
                self.assertEqual([call.args for call in navigation.input.call_args_list],
                                 [('open-settings', (56, 838)), ('return-home', home)])
                self.assertEqual([call.args[0] for call in navigation.wait.call_args_list],
                                 ['settings-page', 'home-page'])
                self.assertTrue(all(not call.kwargs.get('click') and not call.kwargs.get('scroll')
                                    for call in navigation.wait.call_args_list))

    def test_gui_navigation_never_captures_or_clicks_a_foreign_window(self):
        with tempfile.TemporaryDirectory() as temporary:
            navigation = linux_gui.UpdateNavigation(Path(temporary), self.version)
            navigation.window = 42
            navigation.geometry = (0, 0, 1280, 900)
            with patch.object(navigation, 'focus', return_value=77), patch.object(linux_gui, 'run') as invoke:
                self.assertFalse(navigation.capture())
                with self.assertRaisesRegex(RuntimeError, 'outside the focused application window'):
                    navigation.input('open-settings', (56, 838))
                invoke.assert_not_called()
            self.assertEqual(navigation.events[-1]['result'], 'rejected')
            captured = []

            def capture(command, **_kwargs):
                image = command[-1]
                self.assertEqual(image.parent.parent, navigation.report)
                self.assertEqual(image.parent.stat().st_mode & 0o777, 0o700)
                image.write_bytes(b'foreign frame')
                captured.append(image)

            with patch.object(navigation, 'focus', side_effect=[42, 77]), \
                    patch.object(linux_gui, 'run', side_effect=capture), \
                    patch.object(linux_gui, 'recognize_update_screen') as recognize:
                self.assertFalse(navigation.capture())
                self.assertFalse(captured[0].exists())
                recognize.assert_not_called()

    def test_gui_navigation_timeout_preserves_only_sanitized_evidence(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            source = directory/'fixture'
            source.mkdir()
            navigation = linux_gui.UpdateNavigation(source, self.version)
            navigation.window = 42
            navigation.geometry = (0, 0, 1280, 900)
            navigation.phase = 'cancel'
            navigation.step = 'settings-page'
            for name in ('update-screen.png', 'update.log', 'authentication.log', 'desktop.log'):
                (source/name).write_text('raw fixture evidence')
            navigation.lines = [[('SETTINGS', (240, 61, 55, 14)),
                                 ('private-fixture-value', (350, 61, 145, 14)),
                                 ('for', (520, 61, 18, 14))]]
            failure = TimeoutError('settings page was not visible')

            def redact(size, destination, words):
                self.assertEqual(size, (1280, 900))
                self.assertEqual([word['text'] for word in words],
                                 ['Settings', '<redacted>', '<redacted>'])
                destination.write_bytes(b'masked fixture image')

            public = directory/'public'/'desktop-update-this-run'
            with patch.object(navigation, 'focus', return_value=42), \
                    patch.object(linux_gui, 'redact_update_image', side_effect=redact):
                with self.assertRaises(TimeoutError) as outcome, navigation.diagnose_failure():
                    raise failure
                self.assertIs(outcome.exception, failure)
                linux_gui.preserve_report(source, update_report=public)
            self.assertEqual({path.name for path in public.iterdir()},
                             {'update-navigation-failure.png', 'update-navigation.json'})
            self.assertEqual((public/'update-navigation-failure.png').read_bytes(), b'masked fixture image')
            evidence = (public/'update-navigation.json').read_text()
            self.assertNotIn('private-fixture-value', evidence)
            self.assertEqual(json.loads(evidence)[-1]['step'], 'settings-page')

    def test_gui_prompt_requires_the_current_installation_and_fails_on_early_outcome(self):
        status_file = Path('unused')
        locate = Mock(return_value='42')
        previous = {'directory': 'previous', 'state': 'failed', 'version': self.version, 'updated_at': 10}
        attempts = {'previous': previous}
        current = {'directory': 'current', 'state': 'installing', 'version': self.version}
        with patch.object(linux_gui, 'read_object',
                          side_effect=[previous, dict(current, state='preparing'),
                                       current, dict(current, state='failed', message='private detail')]):
            self.assertIsNone(linux_gui.pending_gui_authorization(
                status_file, attempts, locate, version=self.version))
            self.assertIsNone(linux_gui.pending_gui_authorization(
                status_file, attempts, locate, version=self.version))
            locate.assert_not_called()
            self.assertEqual(linux_gui.pending_gui_authorization(
                status_file, attempts, locate, version=self.version), ('42', current))
            with self.assertRaisesRegex(RuntimeError, 'ended before the expected authorization prompt') as error:
                linux_gui.pending_gui_authorization(status_file, attempts, locate, version=self.version)
            self.assertNotIn('private detail', str(error.exception))
            locate.assert_called_once()

    def test_gui_never_types_or_cancels_when_the_verified_prompt_loses_focus(self):
        for allow in (False, True):
            with self.subTest(allow=allow), patch.object(linux_gui, 'run') as invoke, \
                    patch.object(linux_gui, 'output', return_value='77'), \
                    patch.object(linux_gui.time, 'sleep'):
                credential = Mock()
                credential.read_text.return_value = 'syntheticFixtureValue'
                with self.assertRaisesRegex(RuntimeError, 'prompt lost focus'):
                    linux_gui.respond_authorization('42', credential, allow)
                self.assertNotIn('type', [entry.args[0][1] for entry in invoke.call_args_list])
                self.assertNotIn('key', [entry.args[0][1] for entry in invoke.call_args_list])

    def test_same_directory_authorization_requires_a_new_attempt(self):
        previous = {'directory': 'same', 'state': 'failed', 'version': self.version, 'updated_at': 10}
        attempts = {'same': previous, 'another': dict(previous, directory='another', updated_at=12)}
        current = dict(previous, state='installing', updated_at=13)
        locate = Mock(return_value='prompt')
        with patch.object(linux_gui, 'read_object', side_effect=[previous,
                          dict(current, updated_at=10), dict(current, updated_at=None),
                          dict(current, version='0.7.2'), current]):
            for _ in range(4):
                self.assertIsNone(linux_gui.pending_gui_authorization(
                    Path('unused'), attempts, locate, version=self.version))
            locate.assert_not_called()
            self.assertEqual(linux_gui.pending_gui_authorization(
                Path('unused'), attempts, locate, version=self.version), ('prompt', current))

    def test_legacy_same_directory_retry_without_attempt_identity_is_rejected(self):
        previous = {'directory': 'same', 'state': 'failed', 'version': self.version}
        current = dict(previous, state='installing', updated_at=13)
        locate = Mock(return_value='new prompt')
        with patch.object(linux_gui, 'read_object', return_value=current):
            self.assertIsNone(linux_gui.pending_gui_authorization(
                Path('unused'), {'same': previous}, locate, version=self.version))
        locate.assert_not_called()

    def test_gui_dismisses_a_failure_before_retry_and_supports_legacy_records(self):
        with tempfile.TemporaryDirectory() as temporary:
            navigation = linux_gui.UpdateNavigation(Path(temporary), self.version)
            navigation.window = 42
            result = {'directory': 'current', 'acknowledged': False}
            with patch.object(navigation, 'focus', return_value=42), \
                    patch.object(linux_gui, 'run') as invoke, \
                    patch.object(linux_gui.time, 'sleep'), \
                    patch.object(linux_gui, 'read_object',
                                 side_effect=[{'directory': 'previous', 'acknowledged': True},
                                              dict(result, acknowledged=True)]) as read:
                navigation.dismiss_failure(Path('unused'), result)
                self.assertEqual(read.call_count, 2)
                navigation.dismiss_failure(Path('unused'), {'directory': 'legacy'})
                self.assertEqual(read.call_count, 2)
                self.assertEqual(invoke.call_count, 2)
            self.assertEqual([entry['result'] for entry in navigation.events], ['dismissed']*2)

    def test_gui_session_releases_the_observed_lock_before_recovery_and_retries_with_a_new_owner(self):
        checks, events = self.simulate_gui_update_session()
        self.assertEqual(events, ['observe-cancel', 'respond-cancel', 'dismiss-cancel', 'verify-cancel',
                                  'observe-locked', 'respond-locked', 'observe-release',
                                  'dismiss-locked', 'verify-locked',
                                  'observe-retry', 'respond-retry', 'verify-retry'])
        for phase in ('cancel', 'locked'):
            self.assertTrue(checks[phase]['original_gui_owner_retained'])
            self.assertTrue(checks[phase]['core_still_stopped_after_failure'])
        self.assertTrue(checks['retry']['gui_reopened'])
        self.assertTrue(checks['retry']['core_started_and_stopped_by_fixture_after_reopen'])
        self.assertNotIn('core_still_stopped_after_failure', checks['retry'])

    def test_gui_session_rejects_replacing_the_owner_on_cancel_or_lock_failure(self):
        for replaced_phase in ('cancel', 'locked'):
            with self.subTest(phase=replaced_phase):
                with self.assertRaisesRegex(RuntimeError, 'replaced the original GUI owner'):
                    self.simulate_gui_update_session(replaced_phase=replaced_phase)

    def test_gui_session_rejects_a_visible_but_unresponsive_reopened_window(self):
        with self.assertRaisesRegex(TimeoutError, 'renderer stopped responding'):
            self.simulate_gui_update_session(frozen=True)

    def test_gui_session_rejects_a_reused_pkexec_request(self):
        with self.assertRaisesRegex(RuntimeError, 'reused a previous pkexec request'):
            self.simulate_gui_update_session(reused_pkexec=True)

    def simulate_gui_update_session(self, *, replaced_phase=None, frozen=False, reused_pkexec=False):
        case = {'format': 'deb', 'published': {'version': self.version, 'sha256': 'a'*64,
                                              'prerelease': True},
                'previous_version': '0.7.2', 'old_native_version': '0.7.2',
                'new_native_version': '0.7.3~dev.2.live-language'}
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ):
            home = Path(temporary)
            report = home/'report'
            report.mkdir()
            data = report/'profile'
            status_file = data/'update-install.json'
            navigation = Mock()
            checks, events = {}, []
            owner, dialog, version = 41, None, case['previous_version']

            @contextmanager
            def process(*_args, reap_for_restart):
                self.assertTrue(reap_for_restart)
                update.write_json(data/'control.json', {'owner': 'gui', 'pid': owner})
                yield Mock()

            def cli(*args):
                if args[:2] == ('profile', 'add'):
                    profile = data/'profiles'/'fixture.yaml'
                    profile.parent.mkdir()
                    profile.write_text('fixture')
                elif args[:2] == ('profile', 'list'):
                    return [{'id': 'fixture'}]
                elif args[0] == 'status':
                    return {'owner': 'gui', 'running': False, 'version': version}
                elif args[0] == 'version':
                    return {'version': version}
                else:
                    self.fail(f'Unexpected CLI request {args}')

            def navigate(step, *_args, **_kwargs):
                nonlocal dialog
                if step == 'confirm-prerelease':
                    dialog = f'prompt-{navigation.phase}'
                    update.write_json(status_file, pending())

            def pending():
                return {'directory': navigation.phase, 'state': 'installing', 'version': self.version}

            def record(path, value):
                update.write_json(path, value)
                if path.name == 'update-observation-request.json':
                    phase = value['phase']
                    events.append(f'observe-{phase}')
                    acknowledgement = {'phase': phase}
                    if phase == 'release':
                        acknowledgement['lock_conflict'] = 'observed lock failure'
                    update.write_json(home/'update-observation-state.json', acknowledgement)

            def respond(window, _credential, allow):
                nonlocal owner, dialog, version
                phase = navigation.phase
                self.assertEqual((window, allow, events[-1]), (dialog, phase != 'cancel', f'observe-{phase}'))
                events.append(f'respond-{phase}')
                dialog = None
                result = pending()
                if phase == 'retry':
                    result.update(state='succeeded', error=False)
                    owner = 42
                    version = self.version
                    update.write_json(data/'control.json', {'owner': 'gui', 'pid': owner})
                else:
                    message = '授权已取消' if phase == 'cancel' else '安装失败'
                    result.update(state='failed', error=True, message=message)
                    if phase == replaced_phase:
                        owner = 42
                        update.write_json(data/'control.json', {'owner': 'gui', 'pid': owner})
                update.write_json(status_file, result)

            def verify(_format, native):
                events.append(f'verify-{navigation.phase}')
                self.assertEqual(native, case['new_native_version' if navigation.phase == 'retry'
                                              else 'old_native_version'])
                self.assertEqual(owner, 42 if navigation.phase == 'retry' else 41)

            def poll(description, probe, timeout=45):
                result = probe()
                self.assertTrue(result, description)
                return result

            navigation.diagnose_failure.side_effect = nullcontext
            navigation.wait.side_effect = navigate
            navigation.dismiss_failure.side_effect = lambda *_args: events.append(f'dismiss-{navigation.phase}')
            if frozen:
                navigation.verify_responsive.side_effect = TimeoutError('renderer stopped responding')
            with patch.object(linux_gui.time, 'sleep'), \
                    patch.multiple(linux_gui, process=process, cli=cli, write_json=record, wait_for=poll,
                                available_mixed_port=Mock(return_value=10000),
                                UpdateNavigation=Mock(return_value=navigation), stopped_gui_core=Mock(),
                                inspect=Mock(return_value=[{'id': '0x5'}]), output=Mock(return_value='123'),
                                _renderer=Mock(return_value=True), verified_gui_download=Mock(),
                                verify_gui_pkexec=Mock(side_effect=lambda *_: 40 if reused_pkexec else {
                                    'cancel': 40, 'locked': 41, 'retry': 42}[navigation.phase]),
                                process_identity=Mock(side_effect=lambda pid: (pid, 10)),
                                respond_authorization=respond,
                                verify_installed_package=verify):
                linux_gui.gui_update_session(report, case, checks, lambda: dialog, Mock(), Path('unused'))

            navigation.check.assert_called_once_with()
            navigation.wait_for_published_version.assert_called_once_with()
            navigation.verify_responsive.assert_called_once_with()
            return checks, events

    def test_gui_update_report_records_failing_stage_without_command_details(self):
        fixture = {'format': 'deb', 'arch': 'x64', 'previous': {}, 'published': {}}
        with tempfile.TemporaryDirectory() as temporary:
            report = Path(temporary)/'published-gui-update.json'
            failure = RuntimeError('raw subprocess output')
            failure.add_note('raw exception note')

            def install(*_packages, **_kwargs):
                receipt = json.loads(report.read_text())
                self.assertEqual(receipt['status'], 'running')
                self.assertTrue((report.parent/receipt['desktop_report']).is_dir())
                raise failure

            update.write_json(report, {'status': 'passed', 'desktop_report': 'previous-run'})
            with patch.object(update, 'GUI_REPORT', report), \
                    patch.object(update, 'require_disposable_update_runner'), \
                    patch.object(update, 'gui_target', return_value=True), \
                    patch.object(update, 'run_installation', side_effect=install), self.assertRaises(RuntimeError):
                update.exercise_gui(fixture, Path(temporary)/'old.deb')
            result = report.read_text()
            self.assertNotIn('raw', result)
            self.assertEqual(json.loads(result)['failed_step'], 'prepare-desktop')
            self.assertEqual(json.loads(result)['error'], 'RuntimeError')

    def test_gui_update_receipt_cannot_reuse_an_old_desktop_report(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = root/'published-gui-update.json'
            legacy = root/'desktop-update'
            legacy.mkdir()
            old_report = {'status': 'passed', 'checks': {'version_visible_in_gui': True,
                          'cancel': {'outcome': 'failed'},
                          'locked': {'outcome': 'failed', 'native_lock_conflict': 'old trace'},
                          'retry': {'outcome': 'succeeded', 'gui_reopened': True}}}
            update.write_json(legacy/'gui.json', old_report)
            (legacy/'update.log').write_text('old raw log')
            fixture = {'format': 'deb', 'arch': 'x64', 'previous': {'version': '0.7.2'},
                       'published': {'version': self.version, 'sha256': 'a'*64,
                                     'checksum_url': 'unused', 'checksum_sha256': 'b'*64}}

            def checksum(_url, destination, **_kwargs):
                destination.write_text('a'*64+'\n')

            with patch.object(update, 'GUI_REPORT', report), \
                    patch.object(update, 'require_disposable_update_runner'), \
                    patch.object(update, 'gui_target', return_value=True), \
                    patch.object(update, 'download', side_effect=checksum), \
                    patch.object(update, 'run_installation'), patch.object(update, 'install_previous'), \
                    patch.object(linux_gui, 'verify_installed_package'), \
                    patch.object(linux_gui, 'installed_tests') as installed:
                with self.assertRaises(FileNotFoundError):
                    update.exercise_gui(fixture, root/'old.deb')
            receipt = json.loads(report.read_text())
            current = installed.call_args.kwargs['update_report']
            self.assertEqual(receipt['desktop_report'], current.relative_to(root).as_posix())
            self.assertEqual(receipt['status'], 'failed')
            self.assertEqual(list(current.iterdir()), [])
            self.assertEqual(linux_gui.read_object(legacy/'gui.json'), old_report)
            self.assertEqual((legacy/'update.log').read_text(), 'old raw log')

    def test_gui_report_rejects_wrong_exercise_fixture_version_or_user_data(self):
        checks, _ = self.simulate_gui_update_session()
        identity = {'exercise_id': 'current', 'fixture_sha256': 'b'*64}
        report = {**identity, 'status': 'passed', 'checks': checks}

        def validate(value):
            return update.verify_desktop_receipt(
                value, identity, 'deb', 'a'*64, '0.7.2', '0.7.3~dev.2.live-language')

        self.assertIs(validate(report), checks)
        for field, wrong in (('exercise_id', 'old-attempt'), ('fixture_sha256', 'c'*64)):
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'this exercise'):
                validate(dict(report, **{field: wrong}))
        for field, wrong in (('installed_native_version', '0.7.2'), ('download_sha256', 'b'*64),
                             ('settings_preserved', False), ('subscription_preserved', False),
                             ('gui_usable', False)):
            value = copy.deepcopy(report)
            value['checks']['retry'][field] = wrong
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'evidence for retry'):
                validate(value)

    def test_gui_receipt_records_cli_failure_instead_of_leaving_an_old_success(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = root/'published-gui-update.json'
            previous_report = root/'desktop-update-old'/'gui.json'
            update.write_json(previous_report, {'status': 'passed'})
            update.write_json(report, {'status': 'passed', 'desktop_report': previous_report.parent.name})
            fixture = {'schema': 1, 'format': 'deb', 'arch': 'x64', 'previous': {}, 'published': {}}
            update.write_json(root/'fixture.json', fixture)

            def fail_cli(*_args, **_kwargs):
                receipt = linux_gui.read_object(report)
                self.assertEqual(receipt['status'], 'running')
                self.assertNotIn('desktop_report', receipt)
                raise RuntimeError('terminal test failed')

            with patch.object(update, 'GUI_REPORT', report), \
                    patch.object(update, 'require_disposable_update_runner'), \
                    patch.object(update, 'host_arch', return_value='x64'), \
                    patch.object(update, 'fixture_package', return_value=root/'old.deb'), \
                    patch.object(update, 'verify_upgrade_order'), \
                    patch.object(update, 'gui_target', return_value=True), \
                    patch.object(update, 'exercise_cli', side_effect=fail_cli), \
                    patch.object(update, 'exercise_gui') as gui, \
                    self.assertRaisesRegex(RuntimeError, 'terminal test failed'):
                update.exercise(root)
            gui.assert_not_called()
            receipt = linux_gui.read_object(report)
            self.assertEqual(receipt['status'], 'failed')
            self.assertEqual(receipt['failed_step'], 'terminal-update')
            self.assertNotIn('desktop_report', receipt)
            self.assertEqual(linux_gui.read_object(previous_report), {'status': 'passed'})

    @unittest.skipUnless(sys.platform == 'linux', 'requires Linux signal handling')
    def test_interrupt_during_final_gui_write_invalidates_the_current_receipts(self):
        fixture = {'schema': 1, 'format': 'deb', 'arch': 'x64', 'previous': {'version': '1.0.0'},
                   'published': {'version': '2.0.0', 'sha256': 'a'*64,
                                 'checksum_url': 'unused', 'checksum_sha256': 'b'*64}}
        write = update.write_receipt
        for number in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            with self.subTest(signal=number.name), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                update.write_json(root/'fixture.json', fixture)
                stale = root/'desktop-update-old'/'gui.json'
                update.write_json(stale, {'exercise_id': 'stale', 'status': 'passed'})

                def publish(path, receipt):
                    write(path, receipt)
                    if path == update.GUI_REPORT and receipt['status'] == 'passed':
                        os.kill(os.getpid(), number)

                def cli(_fixture, _package, *, run_identity, **_kwargs):
                    write(update.REPORT, {**run_identity, 'status': 'passed'})

                def desktop(*, update_case, update_report, **_kwargs):
                    update.write_json(update_report/'gui.json', {**update_case, 'status': 'passed'})

                def checksum(_url, destination, **_kwargs):
                    destination.write_text('a'*64+'\n')

                with patch.multiple(update, REPORT=root/'cli.json', GUI_REPORT=root/'gui.json',
                                    require_disposable_update_runner=Mock(), host_arch=Mock(return_value='x64'),
                                    gui_target=Mock(return_value=True), fixture_package=Mock(),
                                    verify_upgrade_order=Mock(), exercise_cli=cli, write_receipt=publish,
                                    run_installation=Mock(), install_previous=Mock(), download=checksum,
                                    verify_desktop_receipt=Mock(return_value={})), \
                        patch.multiple(linux_gui, installed_tests=desktop, verify_installed_package=Mock()), \
                        self.assertRaisesRegex(KeyboardInterrupt, f'Interrupted by {number.name}'):
                    update.exercise(root)

                gui = linux_gui.read_object(root/'gui.json')
                for receipt in (gui, linux_gui.read_object(root/'cli.json'),
                                linux_gui.read_object(root/gui['desktop_report']/'gui.json')):
                    self.assertEqual(receipt['exercise_id'], gui['exercise_id'])
                    self.assertEqual((receipt['status'], receipt['failed_step']), ('failed', 'interrupted'))
                self.assertEqual(linux_gui.read_object(stale), {'exercise_id': 'stale', 'status': 'passed'})

    @unittest.skipUnless(sys.platform == 'linux', 'requires Linux signal handling')
    def test_interrupt_while_restoring_signal_handlers_invalidates_the_receipt(self):
        identity = {'exercise_id': 'current'}
        original_signal = signal.signal
        original_handler = signal.getsignal(signal.SIGTERM)

        def restore_with_pending_signal(number, handler):
            if number == signal.SIGTERM and handler == original_handler:
                os.kill(os.getpid(), number)
            return original_signal(number, handler)

        with patch.object(signal, 'signal', side_effect=restore_with_pending_signal), \
                self.assertRaisesRegex(KeyboardInterrupt, 'Interrupted by SIGTERM'), \
                linux_gui.fixture_signals(lambda error: update.interrupt_receipts(identity, error)):
            update.write_receipt(update.REPORT, {**identity, 'status': 'passed'})
        receipt = linux_gui.read_object(update.REPORT)
        self.assertEqual((receipt['status'], receipt['failed_step']), ('failed', 'interrupted'))
        self.assertEqual(signal.getsignal(signal.SIGTERM), original_handler)

    def test_interrupt_preserves_a_receipt_owned_by_another_exercise(self):
        identity = {'exercise_id': 'current'}
        other = {'exercise_id': 'other', 'status': 'passed'}
        update.write_receipt(update.REPORT, other)
        update.write_receipt(update.GUI_REPORT, {**identity, 'status': 'passed'})
        update.interrupt_receipts(identity, KeyboardInterrupt())
        self.assertEqual(linux_gui.read_object(update.REPORT), other)
        receipt = linux_gui.read_object(update.GUI_REPORT)
        self.assertEqual((receipt['status'], receipt['failed_step']), ('failed', 'interrupted'))

    def test_invalid_fixture_replaces_an_old_success_before_parsing(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            gui, cli = root/'gui.json', root/'cli.json'
            for report in (gui, cli):
                update.write_json(report, {'status': 'passed', 'checks': {'stale': True}})
            (root/'fixture.json').write_text('invalid private fixture')
            with patch.multiple(update, REPORT=cli, GUI_REPORT=gui,
                                require_disposable_update_runner=Mock()), \
                    self.assertRaises(json.JSONDecodeError):
                update.exercise(root)
            for report in (gui, cli):
                receipt = linux_gui.read_object(report)
                self.assertEqual(receipt['status'], 'failed')
                self.assertEqual(receipt['failed_step'], 'fixture')
                self.assertEqual(receipt['checks'], {})
                self.assertNotIn('private', report.read_text())

    def test_gui_receipt_leaves_a_user_file_and_its_link_intact(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            user_file = root/'user.json'
            user_file.write_text('user evidence')
            report = root/'published-gui-update.json'
            report.symlink_to(user_file)
            with patch.object(update, 'GUI_REPORT', report), \
                    self.assertRaisesRegex(ValueError, 'symbolic link'):
                update.write_gui_receipt({'status': 'running'})
            self.assertEqual(report.readlink(), user_file)
            self.assertEqual(user_file.read_text(), 'user evidence')

    def test_gui_download_requires_current_gui_plan_and_public_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            data = work/'profile'
            directory = work/'clash-of-rust-update-test'
            directory.mkdir()
            package = directory/self.name
            package.write_bytes(b'public release package')
            published = {'version': self.version, 'package': self.name, 'sha256': update.sha256(package)}
            status = {'directory': str(directory), 'version': self.version}
            plan = {'version': self.version, 'package': str(package), 'parent': 41,
                    'executable': str(linux_gui.APP/'clash-of-rust'), 'data_dir': str(data),
                    'restart': True, 'terminal': None, 'sha256': published['sha256']}
            update.write_json(directory/'plan.json', plan)
            self.assertEqual(linux_gui.verified_gui_download(status, published, 41, work, data), package)
            with self.assertRaisesRegex(RuntimeError, 'staging directory'):
                linux_gui.verified_gui_download(status, published, 41, directory, data)
            with self.assertRaisesRegex(RuntimeError, 'GUI handoff'):
                linux_gui.verified_gui_download(status, published, 42, work, data)
            for change in ({'terminal': {'enabled': True}}, {'executable': '/tmp/cor'}, {'data_dir': '/tmp'}):
                update.write_json(directory/'plan.json', dict(plan, **change))
                with self.assertRaisesRegex(RuntimeError, 'GUI handoff'):
                    linux_gui.verified_gui_download(status, published, 41, work, data)
            update.write_json(directory/'plan.json', plan)
            moved = work/'same-bytes'
            package.rename(moved)
            package.symlink_to(moved)
            with self.assertRaisesRegex(RuntimeError, 'GUI handoff'):
                linux_gui.verified_gui_download(status, published, 41, work, data)
            package.unlink()
            package.write_bytes(b'different package bytes')
            with self.assertRaisesRegex(RuntimeError, 'GUI handoff'):
                linux_gui.verified_gui_download(status, published, 41, work, data)

    @unittest.skipUnless(hasattr(os, 'memfd_create'), 'Linux package snapshot seals')
    def test_gui_only_accepts_pkexec_using_the_installer_frozen_snapshot(self):
        import fcntl

        descriptor = os.memfd_create('cor-update', os.MFD_ALLOW_SEALING)
        with os.fdopen(descriptor, 'wb') as sealed, tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root/'clash-of-rust-update-test'
            directory.mkdir()
            package = directory/self.name
            executable = directory/'update-helper'
            executable.write_bytes(b'installed GUI fixture')
            (root/'clash-of-rust').write_bytes(executable.read_bytes())
            helper = root/'41'
            snapshot = helper/'fd/3'
            snapshot.parent.mkdir(parents=True)
            sealed.write(b'package from GUI')
            sealed.flush()
            snapshot.symlink_to(f'/proc/self/fd/{sealed.fileno()}')
            (helper/'exe').symlink_to(executable)
            (helper/'cmdline').write_bytes(b'\0'.join(
                [os.fsencode(directory/'update-helper'), b'--install-update', os.fsencode(directory), b'']))
            pkexec = root/'42'
            pkexec.mkdir()
            (pkexec/'status').write_text(f'Uid:\t{os.getuid()}\t0\t0\t0\nPPid:\t41\n')
            digest = update.sha256(snapshot)
            tool = '/usr/bin/dpkg'
            prefix = [b'/usr/bin/pkexec', b'--disable-internal-agent', os.fsencode(tool), b'-i']
            (pkexec/'cmdline').write_bytes(b'\0'.join([*prefix, b'/proc/41/fd/3', b'']))
            with patch.object(linux_gui, 'APP', root):
                with self.assertRaisesRegex(RuntimeError, 'sealed package snapshot'):
                    linux_gui.verify_gui_pkexec(package, tool, digest, proc=root)
                fcntl.fcntl(sealed, fcntl.F_ADD_SEALS, fcntl.F_SEAL_WRITE | fcntl.F_SEAL_GROW
                            | fcntl.F_SEAL_SHRINK | fcntl.F_SEAL_SEAL)
                self.assertEqual(linux_gui.verify_gui_pkexec(package, tool, digest, proc=root), 42)
                with patch.object(linux_gui, 'output', return_value='/usr/bin/pkexec') as readlink:
                    self.assertEqual(linux_gui.verify_runner_pkexec(42, os.getuid(), tool, proc=root),
                                     '/proc/41/fd/3')
                    readlink.assert_called_once_with(['sudo', '-n', '/usr/bin/readlink', '--', pkexec/'exe'])
                    readlink.return_value = '/tmp/pkexec'
                    with self.assertRaisesRegex(RuntimeError, 'not the native pkexec'):
                        linux_gui.verify_runner_pkexec(42, os.getuid(), tool, proc=root)
                with self.assertRaisesRegex(RuntimeError, 'match the GUI download'):
                    linux_gui.verify_gui_pkexec(package, tool, '0'*64, proc=root)
                (helper/'exe').unlink()
                (helper/'exe').symlink_to(root/'clash-of-rust')
                with self.assertRaisesRegex(RuntimeError, 'staged GUI installer'):
                    linux_gui.verify_gui_pkexec(package, tool, digest, proc=root)
                (pkexec/'cmdline').write_bytes(b'\0'.join([*prefix, os.fsencode(package), b'']))
                with self.assertRaisesRegex(RuntimeError, 'frozen package'):
                    linux_gui.verify_gui_pkexec(package, tool, digest, proc=root)

    def test_native_lock_conflict_requires_the_observed_installer_and_specific_errno(self):
        snapshot = '/proc/41/fd/3'
        for tool, arguments, conflict, reason in (
                ('/usr/bin/dpkg', '"-i"',
                 'fcntl(4</var/lib/dpkg/lock-frontend>, F_SETLK, {l_type=F_WRLCK}) = -1 EAGAIN (busy)',
                 'dpkg lock-frontend F_SETLK denied'),
                ('/usr/bin/pacman', '"-U", "--noconfirm", "--"',
                 'openat(AT_FDCWD</>, "/var/lib/pacman/db.lck", O_WRONLY|O_CREAT|O_EXCL, 000) = -1 EEXIST',
                 'pacman db.lck EEXIST')):
            started = f'execve("{tool}", ["{tool}", {arguments}, "{snapshot}"], 0x123) = 0'
            with self.subTest(tool=tool):
                trace = f'{started}\n{conflict}\nexit_group(1) = ?'
                self.assertEqual(linux_gui.native_lock_conflict(trace, tool, snapshot), reason)
                for invalid in (trace.replace(snapshot, '/tmp/another-package'),
                                trace.replace(conflict, 'write(2, "disk full", 9) = 9'),
                                trace.replace('exit_group(1)', 'exit_group(0)'),
                                f'{conflict}\n{started}\nexit_group(1) = ?'):
                    with self.assertRaisesRegex(RuntimeError, 'expected database lock conflict'):
                        linux_gui.native_lock_conflict(invalid, tool, snapshot)

    def test_deb_requires_complete_installed_state_audit_and_executables(self):
        with tempfile.TemporaryDirectory() as temporary, patch.object(linux_gui, 'APP', Path(temporary)):
            for name in ('clash-of-rust', 'cor', 'clash-desktop-ui', 'resources/mihomo'):
                executable = Path(temporary)/name
                executable.parent.mkdir(exist_ok=True)
                executable.touch(mode=0o755)
            for status, audit, passed in (('install ok installed\t1.0', '', True),
                                          ('install ok unpacked\t1.0', '', False),
                                          ('install reinstreq installed\t1.0', '', False),
                                          ('install ok installed\t2.0', '', False),
                                          ('install ok installed\t1.0', 'missing control files', False)):
                with self.subTest(status=status, audit=audit), \
                        patch.object(linux_gui, 'output', side_effect=[status, audit]):
                    if passed:
                        linux_gui.verify_installed_package('deb', '1.0')
                    else:
                        with self.assertRaisesRegex(RuntimeError, 'DEB is not fully installed'):
                            linux_gui.verify_installed_package('deb', '1.0')
            executable.unlink()
            with patch.object(linux_gui, 'output', side_effect=['install ok installed\t1.0', '']), \
                    self.assertRaisesRegex(RuntimeError, 'missing executable resources/mihomo'):
                linux_gui.verify_installed_package('deb', '1.0')

    def test_gui_waits_for_autostart_before_stopping_a_new_owner(self):
        statuses = [{'owner': 'gui', 'running': value} for value in
                    (False, True, False, False, True, False, False)]
        with patch.object(linux_gui, 'cli', side_effect=[
                *statuses[:2], {}, *statuses[2:]]) as cli, \
                patch.object(linux_gui, 'core_processes', side_effect=[{43: 42}, {}, {}, {}, {}]), \
                patch.object(linux_gui.time, 'monotonic', side_effect=[0, 0, 0, 0, 1, 1, 2, 2, 3, 4, 4, 6, 6]), \
                patch.object(linux_gui.time, 'sleep'):
            linux_gui.stopped_gui_core(42, startup=True)
        self.assertEqual([call.args for call in cli.call_args_list],
                         [('status',), ('status',), ('stop',), *[('status',)]*5])

    @unittest.skipUnless(sys.platform == 'linux', 'GUI fixture uses Linux signals')
    def test_gui_failure_lock_is_released_after_recovery_and_interruption(self):
        for interrupt_when in (None, 'locked', 'release'):
            with self.subTest(interrupt_when=interrupt_when), tempfile.TemporaryDirectory() as temporary:
                home = Path(temporary)
                (home/'report').mkdir()
                request = home/'report/update-observation-request.json'
                acknowledgement = home/'update-observation-state.json'
                events = []

                @contextmanager
                def lock():
                    events.append('locked')
                    try:
                        yield
                    finally:
                        if interrupt_when == 'release':
                            os.kill(os.getpid(), signal.SIGTERM)
                        events.append('unlocked')

                def poll():
                    phase = linux_gui.read_object(acknowledgement).get('phase')
                    if phase == interrupt_when == 'locked':
                        os.kill(os.getpid(), signal.SIGTERM)
                    if phase == 'retry':
                        child.returncode = 0
                    else:
                        follow = {None: 'cancel', 'cancel': 'locked', 'locked': 'release', 'release': 'retry'}
                        update.write_json(request, {'phase': follow[phase], 'pid': 42})
                    return child.returncode

                child = Mock(returncode=None)
                child.poll.side_effect = poll
                observer = Mock()
                with patch.object(linux_gui.subprocess, 'Popen', return_value=child), \
                        patch.object(linux_gui, 'verify_runner_pkexec', return_value='/proc/41/fd/3'), \
                        patch.object(linux_gui, 'trace_installer', return_value=nullcontext(observer)), \
                        patch.object(linux_gui, 'native_lock_conflict', return_value='lock evidence'), \
                        patch.object(linux_gui, 'stop_child', side_effect=lambda _child: events.append('child stopped')), \
                        patch.object(linux_gui.time, 'sleep'):
                    interrupted = (self.assertRaisesRegex(KeyboardInterrupt, 'Interrupted by SIGTERM')
                                   if interrupt_when else nullcontext())
                    with interrupted, linux_gui.fixture_signals() as check_interrupted:
                        linux_gui.run_update_desktop(
                            ['unused'], home, lock, 'deb', os.getuid(),
                            stop_session=lambda: events.extend(('unit stopped', 'logout')),
                            check_interrupted=check_interrupted)
                stopped = ['child stopped', 'unit stopped', 'logout']
                if interrupt_when == 'locked':
                    self.assertEqual(events, ['locked', *stopped, 'unlocked'])
                elif interrupt_when == 'release':
                    self.assertEqual(events, ['locked', 'unlocked', *stopped])
                else:
                    self.assertEqual(linux_gui.read_object(acknowledgement), {'phase': 'retry'})
                    self.assertEqual(events, ['locked', 'unlocked'])

    @unittest.skipUnless(sys.platform == 'linux', 'GUI fixture uses Linux accounts')
    def test_failed_gui_launch_only_stops_a_verified_random_unit(self):
        units = set()
        for ownership in ('ours', 'foreign', 'missing'):
            with self.subTest(ownership=ownership), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                state = {}
                child = Mock(returncode=1)
                child.poll.return_value = 1

                def launch(arguments, *, cwd, start_new_session):
                    self.assertTrue(start_new_session)
                    arguments = list(map(str, arguments))
                    properties = dict(argument.removeprefix('--property=').split('=', 1)
                                      for argument in arguments if argument.startswith('--property='))
                    unit = next(arg.removeprefix('--unit=') for arg in arguments if arg.startswith('--unit='))
                    self.assertRegex(unit, r'^cor-desktop-[a-f0-9]{24}\.service$')
                    self.assertNotIn(unit, units)
                    units.add(unit)
                    description = next(arg.removeprefix('--description=') for arg in arguments
                                       if arg.startswith('--description='))
                    state.update(Id=unit, LoadState='loaded', ActiveState='active', Transient='yes',
                                 Description=description, User=properties['User'], WorkingDirectory=str(cwd))
                    if ownership == 'foreign':
                        state['Description'] = 'Different fixture'
                    if ownership == 'missing':
                        state.update(LoadState='not-found', ActiveState='inactive')
                    return child

                def run(arguments, **_kwargs):
                    stdout = ''
                    if arguments[:3] == ['sudo', 'install', '-d']:
                        Path(arguments[-1]).mkdir()
                    elif arguments[:2] == ['systemctl', 'show']:
                        stdout = ''.join(f'{key}={value}\n' for key, value in state.items())
                    return subprocess.CompletedProcess(arguments, 0, stdout=stdout)

                with patch('linux_ci.require_actions'), \
                        patch.dict(os.environ, {'RUNNER_ENVIRONMENT': 'github-hosted'}, clear=True), \
                        patch.object(linux_gui.platform, 'freedesktop_os_release', return_value={'ID': 'ubuntu'}), \
                        patch.object(linux_gui, 'REPORT', root), \
                        patch.object(linux_gui, 'gui_account', return_value=SimpleNamespace(pw_uid=1234)), \
                        patch.object(linux_gui.subprocess, 'Popen', side_effect=launch), \
                        patch.object(linux_gui.subprocess, 'run', side_effect=run) as commands, \
                        self.assertRaises(subprocess.CalledProcessError) as raised:
                    linux_gui.installed_tests(update_case={'format': 'deb'}, package_lock=Mock(),
                                              update_report=root/'desktop')
                stopped = [call.args[0][-1] for call in commands.call_args_list
                           if call.args[0][-3:-1] == ['systemctl', 'stop']]
                self.assertEqual(stopped, [state['Id']] if ownership == 'ours' else [])
                queries = [call.args[0][-1] for call in commands.call_args_list
                           if call.args[0][:2] == ['systemctl', 'show']]
                self.assertEqual(queries, [state['Id']])
                self.assertEqual(getattr(raised.exception, '__notes__', []) != [], ownership == 'foreign')

    @unittest.skipUnless(sys.platform == 'linux', 'GUI fixture uses Linux accounts')
    def test_gui_logout_uses_only_the_verified_fixture_account(self):
        user = 'synthetic-fixture'
        home = Path('/synthetic/home')
        fixture = SimpleNamespace(pw_dir=str(home), pw_uid=1234)
        listing = subprocess.CompletedProcess([], 0, stdout=f'1235 {user}\n1234 other\n')
        with patch('pwd.getpwnam', return_value=fixture), patch.object(linux_gui, 'run', return_value=listing) as run:
            with self.assertRaisesRegex(RuntimeError, 'outside the disposable fixture'):
                linux_gui.stop_gui_user(user, home, 1235)
            run.assert_not_called()
            linux_gui.stop_gui_user(user, home, 1234)
            self.assertEqual(run.call_count, 1)
            listing.stdout += f'1234 {user}\n'
            linux_gui.stop_gui_user(user, home, 1234)
            self.assertEqual(run.call_args.args[0], ['sudo', 'loginctl', 'terminate-user', user])

    @unittest.skipUnless(sys.platform == 'linux', 'GUI fixture uses Linux accounts')
    def test_failed_gui_account_creation_does_not_clean_up_an_existing_account(self):
        failure = subprocess.CalledProcessError(9, ['useradd'])

        def run(arguments, **_kwargs):
            if arguments[:2] == ['sudo', 'useradd']:
                raise failure
            return subprocess.CompletedProcess(arguments, 0)

        with tempfile.TemporaryDirectory() as temporary, \
                patch('linux_ci.require_actions'), \
                patch.dict(os.environ, {'RUNNER_ENVIRONMENT': 'github-hosted'}, clear=True), \
                patch.object(linux_gui.platform, 'freedesktop_os_release', return_value={'ID': 'ubuntu'}), \
                patch.object(linux_gui, 'REPORT', Path(temporary)), \
                patch.object(linux_gui, 'gui_account', return_value=SimpleNamespace(pw_uid=1234)) as account, \
                patch.object(linux_gui, 'delete_gui_user') as delete, \
                patch.object(linux_gui, 'stop_gui_user') as logout, \
                patch.object(linux_gui, 'run', side_effect=run), \
                self.assertRaises(subprocess.CalledProcessError) as raised:
            linux_gui.installed_tests(update_case={'format': 'deb'}, package_lock=Mock(),
                                      update_report=Path(temporary)/'desktop')
        self.assertIs(raised.exception, failure)
        account.assert_not_called()
        delete.assert_not_called()
        logout.assert_not_called()

    def test_cleanup_failure_is_reported_and_other_steps_still_run(self):
        denied = Mock(side_effect=PermissionError('cannot remove the rule'))
        later = Mock()
        steps = (('remove polkit rule', denied), ('delete user', later))
        with self.assertRaisesRegex(RuntimeError, 'remove polkit rule'):
            linux_gui.cleanup_steps(*steps)
        later.assert_called_once()
        with self.assertRaises(KeyboardInterrupt) as raised:
            try:
                raise KeyboardInterrupt('interrupted')
            finally:
                linux_gui.cleanup_steps(*steps)
        self.assertIn('cannot remove the rule', raised.exception.__notes__[0])
        self.assertEqual(later.call_count, 2)

    @unittest.skipUnless(sys.platform == 'linux', 'GUI fixture uses Linux signals')
    def test_interrupted_graphical_update_still_removes_the_account_and_saves_failure(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(linux_gui, 'REPORT', Path(temporary)), \
                patch('linux_ci.require_actions'), \
                patch.object(linux_gui.secrets, 'token_hex', return_value='1'*24), \
                patch.object(linux_gui, 'gui_account', return_value=SimpleNamespace(pw_uid=1234)), \
                patch.object(linux_gui, 'stop_fixture_unit'), \
                patch.dict(os.environ, {'RUNNER_ENVIRONMENT': 'github-hosted'}, clear=True), \
                patch.object(linux_gui.platform, 'freedesktop_os_release', return_value={'ID': 'ubuntu'}):
            user = 'corgui' + '1'*24
            rule = Path('/etc/polkit-1/rules.d')/f'10-{user}.rules'
            executed = []

            def run(command, **_kwargs):
                executed.append(command)
                if command[:3] == ['sudo', 'install', '-d']:
                    Path(command[-1]).mkdir()
                failed = command == ['sudo', 'rm', '-f', rule]
                return subprocess.CompletedProcess(command, int(failed), stdout='')

            def interrupt(_command, home, *_args, **_kwargs):
                update.write_json(home/'report'/'gui.json', {'status': 'failed'})
                os.kill(os.getpid(), signal.SIGTERM)

            with patch.object(linux_gui.subprocess, 'run', side_effect=run), \
                    patch.object(linux_gui, 'run_update_desktop', side_effect=interrupt), \
                    self.assertRaisesRegex(KeyboardInterrupt, 'Interrupted by SIGTERM') as raised:
                linux_gui.installed_tests(update_case={'format': 'deb'}, package_lock=Mock(),
                                          update_report=Path(temporary)/'desktop-update')
            self.assertIn('remove disposable polkit rule', raised.exception.__notes__[0])
            self.assertIn(['sudo', 'userdel', '--remove', user], executed)
            self.assertEqual(linux_gui.read_object(Path(temporary)/'desktop-update/gui.json'),
                             {'status': 'failed'})

    @unittest.skipUnless(sys.platform == 'linux', 'GUI fixture uses Linux signals')
    def test_sigterm_during_gui_cleanup_finishes_and_preserves_the_original_failure(self):
        failure = RuntimeError('GUI session failed')
        for original in (None, failure):
            with self.subTest(original=original), tempfile.TemporaryDirectory() as temporary, \
                    patch.object(linux_gui, 'REPORT', Path(temporary)), \
                    patch('linux_ci.require_actions'), \
                    patch.object(linux_gui.secrets, 'token_hex', return_value='1'*24), \
                    patch.object(linux_gui, 'gui_account', return_value=SimpleNamespace(pw_uid=1234)), \
                    patch.object(linux_gui, 'stop_fixture_unit'), \
                    patch.dict(os.environ, {'RUNNER_ENVIRONMENT': 'github-hosted'}, clear=True), \
                    patch.object(linux_gui.platform, 'freedesktop_os_release', return_value={'ID': 'ubuntu'}):
                user = 'corgui'+'1'*24
                rule = Path('/etc/polkit-1/rules.d')/f'10-{user}.rules'
                fixture_rule = Path(temporary)/'rule'
                fixture_account = Path(temporary)/'account'

                def run(command, **_kwargs):
                    if command[:2] == ['sudo', 'useradd']:
                        fixture_account.touch()
                    elif command == ['sudo', 'tee', str(rule)]:
                        fixture_rule.touch()
                    elif command[:3] == ['sudo', 'install', '-d']:
                        Path(command[-1]).mkdir()
                    elif command == ['sudo', 'rm', '-f', rule]:
                        self.assertIs(_kwargs['start_new_session'], True)
                        os.kill(os.getpid(), signal.SIGTERM)
                        fixture_rule.unlink()
                    elif command == ['sudo', 'userdel', '--remove', user]:
                        self.assertIs(_kwargs['start_new_session'], True)
                        os.kill(os.getpid(), signal.SIGINT)
                        fixture_account.unlink()
                    return subprocess.CompletedProcess(command, 0, stdout='')

                def desktop(_command, home, *_args, **_kwargs):
                    update.write_json(home/'report'/'gui.json', {'status': 'failed' if original else 'passed'})
                    if original:
                        raise original

                expected = RuntimeError if original else KeyboardInterrupt
                with patch.object(linux_gui.subprocess, 'run', side_effect=run), \
                        patch.object(linux_gui, 'run_update_desktop', side_effect=desktop), \
                        self.assertRaises(expected) as raised:
                    linux_gui.installed_tests(update_case={'format': 'deb'}, package_lock=Mock(),
                                              update_report=Path(temporary)/'desktop-update')

                self.assertFalse(fixture_rule.exists())
                self.assertFalse(fixture_account.exists())
                self.assertTrue((Path(temporary)/'desktop-update/gui.json').is_file())
                if original:
                    self.assertIs(raised.exception, original)
                    self.assertEqual(original.__notes__, ['Interrupted by SIGTERM'])
                else:
                    self.assertEqual(str(raised.exception), 'Interrupted by SIGTERM')

    @unittest.skipUnless(sys.platform == 'linux', 'native lock worker uses Linux file semantics')
    def test_arch_lock_fallback_removes_only_its_own_inode_after_helper_dies(self):
        spawn = subprocess.Popen
        for foreign in (False, True):
            with self.subTest(foreign=foreign), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                lock = root/'db.lck'
                children = []

                def local_command(command):
                    return [*command[2:-1], str(root/Path(command[-1]).name)]

                def start(command, **kwargs):
                    child = spawn(local_command(command), **kwargs)
                    children.append(child)
                    return child

                def fallback(command, **kwargs):
                    with spawn(local_command(command)) as child:
                        self.assertEqual(child.wait(timeout=kwargs['timeout']), 0)

                with patch.object(update.subprocess, 'Popen', side_effect=start), \
                        patch.object(update, 'invoke', side_effect=fallback):
                    with self.assertRaisesRegex(RuntimeError, 'lock helper failed'):
                        with update.package_manager_lock('pkg'):
                            self.assertTrue(lock.exists())
                            if foreign:
                                lock.unlink()
                                lock.write_text('different transaction')
                            children[0].kill()
                            children[0].wait(timeout=10)
                if foreign:
                    self.assertEqual(lock.read_text(), 'different transaction')
                self.assertEqual(list(root.iterdir()), [lock] if foreign else [])

    def test_each_linux_workflow_forwards_both_optional_update_inputs(self):
        workflows = update.ROOT/'.github/workflows'
        for suite in ('ubuntu', 'debian', 'linuxmint', 'pop', 'zorin', 'fedora', 'opensuse', 'arch'):
            text = (workflows/f'{suite}-compatibility.yml').read_text()
            with self.subTest(suite=suite):
                self.assertIn('update_from_ci: ${{ inputs.update_from_ci }}', text)
                self.assertIn('update_release: ${{ inputs.update_release }}', text)
        shared = (workflows/'linux-compatibility.yml').read_text()
        self.assertIn('COR_LINUX_UPDATE_FROM_CI: ${{ inputs.update_from_ci }}', shared)
        self.assertIn('COR_LINUX_UPDATE_RELEASE: ${{ inputs.update_release }}', shared)


if __name__ == '__main__':
    unittest.main()
