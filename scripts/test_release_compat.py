"""Verify the already-tested Ubuntu 22.04 package on the latest Ubuntu image."""
import importlib.util
import json
import os
from pathlib import Path

from build_support import ROOT, run, sha256
from workflow_artifacts import commit


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
    run([*apt, 'install', '-y', 'dbus', 'xvfb', 'xauth', 'iproute2', 'util-linux'], timeout=600)
    spec = importlib.util.spec_from_file_location('package_test', ROOT/'scripts/test-release-package.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    # Package structure was tested in the native packaging job. This job tests
    # its GUI and routing against the newer distribution, without repackaging.
    module.linux_package_tests(packages[0], binary, structure=False)


if __name__ == '__main__':
    run_compatibility()
