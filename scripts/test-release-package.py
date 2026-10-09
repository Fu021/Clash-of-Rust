"""Package verified CI binaries and test the installation; Cargo is not invoked."""
import argparse
import json
import os
from pathlib import Path
import shutil
import sys

from build_support import ROOT, bundle_directory, host_arch, run, sha256, write_json
from workflow_artifacts import commit, verify_build


def linux_package_tests(package, tun_test, structure=True):
    if structure:
        run(['sudo', sys.executable, ROOT/'scripts/test-deb.py', package])
    # The privileged launcher requires an entirely root-owned installed path.
    run(['sudo', 'chown', 'root:root', '/opt'])
    run(['sudo', 'chmod', '0755', '/opt'])
    run(['sudo', 'apt-get', 'install', '-y', './'+str(package.relative_to(ROOT))])
    run(['dbus-run-session', '--', 'xvfb-run', '-a', sys.executable,
         ROOT/'scripts/test-linux-gui.py', '--executable', '/usr/bin/clash-of-rust'])
    if not Path('/usr/lib/systemd/systemd-resolved').is_file() or not shutil.which('resolvectl'):
        run(['sudo', 'apt-get', 'install', '-y', 'systemd-resolved'])
    run(['sudo', sys.executable, ROOT/'scripts/test-linux-tun.py',
         '--test-binary', tun_test, '--uid', str(os.getuid())])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--system', choices=('windows', 'linux'), required=True)
    parser.add_argument('--arch', choices=('x64', 'arm64'), required=True)
    parser.add_argument('--build', type=Path, required=True)
    parser.add_argument('--ci-run-id', type=int, required=True)
    args = parser.parse_args()
    if (args.system, args.arch) != ('windows' if os.name == 'nt' else 'linux', host_arch()):
        raise ValueError('Packaging tests require the selected native platform')
    manifest = verify_build(args.build, args.system, args.arch, commit())
    # The existing installer test constructs isolated fixture installers from
    # this raw bundle; production installation tests consume the final package.
    shutil.copytree(args.build/'payload', bundle_directory(args.system, args.arch), dirs_exist_ok=True)
    run([sys.executable, ROOT/'scripts/build-installer.py', '--system', args.system,
         '--arch', args.arch, '--prebuilt', args.build])
    extension = f'windows-{args.arch}-setup.exe' if args.system == 'windows' else f"linux-{'amd64' if args.arch == 'x64' else 'arm64'}.deb"
    package = ROOT/'dist'/f"Clash-of-Rust-{manifest['version']}-{extension}"
    if args.system == 'windows':
        run([sys.executable, ROOT/'scripts/test-installer.py', '--package', package])
        run([sys.executable, ROOT/'scripts/scan-defender.py', '--update-signatures', package])
    else:
        tun = args.build/manifest['tests']['linux_tun']
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
