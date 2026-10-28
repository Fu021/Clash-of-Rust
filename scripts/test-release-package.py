"""Package verified CI binaries and test the installation; Cargo is not invoked."""
import argparse
import json
import os
from pathlib import Path
import re
import select
import shutil
import signal
import subprocess
import sys
import tempfile
import time

from build_support import host_system, ROOT, bundle_directory, host_arch, run, sha256, write_json
from workflow_artifacts import commit, verify_build


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


def linux_package_tests(package, tun_test, structure=True):
    if structure:
        # Build/manifest checks already verify payload identity and resources.
        run(['dpkg-deb', '--info', package])
    # The privileged launcher requires an entirely root-owned installed path.
    run(['sudo', 'chown', 'root:root', '/opt'])
    run(['sudo', 'chmod', '0755', '/opt'])
    run(['sudo', 'apt-get', 'install', '-y', './'+str(package.relative_to(ROOT))])
    if not any(Path(path).is_file() for path in (
            '/usr/lib/systemd/systemd-resolved', '/lib/systemd/systemd-resolved')):
        release = Path('/etc/os-release').read_text()
        run(['sudo', 'apt-get', 'install', '-y',
             'systemd' if 'VERSION_ID="20.04"' in release else 'systemd-resolved'])
    if not shutil.which('resolvectl'):
        raise RuntimeError('Installed distribution has no resolvectl')
    if os.environ.get('COR_LINUX_BASELINE'):
        # This checks old user-space libraries; its host kernel is recorded
        # separately and never presented as minimum-kernel evidence.
        linux_gui_smoke()
    else:
        from linux_gui import installed_tests
        installed_tests()
    if not os.environ.get('COR_LINUX_BASELINE'):
        run([sys.executable, ROOT/'scripts/test-proxy-modes.py',
             '--test-binary', tun_test, '--bundle', '/opt/clash-of-rust'])


def run_compatibility(expected=None):
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('Compatibility tests require a disposable GitHub Actions runner')
    packages = list((ROOT/'dist').glob('*.deb'))
    if len(packages) != 1:
        raise ValueError('Expected exactly one native Linux package')
    if expected:
        actual = dict(line.split('=', 1) for line in Path('/etc/os-release').read_text().splitlines() if '=' in line)
        if actual.get('ID', '').strip('"') != 'ubuntu' or actual.get('VERSION_ID', '').strip('"') != expected:
            raise ValueError('Compatibility test distribution does not match '+expected)
    receipt = json.loads((ROOT/'dist/package-receipt.json').read_text())
    binary = ROOT/'target/linux-tun-test'
    if not binary.is_file():
        binary = ROOT/'dist/linux-tun-test'
    if (receipt['commit'] != commit() or receipt['package'] != packages[0].name
        or receipt['sha256'] != sha256(packages[0])
        or receipt['linux_tun_test_sha256'] != sha256(binary)):
        raise ValueError('Compatibility inputs differ from the verified package/test binaries')
    binary.chmod(0o755)
    # The hosted image's Azure mirror can stall on package indices even after
    # falling back for InRelease. Use the official HTTPS mirrors consistently.
    for name, mirror in (('apt-mirrors.txt', 'https://archive.ubuntu.com/ubuntu/'),
                         ('apt-mirrors-security.txt', 'https://security.ubuntu.com/ubuntu/')):
        path = Path('/etc/apt')/name
        if path.is_file():
            run(['sudo', 'tee', path], input=mirror+'\n', text=True)
    apt = ['sudo', 'apt-get', '-o', 'Acquire::http::Timeout=30', '-o',
           'Acquire::https::Timeout=30', '-o', 'Acquire::Retries=2', '-o',
           'Acquire::Languages=none']
    if not os.environ.get('COR_LINUX_GUEST'):
        run([*apt, '-o', 'APT::Update::Error-Mode=any', 'update'], timeout=300)
    run([*apt, 'install', '-y', 'dbus-x11', 'iproute2', 'util-linux', 'libcap2-bin',
         'xvfb', 'x11-utils', 'xdotool', 'wmctrl', 'openbox', 'gnome-shell',
         'gnome-shell-extension-appindicator', 'policykit-1-gnome',
         'libgl1-mesa-dri', 'libegl1', 'mesa-utils'], timeout=900)
    # Package structure was tested in the native packaging job. This job tests
    # its GUI, proxy and routing against this distribution, without repackaging.
    linux_package_tests(packages[0], binary, structure=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--linux-compat', '--ubuntu-latest', dest='linux_compat', action='store_true',
                        help='Test a verified Linux package on the current Ubuntu installation')
    parser.add_argument('--system', choices=('windows', 'linux', 'macos'))
    parser.add_argument('--arch', choices=('x64', 'arm64'))
    parser.add_argument('--build', type=Path)
    parser.add_argument('--ci-run-id', type=int)
    parser.add_argument('--expect-ubuntu')
    args = parser.parse_args()
    inputs = (args.system, args.arch, args.build, args.ci_run_id)
    if args.linux_compat:
        if any(value is not None for value in inputs):
            parser.error('--linux-compat does not accept packaging arguments')
        run_compatibility(args.expect_ubuntu)
        return
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
