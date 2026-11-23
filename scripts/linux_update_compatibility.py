"""Verify an installed Linux client's update against a published release."""
import base64
from contextlib import ExitStack, contextmanager, nullcontext
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import secrets
import selectors
import subprocess
import sys
import tempfile
import time

import compatibility
from build_support import (ROOT, deb_version, download, host_arch, manifest_version,
                           sha256, validate_version, write_json)
from linux_native_package import arch_filename, native_version, rpm_filename
from linux_update_retry import RATE_LIMIT_MESSAGES, ReleaseRateLimit, retry_release_check
from workflow_artifacts import fetch_package_inputs, release_module

APP = Path('/opt/clash-of-rust')
REPORT = ROOT/'target/linux-report/published-update.json'
GUI_REPORT = REPORT.with_name('published-gui-update.json')
INSTALL_ENV = ('HOME', 'USER', 'LOGNAME', 'PATH', 'LANG', 'LANGUAGE', 'LC_ALL', 'LC_CTYPE',
               'LC_MESSAGES', 'TMPDIR', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_CACHE_HOME',
               'XDG_RUNTIME_DIR', 'DBUS_SESSION_BUS_ADDRESS', 'GITHUB_ACTIONS',
               'COR_LINUX_GUEST', 'COR_LINUX_APT_CACHE', 'RUNNER_ENVIRONMENT')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def version_precedence(version):
    base, separator, prerelease = validate_version(version).partition('-')
    identifiers = tuple((0, len(part), part) if part.isdigit() else (1, 0, part)
                        for part in prerelease.split('.')) if separator else ()
    return tuple(map(int, base.split('.'))), not separator, identifiers


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
    require(version_precedence(published['version']) > version_precedence(version),
            'Published version must be newer than the previous version')
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


def installation_command(command, unit, description, timeout):
    return ['sudo', '-n', '/usr/bin/systemd-run', '--scope', '--quiet', '--unit='+unit,
            '--description='+description,
            '--property=KillMode=control-group', '--property=TimeoutStopSec=5s',
            f'--property=RuntimeMaxSec={timeout+120}s', '--',
            '/usr/bin/setpriv', '--reuid', str(os.getuid()), '--regid', str(os.getgid()), '--init-groups',
            sys.executable, '-I', '-B', '-c',
            'import json, os, sys\n'
            'from pathlib import Path\n'
            'with open("/proc/self/cgroup") as groups:\n'
            '    memberships = [line.rstrip().split(":", 2) for line in groups]\n'
            'for _, controllers, group in memberships:\n'
            '    if controllers not in ("", "name=systemd") or not group.endswith("/" + sys.argv[1]): continue\n'
            '    for mount in ("", "unified", "systemd"):\n'
            '        directory = Path("/sys/fs/cgroup") / mount / group.lstrip("/")\n'
            '        procs = directory / "cgroup.procs"\n'
            '        if procs.is_file() and str(os.getpid()) in procs.read_text().splitlines():\n'
            '            print(directory, flush=True)\n'
            '            break\n'
            '    else: continue\n'
            '    break\n'
            'else: sys.exit("Missing installation cgroup")\n'
            'payload = sys.stdin.read()\n'
            'if not payload: sys.exit(125)\n'
            'with open(os.devnull, "rb") as null: os.dup2(null.fileno(), 0)\n'
            'os.execvpe(sys.argv[2], sys.argv[2:], json.loads(payload))',
            unit, *command]


def confirm_installation_scope(child, unit, checkpoint):
    with selectors.DefaultSelector() as selector:
        selector.register(child.stdout, selectors.EVENT_READ)
        deadline = time.monotonic()+15
        while True:
            checkpoint()
            if selector.select(.2):
                ready = os.read(child.stdout.fileno(), 4096).decode()
                break
            if time.monotonic() >= deadline:
                raise TimeoutError('Installation command did not enter its dedicated cgroup')
    cgroup = Path(ready.rstrip('\n'))
    require(ready.count('\n') == 1 and ready.endswith('\n') and cgroup.name == unit
            and cgroup.is_relative_to('/sys/fs/cgroup') and '..' not in cgroup.parts,
            'Installation command did not enter its dedicated cgroup')
    return cgroup


def stop_installation_scope(unit, description, cgroup):
    from linux_gui import stop_fixture_unit

    stop_fixture_unit(unit, Description=description)
    if cgroup is not None:
        try:
            events = (cgroup/'cgroup.events').read_text().splitlines()
        except FileNotFoundError:
            require(not any(path.read_text().strip() for path in cgroup.rglob('cgroup.procs')),
                    'Installation cgroup still contains processes')
        else:
            require('populated 0' in events, 'Installation cgroup still contains processes')


def invoke(command, *, timeout=120, check=True, start_new_session=False, check_interrupted=None,
           installation_scope=None):
    command = list(map(str, command))
    if check_interrupted is None and installation_scope is None:
        result = subprocess.run(command, capture_output=True, text=True, stdin=subprocess.DEVNULL,
                                timeout=timeout, start_new_session=start_new_session)
    else:
        from linux_gui import FixtureProcess, cleanup_steps, stop_process_group

        checkpoint = check_interrupted or (lambda: None)
        checkpoint()
        with ExitStack() as streams:
            launch = command
            stdin = subprocess.DEVNULL
            if installation_scope is not None:
                unit = 'cor-install-' + secrets.token_hex(16) + '.scope'
                description = 'Clash of Rust installation fixture ' + secrets.token_hex(16)
                launch = installation_command(command, unit, description, timeout)
                stdin = subprocess.PIPE
            child = FixtureProcess(launch, stdin=stdin, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True, start_new_session=True)
            streams.callback(child.stdout.close)
            streams.callback(child.stderr.close)
            payload = None
            try:
                if installation_scope is not None:
                    streams.callback(child.stdin.close)
                    cgroup = None
                    installation_scope.callback(
                        cleanup_steps, ('stop installation cgroup',
                                        lambda: stop_installation_scope(unit, description, cgroup)))
                    cgroup = confirm_installation_scope(child, unit, checkpoint)
                    checkpoint()
                    payload = json.dumps({key: os.environ[key] for key in INSTALL_ENV if key in os.environ})
                deadline = time.monotonic() + timeout
                while True:
                    remaining = deadline - time.monotonic()
                    try:
                        stdout, stderr = child.communicate(input=payload, timeout=max(0, min(.25, remaining)))
                        break
                    except subprocess.TimeoutExpired:
                        payload = None
                        checkpoint()
                        if time.monotonic() >= deadline:
                            raise subprocess.TimeoutExpired(command, timeout) from None
                checkpoint()
                result = subprocess.CompletedProcess(command, child.poll(), stdout, stderr)
            except BaseException:
                if installation_scope is not None:
                    cleanup_steps(('close isolated installation input', child.stdin.close),
                                  ('stop isolated installation', installation_scope.close))
                cleanup_steps(('reap update command process group', lambda: stop_process_group(child)))
                raise
            if installation_scope is None:
                cleanup_steps(('reap update command process group', lambda: stop_process_group(child)))
            else:
                child.reap()
    if check:
        require_command_success(result)
    return result


def require_command_success(result):
    if result.returncode:
        raise RuntimeError(f'Command failed ({result.returncode}): {(result.stdout + result.stderr)[-4000:]}')


def cli(data, *arguments, timeout=120, check=True, check_interrupted=None, installation_scope=None):
    command = [APP/'cor', '--data-dir', data, '--resources', APP/'resources', '--json', *arguments]
    if arguments[:2] not in (('update', 'check'), ('update', 'download'), ('update', 'install')):
        result = invoke(command, timeout=timeout, check=check, check_interrupted=check_interrupted,
                        installation_scope=installation_scope)
    else:
        def attempt():
            with ExitStack() as scope:
                result = invoke(command, timeout=timeout, check=False, check_interrupted=check_interrupted,
                                installation_scope=scope if installation_scope is not None else None)
                limited = (result.returncode and any(message in result.stderr for message in RATE_LIMIT_MESSAGES)
                           and not os.path.lexists(data/'update-install.json'))
                if not limited:
                    if installation_scope is not None:
                        installation_scope.callback(scope.pop_all().close)
                    return result
            raise ReleaseRateLimit(result.stderr.strip())

        result = retry_release_check(attempt, ' '.join(arguments[:2]), check_interrupted or (lambda: None))
        if check:
            require_command_success(result)
    if not check:
        return result
    return json.loads(result.stdout)['data']


def endpoint(data):
    try:
        return json.loads((data/'control.json').read_text())['pid']
    except (FileNotFoundError, ValueError, KeyError):
        return None


def wait_for(probe, description, timeout=60):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = probe()
        if value:
            return value
        time.sleep(.2)
    raise TimeoutError(description)


def run_installation(code, *arguments, check_interrupted):
    with ExitStack() as scope:
        invoke([sys.executable, '-I', '-B', '-c',
                'import sys\nsys.path.insert(0, sys.argv.pop(1))\n'+code,
                Path(__file__).resolve().parent, *arguments], timeout=1200,
               check_interrupted=check_interrupted, installation_scope=scope)


def install_previous(package_format, package, *, check_interrupted=None):
    if check_interrupted is not None:
        return run_installation(
            'from pathlib import Path\nfrom linux_update_compatibility import install_previous\n'
            'install_previous(sys.argv[1], Path(sys.argv[2]))',
            package_format, package, check_interrupted=check_interrupted)
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
    if package_format == 'rpm':
        name, epoch, version = invoke([
            'rpm', '-q', '--qf', '%{NAME}\n%{EPOCHNUM}\n%{VERSION}-%{RELEASE}', 'clash-of-rust',
        ], timeout=15).stdout.splitlines()
        require(name == 'clash-of-rust' and epoch == '0',
                'Installed RPM identity or epoch differs from the release packages')
        return version
    from linux_native_guest import package_version
    return package_version()


def expected_version(package_format, version):
    if package_format == 'deb':
        return deb_version(version)
    return native_version(version, '~' if package_format == 'rpm' else 'pre.') + '-1'


def verify_upgrade_order(fixture):
    previous, published = (fixture[key]['version'] for key in ('previous', 'published'))
    require(version_precedence(published) > version_precedence(previous),
            'Published version must be newer than the previous version')
    package_format = fixture['format']
    old_native = expected_version(package_format, previous)
    new_native = expected_version(package_format, published)
    if package_format == 'deb':
        require(invoke(['dpkg', '--compare-versions', old_native, 'lt', new_native], timeout=15,
                       check=False).returncode == 0,
                f'DEB package manager did not order {old_native} before {new_native}')
    else:
        from linux_native_guest import verify_upgrade_order as verify_native_upgrade_order

        verify_native_upgrade_order(old_native, new_native)


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
                                 stdout=subprocess.PIPE, stderr=errors, start_new_session=True)
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
                    invoke([*command, 'cleanup', marker], timeout=15, start_new_session=True)
                steps.append(('remove owned Arch database lock', remove_owned_lock))
            steps.append(('close package lock status pipe', child.stdout.close))
            cleanup_steps(*steps)


def update_receipt(fixture):
    return {'schema': 1, 'status': 'running', 'format': fixture['format'],
            'previous': fixture['previous'], 'published': fixture['published'], 'checks': {}}


def write_receipt(path, receipt):
    path.parent.mkdir(parents=True, exist_ok=True)
    require(not path.is_symlink(), 'The update receipt path is a symbolic link')
    with tempfile.TemporaryDirectory(prefix='.cor-update-receipt-', dir=path.parent) as directory:
        staged = Path(directory)/path.name
        write_json(staged, receipt)
        staged.replace(path)


def write_gui_receipt(receipt):
    write_receipt(GUI_REPORT, receipt)


def interrupt_receipts(identity, error):
    from linux_gui import cleanup_steps, read_object

    failure = {**identity, 'status': 'failed', 'failed_step': 'interrupted',
               'error': type(error).__name__}

    def record(path):
        current = read_object(path) if not path.is_symlink() else {}
        if current.get('exercise_id') == identity['exercise_id']:
            write_receipt(path, {**current, **failure})

    def record_desktop():
        gui = read_object(GUI_REPORT) if not GUI_REPORT.is_symlink() else {}
        name = gui.get('desktop_report')
        if (gui.get('exercise_id') != identity['exercise_id'] or not isinstance(name, str)
                or not re.fullmatch(r'desktop-update-[a-z0-9_]+', name)):
            return
        directory = GUI_REPORT.parent/name
        path = directory/'gui.json'
        if not directory.is_symlink() and not path.is_symlink():
            desktop = read_object(path)
            if all(desktop.get(key) == value for key, value in identity.items()):
                write_receipt(path, {**desktop, **failure})

    cleanup_steps(('invalidate terminal receipt', lambda: record(REPORT)),
                  ('invalidate graphical receipt', lambda: record(GUI_REPORT)),
                  ('invalidate desktop receipt', record_desktop))


def verify_desktop_receipt(desktop, identity, package_format, digest, old_version, new_version):
    require(isinstance(desktop, dict) and desktop.get('status') == 'passed'
            and all(desktop.get(key) == value for key, value in identity.items()),
            'No passing graphical update session report for this exercise')
    checks = desktop['checks']
    tool = {'deb': '/usr/bin/dpkg', 'pkg': '/usr/bin/pacman'}[package_format]
    require(checks.get('version_visible_in_gui') is True, 'Published version was not visible in the GUI')
    for phase, version, outcome in (('cancel', old_version, 'failed'), ('locked', old_version, 'failed'),
                                    ('retry', new_version, 'succeeded')):
        result = checks.get(phase, {})
        require(result.get('outcome') == outcome and result.get('installed_native_version') == version
                and result.get('download_sha256') == digest and result.get('graphical_pkexec') == tool
                and all(result.get(key) is True for key in ('gui_usable', 'sealed_download',
                        'native_package_complete', 'settings_preserved', 'subscription_preserved')),
                f'Incomplete graphical update evidence for {phase}')
    require(all(checks[phase].get('original_gui_owner_retained') is True
                and checks[phase].get('core_still_stopped_after_failure') is True
                for phase in ('cancel', 'locked'))
            and checks['locked'].get('native_lock_conflict')
            and checks['retry'].get('gui_reopened') is True
            and checks['retry'].get('core_started_and_stopped_by_fixture_after_reopen') is True,
            'Incomplete graphical installation failure or reopen evidence')
    return checks


def exercise_gui(fixture, old_package, *, check_interrupted=None, run_identity=None):
    from linux_gui import fixture_sudo, installed_tests, verify_installed_package

    require_disposable_update_runner()
    package_format = fixture['format']
    require(gui_target(package_format, fixture['arch']), 'No graphical update target on this runner')
    previous, published = fixture['previous'], fixture['published']
    identity = run_identity or {'exercise_id': secrets.token_hex(16)}
    report = {**update_receipt(fixture), **identity}
    checkpoint = check_interrupted or (lambda: None)
    step = 'prepare-desktop'
    try:
        GUI_REPORT.parent.mkdir(parents=True, exist_ok=True)
        desktop_report = Path(tempfile.mkdtemp(prefix='desktop-update-', dir=GUI_REPORT.parent))
        report['desktop_report'] = desktop_report.name
        write_gui_receipt(report)
        checkpoint()
        if package_format == 'deb':
            run_installation('from linux_apt import install\ninstall(sys.argv[1:])',
                             'xvfb', 'xauth', 'x11-utils', 'xdotool', 'wmctrl', 'openbox',
                             'policykit-1-gnome', 'fonts-noto-cjk', 'scrot', 'tesseract-ocr',
                             'tesseract-ocr-chi-sim', 'tesseract-ocr-eng', 'python3-pil', 'strace',
                             check_interrupted=checkpoint)
        else:
            run_installation('from linux_native_guest import install_dependencies\n'
                             'install_dependencies(sys.argv[1:])',
                             'xorg-server-xvfb', 'xorg-xauth', 'xorg-xdpyinfo', 'xorg-xwininfo',
                             'xdotool', 'wmctrl', 'openbox', 'mate-polkit', 'noto-fonts-cjk',
                             'scrot', 'tesseract', 'tesseract-data-chi_sim', 'tesseract-data-eng',
                             'python-pillow', 'strace', check_interrupted=checkpoint)
            checkpoint()
            if Path('/usr/lib/systemd/system/polkit-agent-helper.socket').is_file():
                fixture_sudo('systemctl', 'start', 'polkit-agent-helper.socket')
        checkpoint()
        step = 'install-previous'
        install_previous(package_format, old_package, check_interrupted=checkpoint)
        checkpoint()
        old_version = expected_version(package_format, previous['version'])
        new_version = expected_version(package_format, published['version'])
        verify_installed_package(package_format, old_version)
        report['checks']['previous_package_installed'] = True
        step = 'verify-published-checksum'
        with tempfile.TemporaryDirectory(prefix='cor-gui-update-checksum-') as temporary:
            checksum = Path(temporary)/'package.sha256'
            download(published['checksum_url'], checksum, digest=published['checksum_sha256'])
            require(checksum.read_text(encoding='ascii').strip() == published['sha256'],
                    'The published checksum differs from the package digest')
        report['checks']['published_checksum_verified'] = True
        step = 'desktop-session'
        checkpoint()
        installed_tests(update_case={
            **identity,
            'format': package_format, 'published': published, 'previous_version': previous['version'],
            'old_native_version': old_version,
            'new_native_version': new_version,
        }, package_lock=lambda: package_manager_lock(package_format), update_report=desktop_report,
            check_interrupted=check_interrupted)
        checkpoint()
        desktop = json.loads((desktop_report/'gui.json').read_text())
        report['checks'].update(verify_desktop_receipt(
            desktop, identity, package_format, published['sha256'], old_version, new_version))
        step = 'verify-updated-package'
        verify_installed_package(package_format, new_version)
        checkpoint()
        report['status'] = 'passed'
    except BaseException as error:
        report.update(status='failed', failed_step=step, error=type(error).__name__)
        raise
    finally:
        write_gui_receipt(report)


@contextmanager
def terminal_authorization(*, check_interrupted=None):
    import pwd
    from linux_gui import cleanup_steps, fixture_sudo

    checkpoint = check_interrupted or (lambda: None)
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
        checkpoint()
        fixture_sudo('visudo', '-cf', source, timeout=15)
        checkpoint()
        try:
            fixture_sudo('install', '-m', '0440', source, destination, timeout=15)
            checkpoint()
            invoke(['sudo', '-n', '-v'], timeout=15, start_new_session=True)
            checkpoint()
            yield
        finally:
            cleanup_steps(('remove fixture sudoers rule',
                           lambda: fixture_sudo('rm', '-f', '--', destination, timeout=15)))


def require_disposable_update_runner():
    if (os.environ.get('GITHUB_ACTIONS') != 'true' or sys.platform != 'linux'
            or os.geteuid() == 0 or (os.environ.get('COR_LINUX_GUEST') != 'true'
                                     and os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted')):
        raise RuntimeError('Published update tests require a disposable Actions runner or compatibility VM')


def exercise_cli(fixture, old_package, *, check_interrupted=None, run_identity=None):
    from linux_gui import stop_child

    require_disposable_update_runner()
    checkpoint = check_interrupted or (lambda: None)
    package_format = fixture['format']
    previous, published = fixture['previous'], fixture['published']
    report = {**update_receipt(fixture), **(run_identity or {}), 'status': 'failed'}
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    try:
        checkpoint()
        install_previous(package_format, old_package, check_interrupted=checkpoint)
        checkpoint()
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
            checkpoint()
            fixture_file = work/'fixture.yaml'
            fixture_file.write_text("proxies: []\nproxy-groups: []\nrules: ['MATCH,DIRECT']\n")
            with (work/'daemon.log').open('w') as log, ExitStack() as installation:
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
                    available = cli(data, 'update', 'check', timeout=300, check_interrupted=check_interrupted)
                    checkpoint()
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
                        refused = cli(data, 'update', 'install', '--yes', check=False, timeout=300,
                                      check_interrupted=check_interrupted)
                        require(refused.returncode != 0 and manager in refused.stdout + refused.stderr,
                                'RPM installation did not direct the user to its package manager')
                        report['checks']['package_manager_hint'] = True
                        candidate = work/published['package']
                        download(published['url'], candidate, digest=published['sha256'])
                        checkpoint()
                        run_installation('from pathlib import Path\nfrom linux_native_guest import install_package\n'
                                         'install_package(Path(sys.argv[1]))', candidate,
                                         check_interrupted=checkpoint)
                        require(daemon.pid == original, 'Endpoint did not belong to the fixture backend')
                        stop_child(daemon)
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
                        result = cli(data, 'update', 'download', '--output', downloaded, timeout=600,
                                     check_interrupted=check_interrupted)
                        require(result.get('sha256') == published['sha256']
                                and sha256(downloaded) == published['sha256'],
                                'The downloaded update differs from the published package')
                        report['checks']['published_download_verified'] = True
                        checkpoint()
                        authorization = (terminal_authorization(check_interrupted=checkpoint)
                                         if package_format == 'deb' else nullcontext())
                        with authorization:
                            cli(data, 'update', 'install', '--yes', '--no-start', timeout=900,
                                check_interrupted=checkpoint, installation_scope=installation)
                        checkpoint()
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
                    stopped_check = ('fixture_restarted_backend_without_core' if package_format == 'rpm'
                                     else 'stopped_core_preserved')
                    report['checks'][stopped_check] = True
                finally:
                    stop_child(daemon)
        checkpoint()
        report['status'] = 'passed'
    except (Exception, KeyboardInterrupt) as error:
        report['error'] = f'{type(error).__name__}: {error}'
        raise
    finally:
        write_receipt(REPORT, report)


def exercise(directory):
    from linux_gui import fixture_signals

    require_disposable_update_runner()
    identity = {'exercise_id': secrets.token_hex(16)}
    with fixture_signals(lambda error: interrupt_receipts(identity, error)) as check_interrupted:
        receipt = {'schema': 1, 'status': 'running', 'checks': {}, **identity}
        write_gui_receipt(receipt)
        write_receipt(REPORT, receipt)
        step = 'fixture'
        try:
            raw = (directory/'fixture.json').read_bytes()
            identity['fixture_sha256'] = hashlib.sha256(raw).hexdigest()
            fixture = json.loads(raw)
            require(isinstance(fixture, dict) and fixture.get('schema') == 1
                    and fixture['arch'] == host_arch() and fixture['format'] in ('deb', 'rpm', 'pkg'),
                    'Update fixture is incompatible')
            report_base = {**update_receipt(fixture), **identity}
            graphical = gui_target(fixture['format'], fixture['arch'])
            check_interrupted()
            old_package = fixture_package(directory, fixture)
            receipt = report_base
            verify_upgrade_order(fixture)
            check_interrupted()
            write_gui_receipt(receipt)
            step = 'terminal-update'
            exercise_cli(fixture, old_package, check_interrupted=check_interrupted, run_identity=identity)
            check_interrupted()
        except BaseException as error:
            receipt = {**receipt, **identity, 'status': 'failed', 'failed_step': step,
                       'error': type(error).__name__}
            write_gui_receipt(receipt)
            if step == 'fixture':
                write_receipt(REPORT, receipt)
            raise
        if graphical:
            exercise_gui(fixture, old_package, check_interrupted=check_interrupted, run_identity=identity)
        else:
            write_gui_receipt({**receipt, 'status': 'skipped'})


if __name__ == '__main__':
    if len(sys.argv) != 2:
        raise SystemExit('Pass the verified update fixture directory')
    exercise(Path(sys.argv[1]))
