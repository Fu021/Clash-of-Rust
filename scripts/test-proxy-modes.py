"""Focused native system proxy/HTTP/TUN traffic checks on disposable Actions VMs."""
import argparse
import contextlib
import os
from pathlib import Path
import sys
import shutil
import tempfile
import subprocess

from build_support import ROOT, run, host_system


@contextlib.contextmanager
def macos_network_service(report):
    label = f'org.clashofrust.network.{os.getuid()}'
    directory = Path('/Library/PrivilegedHelperTools')/label
    agent = Path('/Library/LaunchDaemons')/(label+'.plist')
    if directory.exists() or agent.exists():
        raise RuntimeError('Refusing to overwrite an existing network service')
    try:
        yield
    finally:
        if agent.exists() or directory.exists():
            state = subprocess.run(['/usr/bin/sudo', '-n', '/bin/launchctl', 'print', 'system/'+label], capture_output=True, text=True, timeout=15)
            (report/'proxy-smoke-network-service-launchd.log').write_text(state.stdout+state.stderr)
            service_log = directory/'network-service.log'
            if service_log.exists():
                output = subprocess.run(['/usr/bin/sudo', '-n', '/bin/cat', service_log], capture_output=True, text=True, timeout=15)
                (report/'proxy-smoke-network-service.log').write_text(output.stdout+output.stderr)
            subprocess.run(['/usr/bin/sudo', '-n', '/bin/launchctl', 'bootout', 'system/'+label], capture_output=True, timeout=20)
            subprocess.run(['/usr/bin/sudo', '-n', '/bin/rm', '-rf', directory, agent], check=True, timeout=20)


def check(binary, bundle, test_case='basic_system_proxy_and_tun'):
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('Proxy/TUN checks require a disposable Actions runner')
    system = host_system()
    resources = bundle/'resources'
    helper = bundle/('clash-tun-launcher.exe' if system == 'windows' else 'clash-tun-launcher')
    if system == 'macos' and bundle.suffix == '.app':
        resources = bundle/'Contents/Resources/resources'
        helper = bundle/'Contents/MacOS/clash-tun-launcher'
    env = dict(os.environ, CLASH_PROXY_SMOKE='1', MIHOMO_TEST_RESOURCES=str(resources.resolve()))
    cli = (bundle/'Contents/MacOS/cor' if system == 'macos' and bundle.suffix == '.app'
           else bundle/('cor.exe' if system == 'windows' else 'cor'))
    env['MIHOMO_TEST_CLI'] = str(cli.resolve(strict=True))
    command = [binary, test_case, '--exact', '--ignored', '--nocapture']
    if system == 'linux':
        sudo = ['sudo']
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
                run([*sudo, sys.executable, ROOT/'scripts/test-linux-tun.py',
                     '--test-binary', binary, '--uid', os.getuid(), '--test-case', test_case])
            finally:
                run(['sudo', 'rm', '-rf', installed])
        else:
            run([*sudo, sys.executable, ROOT/'scripts/test-linux-tun.py',
                 '--test-binary', binary, '--uid', os.getuid(), '--test-case', test_case])
    elif system == 'macos':
        report = ROOT/'target/macos-report'
        report.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='clash-proxy-smoke-') as temporary, macos_network_service(report):
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
            run(command, env=env, timeout=240)
    elif system == 'windows':
        from windows_ci import allow_ordinary_user_directory, run_unelevated
        # CI tests raw binaries; Publish Release has already installed the
        # package and its service. Only remove the registration we created.
        query = subprocess.run(['sc.exe', 'query', 'ClashOfRust.Network'], capture_output=True)
        own_service = query.returncode == 1060
        if query.returncode not in (0, 1060):
            raise RuntimeError('Cannot inspect Windows network service')
        if own_service:
            run([helper, '--windows-service-install'])
        try:
            env['CLASH_EXPECT_UNELEVATED'] = '1'
            run_unelevated(command, env)
            # Recreate the machine boot condition: SCM registration survives,
            # the privileged process is gone, and a normal user starts it again.
            run([helper, '--windows-service-stop'])
            run_unelevated(command, env)
            # A GUI crash must not leave a SYSTEM core, open controller ports,
            # or an adapter preventing the next ordinary launch from using TUN.
            import json, socket, time
            with tempfile.TemporaryDirectory(prefix='windows-service-crash-') as temporary:
                allow_ordinary_user_directory(temporary)
                ready = Path(temporary)/'ready.json'
                crashed = {}
                def kill_owner(kernel, process):
                    deadline = time.monotonic()+45
                    while not ready.is_file():
                        if time.monotonic() >= deadline:
                            kernel.TerminateProcess(process, 0xCAFE)
                            raise TimeoutError('Ordinary TUN crash fixture did not become ready')
                        time.sleep(0.1)
                    crashed.update(json.loads(ready.read_text()))
                    if not kernel.TerminateProcess(process, 0xCAFE):
                        raise RuntimeError('Could not simulate ordinary GUI process loss')
                run_unelevated(command, dict(env, CLASH_SERVICE_CRASH_REPORT=str(ready)),
                               on_start=kill_owner, expected_code=0xCAFE)
                deadline = time.monotonic()+15
                for port in (crashed['controller'], crashed['mixed']):
                    while True:
                        with socket.socket() as listener:
                            try:
                                listener.bind(('127.0.0.1', port))
                                break
                            except OSError:
                                if time.monotonic() >= deadline:
                                    raise RuntimeError('Service core survived its ordinary GUI owner')
                                time.sleep(0.1)
                # Listener closure precedes the core's final file/Defender handle
                # release on Windows. Keep a bounded cleanup wait; do not hide a
                # surviving process or permanent permissions failure.
                deadline = time.monotonic()+10
                while Path(crashed['temporary']).exists():
                    try:
                        shutil.rmtree(crashed['temporary'])
                    except OSError as error:
                        if getattr(error,'winerror',None) not in (5,32,145) or time.monotonic()>=deadline:
                            raise
                        time.sleep(0.1)
            run_unelevated(command, env)
        finally:
            if own_service:
                run([helper, '--windows-service-uninstall'])
    else:
        run(command, env=env, timeout=240)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--test-binary', type=Path, required=True)
    parser.add_argument('--bundle', type=Path, required=True)
    args = parser.parse_args()
    check(args.test_binary.resolve(strict=True), args.bundle.resolve(strict=True))


if __name__ == '__main__':
    main()
