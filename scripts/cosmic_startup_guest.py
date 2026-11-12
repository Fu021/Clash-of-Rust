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


def session_environment(account):
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if entry.stat().st_uid != account.pw_uid or (entry / 'comm').read_text().strip() != 'cosmic-session':
                continue
            values = dict(item.split('=', 1) for item in
                          (entry / 'environ').read_bytes().decode().split('\0') if '=' in item)
            return {
                'HOME': account.pw_dir, 'USER': account.pw_name,
                'XDG_RUNTIME_DIR': '/run/user/' + str(account.pw_uid),
                'DBUS_SESSION_BUS_ADDRESS': 'unix:path=/run/user/' + str(account.pw_uid) + '/bus',
                'XDG_CURRENT_DESKTOP': 'COSMIC', 'XDG_SESSION_TYPE': 'wayland',
                **{key: values[key] for key in ('DISPLAY', 'WAYLAND_DISPLAY', 'XAUTHORITY') if key in values},
            }
        except (OSError, ValueError):
            continue
    return None


def trace(account, background, visible):
    for path in (Path(account.pw_dir) / '.local/share').glob('*/desktop-startup.json'):
        try:
            value = json.loads(path.read_text())
        except (OSError, ValueError):
            continue
        status = value.get('status', {})
        if (value.get('background_requested') is background
                and status.get('tray_state') == 'created'
                and status.get('window_visible') is visible
                and Path('/proc', str(value.get('pid'))).is_dir()):
            return value
    return None


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
        subprocess.run(['apt-get', 'update'], check=True, timeout=300)
        subprocess.run(
            ['apt-get', 'install', '--no-install-recommends', '-y', '/mnt/cor-test/candidate.deb'],
            check=True, timeout=300, env={**os.environ, 'DEBIAN_FRONTEND': 'noninteractive'},
        )
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
        restart_login(account)
        background = stage == 'background-login'
        report['startup'] = wait_for(lambda: trace(account, background, not background))
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
        command('runuser', '-u', account.pw_name, '--', 'env',
                *(key + '=' + value for key, value in environment.items()),
                'gtk-launch', 'clash-of-rust')
        report['startup'] = wait_for(lambda: trace(account, True, True))
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
