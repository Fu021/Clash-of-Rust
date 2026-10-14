"""Focused native system proxy/HTTP/TUN traffic checks on disposable Actions VMs."""
import argparse
import importlib.util
import os
from pathlib import Path
import sys

from build_support import ROOT, run, host_system


def check(binary, bundle):
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('Proxy/TUN checks require a disposable Actions runner')
    system = host_system()
    resources = bundle/'resources'
    helper = bundle/('clash-tun-launcher.exe' if system == 'windows' else 'clash-tun-launcher')
    if system == 'macos' and bundle.suffix == '.app':
        resources = bundle/'Contents/Resources/resources'
        helper = bundle/'Contents/MacOS/clash-tun-launcher'
    env = dict(os.environ, CLASH_PROXY_SMOKE='1', MIHOMO_TEST_RESOURCES=str(resources.resolve()))
    command = [binary, 'basic_system_proxy_and_tun', '--exact', '--ignored', '--nocapture']
    if system == 'linux':
        installed = Path('/opt/clash-of-rust')
        if bundle.resolve() != installed:
            # Install only the raw tested binaries/resources, never package in CI.
            if installed.exists():
                raise RuntimeError('Refusing to overwrite an existing installation')
            try:
                run(['sudo', 'mkdir', '-p', installed/'libexec'])
                run(['sudo', 'cp', '-a', str(bundle.resolve())+'/.', installed])
                run(['sudo', 'cp', '/usr/bin/resolvectl', installed/'libexec/resolvectl'])
                run(['sudo', 'chown', '-R', 'root:root', installed])
                run(['sudo', 'chmod', '0755', '/opt', installed])
                run(['sudo', sys.executable, ROOT/'scripts/test-linux-tun.py', '--basic',
                     '--test-binary', binary, '--uid', os.getuid()])
            finally:
                run(['sudo', 'rm', '-rf', installed])
        else:
            run(['sudo', sys.executable, ROOT/'scripts/test-linux-tun.py', '--basic',
                 '--test-binary', binary, '--uid', os.getuid()])
    elif system == 'macos':
        spec = importlib.util.spec_from_file_location('mac_flows', ROOT/'scripts/test-macos-flows.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        report = ROOT/'target/macos-report'
        report.mkdir(parents=True, exist_ok=True)
        with module.network_service_for_ci(report, 'proxy-smoke'):
            # Production helper and launchd service, ordinary-user client. Only
            # initial administrative installation is performed by runner sudo.
            run(['/usr/bin/sudo', '-n', helper, '--macos-install-service', os.getuid()])
            run(command, env=env, timeout=90)
    else:
        run(command, env=env, timeout=90)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--test-binary', type=Path, required=True)
    parser.add_argument('--bundle', type=Path, required=True)
    args = parser.parse_args()
    check(args.test_binary.resolve(strict=True), args.bundle.resolve(strict=True))


if __name__ == '__main__':
    main()
