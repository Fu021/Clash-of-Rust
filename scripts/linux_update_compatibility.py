"""Verify an installed Linux client's update against a published release."""
import base64
from contextlib import contextmanager, nullcontext
import json
import os
from pathlib import Path
import platform
import re
import secrets
import selectors
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
GUI_REPORT = REPORT.with_name('published-gui-update.json')


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


def gui_target(package_format, arch):
    release = platform.freedesktop_os_release()
    if package_format == 'pkg':
        return (arch == 'x64' and release.get('ID') == 'arch'
                and os.environ.get('COR_LINUX_GUEST') == 'true')
    return (package_format == 'deb' and arch == 'x64'
            and (release.get('ID'), release.get('VERSION_ID')) == ('ubuntu', '24.04')
            and os.environ.get('RUNNER_ENVIRONMENT') == 'github-hosted')


@contextmanager
def package_manager_lock(package_format):
    from linux_gui import cleanup_steps, stop_child

    require(package_format in ('deb', 'pkg'), 'Unsupported graphical installation lock')
    marker = '/var/lib/pacman/.cor-gui-' + secrets.token_hex(16)
    worker = """
import fcntl
import os
from pathlib import Path
import select
import signal
import sys

mode, marker = sys.argv[1:]
marker = Path(marker)
path = marker.with_name('db.lck')

def cleanup():
    try:
        ours = marker.stat()
    except FileNotFoundError:
        return
    try:
        current = path.lstat()
    except FileNotFoundError:
        current = None
    if current and (ours.st_dev, ours.st_ino) == (current.st_dev, current.st_ino):
        path.unlink()
    marker.unlink()

def interrupt(number, frame):
    raise SystemExit(128 + number)

def hold():
    print('locked', flush=True)
    if not select.select([sys.stdin], [], [], 420)[0]:
        raise TimeoutError('Native database lock lease expired')

for number in (signal.SIGHUP, signal.SIGINT, signal.SIGTERM):
    signal.signal(number, interrupt)

if mode == 'cleanup':
    cleanup()
elif mode == 'deb':
    with open('/var/lib/dpkg/lock-frontend', 'r+b') as lock:
        fcntl.lockf(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        hold()
else:
    with marker.open('xb'):
        try:
            os.link(marker, path)
            hold()
            if not marker.samefile(path):
                raise RuntimeError('The reserved Arch database lock was replaced')
        finally:
            cleanup()
"""
    command = ['sudo', '-n', '/usr/bin/python3', '-I', '-c', worker]
    with tempfile.TemporaryFile(mode='w+b') as errors:
        child = subprocess.Popen([*command, package_format, marker], stdin=subprocess.PIPE,
                                 stdout=subprocess.PIPE, stderr=errors)
        acquired = False

        def details():
            return os.pread(errors.fileno(), 4096, 0).decode(errors='replace').strip()

        def release():
            child.stdin.close()
            status = child.wait(timeout=15)
            if status and acquired:
                raise RuntimeError(f'Package manager lock helper failed ({status}): {details()}')

        try:
            with selectors.DefaultSelector() as selector:
                selector.register(child.stdout, selectors.EVENT_READ)
                if not selector.select(15):
                    raise TimeoutError('Package manager lock helper did not report acquisition')
                if child.stdout.readline() != b'locked\n':
                    raise RuntimeError(f'Could not acquire the {package_format} database lock: {details()}')
            acquired = True
            yield
        finally:
            steps = [('release package manager database lock', release),
                     ('stop package lock helper', lambda: stop_child(child))]
            if package_format == 'pkg':
                def remove_owned_lock():
                    if child.poll() is None:
                        raise RuntimeError('Package lock helper survived and may still own the native lock')
                    invoke([*command, 'cleanup', marker], timeout=15)
                steps.append(('remove owned Arch database lock', remove_owned_lock))
            steps.append(('close package lock status pipe', child.stdout.close))
            cleanup_steps(*steps)


def exercise_gui(fixture, old_package):
    from linux_gui import installed_tests, verify_installed_package

    require_disposable_update_runner()
    package_format = fixture['format']
    require(gui_target(package_format, fixture['arch']), 'No graphical update target on this runner')
    previous, published = fixture['previous'], fixture['published']
    report = {'schema': 1, 'status': 'failed', 'format': package_format,
              'previous': previous, 'published': published, 'checks': {}}
    try:
        if package_format == 'deb':
            from linux_apt import install
            install(['xvfb', 'xauth', 'x11-utils', 'xdotool', 'wmctrl', 'openbox',
                     'policykit-1-gnome', 'fonts-noto-cjk', 'scrot', 'tesseract-ocr',
                     'tesseract-ocr-chi-sim', 'tesseract-ocr-eng', 'strace'])
        else:
            from linux_native_guest import install_dependencies
            install_dependencies(['xorg-server-xvfb', 'xorg-xauth', 'xorg-xdpyinfo', 'xorg-xwininfo',
                                  'xdotool', 'wmctrl', 'openbox', 'mate-polkit', 'noto-fonts-cjk',
                                  'scrot', 'tesseract', 'tesseract-data-chi_sim', 'tesseract-data-eng', 'strace'])
            if Path('/usr/lib/systemd/system/polkit-agent-helper.socket').is_file():
                invoke(['sudo', '-n', 'systemctl', 'start', 'polkit-agent-helper.socket'])
        install_previous(package_format, old_package)
        old_version = expected_version(package_format, previous['version'])
        verify_installed_package(package_format, old_version)
        report['checks']['previous_package_installed'] = True
        with tempfile.TemporaryDirectory(prefix='cor-gui-update-checksum-') as temporary:
            checksum = Path(temporary)/'package.sha256'
            download(published['checksum_url'], checksum, digest=published['checksum_sha256'])
            require(checksum.read_text(encoding='ascii').strip() == published['sha256'],
                    'The published checksum differs from the package digest')
        report['checks']['published_checksum_verified'] = True
        installed_tests(update_case={
            'format': package_format, 'published': published, 'previous_version': previous['version'],
            'old_native_version': old_version,
            'new_native_version': expected_version(package_format, published['version']),
        }, package_lock=lambda: package_manager_lock(package_format))
        desktop = json.loads((GUI_REPORT.parent/'desktop-update'/'gui.json').read_text())
        checks = desktop.get('checks', {})
        require(desktop.get('status') == 'passed'
                and checks.get('version_visible_in_gui') is True
                and checks.get('cancel', {}).get('outcome') == 'failed'
                and checks.get('locked', {}).get('outcome') == 'failed'
                and checks['locked'].get('native_lock_conflict')
                and checks.get('retry', {}).get('outcome') == 'succeeded'
                and checks['retry'].get('gui_reopened') is True,
                'No passing graphical update session report')
        report['checks'].update(checks)
        verify_installed_package(package_format, expected_version(package_format, published['version']))
        report['status'] = 'passed'
    except BaseException as error:
        report['error'] = f'{type(error).__name__}: {error}'
        if getattr(error, '__notes__', None):
            report['error_notes'] = error.__notes__
        raise
    finally:
        write_json(GUI_REPORT, report)


@contextmanager
def terminal_authorization():
    import pwd
    user = pwd.getpwuid(os.geteuid()).pw_name
    require(re.fullmatch(r'[a-zA-Z_][a-zA-Z0-9_.-]*\$?', user) is not None,
            'Invalid compatibility test user')
    destination = Path('/etc/sudoers.d')/f'cor-update-{secrets.token_hex(12)}'
    with tempfile.TemporaryDirectory(prefix='cor-update-sudo-') as temporary:
        directory = Path(temporary)
        directory.chmod(0o755)
        source = directory/'sudoers'
        source.write_text(f'Defaults:{user} verifypw=never\n')
        source.chmod(0o644)
        invoke(['sudo', '-n', 'visudo', '-cf', source])
        invoke(['sudo', '-n', 'install', '-m', '0440', source, destination])
        try:
            invoke(['sudo', '-n', '-v'])
            yield
        finally:
            invoke(['sudo', '-n', 'rm', '-f', '--', destination])


def require_disposable_update_runner():
    if (os.environ.get('GITHUB_ACTIONS') != 'true' or sys.platform != 'linux'
            or os.geteuid() == 0 or (os.environ.get('COR_LINUX_GUEST') != 'true'
                                     and os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted')):
        raise RuntimeError('Published update tests require a disposable Actions runner or compatibility VM')


def exercise_cli(directory):
    require_disposable_update_runner()
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
                        authorization = terminal_authorization() if package_format == 'deb' else nullcontext()
                        with authorization:
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

    return fixture, old_package


def exercise(directory):
    fixture, old_package = exercise_cli(directory)
    if gui_target(fixture['format'], fixture['arch']):
        exercise_gui(fixture, old_package)


if __name__ == '__main__':
    if len(sys.argv) != 2:
        raise SystemExit('Pass the verified update fixture directory')
    exercise(Path(sys.argv[1]))
