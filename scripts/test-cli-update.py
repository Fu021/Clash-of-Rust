"""Exercise installed Linux terminal update handoffs on disposable Actions runners."""
import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import uuid

from build_support import ROOT, sha256, write_json


APP = Path('/opt/clash-of-rust')
CLI = APP/'cor'
REPORT = ROOT/'target/linux-report'


def invoke(command, **kwargs):
    return subprocess.run(list(map(str, command)), check=True, timeout=120,
                          text=True, capture_output=True, **kwargs)


def wait_for(probe, description, timeout=45):
    deadline = time.monotonic()+timeout
    while time.monotonic() < deadline:
        value = probe()
        if value:
            return value
        time.sleep(.1)
    raise TimeoutError(description)


def cli(data, *arguments):
    result = invoke([CLI, '--data-dir', data, '--resources', APP/'resources',
                     '--json', *arguments], stdin=subprocess.DEVNULL)
    return json.loads(result.stdout)['data']


def endpoint(data):
    path = data/'control.json'
    return json.loads(path.read_text()) if path.exists() else None


def stage(package, data, running):
    directory = Path(tempfile.mkdtemp(prefix='clash-of-rust-update-'))
    staged = directory/'package.deb'
    shutil.copy2(package, staged)
    shutil.copy2(CLI, directory/'update-helper')
    settings = json.loads((data/'settings.json').read_text())
    settings_bytes = json.dumps(settings, ensure_ascii=False, separators=(',', ':')).encode()
    version = invoke(['dpkg-deb', '-f', package, 'Version']).stdout.strip()
    plan = {
        'package': str(staged), 'executable': str(APP/'clash-of-rust'),
        'sha256': sha256(staged), 'version': version, 'parent': endpoint(data)['pid'],
        'restart': False, 'require_commit': True, 'data_dir': str(data),
        'terminal': {
            'executable': str(CLI), 'resources': str(APP/'resources'),
            'running': running, 'settings_sha256': hashlib.sha256(settings_bytes).hexdigest(),
        },
    }
    write_json(directory/'plan.json', plan)
    write_json(data/'update-install.json', {
        'version': version, 'state': 'preparing', 'message': 'CI terminal installation',
        'error': False, 'updated_at': int(time.time()), 'directory': str(directory),
    })
    return directory


def run_helper(directory, data):
    result = subprocess.run([str(directory/'update-helper'), '--install-update-terminal', str(directory)],
                            stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=180)
    status = cli(data, 'update', 'status')
    if result.returncode != 0 or status['state'] not in ('failed', 'succeeded'):
        raise AssertionError(f'Installer did not record a terminal result: {status}: {result.stderr[-2000:]}')
    return status


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def exercise(package, report, phase):
    report.mkdir(parents=True, exist_ok=True)
    data = report/'profile'
    data.mkdir(mode=0o700, exist_ok=True)
    environment = dict(os.environ)
    for key in ('DISPLAY', 'WAYLAND_DISPLAY', 'WAYLAND_SOCKET', 'DBUS_SESSION_BUS_ADDRESS'):
        environment.pop(key, None)
    with (report/f'{phase}-daemon.log').open('w') as log:
        daemon = subprocess.Popen(
            [CLI, '--data-dir', data, '--resources', APP/'resources', 'run', '--no-core', '--quiet'],
            env=environment, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
            start_new_session=True)
        directories = []
        checks = {}
        try:
            wait_for(lambda: endpoint(data), 'Isolated update daemon did not start')
            cli(data, 'config', 'set', 'update_prereleases', 'true')
            original = endpoint(data)['pid']
            if phase == 'denied':
                directory = stage(package, data, False)
                directories.append(directory)
                status = run_helper(directory, data)
                require(status['state'] == 'failed' and status['error'], 'Denied sudo was reported successful')
                require('未获得安装授权' in status['message'], 'Noninteractive sudo denial was not diagnosed')
                require(endpoint(data)['pid'] == original, 'Authorization denial stopped the daemon')
                require(cli(data, 'config', 'get', 'update_prereleases')['update_prereleases'],
                        'Authorization denial changed the update channel')
                checks['noninteractive_authorization_denial_keeps_daemon'] = True
            else:
                for running in (False, True):
                    if running:
                        cli(data, 'proxy', 'off')
                        cli(data, 'start')
                    directory = stage(package, data, running)
                    directories.append(directory)
                    payload = directory/'package.deb'
                    payload.write_bytes(b'not a Debian archive\n')
                    plan = json.loads((directory/'plan.json').read_text())
                    plan['sha256'] = sha256(payload)
                    write_json(directory/'plan.json', plan)
                    status = run_helper(directory, data)
                    require(status['state'] == 'failed' and status['error'], 'Invalid DEB was reported successful')
                    require('DEB 安装失败' in status['message'], 'dpkg failure detail was lost')
                    require('已恢复更新前' in status['message'], 'Failed installation did not recover')
                    require(cli(data, 'status')['running'] == running, 'Recovery changed the core running state')
                    require(cli(data, 'config', 'get', 'update_prereleases')['update_prereleases'],
                            'Recovery lost the prerelease setting')
                    checks[f'dpkg_failure_recovers_{"core" if running else "daemon_only"}'] = True
                directory = stage(package, data, True)
                directories.append(directory)
                original_settings = (data/'settings.json').read_bytes()
                original_binary = sha256(CLI)
                status = run_helper(directory, data)
                require(status['state'] == 'succeeded' and not status['error'], 'Valid DEB installation failed')
                require(not (data/'control.json').exists(), 'Successful update left the old daemon running')
                require((data/'settings.json').read_bytes() == original_settings, 'Successful update changed settings')
                require(sha256(CLI) == original_binary, 'Reinstall did not preserve the verified CLI bytes')
                require(not directory.exists(), 'Successful helper did not clean its temporary directory')
                checks['real_deb_install_waits_for_success_and_preserves_settings'] = True
            write_json(report/f'{phase}-result.json', {'status': 'passed', 'checks': checks})
        except Exception as error:
            write_json(report/f'{phase}-result.json',
                       {'status': 'failed', 'checks': checks, 'error': str(error)})
            raise
        finally:
            with contextlib.suppress(Exception):
                cli(data, 'stop')
            with contextlib.suppress(ProcessLookupError):
                os.killpg(daemon.pid, signal.SIGTERM)
            try:
                daemon.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(daemon.pid, signal.SIGKILL)
                daemon.wait(timeout=5)
            for directory in directories:
                shutil.rmtree(directory, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--package', required=True, type=Path)
    parser.add_argument('--phase', choices=('denied', 'allowed'))
    parser.add_argument('--report', type=Path, default=REPORT/'terminal-update')
    args = parser.parse_args()
    if os.environ.get('GITHUB_ACTIONS') != 'true' or sys.platform != 'linux':
        raise RuntimeError('Terminal installation tests require a disposable Actions Linux runner')
    package = args.package.resolve(strict=True)
    if args.phase:
        require(os.geteuid() != 0, 'Terminal update must be tested as an ordinary user')
        exercise(package, args.report.resolve(), args.phase)
        return
    user = 'corupdate'+uuid.uuid4().hex[:8]
    sudoers = Path('/etc/sudoers.d')/user
    with tempfile.TemporaryDirectory(prefix='cor-installed-update-') as temporary:
        staging = Path(temporary)
        staging.chmod(0o755)
        staged = staging/package.name
        shutil.copy2(package, staged)
        staged.chmod(0o644)
        args.report.mkdir(parents=True, exist_ok=True)
        try:
            invoke(['sudo', '-n', 'useradd', '--create-home', '--user-group', '--shell', '/bin/bash', user])
            for phase in ('denied', 'allowed'):
                output = staging/phase
                invoke(['sudo', '-n', 'install', '-d', '-m', '0700', '-o', user, '-g', user, output])
                if phase == 'allowed':
                    with (staging/'sudoers').open('w') as stream:
                        stream.write(f'{user} ALL=(root) NOPASSWD: /usr/bin/dpkg\n')
                    invoke(['sudo', '-n', 'install', '-m', '0440', staging/'sudoers', sudoers])
                    invoke(['sudo', '-n', 'visudo', '-cf', sudoers])
                command = [
                    'sudo', '-n', '-u', user, '-H', 'env', 'GITHUB_ACTIONS=true',
                    'PYTHONDONTWRITEBYTECODE=1', sys.executable, Path(__file__).resolve(),
                    '--package', staged, '--phase', phase, '--report', output,
                ]
                result = subprocess.run(list(map(str, command)), capture_output=True, text=True, timeout=600)
                (args.report/f'{phase}-output.log').write_text(result.stdout+result.stderr)
                for name in (f'{phase}-result.json', f'{phase}-daemon.log'):
                    source = output/name
                    present = subprocess.run(['sudo', '-n', 'test', '-f', source], timeout=10)
                    if present.returncode == 0:
                        invoke(['sudo', '-n', 'install', '-m', '0644', '-o', str(os.getuid()),
                                '-g', str(os.getgid()), source, args.report/name])
                result.check_returncode()
            print('Installed terminal update: sudo denial, real dpkg failures, recovery and reinstall passed',
                  flush=True)
        finally:
            invoke(['sudo', '-n', 'rm', '-f', sudoers])
            subprocess.run(['sudo', '-n', 'pkill', '-TERM', '-u', user], check=False, timeout=10)
            subprocess.run(['sudo', '-n', 'userdel', '--remove', user], check=False, timeout=30)
            invoke(['sudo', '-n', 'chown', '-R', f'{os.getuid()}:{os.getgid()}', staging])


if __name__ == '__main__':
    main()
