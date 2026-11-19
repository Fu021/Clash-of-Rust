"""Verify candidate autostart through real desktop logins in a disposable VM."""
import argparse
from dataclasses import dataclass
import json
import os
from pathlib import Path
import platform
import pwd
import secrets
import subprocess

from build_support import ROOT, run, sha256, write_json
from linux_apt import install
from linux_gui import core_processes, output, owner_tray, wait_for, wayland_sockets

USER = 'cor-desktop-login'
HOME = Path('/home') / USER
MENU = Path('/usr/share/applications/clash-of-rust.desktop')
AUTOSTART = HOME / '.config/autostart/clash-of-rust.desktop'
REPORT = ROOT / 'target/linux-report/desktop-login'
EXTENSION = 'ubuntu-appindicators@ubuntu.com'
LIGHTDM_CONFIG = Path('/etc/lightdm/lightdm.conf.d/99-cor-compatibility.conf')


@dataclass(frozen=True)
class DesktopLogin:
    name: str
    shell: str
    service: str
    pam_auto_login: str
    environment_name: str


GNOME = DesktopLogin('GNOME', 'gnome-shell', 'gdm.service', 'gdm-autologin', 'GNOME')
CINNAMON = DesktopLogin('Cinnamon', 'cinnamon', 'lightdm.service', 'lightdm-autologin', 'X-Cinnamon')


def as_user(*arguments):
    return ['sudo', '-H', '-u', USER, '--', *arguments]


def configure_lightdm(session):
    run(['sudo', 'install', '-d', '-m', '0755', LIGHTDM_CONFIG.parent])
    config = (f'[Seat:*]\nuser-session={session}\nautologin-session={session}\n'
              f'autologin-user={USER}\nautologin-user-timeout=0\n')
    run(['sudo', 'tee', LIGHTDM_CONFIG], input=config, text=True, stdout=subprocess.DEVNULL)


def prepare(report, desktop, version):
    packages = ['dbus-user-session', 'libgtk-3-bin', 'wmctrl']
    if desktop == GNOME:
        packages.extend(['gdm3', 'gnome-session', 'xserver-xorg'])
        if version == '13':
            packages.append('gnome-session-xsession')
    elif not Path('/usr/share/xsessions/cinnamon.desktop').is_file():
        raise RuntimeError('The installed Mint guest lacks the Cinnamon X11 session')
    install(packages, timeout=600, no_recommends=True)
    run(['sudo', 'systemctl', 'stop', desktop.service])
    groups = ['--groups', 'sudo'] if desktop == CINNAMON else []
    run(['sudo', 'useradd', '--create-home', '--shell', '/bin/bash', *groups, USER])
    password = secrets.token_hex(24)
    run(['sudo', 'chpasswd'], input=f'{USER}:{password}\n', text=True)
    run(as_user('mkdir', '-p', AUTOSTART.parent))
    run(as_user('ln', '-s', MENU, AUTOSTART))
    if desktop == GNOME:
        run(as_user('dbus-run-session', '--', 'gsettings', 'set', 'org.gnome.shell',
                    'enabled-extensions', f"['{EXTENSION}']"))
        config = ('[daemon]\nWaylandEnable=false\nAutomaticLoginEnable=true\n'
                  f'AutomaticLogin={USER}\n')
        run(['sudo', 'tee', '/etc/gdm3/daemon.conf'], input=config, text=True,
            stdout=subprocess.DEVNULL)
    else:
        from linux_mint_desktop import stage_authorization
        configure_lightdm('cinnamon')
        stage_authorization(HOME, USER, password)
    run(['sudo', 'systemctl', 'set-default', 'graphical.target'])
    run(['sudo', 'systemctl', 'add-wants', 'graphical.target', desktop.service])
    if '--background' in MENU.read_text():
        raise RuntimeError('The installed menu must launch in the foreground')
    report.update(menu_sha256=sha256(MENU), boot_id=boot_id())


def boot_id():
    return Path('/proc/sys/kernel/random/boot_id').read_text().strip()


def active_desktop(uid, desktop, session_type='x11'):
    sessions = output(['loginctl', 'list-sessions', '--no-legend']).splitlines()
    for line in sessions:
        session, owner, *_ = line.split()
        if owner != str(uid):
            continue
        info = dict(field.split('=', 1) for field in output(
            ['loginctl', 'show-session', session, '-p', 'Type', '-p', 'Class', '-p', 'State',
             '-p', 'Desktop', '-p', 'Service']).splitlines())
        if (info.get('Type'), info.get('Class'), info.get('State'), info.get('Service')) != (
                session_type, 'user', 'active', desktop.pam_auto_login):
            continue
        shell = subprocess.run(['pgrep', '-u', str(uid), '-x', desktop.shell],
                               capture_output=True, text=True, timeout=15)
        for pid in shell.stdout.split():
            try:
                data = output(['sudo', 'cat', f'/proc/{pid}/environ'])
                values = dict(item.split('=', 1) for item in data.split('\0') if '=' in item)
                display = {name: values[name] for name in ('DISPLAY', 'XAUTHORITY') if values.get(name)}
                if session_type == 'x11' and 'DISPLAY' not in display:
                    continue
                if values.get('XDG_SESSION_ID', session) != session:
                    continue
            except subprocess.CalledProcessError:
                continue
            environment = dict(display, XDG_RUNTIME_DIR=f'/run/user/{uid}',
                               DBUS_SESSION_BUS_ADDRESS=f'unix:path=/run/user/{uid}/bus',
                               XDG_CURRENT_DESKTOP=values.get('XDG_CURRENT_DESKTOP', desktop.environment_name))
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
                and status.get('tray_state') == 'created'
                and status.get('window_visible') is visible
                and Path('/proc', str(value['pid'])).stat().st_uid == uid):
            return value
    except (OSError, ValueError, TypeError, KeyError, subprocess.CalledProcessError):
        pass
    return None


def verify(stage, report, desktop):
    background = stage == 'background-login'
    previous = json.loads((REPORT/('first-login.json' if background else 'prepare.json')).read_text())
    report['boot_id'] = boot_id()
    if report['boot_id'] == previous['boot_id']:
        raise RuntimeError(f'{desktop.name} verification requires a new system boot')
    uid = pwd.getpwnam(USER).pw_uid
    login, environment = wait_for(f'{desktop.service} did not open a real {desktop.name} X11 session',
                                  lambda: active_desktop(uid, desktop), timeout=180)
    report['login'] = login
    prior_session = previous.get('startup', {}).get('session_id')
    trace = wait_for(f'{desktop.name} did not autostart the candidate', lambda: startup(
        uid, prior_session, background, not background), timeout=90)
    report['startup'] = trace
    pid = trace['pid']
    try:
        desktop_call(environment, 'busctl', '--user', 'call', 'org.freedesktop.DBus',
                     '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'GetId')
    except subprocess.CalledProcessError as error:
        raise RuntimeError(f'Could not connect to the {desktop.name} user session bus') from error
    report['tray_item'] = wait_for(f'{desktop.name} did not register the candidate tray PID',
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
        run(['sudo', 'systemctl', 'stop', desktop.service])
        run(['sudo', 'systemctl', 'stop', f'user@{uid}.service'])
    else:
        report['foreground_windows'] = wait_for('The foreground window was not mapped', windows)
        if desktop == CINNAMON:
            from linux_mint_desktop import authorization_result
            report['authorization'] = wait_for('Cinnamon authorization probe did not return',
                                               lambda: authorization_result(HOME, USER, login['session']),
                                               timeout=250)


def prepare_cinnamon_gui(backend):
    if backend == 'x11':
        run(as_user('unlink', AUTOSTART))
        return
    if not Path('/usr/share/wayland-sessions/cinnamon-wayland.desktop').is_file():
        raise RuntimeError('The installed Mint guest lacks the Cinnamon Wayland session')
    configure_lightdm('cinnamon-wayland')


def verify_cinnamon_gui(environment, backend):
    from linux_mint_desktop import PROBE_DIR
    environment = dict(environment, COR_LINUX_GUEST='true', LC_ALL='C.UTF-8')
    command = as_user('env', *(f'{key}={value}' for key, value in environment.items()),
                      '/opt/cor-test-python/bin/python3', '-B',
                      HOME/PROBE_DIR/'linux_mint_desktop.py', backend)
    source = HOME/f'gui-{backend}'
    try:
        run(command, timeout=420)
    finally:
        for name in (f'{backend}.log', 'dbus-probes.log'):
            try:
                log = output(as_user('cat', source/name), stderr=subprocess.DEVNULL)
            except subprocess.CalledProcessError:
                continue
            REPORT.mkdir(parents=True, exist_ok=True)
            (REPORT/f'cinnamon-{backend}-{name}').write_text(log+'\n')
    return json.loads(output(as_user('cat', source/'gui.json')))


def verify_native_cinnamon(report, backend):
    previous_stage = 'background-login' if backend == 'x11' else 'x11'
    previous = json.loads((REPORT/f'{previous_stage}.json').read_text())
    report['boot_id'] = boot_id()
    if previous['success'] is not True or report['boot_id'] == previous['boot_id']:
        raise RuntimeError(f'Cinnamon {backend} verification requires a new boot after {previous_stage}')
    uid = pwd.getpwnam(USER).pw_uid
    login, environment = wait_for(f'LightDM did not open a real Cinnamon {backend} session',
                                  lambda: active_desktop(uid, CINNAMON, backend), timeout=180)
    report['login'] = login

    if backend == 'wayland':
        def socket():
            sockets = wayland_sockets(Path(environment['XDG_RUNTIME_DIR']))
            if len(sockets) > 1:
                raise RuntimeError(f'Cinnamon created multiple Wayland sockets: {sorted(sockets)}')
            return next(iter(sockets), None)

        environment['WAYLAND_DISPLAY'] = wait_for('Cinnamon did not create a Wayland socket', socket)
    existing = core_processes(uid=uid)
    if existing:
        raise RuntimeError(f'Cinnamon {backend} session has unexpected cores before GUI launch: {existing}')
    report['gui'] = verify_cinnamon_gui(environment, backend)
    run(['sudo', 'systemctl', 'stop', CINNAMON.service])
    run(['sudo', 'systemctl', 'stop', f'user@{uid}.service'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    native_stages = ('prepare-x11', 'x11', 'prepare-wayland', 'wayland')
    parser.add_argument('stage', choices=('prepare', 'first-login', 'background-login', *native_stages))
    stage = parser.parse_args().stage
    release = platform.freedesktop_os_release()
    desktop = {'debian': GNOME, 'linuxmint': CINNAMON}.get(release.get('ID'))
    if (os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('COR_LINUX_GUEST') != 'true'
            or desktop is None or os.geteuid() == 0):
        raise RuntimeError('This check requires a disposable Debian or Linux Mint compatibility VM')
    if stage in native_stages and desktop != CINNAMON:
        raise RuntimeError('The native GUI stages require the Cinnamon guest')
    report = {'stage': stage, 'success': False}
    try:
        if stage == 'prepare':
            prepare(report, desktop, release.get('VERSION_ID'))
        elif stage in native_stages:
            if stage.startswith('prepare-'):
                prepare_cinnamon_gui(stage.removeprefix('prepare-'))
            else:
                verify_native_cinnamon(report, stage)
        else:
            verify(stage, report, desktop)
        report['success'] = True
    except Exception as error:
        report['error'] = str(error)
        try:
            report[desktop.service.removesuffix('.service') + '_journal'] = output(
                ['sudo', 'journalctl', '-b', '-u', desktop.service, '-n', '80', '--no-pager'])
        except (OSError, subprocess.SubprocessError):
            pass
        raise
    finally:
        write_json(REPORT/f'{stage}.json', report)


if __name__ == '__main__':
    main()
