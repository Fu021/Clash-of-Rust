"""Package verified CI binaries and test the installation; Cargo is not invoked."""
import argparse
import json
import os
from pathlib import Path
import shutil
import sys

from build_support import host_system, ROOT, bundle_directory, host_arch, run, sha256, write_json
from workflow_artifacts import commit, verify_build


def linux_package_tests(package, tun_test, structure=True):
    if structure:
        # Build/manifest checks already verify payload identity and resources.
        run(['dpkg-deb', '--info', package])
    # The privileged launcher requires an entirely root-owned installed path.
    run(['sudo', 'chown', 'root:root', '/opt'])
    run(['sudo', 'chmod', '0755', '/opt'])
    run(['sudo', 'apt-get', 'install', '-y', './'+str(package.relative_to(ROOT))])
    if not Path('/usr/lib/systemd/systemd-resolved').is_file() or not shutil.which('resolvectl'):
        run(['sudo', 'apt-get', 'install', '-y', 'systemd-resolved'])
    run([sys.executable, ROOT/'scripts/test-proxy-modes.py',
         '--test-binary', tun_test, '--bundle', '/opt/clash-of-rust'])


def run_compatibility():
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('Compatibility tests require a disposable GitHub Actions runner')
    packages = list((ROOT/'dist').glob('*.deb'))
    if len(packages) != 1:
        raise ValueError('Expected exactly one Linux x64 package')
    receipt = json.loads((ROOT/'dist/package-receipt.json').read_text())
    binary = ROOT/'target/linux-tun-test'
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
    run([*apt, '-o', 'APT::Update::Error-Mode=any', 'update'], timeout=300)
    run([*apt, 'install', '-y', 'dbus', 'iproute2', 'util-linux', 'libcap2-bin'], timeout=600)
    # Package structure was tested in the native packaging job. This job tests
    # its proxy and routing against the newer distribution, without repackaging.
    linux_package_tests(packages[0], binary, structure=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ubuntu-latest', action='store_true',
                        help='Test the verified Ubuntu 22.04 package on the latest Ubuntu runner')
    parser.add_argument('--system', choices=('windows', 'linux', 'macos'))
    parser.add_argument('--arch', choices=('x64', 'arm64'))
    parser.add_argument('--build', type=Path)
    parser.add_argument('--ci-run-id', type=int)
    args = parser.parse_args()
    inputs = (args.system, args.arch, args.build, args.ci_run_id)
    if args.ubuntu_latest:
        if any(value is not None for value in inputs):
            parser.error('--ubuntu-latest does not accept packaging arguments')
        run_compatibility()
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
    if args.system == 'windows':
        run([sys.executable, ROOT/'scripts/test-installer.py', '--package', package,
             '--test-binary', args.build/manifest['tests']['core_integration']])
        run([sys.executable, ROOT/'scripts/scan-defender.py', '--update-signatures', package])
    elif args.system == 'macos':
        from macos_ci import package_tests
        package_tests(package, manifest, args.build)
    else:
        tun = args.build/manifest['tests']['core_integration']
        linux_package_tests(package, tun)
        if args.arch == 'x64':
            shutil.copy2(tun, ROOT/'dist/linux-tun-test')
    # Written only after every package/installed-program test has succeeded.
    receipt = {
        'schema': 1, 'commit': manifest['commit'], 'version': manifest['version'],
        'system': args.system, 'arch': args.arch, 'ci_run_id': args.ci_run_id,
        'build_artifact': json.loads((args.build/'source-artifact.json').read_text()),
        'package': package.name, 'sha256': sha256(package), 'tests_passed': True,
    }
    if args.system == 'linux' and args.arch == 'x64':
        receipt['linux_tun_test_sha256'] = sha256(ROOT/'dist/linux-tun-test')
    write_json(ROOT/'dist/package-receipt.json', receipt)


if __name__ == '__main__':
    main()
