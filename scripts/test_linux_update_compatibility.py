"""Offline checks for published Linux update fixtures."""
import base64
from contextlib import contextmanager, nullcontext
import copy
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

import linux_update_compatibility as update
import linux_gui


class LinuxPublishedUpdateTests(unittest.TestCase):
    def setUp(self):
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

    def test_cleanup_only_stops_the_daemon_for_the_fixture_data(self):
        data = Path('/tmp/isolated-update-data')
        commands = (
            (b'/opt/clash-of-rust/cor\0--data-dir\0/tmp/isolated-update-data\0', True),
            (b'/opt/clash-of-rust/cor\0--data-dir\0/home/other/data\0', False),
            (b'/usr/bin/other\0/tmp/isolated-update-data\0', False),
        )
        for command, should_stop in commands:
            with self.subTest(command=command), \
                    patch.object(update, 'endpoint', return_value=1234), \
                    patch.object(Path, 'read_bytes', return_value=command), \
                    patch.object(update.os, 'kill') as kill:
                update.stop_fixture_daemon(data)
            if should_stop:
                kill.assert_called_once_with(1234, signal.SIGTERM)
            else:
                kill.assert_not_called()

    def test_cleanup_tolerates_a_daemon_exiting_before_it_can_be_stopped(self):
        data = Path('/tmp/isolated-update-data')
        with patch.object(update, 'endpoint', return_value=1234), \
                patch.object(Path, 'read_bytes', return_value=os.fsencode(update.APP/'cor') + b'\0'
                             + os.fsencode(data) + b'\0'), \
                patch.object(update.os, 'kill', side_effect=OSError):
            update.stop_fixture_daemon(data)

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
                           'previous': {'version': '0.7.3-dev.1.test', 'package': 'old.deb',
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
                        patch.object(update, 'installed_version', side_effect=[
                            update.expected_version('deb', fixture['previous']['version']),
                            update.expected_version('deb', version)]), \
                        patch.object(update, 'wait_for', side_effect=[1234, 5678]), \
                        patch.object(update, 'stop_fixture_daemon'), \
                        patch.object(update, 'terminal_authorization', return_value=nullcontext()), \
                        patch.object(update.subprocess, 'Popen', return_value=daemon), \
                        patch.object(update, 'download', side_effect=fake_download), \
                        patch.object(update, 'cli', side_effect=fake_cli):
                    if scenario in ('prerelease', 'stable'):
                        update.exercise_cli(root)
                        self.assertEqual(json.loads(report.read_text())['status'], 'passed')
                    else:
                        with self.assertRaises(ValueError):
                            update.exercise_cli(root)
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
        root = Path('/isolated-fixture')
        package = root/'previous.deb'
        fixture = {'format': 'deb', 'arch': 'x64'}
        failure = RuntimeError('CLI failed')
        for hosted, error in ((True, None), (False, None), (True, failure)):
            with self.subTest(hosted=hosted, error=error), \
                    patch.dict(os.environ, {'RUNNER_ENVIRONMENT': 'github-hosted'} if hosted else {}, clear=True), \
                    patch.object(update.platform, 'freedesktop_os_release',
                                 return_value={'ID': 'ubuntu', 'VERSION_ID': '24.04'}), \
                    patch.object(update, 'exercise_cli', return_value=(fixture, package), side_effect=error) as cli, \
                    patch.object(update, 'exercise_gui') as gui:
                if error:
                    with self.assertRaisesRegex(RuntimeError, 'CLI failed'):
                        update.exercise(root)
                else:
                    update.exercise(root)
                cli.assert_called_once_with(root)
                if hosted and not error:
                    gui.assert_called_once_with(fixture, package)
                else:
                    gui.assert_not_called()

    def test_gui_update_language_overrides_runner_messages_and_preserves_utf8(self):
        with patch.dict(os.environ, {'LANG': 'C.UTF-8', 'LC_ALL': 'en_US.UTF-8', 'LANGUAGE': 'en'}, clear=True):
            env = linux_gui.update_gui_environment(Path('/isolated-downloads'))
        self.assertEqual((env['LANG'], env['LC_ALL'], env['LC_MESSAGES'], env['LANGUAGE']),
                         ('C.UTF-8', '', 'zh_CN.UTF-8', 'zh_CN'))
        self.assertEqual(env['TMPDIR'], '/isolated-downloads')

    def test_gui_labels_combine_word_geometry_and_match_the_published_version(self):
        lines = linux_gui.recognized_lines(
            'level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n'
            '5\t1\t1\t1\t1\t1\t10\t30\t24\t14\t90\t检查\n'
            '5\t1\t1\t1\t1\t2\t36\t30\t24\t14\t90\t更新\n'
            '5\t1\t2\t1\t1\t1\t50\t100\t26\t14\t90\t更新\n'
            '5\t1\t3\t1\t1\t1\t60\t130\t24\t14\t90\t发现\n'
            '5\t1\t3\t1\t1\t2\t86\t130\t200\t14\t90\t0.7.3–dev.2.live-language\n')
        self.assertEqual(linux_gui.screen_label(lines, '检查更新'), (35, 37))
        self.assertEqual(linux_gui.screen_label(lines, '更新'), (63, 107))
        self.assertIs(linux_gui.screen_label(lines, self.version, contains=True), True)
        self.assertIsNone(linux_gui.screen_label(lines, '下载安装'))

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
                        patch.object(linux_gui, 'stop_desktop_unit', side_effect=lambda _unit: events.append('unit stopped')), \
                        patch.object(linux_gui, 'stop_gui_user', side_effect=lambda _user: events.append('logout')), \
                        patch.object(linux_gui, 'kill_gui_user_processes'), \
                        patch.object(linux_gui.time, 'sleep'):
                    interrupted = (self.assertRaisesRegex(KeyboardInterrupt, 'Interrupted by SIGTERM')
                                   if interrupt_when else nullcontext())
                    with interrupted, linux_gui.fixture_signals() as check_interrupted:
                        linux_gui.run_update_desktop(
                            ['unused'], home, 'fixture.service', lock, 'deb', os.getuid(), 'fixture-user',
                            check_interrupted=check_interrupted)
                stopped = ['child stopped', 'unit stopped', 'logout']
                if interrupt_when == 'locked':
                    self.assertEqual(events, ['locked', *stopped, 'unlocked'])
                elif interrupt_when == 'release':
                    self.assertEqual(events, ['locked', 'unlocked', *stopped])
                else:
                    self.assertEqual(linux_gui.read_object(acknowledgement), {'phase': 'retry'})
                    self.assertEqual(events, ['locked', 'unlocked'])

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
                patch.dict(os.environ, {'RUNNER_ENVIRONMENT': 'github-hosted'}, clear=True), \
                patch.object(linux_gui.platform, 'freedesktop_os_release', return_value={'ID': 'ubuntu'}):
            user = 'corgui' + str(os.getpid())
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
                linux_gui.installed_tests(update_case={'format': 'deb'}, package_lock=Mock())
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
                    patch.dict(os.environ, {'RUNNER_ENVIRONMENT': 'github-hosted'}, clear=True), \
                    patch.object(linux_gui.platform, 'freedesktop_os_release', return_value={'ID': 'ubuntu'}):
                user = 'corgui'+str(os.getpid())
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
                        os.kill(os.getpid(), signal.SIGTERM)
                        fixture_rule.unlink()
                    elif command == ['sudo', 'userdel', '--remove', user]:
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
                    linux_gui.installed_tests(update_case={'format': 'deb'}, package_lock=Mock())

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
