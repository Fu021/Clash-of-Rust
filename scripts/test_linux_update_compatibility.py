"""Offline checks for published Linux update fixtures."""
import base64
from contextlib import nullcontext
import copy
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

import linux_update_compatibility as update


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
        with patch.dict(os.environ, {}, clear=True), \
                patch.object(update, 'install_previous') as install, \
                self.assertRaisesRegex(RuntimeError, 'disposable Actions'):
            update.exercise(Path('/not-a-fixture'))
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
                with patch.dict(os.environ, environment), patch.object(update.sys, 'platform', 'linux'), \
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
