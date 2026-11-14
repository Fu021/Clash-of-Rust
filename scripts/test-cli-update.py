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
    result = subprocess.run(list(map(str, command)), timeout=120,
                            text=True, capture_output=True, **kwargs)
    if result.returncode:
        print((result.stdout+result.stderr)[-8000:], file=sys.stderr, flush=True)
    result.check_returncode()
    return result


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


def stage(package, data, running, user_service=None):
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
    if user_service is not None:
        plan['terminal']['user_service'] = user_service
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


def stage_test_driver(directory):
    for name in ('test-cli-update.py', 'build_support.py'):
        target = directory/name
        shutil.copy2(Path(__file__).with_name(name), target)
        target.chmod(0o644)
    return directory/'test-cli-update.py'


def exercise_read_only(report):
    data = report/'uninitialized'
    for arguments in (('node', 'list'), ('connection', 'list'), ('service', 'status')):
        value = cli(data, *arguments)
        require(not data.exists(), f'Read-only query created its data directory: {arguments}')
        if arguments[0] != 'service':
            require(value['offline'] and value['items'] == [], 'Offline query invented live results')
    for arguments in (('node', 'select'), ('node', 'select', 'PROXY'),
                      ('connection', 'show', 'missing'),
                      ('connection', 'list', '--watch')):
        result = subprocess.run([str(CLI), '--data-dir', str(data), *arguments],
                                stdin=subprocess.DEVNULL, text=True, capture_output=True, timeout=15)
        require(result.returncode != 0, f'Noninteractive or offline operation succeeded: {arguments}')
        require(not data.exists(), f'Rejected operation started a backend: {arguments}')


def exercise_picker(data):
    import pty
    import select

    group = '默认策略'
    for answer, expected in ((b'DIRECT\n1\n', 0), (b'q\n', 1), (None, 130)):
        master, slave = pty.openpty()
        child = subprocess.Popen([str(CLI), '--data-dir', str(data), 'node', 'select', group],
                                 stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
        os.close(slave)
        try:
            output = bytearray()
            deadline = time.monotonic()+20
            while '输入编号'.encode() not in output:
                require(time.monotonic() < deadline and child.poll() is None,
                        'Interactive node selection did not prompt')
                if select.select([master], [], [], .1)[0]:
                    output.extend(os.read(master, 8192))
            if answer is None:
                child.send_signal(signal.SIGINT)
            else:
                os.write(master, answer)
            require(child.wait(timeout=15) == expected, 'Interactive selection returned the wrong exit code')
            members = cli(data, 'node', 'list', group)['items']
            require([row['name'] for row in members if row['selected']] == ['DIRECT'],
                    'Selection, cancellation or Ctrl+C changed the wrong node')
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=5)
            os.close(master)


def exercise(package, report, phase):
    report.mkdir(parents=True, exist_ok=True)
    exercise_read_only(report)
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
            original_endpoint = endpoint(data)
            before = (data/'settings.json').read_bytes()
            nodes = cli(data, 'node', 'list')
            connections = cli(data, 'connection', 'list')
            require(nodes['offline'] and connections['offline'], 'Stopped core was reported live')
            require(not cli(data, 'status')['running'], 'Read-only queries started a stopped core')
            require(endpoint(data) == original_endpoint, 'Read-only queries replaced the backend')
            require((data/'settings.json').read_bytes() == before, 'Read-only queries changed settings')
            checks['read_only_queries_do_not_initialize_or_start'] = True
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
                for running in (False, True):
                    if running:
                        cli(data, 'start')
                    else:
                        cli(data, 'stop')
                        cli(data, 'config', 'get', 'update_prereleases')
                    original = endpoint(data)['pid']
                    directory = stage(package, data, running)
                    directories.append(directory)
                    original_settings = (data/'settings.json').read_bytes()
                    original_binary = sha256(CLI)
                    status = run_helper(directory, data)
                    require(status['state'] == 'succeeded' and not status['error'],
                            'Valid DEB installation failed')
                    require(endpoint(data)['pid'] != original, 'Successful update reused the old daemon')
                    require(cli(data, 'status')['running'] == running,
                            'Successful update changed the core running state')
                    require((data/'settings.json').read_bytes() == original_settings,
                            'Successful update changed settings')
                    require(sha256(CLI) == original_binary,
                            'Reinstall did not preserve the verified CLI bytes')
                    require(not directory.exists(), 'Successful helper did not clean its temporary directory')
                    checks[f'real_deb_install_restores_{"core" if running else "daemon_only"}'] = True
                exercise_picker(data)
                checks['terminal_picker_search_cancel_and_interrupt'] = True
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


def exercise_service(package, report):
    report.mkdir(parents=True, exist_ok=True)
    data = report/'profile'
    data.mkdir(mode=0o700, exist_ok=True)
    service = 'clash-of-rust-cli.service'
    unit = Path.home()/'.config/systemd/user'/service
    directories = []
    checks = {}
    try:
        cli(data, 'config', 'set', 'update_prereleases', 'true')
        cli(data, 'proxy', 'off')
        cli(data, 'service', 'install', '--yes')
        service_status = cli(data, 'service', 'status')
        require(service_status['installed'] and service_status['enabled']
                and service_status['active'] is False,
                'Installed but inactive service was not distinguished')
        original_unit = unit.read_bytes()
        for managed in (False, True):
            cli(data, 'start')
            original = endpoint(data)['pid']
            service_status = cli(data, 'service', 'status')
            require(service_status['ownership'] == ('service' if managed else 'detached'),
                    'Service status did not distinguish detached ownership')
            require(service_status['owner_pid'] == original, 'Service status returned a stale owner')
            if managed:
                require(int(invoke(['systemctl', '--user', 'show', service, '-p', 'MainPID', '--value'])
                            .stdout.strip()) == original, 'The previous update did not restore service ownership')
                require(service in Path(f'/proc/{original}/cgroup').read_text(),
                        'The daemon is outside its systemd service')
            else:
                require(int(invoke(['systemctl', '--user', 'show', service, '-p', 'MainPID', '--value'])
                            .stdout.strip()) == 0, 'The detached scenario unexpectedly started the service')
            recovery = {
                'unit_file': str(unit), 'unit_sha256': sha256(unit),
                'enabled': 'enabled', 'tun_authorization': False,
            }
            original_settings = (data/'settings.json').read_bytes()
            directory = stage(package, data, True, user_service=recovery)
            directories.append(directory)
            status = run_helper(directory, data)
            require(status['state'] == 'succeeded' and not status['error'],
                    'Service-managed update failed: '+status['message'])
            current = endpoint(data)['pid']
            require(current != original, 'Update left the old service process running')
            require(current == int(invoke(['systemctl', '--user', 'show', service, '-p', 'MainPID', '--value'])
                                   .stdout.strip()), 'Updated daemon is outside the original service')
            service_status = cli(data, 'service', 'status')
            require(service_status['active'] and service_status['ownership'] == 'service'
                    and service_status['main_pid'] == current,
                    'Updated service status does not match the actual process')
            require(cli(data, 'status')['running'], 'Updated service did not restore the core')
            require(unit.read_bytes() == original_unit, 'Update rewrote the login service')
            require(invoke(['systemctl', '--user', 'is-enabled', service]).stdout.strip() == 'enabled',
                    'Update disabled the login service')
            require((data/'settings.json').read_bytes() == original_settings,
                    'Service recovery changed settings')
            checks['service_restore' if managed else 'enabled_service_adopts_detached_daemon'] = True
        write_json(report/'service-result.json', {'status': 'passed', 'checks': checks})
    except Exception as error:
        write_json(report/'service-result.json', {'status': 'failed', 'checks': checks, 'error': str(error)})
        raise
    finally:
        subprocess.run(['systemctl', '--user', 'stop', service], capture_output=True, timeout=50)
        with contextlib.suppress(Exception):
            cli(data, 'service', 'remove', '--yes')
        with contextlib.suppress(ProcessLookupError, TypeError):
            current = endpoint(data)
            if current:
                os.kill(current['pid'], signal.SIGTERM)
        for directory in directories:
            shutil.rmtree(directory, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--package', required=True, type=Path)
    parser.add_argument('--phase', choices=('denied', 'allowed', 'service'))
    parser.add_argument('--report', type=Path, default=REPORT/'terminal-update')
    args = parser.parse_args()
    if os.environ.get('GITHUB_ACTIONS') != 'true' or sys.platform != 'linux':
        raise RuntimeError('Terminal installation tests require a disposable Actions Linux runner')
    package = args.package.resolve(strict=True)
    if args.phase:
        require(os.geteuid() != 0, 'Terminal update must be tested as an ordinary user')
        if args.phase == 'service':
            exercise_service(package, args.report.resolve())
        else:
            exercise(package, args.report.resolve(), args.phase)
        return
    user = 'corupdate'+uuid.uuid4().hex[:8]
    uid = None
    sudoers = Path('/etc/sudoers.d')/user
    with tempfile.TemporaryDirectory(prefix='cor-installed-update-') as temporary:
        staging = Path(temporary)
        staging.chmod(0o755)
        staged = staging/package.name
        shutil.copy2(package, staged)
        staged.chmod(0o644)
        driver = stage_test_driver(staging)
        args.report.mkdir(parents=True, exist_ok=True)
        try:
            invoke(['sudo', '-n', 'useradd', '--create-home', '--user-group', '--shell', '/bin/bash', user])
            uid = invoke(['id', '-u', user]).stdout.strip()
            for phase in ('denied', 'allowed', 'service'):
                output = staging/phase
                invoke(['sudo', '-n', 'install', '-d', '-m', '0700', '-o', user, '-g', user, output])
                if phase == 'allowed':
                    with (staging/'sudoers').open('w') as stream:
                        stream.write(f'{user} ALL=(root) NOPASSWD: /usr/bin/dpkg\n')
                    invoke(['sudo', '-n', 'install', '-m', '0440', staging/'sudoers', sudoers])
                    invoke(['sudo', '-n', 'visudo', '-cf', sudoers])
                if phase == 'service':
                    invoke(['sudo', '-n', 'systemctl', 'start', f'user@{uid}.service'])
                command = [
                    'sudo', '-n', '-u', user, '-H', 'env', 'GITHUB_ACTIONS=true',
                    'PYTHONDONTWRITEBYTECODE=1',
                    f'XDG_RUNTIME_DIR=/run/user/{uid}', f'DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/{uid}/bus',
                    sys.executable, driver,
                    '--package', staged, '--phase', phase, '--report', output,
                ]
                result = subprocess.run(list(map(str, command)), cwd=staging,
                                        capture_output=True, text=True, timeout=600)
                (args.report/f'{phase}-output.log').write_text(result.stdout+result.stderr)
                for name in (f'{phase}-result.json', f'{phase}-daemon.log'):
                    source = output/name
                    present = subprocess.run(['sudo', '-n', 'test', '-f', source], timeout=10)
                    if present.returncode == 0:
                        invoke(['sudo', '-n', 'install', '-m', '0644', '-o', str(os.getuid()),
                                '-g', str(os.getgid()), source, args.report/name])
                if result.returncode:
                    print((result.stdout+result.stderr)[-8000:], file=sys.stderr, flush=True)
                result.check_returncode()
            print('Installed terminal update: sudo denial, real dpkg failures, recovery and reinstall passed',
                  flush=True)
        finally:
            invoke(['sudo', '-n', 'rm', '-f', sudoers])
            if uid is not None:
                subprocess.run(['sudo', '-n', 'systemctl', 'stop', f'user@{uid}.service'],
                               check=False, timeout=60)
            subprocess.run(['sudo', '-n', 'pkill', '-TERM', '-u', user], check=False, timeout=10)
            subprocess.run(['sudo', '-n', 'userdel', '--remove', user], check=False, timeout=30)
            invoke(['sudo', '-n', 'chown', '-R', f'{os.getuid()}:{os.getgid()}', staging])


if __name__ == '__main__':
    main()
