"""Focused native system proxy/HTTP/TUN traffic checks on disposable Actions VMs."""
import argparse
import importlib.util
import os
from pathlib import Path
import sys
import shutil
import tempfile

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
                run(['sudo', 'chown', 'root:root', '/opt'])
                run(['sudo', 'chmod', '-R', 'go-w', installed])
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
        with tempfile.TemporaryDirectory(prefix='clash-proxy-smoke-') as temporary, module.network_service_for_ci(report, 'proxy-smoke'):
            # Mac's production service/bridge locates its bundled core relative
            # to current_exe, rather than the integration-test environment.
            # Preserve the installed app's seal; stage the unchanged test and
            # helper next to a link to its actual signed resources instead.
            stage = Path(temporary)
            shutil.copy2(binary, stage/'proxy-smoke')
            shutil.copy2(helper, stage/'clash-tun-launcher')
            if bundle.suffix != '.app':
                # Intel's raw Rust executable is unsigned until packaging.
                # Give this isolated test copy the same ad-hoc signature used
                # by build_app, preserving the exported prebuilt bytes.
                run(['/usr/bin/codesign', '--force', '--sign', '-', stage/'clash-tun-launcher'])
            run(['/usr/bin/codesign', '--verify', '--strict', stage/'clash-tun-launcher'])
            (stage/'resources').symlink_to(resources.resolve(), target_is_directory=True)
            command[0] = stage/'proxy-smoke'
            # Production helper and launchd service, ordinary-user client. Only
            # initial administrative installation is performed by runner sudo.
            run(['/usr/bin/sudo', '-n', stage/'clash-tun-launcher', '--macos-install-service', os.getuid()])
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
