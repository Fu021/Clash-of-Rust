"""Package verified CI binaries and test the installation; Cargo is not invoked."""
import argparse
import json
import os
from pathlib import Path
import platform
import re
import runpy
import select
import shutil
import signal
import subprocess
import sys
import tempfile
import time

from build_support import download, host_system, ROOT, bundle_directory, host_arch, run, sha256, write_json
from workflow_artifacts import commit, verify_build
from linux_apt import OPTIONS as APT_OPTIONS, install as install_packages, update_indexes
from linux_apt import privileged as run_privileged_with_timeout
from linux_compatibility_targets import UBUNTU_BASELINE
from linux_guest_environment import is_disposable_test_environment, is_local_guest


GUEST_PACKAGE_CONFIG = """\
path-exclude=/usr/share/doc/*
path-include=/usr/share/doc/*/copyright
path-exclude=/usr/share/man/*
path-include=/usr/share/man/man1/*
path-exclude=/usr/share/info/*
path-exclude=/usr/share/lintian/*
"""
GUEST_MAN_DB_MARKER = '/var/lib/man-db/auto-update'
PREVIOUS_DEB_VERSION = '0.7.2'
PREVIOUS_DEB_SHA256 = {
    'amd64': 'fac1162628914400446299d3bb22621cbc10a2c3b3aded03eec310458737c223',
    'arm64': '0bda8e2cf88cdedf2b047688b91d87616f6283e77a421a4398dddfde62e04b7d',
}


def prepare_guest_package_environment():
    """Keep disposable old-kernel VMs from indexing unrelated documentation."""
    if os.environ.get('COR_LINUX_GUEST') != 'true':
        return
    # man-db caches this debconf value in a marker file. Disable both so an
    # upgraded package cannot re-enable the expensive full index trigger.
    run(['sudo', 'debconf-set-selections'],
        input='man-db man-db/auto-update boolean false\n', text=True)
    run(['sudo', 'tee', '/etc/dpkg/dpkg.cfg.d/99-cor-ci-no-docs'],
        input=GUEST_PACKAGE_CONFIG, text=True)
    run(['sudo', 'rm', '-f', GUEST_MAN_DB_MARKER])


def linux_gui_smoke():
    """Boot the installed GUI in an isolated X11/DBus session, without screenshots."""
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('Installed GUI tests require a disposable GitHub Actions runner')
    processes = []
    with tempfile.TemporaryDirectory(prefix='cor-linux-gui-') as temporary:
        with (Path(temporary)/'startup.log').open('w+') as log:
            try:
                read_fd, write_fd = os.pipe()
                try:
                    display = subprocess.Popen(
                        ['Xvfb', '-displayfd', str(write_fd), '-screen', '0',
                         '1280x900x24', '-nolisten', 'tcp', '-ac'],
                        pass_fds=(write_fd,), stdout=log, stderr=subprocess.STDOUT,
                        start_new_session=True)
                    processes.append(display)
                finally:
                    os.close(write_fd)
                with os.fdopen(read_fd) as display_output:
                    if not select.select([display_output], [], [], 10)[0]:
                        raise RuntimeError('Xvfb did not provide a display within 10 seconds')
                    number = display_output.readline().strip()
                if not number.isdecimal() or display.poll() is not None:
                    raise RuntimeError('Xvfb failed to initialize')
                env = dict(os.environ, DISPLAY=':'+number,
                           CLASH_OF_RUST_DATA_DIR=str(Path(temporary)/'profile'),
                           GSETTINGS_BACKEND='memory')
                for key in ('WAYLAND_DISPLAY', 'WAYLAND_SOCKET', 'DBUS_SESSION_BUS_ADDRESS'):
                    env.pop(key, None)
                application = subprocess.Popen(
                    ['dbus-run-session', '--', '/opt/clash-of-rust/clash-of-rust'],
                    env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                processes.append(application)
                deadline = time.monotonic()+45
                while time.monotonic() < deadline:
                    if application.poll() is not None:
                        raise RuntimeError(f'Installed GUI exited with status {application.returncode}')
                    tree = subprocess.check_output(
                        ['xwininfo', '-root', '-tree'], env=env, text=True, timeout=5)
                    windows = re.findall(r'^\s*(0x[0-9a-fA-F]+)\s+"Clash of Rust', tree, re.M)
                    for window in windows:
                        info = subprocess.check_output(
                            ['xwininfo', '-id', window], env=env, text=True, timeout=5)
                        width = re.search(r'Width:\s+(\d+)', info)
                        height = re.search(r'Height:\s+(\d+)', info)
                        if ('Map State: IsViewable' in info and width and height
                                and int(width[1]) >= 800 and int(height[1]) >= 450):
                            time.sleep(2)
                            if application.poll() is not None:
                                raise RuntimeError('Installed GUI exited after showing its window')
                            print(f'Installed Linux GUI started: {width[1]}×{height[1]} mapped window',
                                  flush=True)
                            return
                    time.sleep(0.25)
                raise RuntimeError('Installed GUI did not show a usable window within 45 seconds')
            except Exception:
                log.flush()
                log.seek(0)
                print(log.read(), flush=True)
                raise
            finally:
                for process in reversed(processes):
                    try:
                        os.killpg(process.pid, signal.SIGTERM)
                    except ProcessLookupError:
                        pass
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait(timeout=5)


def require_linux_polkit_tools():
    for tool in ('pkexec', 'pkttyagent'):
        if not shutil.which(tool):
            raise RuntimeError('The installed package did not provide the polkit tool: ' + tool)


def install_linux_deb(package):
    # The privileged launcher requires an entirely root-owned installed path.
    run(['sudo', 'chown', 'root:root', '/opt'])
    run(['sudo', 'chmod', '0755', '/opt'])
    if os.environ.get('COR_LINUX_BASELINE'):
        # The build image removes package indexes after installing build tools.
        update_indexes()
    package_argument = './'+str(package.relative_to(ROOT))
    install_packages([package_argument])


def linux_package_tests(package, tun_test, structure=True):
    from ci_test_policy import extended_tests, update_tests
    if structure:
        # Build/manifest checks already verify payload identity and resources.
        run(['dpkg-deb', '--info', package])
    install_linux_deb(package)
    if os.environ.get('COR_LINUX_BASELINE'):
        require_linux_polkit_tools()
    if not any(Path(path).is_file() for path in (
            '/usr/lib/systemd/systemd-resolved', '/lib/systemd/systemd-resolved')):
        release = Path('/etc/os-release').read_text()
        install_packages(['systemd' if 'VERSION_ID="20.04"' in release else 'systemd-resolved'])
    if not shutil.which('resolvectl'):
        raise RuntimeError('Installed distribution has no resolvectl')
    if extended_tests() and os.environ.get('COR_LINUX_BASELINE'):
        # This checks old user-space libraries; its host kernel is recorded
        # separately and never presented as minimum-kernel evidence.
        linux_gui_smoke()
    elif extended_tests():
        release = platform.freedesktop_os_release()
        if release.get('ID') != 'linuxmint' and (release.get('ID'), release.get('VERSION_ID')) != ('pop', '24.04'):
            from linux_gui import installed_tests
            installed_tests()
    if not os.environ.get('COR_LINUX_BASELINE'):
        run([sys.executable, ROOT/'scripts/test-proxy-modes.py',
             '--test-binary', tun_test, '--bundle', '/opt/clash-of-rust'])
        if update_tests():
            run([sys.executable, ROOT/'scripts/test-cli-update.py', '--package', package], timeout=900)


def compatibility_dependencies(*, release=None, extended=None):
    from ci_test_policy import extended_tests
    from ubuntu_vm_environment import TEST_PACKAGES
    if extended is None:
        extended = extended_tests()
    if extended:
        release = release if release is not None else platform.freedesktop_os_release()
        distribution = release.get('ID')
        if distribution == 'debian':
            return ['mate-polkit' if package == 'policykit-1-gnome' else package
                    for package in TEST_PACKAGES]
        if distribution == 'linuxmint' or (distribution, release.get('VERSION_ID')) == ('pop', '24.04'):
            return ['git', 'dbus-x11', 'iproute2', 'util-linux', 'libcap2-bin',
                    'x11-utils', 'xdotool', 'wmctrl']
        return list(TEST_PACKAGES)
    if os.environ.get('COR_LINUX_GUEST') == 'true':
        release = release if release is not None else platform.freedesktop_os_release()
        if (release.get('ID'), release.get('VERSION_ID')) == ('ubuntu', UBUNTU_BASELINE):
            return list(TEST_PACKAGES)
    basic = ['dbus-x11', 'iproute2', 'util-linux', 'libcap2-bin']
    if os.environ.get('COR_LINUX_COMPAT_CONTAINER') != 'true':
        basic.insert(0, 'git')
    return basic


def compatibility_download_dependencies():
    dependencies = runpy.run_path(str(ROOT/'scripts/build-installer.py'))['GUI_DEPENDENCIES']
    return list(dict.fromkeys([
        *compatibility_dependencies(),
        *(dependency.split('|', 1)[0].strip() for dependency in dependencies),
    ]))


def prepare_compatibility_environment():
    if not is_disposable_test_environment():
        raise RuntimeError('Compatibility preparation requires a disposable Actions runner or VM')
    if os.environ.get('COR_UBUNTU_VM_FROZEN') == 'true':
        from build_inputs import manifest
        from ubuntu_vm_environment import (
            check_environment, frozen_environments, inspect_environment, verify_frozen,
        )
        value = manifest()
        frozen = frozen_environments(value or {}).get(host_arch())
        if not frozen or frozen.get('status') != 'frozen':
            raise ValueError('Ubuntu guest lacks the frozen CI environment')
        verify_frozen(Path(os.environ['BUILD_INPUTS_DIR']), frozen, value['upstream']['linux'])
        check_environment(inspect_environment(), frozen['lock'])
        return
    # The hosted image's Azure mirror can stall on package indices even after
    # falling back for InRelease. Use the official HTTPS mirrors consistently.
    release = platform.freedesktop_os_release()
    if release.get('ID') == 'ubuntu':
        archive = 'https://archive.ubuntu.com/ubuntu/'
        security = 'https://security.ubuntu.com/ubuntu/'
        if host_arch() == 'arm64' and release.get('VERSION_ID') in ('20.04', '22.04', '24.04'):
            archive = security = 'https://ports.ubuntu.com/ubuntu-ports/'
        for name, mirror in (('apt-mirrors.txt', archive), ('apt-mirrors-security.txt', security)):
            path = Path('/etc/apt')/name
            if path.is_file():
                run(['sudo', 'tee', path], input=mirror+'\n', text=True)
    prepare_guest_package_environment()
    if not os.environ.get('COR_LINUX_GUEST'):
        update_indexes()
    package_timeout = 1800 if platform.machine().lower() in ('aarch64', 'arm64') else 900
    if os.environ.get('COR_LINUX_VM_ACCELERATION') == 'tcg' and is_local_guest():
        package_timeout = 3600
    install_packages(compatibility_dependencies(), package_timeout, no_recommends=True)


def run_upgrade_cli(data, report, phase, *commands):
    from linux_gui import process, wait_for

    command = ['/usr/bin/cor', '--data-dir', data]
    with (report/(phase+'-daemon.log')).open('w') as log:
        with process([*command, 'run', '--no-core', '--quiet'], log) as daemon:
            def ready():
                if daemon.poll() is not None:
                    raise RuntimeError('Upgrade fixture control daemon exited early')
                try:
                    endpoint = json.loads((data/'control.json').read_text())
                except (FileNotFoundError, ValueError):
                    return False
                return endpoint.get('pid') == daemon.pid

            wait_for('Upgrade fixture control endpoint did not start', ready)
            results = []
            for arguments in commands:
                response = subprocess.check_output(
                    [*command, '--json', *arguments], text=True, timeout=30)
                results.append(json.loads(response)['data'])
            return results


def stage_previous_deb(directory, architecture, source=None):
    digest = PREVIOUS_DEB_SHA256[architecture]
    name = f'Clash-of-Rust-{PREVIOUS_DEB_VERSION}-linux-{architecture}.deb'
    destination = directory/name
    if source is None:
        url = f'https://github.com/Fu021/Clash-of-Rust/releases/download/v{PREVIOUS_DEB_VERSION}/{name}'
        download(url, destination, digest=digest)
    else:
        if not is_local_guest():
            raise RuntimeError('A staged upgrade input requires a disposable local VM')
        shutil.copyfile(source, destination)
        if sha256(destination) != digest:
            raise ValueError('Staged upgrade input differs from the published DEB')
    return destination, digest


def deb_upgrade_and_uninstall(package, *, previous_deb=None):
    distribution = platform.freedesktop_os_release().get('ID')
    supported = distribution in ('ubuntu', 'debian', 'linuxmint', 'pop', 'zorin')
    if (os.geteuid() == 0 or os.environ.get('COR_LINUX_GUEST') != 'true'
            or not is_disposable_test_environment() or not supported):
        raise RuntimeError('DEB installation lifecycle requires an ordinary user in a disposable compatibility VM')
    architecture = subprocess.check_output(['dpkg-deb', '--field', package, 'Architecture'], text=True).strip()
    report = ROOT/'target/linux-report'/f'{distribution}-lifecycle'
    report.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='cor-deb-lifecycle-') as temporary:
        directory = Path(temporary)
        directory.chmod(0o755)
        previous, previous_digest = stage_previous_deb(directory, architecture, previous_deb)
        candidate_version = subprocess.check_output(
            ['dpkg-deb', '--field', package, 'Version'], text=True).strip()
        run(['dpkg', '--compare-versions', PREVIOUS_DEB_VERSION, 'lt', candidate_version])
        installed_version = ['dpkg-query', '--show', '--showformat=${Version}', 'clash-of-rust']
        purge = [*APT_OPTIONS, 'purge', '-y', 'clash-of-rust']
        verify_package_lifecycle(
            package, previous, candidate_version, PREVIOUS_DEB_VERSION, directory, report,
            lambda path: install_packages([path]), lambda: run_privileged_with_timeout(purge, 300),
            lambda: subprocess.check_output(installed_version, text=True),
            previous_source_digest=previous_digest)


def verify_package_lifecycle(package, previous, candidate_version, previous_version,
                             directory, report, install, remove, installed_version,
                             *, previous_source_digest):
    if installed_version() != candidate_version:
        raise RuntimeError('The clean installation did not install the candidate version')
    remove()
    install(previous)
    if installed_version() != previous_version:
        raise RuntimeError('Upgrade fixture is missing the previous installed version')
    data = directory/'profile'
    fixture = directory/'fixture.yaml'
    fixture.write_text("proxies: []\nproxy-groups: []\nrules: ['MATCH,DIRECT']\n")
    profile_name = 'Package upgrade fixture'
    list_profile = ('profile', 'list', '--filter', profile_name)
    _, _, profiles_before = run_upgrade_cli(
        data, report, 'before', ('config', 'set', 'update_prereleases', 'true'),
        ('profile', 'add', profile_name, fixture), list_profile)
    if len(profiles_before) != 1:
        raise RuntimeError('The previous installation could not save the upgrade fixture')
    profile_file = data/'profiles'/f'{profiles_before[0]["id"]}.yaml'
    profile_content = profile_file.read_bytes()

    install(package)
    if installed_version() != candidate_version:
        raise RuntimeError('Package upgrade did not install the candidate version')
    settings, profiles_after = run_upgrade_cli(
        data, report, 'after', ('config', 'get', 'update_prereleases'), list_profile)
    if settings != {'update_prereleases': True}:
        raise RuntimeError('Package upgrade lost the saved update channel')
    if [(profile['id'], profile['name']) for profile in profiles_after] != [
        (profiles_before[0]['id'], profile_name),
    ]:
        raise RuntimeError('Package upgrade lost the saved profile')
    if profile_file.read_bytes() != profile_content:
        raise RuntimeError('Package upgrade changed the saved profile content')
    user_files = [data/'settings.json', data/'profiles.json', profile_file]
    contents = [path.read_bytes() for path in user_files]
    remove()
    artifacts = ('/opt/clash-of-rust', '/usr/bin/cor', '/usr/bin/clash-of-rust',
                 '/usr/share/applications/clash-of-rust.desktop',
                 '/usr/share/polkit-1/actions/org.clashofrust.tun.policy')
    if any(os.path.lexists(path) for path in artifacts):
        raise RuntimeError('Package uninstall left installed application files')
    if [path.read_bytes() for path in user_files] != contents:
        raise RuntimeError('Package uninstall changed user data')
    write_json(report/'result.json', {'previous_version': previous_version,
                                     'previous_sha256': sha256(previous),
                                     'previous_source_sha256': previous_source_digest,
                                     'installed_version': candidate_version,
                                     'candidate_sha256': sha256(package),
                                     'upgrade_preserved_settings_and_profile': True,
                                     'uninstall_preserved_user_data': True})


def native_upgrade_and_uninstall(deb, *, previous_deb=None):
    from linux_native_guest import (
        RPM, distribution, install_package, package_version, remove_package, verify_upgrade_order,
    )
    from linux_native_package import build_arch, build_rpm, native_version, verify_rpm

    distro = distribution()
    separator = '~' if distro in RPM else 'pre.'
    for old, new in (('0.7.3-fix.2.native-install', '0.7.3-fix.10.native-install'),
                     ('0.7.3-fix.10.native-install', '0.7.3'), ('0.7.3-1', '0.7.3')):
        verify_upgrade_order(native_version(old, separator)+'-1', native_version(new, separator)+'-1')
    report = ROOT/'target/linux-report'/f'{distro}-lifecycle'
    report.mkdir(parents=True, exist_ok=True)
    receipt = json.loads((deb.parent/'package-receipt.json').read_text())
    arch = receipt['arch']
    if distro in RPM:
        candidate = verify_rpm(deb.parent/'linux-rpm', receipt)
    else:
        candidate, = (deb.parent/'linux-arch').glob('*.pkg.tar.zst')
    architecture = {'x64': 'amd64', 'arm64': 'arm64'}[arch]
    with tempfile.TemporaryDirectory(prefix='cor-native-lifecycle-') as temporary:
        directory = Path(temporary)
        previous_source, previous_digest = stage_previous_deb(directory, architecture, previous_deb)
        previous = (build_rpm(previous_source, PREVIOUS_DEB_VERSION, directory, arch) if distro in RPM
                    else build_arch(previous_source, PREVIOUS_DEB_VERSION, directory, previous_digest))
        previous_version, candidate_version = package_version(previous), package_version(candidate)
        verify_upgrade_order(previous_version, candidate_version)
        verify_package_lifecycle(candidate, previous, candidate_version, previous_version,
                                 directory, report, install_package, remove_package, package_version,
                                 previous_source_digest=previous_digest)


def native_package_tests(deb, receipt, binary):
    from ci_test_policy import extended_tests
    from linux_native_guest import RPM, distribution, install_package, package_version
    from linux_native_package import build_arch, verify_rpm

    distro = distribution()
    package = (verify_rpm(deb.parent/'linux-rpm', receipt) if distro in RPM else
               build_arch(deb, receipt['version'], deb.parent/'linux-arch', receipt['sha256']))
    run(['sudo', 'chown', 'root:root', '/opt'])
    run(['sudo', 'chmod', '0755', '/opt'])
    install_package(package)
    if distro == 'arch' and package_version() != package_version(package):
        raise ValueError('Installed Arch version differs from the verified package')
    if distro == 'arch':
        run([sys.executable, ROOT/'scripts/test-cli-update.py', '--package', package,
             '--arch-version', receipt['version'], '--report', ROOT/'target/linux-report'],
            timeout=300)
    require_linux_polkit_tools()
    if extended_tests():
        from linux_gui import installed_tests
        installed_tests()
    run([sys.executable, ROOT/'scripts/test-proxy-modes.py',
         '--test-binary', binary, '--bundle', '/opt/clash-of-rust'])
    digest = sha256(package)
    write_json(ROOT/'target/linux-report/native-package.json',
               {'candidate': package.name, 'sha256': digest,
                'source_deb_sha256': receipt['sha256']})
    if distro == 'arch':
        write_json(package.parent/'arch-receipt.json', {
            'schema': 1, 'commit': receipt['commit'], 'ci_run_id': receipt['ci_run_id'],
            'source_sha256': receipt['sha256'], 'package': package.name, 'sha256': digest,
        })


def run_compatibility(distribution, version, arch, *, prepare_desktop_login=False):
    from ci_test_policy import extended_tests

    if not is_disposable_test_environment():
        raise RuntimeError('Compatibility tests require a disposable Actions runner or VM')
    release = platform.freedesktop_os_release()
    actual = (release.get('ID'), release.get('VERSION_ID', ''), host_arch())
    if actual != (distribution, version, arch):
        raise ValueError(f'Compatibility test expected {(distribution, version, arch)}, got {actual}')
    if prepare_desktop_login and (distribution != 'ubuntu' or os.environ.get('COR_LINUX_GUEST') != 'true'):
        raise RuntimeError('Desktop login preparation requires a disposable Ubuntu compatibility VM')
    packages = list((ROOT/'dist').glob('*.deb'))
    if len(packages) != 1:
        raise ValueError('Expected exactly one native Linux package')
    receipt = json.loads((ROOT/'dist/package-receipt.json').read_text())
    binary = ROOT/'target/linux-tun-test'
    if not binary.is_file():
        binary = ROOT/'dist/linux-tun-test'
    if (receipt['commit'] != commit() or receipt['system'] != 'linux' or receipt['arch'] != arch
        or receipt['tests_passed'] is not True or receipt['package'] != packages[0].name
        or receipt['sha256'] != sha256(packages[0])
        or receipt['linux_tun_test_sha256'] != sha256(binary)):
        raise ValueError('Compatibility inputs differ from the verified package/test binaries')
    binary.chmod(0o755)
    from linux_native_guest import DISTROS, prepare as prepare_native_environment
    native = distribution in DISTROS
    prepare = prepare_native_environment if native else prepare_compatibility_environment
    started = time.monotonic()
    prepared = False
    try:
        prepare()
        prepared = True
    finally:
        write_json(ROOT/'target/linux-report/compatibility-environment.json',
                   {'seconds': round(time.monotonic() - started, 3), 'success': prepared})
    if prepare_desktop_login:
        install_linux_deb(packages[0])
        return
    if native:
        native_package_tests(packages[0], receipt, binary)
    else:
        linux_package_tests(packages[0], binary, structure=False)
    if extended_tests():
        from cli_tui import verify
        installed = Path('/opt/clash-of-rust')
        verify(installed/'cor', installed/'resources', ROOT/'target/linux-report/tui')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--linux-compat', action='store_true',
                        help='Test a verified Linux package on a specified distribution and architecture')
    parser.add_argument('--prepare-desktop-login', action='store_true',
                        help='Install the verified DEB for the Ubuntu VM login probes')
    parser.add_argument('--system', choices=('windows', 'linux', 'macos'))
    parser.add_argument('--arch', choices=('x64', 'arm64'))
    parser.add_argument('--build', type=Path)
    parser.add_argument('--ci-run-id', type=int)
    parser.add_argument('--expect-linux', nargs=2, metavar=('DISTRIBUTION', 'VERSION'))
    parser.add_argument('--expect-arch', choices=('x64', 'arm64'))
    args = parser.parse_args()
    inputs = (args.system, args.arch, args.build, args.ci_run_id)
    if args.linux_compat:
        if any(value is not None for value in inputs):
            parser.error('--linux-compat does not accept packaging arguments')
        if args.expect_linux is None or args.expect_arch is None:
            parser.error('--linux-compat requires --expect-linux and --expect-arch')
        run_compatibility(*args.expect_linux, args.expect_arch, prepare_desktop_login=args.prepare_desktop_login)
        return
    if args.expect_linux is not None or args.expect_arch is not None or args.prepare_desktop_login:
        parser.error('Compatibility options require --linux-compat')
    if any(value is None for value in inputs):
        parser.error('--system, --arch, --build and --ci-run-id are required for packaging')
    if (args.system, args.arch) != (host_system(), host_arch()):
        raise ValueError('Packaging tests require the selected native platform')
    manifest = verify_build(args.build, args.system, args.arch, commit())
    # Installed-program checks consume the final package and compare it with
    # the verified CI bundle.
    shutil.copytree(args.build/'payload', bundle_directory(args.system, args.arch), dirs_exist_ok=True)
    run([sys.executable, ROOT/'scripts/build-installer.py', '--system', args.system,
         '--arch', args.arch, '--prebuilt', args.build])
    extension = f'windows-{args.arch}-setup.exe' if args.system == 'windows' else f'macos-{args.arch}.dmg' if args.system == 'macos' else f"linux-{'amd64' if args.arch == 'x64' else 'arm64'}.deb"
    package = ROOT/'dist'/f"Clash-of-Rust-{manifest['version']}-{extension}"
    defender = None
    if args.system == 'windows':
        from windows_ci import package_tests
        defender = package_tests(package, manifest, args.build)
    elif args.system == 'macos':
        from macos_ci import package_tests
        package_tests(package, manifest, args.build)
    else:
        tun = args.build/manifest['tests']['core_integration']
        linux_package_tests(package, tun)
        shutil.copy2(tun, ROOT/'dist/linux-tun-test')
    # Written only after every package/installed-program test has succeeded.
    receipt = {
        'schema': 1, 'commit': manifest['commit'], 'version': manifest['version'],
        'system': args.system, 'arch': args.arch, 'ci_run_id': args.ci_run_id,
        'build_artifact': json.loads((args.build/'source-artifact.json').read_text()),
        'package': package.name, 'sha256': sha256(package), 'tests_passed': True,
    }
    if args.system == 'linux':
        receipt['linux_tun_test_sha256'] = sha256(ROOT/'dist/linux-tun-test')
    if defender is not None:
        receipt['defender'] = defender
    from build_inputs import package_contents
    receipt['contents'] = package_contents(args.build)
    write_json(ROOT/'dist/package-receipt.json', receipt)


if __name__ == '__main__':
    main()
