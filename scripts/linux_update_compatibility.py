"""Verify an installed Linux client's update against a published release."""
import base64
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import time

import compatibility
from build_support import (ROOT, deb_version, download, host_arch, manifest_version,
                           sha256, validate_version, write_json)
from linux_native_package import arch_filename, native_version, rpm_filename
from workflow_artifacts import fetch_package_inputs, release_module

APP = Path('/opt/clash-of-rust')
REPORT = ROOT/'target/linux-report/published-update.json'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def previous_version(client, commit):
    value = client('/contents/Cargo.toml?ref=' + commit)
    require(value.get('type') == 'file' and value.get('encoding') == 'base64'
            and 0 < value.get('size', 0) <= 256*1024, 'Invalid previous source manifest')
    content = base64.b64decode(''.join(value['content'].split()), validate=True)
    require(len(content) == value['size'], 'Previous source manifest size differs')
    return validate_version(manifest_version(content.decode('utf-8')))


def release_asset(client, tag, package_format, arch):
    require(tag.startswith('v'), 'A published version tag is required')
    version = validate_version(tag[1:])
    release = client('/releases/tags/' + tag)
    require(release.get('tag_name') == tag and release.get('draft') is False
            and type(release.get('prerelease')) is bool
            and release['prerelease'] == ('-' in version),
            'The requested release is not published')
    if package_format == 'deb':
        suffix = {'x64': 'amd64', 'arm64': 'arm64'}[arch]
        name = f'Clash-of-Rust-{version}-linux-{suffix}.deb'
    elif package_format == 'rpm':
        name = rpm_filename(version, arch)
    elif package_format == 'pkg' and arch == 'x64':
        name = arch_filename(version)
    else:
        raise ValueError('Unsupported published update package')
    assets = release.get('assets', [])
    matches = [asset for asset in assets if asset.get('name') == name]
    checksums = [asset for asset in assets if asset.get('name') == name + '.sha256']
    require(len(matches) == len(checksums) == 1, 'Published package or checksum is missing or duplicated')
    asset, checksum = matches[0], checksums[0]
    url = f'https://github.com/{client.repo}/releases/download/{tag}/{name}'
    require(asset.get('state') == checksum.get('state') == 'uploaded'
            and asset.get('browser_download_url') == url
            and checksum.get('browser_download_url') == url + '.sha256'
            and re.fullmatch(r'sha256:[a-f0-9]{64}', asset.get('digest') or '')
            and re.fullmatch(r'sha256:[a-f0-9]{64}', checksum.get('digest') or '')
            and 0 < asset.get('size', 0) <= 512*1024*1024
            and 0 < checksum.get('size', 0) <= 1024,
            'Published package metadata is invalid')
    obj = client('/git/ref/tags/' + tag)['object']
    for _ in range(8):
        if obj['type'] != 'tag':
            break
        obj = client('/git/tags/' + obj['sha'])['object']
    require(obj.get('type') == 'commit' and re.fullmatch(r'[a-f0-9]{40}', obj.get('sha', '')),
            'Published tag does not identify a commit')
    return {'version': version, 'tag': tag, 'commit': obj['sha'],
            'prerelease': release.get('prerelease') is True, 'package': name,
            'sha256': asset['digest'][7:], 'url': url,
            'checksum_sha256': checksum['digest'][7:], 'checksum_url': url + '.sha256'}


def prepare(client, old_run_id, tag, package_format, arch, directory):
    require(type(old_run_id) is int and old_run_id > 0, 'A previous CI run is required')
    old_run = client(f'/actions/runs/{old_run_id}')
    commit = old_run.get('head_sha', '')
    require(re.fullmatch(r'[a-f0-9]{40}', commit), 'Previous CI commit is invalid')
    compatibility.validate_run(old_run, client.repo, commit)
    require(old_run.get('id') == old_run_id and old_run.get('event') == 'push',
            'Previous fixture must come from a successful push CI')
    version = previous_version(client, commit)
    published = release_asset(client, tag, package_format, arch)
    require(version != published['version'], 'Previous and published versions must differ')
    directory.mkdir(parents=True, exist_ok=False)
    source = directory/'source'
    fetch_package_inputs(client, old_run_id, arch, source, commit, old_run_id,
                         rpm=package_format == 'rpm', version=version)
    suffix = {'x64': 'amd64', 'arm64': 'arm64'}[arch]
    old_package = source/'dist'/f'Clash-of-Rust-{version}-linux-{suffix}.deb'
    if package_format == 'rpm':
        old_package = source/'dist/linux-rpm'/rpm_filename(version, arch)
    elif package_format == 'pkg':
        receipt = compatibility.verified_suite(client, old_run, 'arch')
        release_module().download_release_arch(
            client, receipt['arch_package'], old_run_id, commit, version, source/'dist')
        old_package = source/'dist'/arch_filename(version)
    fixture = {
        'schema': 1, 'format': package_format, 'arch': arch,
        'previous': {'version': version, 'commit': commit, 'ci_run_id': old_run_id,
                     'package': old_package.relative_to(directory).as_posix(),
                     'sha256': sha256(old_package)},
        'published': published,
    }
    write_json(directory/'fixture.json', fixture)
    return fixture


def fixture_package(directory, fixture):
    relative = fixture['previous']['package']
    require(isinstance(relative, str) and not Path(relative).is_absolute()
            and '..' not in Path(relative).parts, 'Invalid previous package path')
    root = directory.resolve(strict=True)
    package = (directory/relative).resolve(strict=True)
    require(package.is_relative_to(root) and package.is_file()
            and sha256(package) == fixture['previous']['sha256'],
            'Previous package differs from its verified fixture')
    return package


def invoke(command, *, timeout=120, check=True):
    result = subprocess.run(list(map(str, command)), capture_output=True, text=True,
                            stdin=subprocess.DEVNULL, timeout=timeout)
    if check and result.returncode:
        raise RuntimeError(f'Command failed ({result.returncode}): {(result.stdout + result.stderr)[-4000:]}')
    return result


def cli(data, *arguments, timeout=120, check=True):
    result = invoke([APP/'cor', '--data-dir', data, '--resources', APP/'resources',
                     '--json', *arguments], timeout=timeout, check=check)
    if not check:
        return result
    return json.loads(result.stdout)['data']


def endpoint(data):
    try:
        return json.loads((data/'control.json').read_text())['pid']
    except (FileNotFoundError, ValueError, KeyError):
        return None


def stop_fixture_daemon(data):
    pid = endpoint(data)
    if type(pid) is not int or pid <= 1:
        return
    try:
        command = (Path('/proc')/str(pid)/'cmdline').read_bytes().split(b'\0')
        if command and command[0] == os.fsencode(APP/'cor') and os.fsencode(data) in command:
            os.kill(pid, signal.SIGTERM)
    except OSError:
        return


def wait_for(probe, description, timeout=60):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = probe()
        if value:
            return value
        time.sleep(.2)
    raise TimeoutError(description)


def install_previous(package_format, package):
    if package_format == 'deb':
        from linux_apt import OPTIONS, install, privileged
        privileged([*OPTIONS, 'purge', '-y', 'clash-of-rust'], 300)
        install([package])
    else:
        from linux_native_guest import install_package, remove_package
        query = ['pacman', '-Qq'] if package_format == 'pkg' else ['rpm', '-q', '--quiet']
        result = invoke([*query, 'clash-of-rust'], check=False)
        require(result.returncode in (0, 1), 'Could not check the previously installed package')
        if result.returncode == 0:
            remove_package()
        install_package(package)


def installed_version(package_format):
    if package_format == 'deb':
        return invoke(['dpkg-query', '--show', '--showformat=${Version}', 'clash-of-rust']).stdout.strip()
    from linux_native_guest import package_version
    return package_version()


def expected_version(package_format, version):
    if package_format == 'deb':
        return deb_version(version)
    return native_version(version, '~' if package_format == 'rpm' else 'pre.') + '-1'


def exercise(directory):
    if (os.environ.get('GITHUB_ACTIONS') != 'true' or sys.platform != 'linux'
            or os.geteuid() == 0 or (os.environ.get('COR_LINUX_GUEST') != 'true'
                                     and os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted')):
        raise RuntimeError('Published update tests require a disposable Actions runner or compatibility VM')
    fixture = json.loads((directory/'fixture.json').read_text())
    package_format = fixture['format']
    require(fixture.get('schema') == 1 and fixture['arch'] == host_arch()
            and package_format in ('deb', 'rpm', 'pkg'), 'Update fixture is incompatible')
    old_package = fixture_package(directory, fixture)
    previous, published = fixture['previous'], fixture['published']
    report = {'schema': 1, 'status': 'failed', 'format': package_format,
              'previous': previous, 'published': published, 'checks': {}}
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    try:
        install_previous(package_format, old_package)
        require(installed_version(package_format) == expected_version(package_format, previous['version']),
                'The previous package was not installed')
        report['checks']['previous_package_installed'] = True
        with tempfile.TemporaryDirectory(prefix='cor-published-update-') as temporary:
            work = Path(temporary)
            data = work/'data'
            checksum = work/(published['package'] + '.sha256')
            download(published['checksum_url'], checksum, digest=published['checksum_sha256'])
            require(checksum.read_text(encoding='ascii').strip() == published['sha256'],
                    'The published checksum differs from the package digest')
            report['checks']['published_checksum_verified'] = True
            fixture_file = work/'fixture.yaml'
            fixture_file.write_text("proxies: []\nproxy-groups: []\nrules: ['MATCH,DIRECT']\n")
            with (work/'daemon.log').open('w') as log:
                daemon = subprocess.Popen(
                    [APP/'cor', '--data-dir', data, '--resources', APP/'resources',
                     'run', '--no-core', '--quiet'],
                    stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
                    start_new_session=True)
                try:
                    original = wait_for(lambda: endpoint(data), 'Previous backend did not start')
                    cli(data, 'config', 'set', 'update_prereleases',
                        str(published['prerelease']).lower())
                    cli(data, 'profile', 'add', 'Update fixture', fixture_file)
                    profiles = cli(data, 'profile', 'list', '--filter', 'Update fixture')
                    require(len(profiles) == 1, 'Previous version did not save the fixture subscription')
                    profile = data/'profiles'/f'{profiles[0]["id"]}.yaml'
                    profile_digest = sha256(profile)
                    previous_settings = json.loads((data/'settings.json').read_text())
                    available = cli(data, 'update', 'check', timeout=300)
                    require(available.get('available') is True
                            and available.get('version') == published['version']
                            and available.get('preview') is published['prerelease'],
                            'The installed version did not discover the expected published release')
                    report['checks']['published_version_discovered'] = True
                    if package_format == 'rpm':
                        distro = __import__('platform').freedesktop_os_release()['ID']
                        manager = 'dnf' if distro == 'fedora' else 'zypper'
                        require(available.get('package') is None and available.get('package_manager') == manager,
                                'RPM update did not report the expected package manager')
                        refused = cli(data, 'update', 'install', '--yes', check=False, timeout=300)
                        require(refused.returncode != 0 and manager in refused.stdout + refused.stderr,
                                'RPM installation did not direct the user to its package manager')
                        report['checks']['package_manager_hint'] = True
                        candidate = work/published['package']
                        download(published['url'], candidate, digest=published['sha256'])
                        from linux_native_guest import install_package
                        install_package(candidate)
                        os.kill(original, signal.SIGTERM)
                        daemon.wait(timeout=30)
                        daemon = subprocess.Popen(
                            [APP/'cor', '--data-dir', data, '--resources', APP/'resources',
                             'run', '--no-core', '--quiet'],
                            stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
                            start_new_session=True)
                        wait_for(lambda: (pid if (pid := endpoint(data)) != original else None),
                                 'Updated backend did not start')
                        report['checks']['package_manager_upgrade'] = True
                    else:
                        require(available.get('package') == published['package']
                                and available.get('package_manager') is None,
                                'The client selected an unexpected update package')
                        downloaded = work/published['package']
                        result = cli(data, 'update', 'download', '--output', downloaded, timeout=600)
                        require(result.get('sha256') == published['sha256']
                                and sha256(downloaded) == published['sha256'],
                                'The downloaded update differs from the published package')
                        report['checks']['published_download_verified'] = True
                        cli(data, 'update', 'install', '--yes', '--no-start', timeout=900)
                        status = cli(data, 'update', 'status')
                        require(status.get('state') == 'succeeded' and status.get('error') is False,
                                'The application did not report a successful installation')
                        wait_for(lambda: (pid if (pid := endpoint(data)) != original else None),
                                 'Application update did not restore the backend')
                        report['checks']['application_update_installed'] = True
                    require(installed_version(package_format) == expected_version(package_format, published['version']),
                            'Installed package version differs from the published release')
                    require(cli(data, 'version')['version'] == published['version'],
                            'The installed client binary does not report the published version')
                    require(cli(data, 'config', 'get', 'update_prereleases')['update_prereleases']
                            is published['prerelease'],
                            'The update lost the selected release channel')
                    current_settings = json.loads((data/'settings.json').read_text())
                    require(all(key in current_settings and current_settings[key] == value
                                for key, value in previous_settings.items()),
                            'The update changed a saved setting')
                    after = cli(data, 'profile', 'list', '--filter', 'Update fixture')
                    require(len(after) == 1 and after[0]['id'] == profiles[0]['id']
                            and sha256(profile) == profile_digest, 'The update changed the fixture subscription')
                    status = cli(data, 'status')
                    require(status['version'] == published['version'], 'The backend did not run the updated version')
                    require(status['running'] is False, 'The update started the stopped core')
                    report['checks']['version_and_user_data_preserved'] = True
                    report['checks']['stopped_core_preserved'] = True
                finally:
                    stop_fixture_daemon(data)
                    if daemon.poll() is None:
                        daemon.terminate()
                    try:
                        daemon.wait(timeout=30)
                    except subprocess.TimeoutExpired:
                        daemon.kill()
                        daemon.wait(timeout=10)
        report['status'] = 'passed'
    except Exception as error:
        report['error'] = f'{type(error).__name__}: {error}'
        raise
    finally:
        write_json(REPORT, report)


if __name__ == '__main__':
    if len(sys.argv) != 2:
        raise SystemExit('Pass the verified update fixture directory')
    exercise(Path(sys.argv[1]))
