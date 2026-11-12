"""Run only inside the disposable official Pop!_OS live guest."""
import json
import os
from pathlib import Path
import pwd
import subprocess
import sys
import time


def command(*arguments, timeout=60):
    return subprocess.check_output(arguments, text=True, timeout=timeout)


def wait_for(read, timeout=180):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = read()
        if value:
            return value
        time.sleep(2)
    raise TimeoutError('COSMIC login condition did not become ready')


def apt(*arguments, timeout=600):
    deadline = time.monotonic() + timeout
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError('The live session package-manager lock did not become available')
        result = subprocess.run(
            ['apt-get', '-o', 'DPkg::Lock::Timeout=60', '-o', 'Dpkg::Use-Pty=0',
             '-o', 'APT::Color=0', *arguments],
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=remaining,
            env={**os.environ, 'DEBIAN_FRONTEND': 'noninteractive', 'LC_ALL': 'C'},
        )
        print(result.stdout, end='', flush=True)
        if result.returncode == 0:
            return
        if not ('Could not get lock ' in result.stdout and 'held by process ' in result.stdout):
            raise subprocess.CalledProcessError(result.returncode, result.args, output=result.stdout)
        time.sleep(min(5, max(0, deadline - time.monotonic())))


def user():
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if (entry / 'comm').read_text().strip() != 'cosmic-session':
                continue
            account = pwd.getpwuid(entry.stat().st_uid)
            if account.pw_uid > 0 and account.pw_dir.startswith('/home/'):
                return account
        except (KeyError, OSError):
            continue
    return None


def session_environment(account, processes=Path('/proc')):
    for entry in processes.iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if (entry.stat().st_uid != account.pw_uid
                    or (entry / 'comm').read_text().strip() not in ('cosmic-session', 'cosmic-panel', 'cosmic-bg')):
                continue
            values = dict(item.split('=', 1) for item in
                          (entry / 'environ').read_bytes().decode().split('\0') if '=' in item)
            runtime = Path('/run/user', str(account.pw_uid))
            if not values.get('DISPLAY') or not values.get('WAYLAND_DISPLAY'):
                continue
            if not (runtime / values['WAYLAND_DISPLAY']).is_socket():
                continue
            return {
                'HOME': account.pw_dir, 'USER': account.pw_name,
                'XDG_RUNTIME_DIR': str(runtime),
                'DBUS_SESSION_BUS_ADDRESS': 'unix:path=' + str(runtime / 'bus'),
                'XDG_CURRENT_DESKTOP': 'COSMIC', 'XDG_SESSION_TYPE': 'wayland',
                **{key: values[key] for key in ('DISPLAY', 'WAYLAND_DISPLAY', 'XAUTHORITY') if key in values},
            }
        except (OSError, ValueError):
            continue
    return None


def startup_traces(account):
    for path in (Path(account.pw_dir) / '.local/share').glob('*/desktop-startup.json'):
        try:
            value = json.loads(path.read_text())
        except (OSError, ValueError):
            continue
        if value.get('schema') == 1 and value.get('kind') == 'desktop_startup':
            yield value


def matches_startup(value, background, visible, *, session=None, excluded=()):
    session_id = value.get('session_id')
    status = value.get('status', {})
    return (
        isinstance(session_id, str) and bool(session_id)
        and session_id not in excluded
        and (session is None or session_id == session)
        and value.get('background_requested') is background
        and status.get('tray_state') == 'created'
        and status.get('window_visible') is visible
    )


def trace(account, background, visible, *, session=None, excluded=()):
    matches = []
    for value in startup_traces(account):
        if not matches_startup(value, background, visible, session=session, excluded=excluded):
            continue
        try:
            pid = int(value.get('pid'))
            if Path('/proc', str(pid)).stat().st_uid == account.pw_uid:
                matches.append(value)
        except (OSError, TypeError, ValueError):
            continue
    if len(matches) > 1:
        raise RuntimeError('More than one live startup record matches the login stage')
    return matches[0] if matches else None


def restart_login(account):
    before = command('pgrep', '-u', str(account.pw_uid), '-x', 'cosmic-session').split()
    subprocess.run(['systemctl', 'restart', 'display-manager'], check=True, timeout=60)

    def new_session():
        result = subprocess.run(
            ['pgrep', '-u', str(account.pw_uid), '-x', 'cosmic-session'],
            capture_output=True, text=True, timeout=10,
        )
        return result.returncode == 0 and result.stdout.split() != before and session_environment(account)
    wait_for(new_session)


def main(stage):
    if os.geteuid() != 0 or not Path('/mnt/cor-test/candidate.deb').is_file():
        raise RuntimeError('This probe is restricted to its disposable VM')
    os_release = Path('/etc/os-release').read_text()
    if 'ID=pop' not in os_release and 'ID="pop"' not in os_release:
        raise RuntimeError('The test requires actual Pop!_OS, not an Ubuntu substitute')
    account = wait_for(user, timeout=300)
    environment = wait_for(lambda: session_environment(account))
    autostart = Path(account.pw_dir) / '.config/autostart/clash-of-rust.desktop'
    report = {'stage': stage, 'kernel': command('uname', '-r').strip(),
              'system': os_release, 'desktop': 'COSMIC', 'session_type': 'wayland'}
    if stage == 'prepare':
        apt('update', timeout=300)
        apt('install', '--no-install-recommends', '-y', '/mnt/cor-test/candidate.deb', timeout=300)
        report['packages'] = command(
            'dpkg-query', '-W', '-f=${Package} ${Version}\\n',
            'cosmic-session', 'cosmic-comp', 'cosmic-panel', 'cosmic-settings',
            'clash-of-rust',
        )
        autostart.parent.mkdir(parents=True, exist_ok=True)
        os.chown(autostart.parent, account.pw_uid, account.pw_gid)
        if autostart.exists() or autostart.is_symlink():
            raise RuntimeError('Unexpected pre-existing startup entry in the clean live session')
        menu = Path('/usr/share/applications/clash-of-rust.desktop')
        report['menu'] = menu.read_text()
        if '--background' in report['menu']:
            raise AssertionError('Manual application menu must stay a foreground launch')
        autostart.symlink_to(menu)
        os.lchown(autostart, account.pw_uid, account.pw_gid)
        report['seeded_menu_link'] = True
    elif stage in ('first-login', 'background-login'):
        previous = {value.get('session_id') for value in startup_traces(account)}
        restart_login(account)
        background = stage == 'background-login'
        report['startup'] = wait_for(
            lambda: trace(account, background, not background, excluded=previous)
        )
        if report['startup'].get('session_id') in previous:
            raise AssertionError('The login reused an earlier startup session')
        if autostart.is_symlink() or '--background' not in autostart.read_text():
            raise AssertionError('The menu link was not migrated to background autostart')
        if autostart.stat().st_mode & 0o777 != 0o600:
            raise AssertionError('The migrated entry permissions differ from 0600')
        report['migrated_background_entry'] = True
        report['menu_unchanged'] = '--background' not in Path(
            '/usr/share/applications/clash-of-rust.desktop'
        ).read_text()
        if not report['menu_unchanged']:
            raise AssertionError('Autostart migration changed the application menu')
    elif stage == 'manual-launch':
        previous = json.loads(Path('/var/tmp/cor-startup-background-login.json').read_text())
        startup = previous.get('startup', {})
        session = startup.get('session_id')
        pid = startup.get('pid')
        if not isinstance(session, str) or not session or not isinstance(pid, int):
            raise AssertionError('The background login did not record its live application')
        command('runuser', '-u', account.pw_name, '--', 'env',
                *(key + '=' + value for key, value in environment.items()),
                'gtk-launch', 'clash-of-rust')
        report['startup'] = wait_for(
            lambda: trace(account, True, True, session=session)
        )
        if report['startup'].get('pid') != pid:
            raise AssertionError('Manual launch created a different application process')
    else:
        raise ValueError('Unknown probe stage')
    Path('/var/tmp/cor-startup-' + stage + '.json').write_text(json.dumps(report))
    print(stage + ': verified', flush=True)


if __name__ == '__main__':
    try:
        main(sys.argv[1])
    except Exception as error:
        Path('/var/tmp/cor-startup-' + sys.argv[1] + '.json').write_text(json.dumps({
            'stage': sys.argv[1], 'error': type(error).__name__, 'detail': str(error),
        }))
        raise
