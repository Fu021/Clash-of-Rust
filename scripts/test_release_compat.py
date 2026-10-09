"""Verify the already-tested Ubuntu 22.04 package on the latest Ubuntu image."""
import importlib.util
import json
from pathlib import Path

from build_support import ROOT, run, sha256
from workflow_artifacts import commit


def run_compatibility():
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
    run(['sudo', 'apt-get', 'update'])
    run(['sudo', 'apt-get', 'install', '-y', 'dbus', 'xvfb', 'xauth', 'iproute2', 'util-linux'])
    spec = importlib.util.spec_from_file_location('package_test', ROOT/'scripts/test-release-package.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    # Package structure was tested in the native packaging job. This job tests
    # its GUI and routing against the newer distribution, without repackaging.
    module.linux_package_tests(packages[0], binary, structure=False)


if __name__ == '__main__':
    run_compatibility()
