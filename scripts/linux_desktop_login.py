"""Verify candidate autostart through real Debian GNOME logins in a disposable VM."""
import argparse
import json
import os
from pathlib import Path
import platform
import pwd
import secrets
import subprocess

from build_support import ROOT, run, sha256, write_json
from linux_apt import install
from linux_gui import output, owner_tray, wait_for

USER = 'cor-desktop-login'
HOME = Path('/home') / USER
MENU = Path('/usr/share/applications/clash-of-rust.desktop')
AUTOSTART = HOME / '.config/autostart/clash-of-rust.desktop'
REPORT = ROOT / 'target/linux-report/desktop-login'
EXTENSION = 'ubuntu-appindicators@ubuntu.com'
DISPLAY_MANAGER = 'gdm.service'


def as_user(*arguments):
    return ['sudo', '-H', '-u', USER, '--', *arguments]


def prepare(report):
    packages = ['dbus-user-session', 'gdm3', 'gnome-session', 'xserver-xorg', 'libgtk-3-bin']
    if platform.freedesktop_os_release().get('VERSION_ID') == '13':
        packages.append('gnome-session-xsession')
    install(packages, timeout=600, no_recommends=True)
    run(['sudo', 'systemctl', 'stop', DISPLAY_MANAGER])
    run(['sudo', 'useradd', '--create-home', '--shell', '/bin/bash', USER])
    run(['sudo', 'chpasswd'], input=f'{USER}:{secrets.token_hex(24)}\n', text=True)
    run(as_user('mkdir', '-p', AUTOSTART.parent))
    run(as_user('ln', '-s', MENU, AUTOSTART))
    run(as_user('dbus-run-session', '--', 'gsettings', 'set', 'org.gnome.shell',
                'enabled-extensions', f"['{EXTENSION}']"))
    config = ('[daemon]\nWaylandEnable=false\nAutomaticLoginEnable=true\n'
              f'AutomaticLogin={USER}\n')
    run(['sudo', 'tee', '/etc/gdm3/daemon.conf'], input=config, text=True,
        stdout=subprocess.DEVNULL)
    run(['sudo', 'systemctl', 'set-default', 'graphical.target'])
    run(['sudo', 'systemctl', 'add-wants', 'graphical.target', DISPLAY_MANAGER])
    if '--background' in MENU.read_text():
        raise RuntimeError('The installed menu must launch in the foreground')
    report.update(menu_sha256=sha256(MENU), boot_id=boot_id())


def boot_id():
    return Path('/proc/sys/kernel/random/boot_id').read_text().strip()


def desktop(uid):
    sessions = output(['loginctl', 'list-sessions', '--no-legend']).splitlines()
    for line in sessions:
        session, owner, *_ = line.split()
        if owner != str(uid):
            continue
        info = dict(field.split('=', 1) for field in output(
            ['loginctl', 'show-session', session, '-p', 'Type', '-p', 'Class', '-p', 'State',
             '-p', 'Desktop', '-p', 'Service']).splitlines())
        if (info.get('Type'), info.get('Class'), info.get('State'), info.get('Service')) != (
                'x11', 'user', 'active', 'gdm-autologin'):
            continue
        shell = subprocess.run(['pgrep', '-u', str(uid), '-x', 'gnome-shell'],
                               capture_output=True, text=True, timeout=15)
        for pid in shell.stdout.split():
            try:
                data = output(['sudo', 'cat', f'/proc/{pid}/environ'])
                values = dict(item.split('=', 1) for item in data.split('\0') if '=' in item)
                display = {name: values[name] for name in ('DISPLAY', 'XAUTHORITY') if values.get(name)}
                if 'DISPLAY' not in display:
                    continue
            except subprocess.CalledProcessError:
                continue
            environment = dict(display, XDG_RUNTIME_DIR=f'/run/user/{uid}',
                               DBUS_SESSION_BUS_ADDRESS=f'unix:path=/run/user/{uid}/bus',
                               XDG_CURRENT_DESKTOP=values.get('XDG_CURRENT_DESKTOP', 'GNOME'))
            return {'session': session, 'shell_pid': int(pid), 'logind': info}, environment
    return None


def desktop_call(environment, *arguments):
    return output(as_user('env', *(f'{key}={value}' for key, value in environment.items()), *arguments))


def startup(uid, previous, background, visible, current=None):
    try:
        files = output(as_user('find', HOME/'.local/share', '-maxdepth', '2', '-name',
                               'desktop-startup.json', '-type', 'f'),
                       stderr=subprocess.DEVNULL).splitlines()
        if len(files) != 1:
            return None
        value = json.loads(output(as_user('cat', files[0])))
        session = value['session_id']
        status = value['status'] or {}
        if (value['kind'] == 'desktop_startup' and value['schema'] == 1 and session
                and session != previous and (current is None or session == current)
                and value['background_requested'] is background
                and status.get('tray_state') == 'created' and status.get('window_visible') is visible
                and Path('/proc', str(value['pid'])).stat().st_uid == uid):
            return value
    except (OSError, ValueError, TypeError, KeyError, subprocess.CalledProcessError):
        pass
    return None


def verify(stage, report):
    background = stage == 'background-login'
    previous = json.loads((REPORT/('first-login.json' if background else 'prepare.json')).read_text())
    report['boot_id'] = boot_id()
    if report['boot_id'] == previous['boot_id']:
        raise RuntimeError('GNOME verification requires a new system boot')
    uid = pwd.getpwnam(USER).pw_uid
    login, environment = wait_for('GDM did not open a real GNOME X11 session', lambda: desktop(uid), timeout=180)
    report['login'] = login
    prior_session = previous.get('startup', {}).get('session_id')
    trace = wait_for('GNOME did not autostart the candidate', lambda: startup(
        uid, prior_session, background, not background), timeout=90)
    report['startup'] = trace
    pid = trace['pid']
    try:
        desktop_call(environment, 'busctl', '--user', 'call', 'org.freedesktop.DBus',
                     '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'GetId')
    except subprocess.CalledProcessError as error:
        raise RuntimeError('Could not connect to the GNOME user session bus') from error
    report['tray_item'] = wait_for('GNOME AppIndicator did not register the candidate PID',
                                   lambda: owner_tray(pid, lambda command: desktop_call(environment, *command)))
    menu_sha256 = sha256(MENU)
    entry = output(as_user('cat', AUTOSTART))
    mode = output(as_user('stat', '--format=%a:%F', AUTOSTART), env=dict(os.environ, LC_ALL='C'))
    if menu_sha256 != previous['menu_sha256'] or mode != '600:regular file' or '--background' not in entry:
        raise RuntimeError('Legacy autostart did not migrate while preserving the installed menu')
    report.update(menu_sha256=menu_sha256, migrated_background_entry=True)

    def windows():
        return [line.split(maxsplit=1)[0] for line in desktop_call(environment, 'wmctrl', '-l').splitlines()
                if line.endswith(' Clash of Rust')]

    def window_state(visible):
        state = startup(uid, prior_session, True, visible, current=trace['session_id'])
        if state is None:
            return None
        if state['pid'] != pid:
            raise RuntimeError('Reopening the window changed the desktop owner')
        mapped = windows()
        return {'owner_pid': pid, 'windows': mapped} if bool(mapped) == visible else None

    if background:
        if windows():
            raise RuntimeError('Background autostart displayed the main window')
        service, path = report['tray_item']
        desktop_call(environment, 'gdbus', 'call', '--session', '--dest', service, '--object-path', path,
                     '--method', 'org.kde.StatusNotifierItem.Activate', '0', '0')
        tray = wait_for('The tray did not reopen the existing owner', lambda: window_state(True))
        report['tray_reopen'] = tray
        for window in tray['windows']:
            desktop_call(environment, 'wmctrl', '-ic', window)
        report['closed_window'] = wait_for('Closing the window did not return the owner to the tray',
                                           lambda: window_state(False))
        desktop_call(environment, 'gtk-launch', MENU.stem)
        report['menu_reopen'] = wait_for('The application menu did not reopen the existing owner',
                                         lambda: window_state(True))
        run(['sudo', 'systemctl', 'stop', DISPLAY_MANAGER])
        run(['sudo', 'systemctl', 'stop', f'user@{uid}.service'])
    else:
        report['foreground_windows'] = wait_for('The foreground window was not mapped', windows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('stage', choices=('prepare', 'first-login', 'background-login'))
    stage = parser.parse_args().stage
    if (os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('COR_LINUX_GUEST') != 'true'
            or platform.freedesktop_os_release().get('ID') != 'debian' or os.geteuid() == 0):
        raise RuntimeError('This check requires a disposable Debian compatibility VM')
    report = {'stage': stage, 'success': False}
    try:
        if stage == 'prepare':
            prepare(report)
        else:
            verify(stage, report)
        report['success'] = True
    except Exception as error:
        report['error'] = str(error)
        try:
            report['gdm_journal'] = output(
                ['sudo', 'journalctl', '-b', '-u', DISPLAY_MANAGER, '-n', '80', '--no-pager'])
        except (OSError, subprocess.SubprocessError):
            pass
        raise
    finally:
        write_json(REPORT/f'{stage}.json', report)


if __name__ == '__main__':
    main()
