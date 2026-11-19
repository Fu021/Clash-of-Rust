"""Verify candidate autostart through real desktop logins in a disposable VM."""
import argparse
import configparser
from dataclasses import dataclass
import json
import os
from pathlib import Path
import platform
import pwd
import re
import secrets
import struct
import subprocess
import tempfile
import tomllib

from build_support import ROOT, run, sha256, write_json
from linux_apt import install
from linux_gui import core_processes, output, owner_tray, wait_for

USER = 'cor-desktop-login'
HOME = Path('/home') / USER
MENU = Path('/usr/share/applications/clash-of-rust.desktop')
AUTOSTART = HOME / '.config/autostart/clash-of-rust.desktop'
REPORT = ROOT / 'target/linux-report/desktop-login'
EXTENSION = 'ubuntu-appindicators@ubuntu.com'
LIGHTDM_CONFIG = Path('/etc/lightdm/lightdm.conf.d/99-cor-compatibility.conf')
COSMIC_GREETER_CONFIG = Path('/etc/greetd/cosmic-greeter.toml')
COSMIC_PROBES = HOME/'cor-desktop-probes'
DISPLAYMANAGER_SYSCONFIG = Path('/etc/sysconfig/displaymanager')


@dataclass(frozen=True)
class DesktopLogin:
    name: str
    shell: str
    service: str
    pam_auto_login: str
    environment_name: str


GNOME = DesktopLogin('GNOME', 'gnome-shell', 'gdm.service', 'gdm-autologin', 'GNOME')
POP_GNOME = DesktopLogin('Pop GNOME', 'gnome-shell', 'gdm.service', 'gdm-autologin', 'pop:GNOME')
ZORIN_GNOME = DesktopLogin('Zorin GNOME', 'gnome-shell', 'gdm.service', 'gdm-autologin', 'zorin:GNOME')
CINNAMON = DesktopLogin('Cinnamon', 'cinnamon', 'lightdm.service', 'lightdm-autologin', 'X-Cinnamon')
COSMIC = DesktopLogin('COSMIC', 'cosmic-panel', 'cosmic-greeter.service', 'cosmic-greeter', 'COSMIC')
GNOME_WAYLAND = DesktopLogin('GNOME Wayland', 'gnome-shell', 'gdm.service', 'gdm-autologin', 'GNOME')


def as_user(*arguments):
    return ['sudo', '-H', '-u', USER, '--', *arguments]


def configure_lightdm(session):
    run(['sudo', 'install', '-d', '-m', '0755', LIGHTDM_CONFIG.parent])
    config = (f'[Seat:*]\nuser-session={session}\nautologin-session={session}\n'
              f'autologin-user={USER}\nautologin-user-timeout=0\n')
    run(['sudo', 'tee', LIGHTDM_CONFIG], input=config, text=True, stdout=subprocess.DEVNULL)


def native_gnome_session(directory, desktop):
    for path in sorted(directory.glob('*.desktop')):
        entry = configparser.ConfigParser(interpolation=None, strict=False)
        entry.read(path)
        names = entry.get('Desktop Entry', 'DesktopNames', fallback='').strip(';').replace(';', ':')
        if names.casefold() == desktop.environment_name.casefold():
            return path.stem
    raise RuntimeError(f'No installed {desktop.name} session found in {directory}')


def configure_tumbleweed_autologin():
    config, replacements = re.subn(r'^DISPLAYMANAGER_AUTOLOGIN=.*$', f'DISPLAYMANAGER_AUTOLOGIN="{USER}"',
                                   DISPLAYMANAGER_SYSCONFIG.read_text(), flags=re.MULTILINE)
    if replacements != 1:
        raise RuntimeError('Expected one DISPLAYMANAGER_AUTOLOGIN setting from openSUSE')
    run(['sudo', 'tee', DISPLAYMANAGER_SYSCONFIG], input=config, text=True, stdout=subprocess.DEVNULL)


def prepare(report, desktop, release):
    distribution = release['ID']
    packages = ['dbus-user-session', 'libgtk-3-bin', 'wmctrl']
    if distribution == 'debian':
        packages.extend(['gdm3', 'gnome-session', 'xserver-xorg'])
        if release['VERSION_ID'] == '13':
            packages.append('gnome-session-xsession')
    elif desktop == CINNAMON and not Path('/usr/share/xsessions/cinnamon.desktop').is_file():
        raise RuntimeError('The installed Mint guest lacks the Cinnamon X11 session')
    elif desktop == COSMIC:
        packages.extend(['gcc', 'libc6-dev', 'libwayland-dev', 'libwayland-bin', 'wayland-protocols',
                         'python3-gi', 'glib-networking'])
    if desktop != GNOME_WAYLAND:
        install(packages, timeout=guest_budget(600, 1800), no_recommends=True)
    run(['sudo', 'systemctl', 'stop', desktop.service])
    groups = ['--groups', 'sudo'] if desktop in (CINNAMON, COSMIC) else []
    run(['sudo', 'useradd', '--create-home', '--shell', '/bin/bash', *groups, USER])
    password = secrets.token_hex(24)
    run(['sudo', 'chpasswd'], input=f'{USER}:{password}\n', text=True)
    run(as_user('mkdir', '-p', AUTOSTART.parent))
    run(as_user('ln', '-s', MENU, AUTOSTART))
    if desktop == GNOME_WAYLAND:
        from linux_gnome_login import install as install_probe
        install_probe(as_user)
        session = native_gnome_session(Path('/usr/share/wayland-sessions'), desktop)
        run(['sudo', 'install', '-d', '-m', '0755', '/var/lib/AccountsService/users'])
        run(['sudo', 'tee', Path('/var/lib/AccountsService/users')/USER], text=True,
            input=f'[User]\nSession={session}\nXSession={session}\n', stdout=subprocess.DEVNULL)
        config = f'[daemon]\nWaylandEnable=true\nAutomaticLoginEnable=true\nAutomaticLogin={USER}\n'
        config_path = '/etc/gdm/custom.conf'
        run(['sudo', 'tee', config_path], text=True, input=config, stdout=subprocess.DEVNULL)
        if distribution == 'opensuse-tumbleweed':
            configure_tumbleweed_autologin()
    elif desktop in (GNOME, POP_GNOME, ZORIN_GNOME):
        if desktop != GNOME:
            session = native_gnome_session(Path('/usr/share/xsessions'), desktop)
            run(['sudo', 'tee', Path('/var/lib/AccountsService/users')/USER], text=True,
                input=f'[User]\nSession={session}\nXSession={session}\n', stdout=subprocess.DEVNULL)
        else:
            run(as_user('dbus-run-session', '--', 'gsettings', 'set', 'org.gnome.shell',
                        'enabled-extensions', f"['{EXTENSION}']"))
        config = ('[daemon]\nWaylandEnable=false\nAutomaticLoginEnable=true\n'
                  f'AutomaticLogin={USER}\n')
        config_path = '/etc/gdm3/daemon.conf' if desktop == GNOME else '/etc/gdm3/custom.conf'
        run(['sudo', 'tee', config_path], input=config, text=True,
            stdout=subprocess.DEVNULL)
    elif desktop == CINNAMON:
        from linux_mint_desktop import stage_authorization
        configure_lightdm('cinnamon')
        stage_authorization(HOME, USER, password)
    else:
        from linux_cosmic_windows import prepare as prepare_windows
        config = COSMIC_GREETER_CONFIG.read_text()
        if 'initial_session' in tomllib.loads(config) or not Path('/usr/bin/start-cosmic').is_file():
            raise RuntimeError('Expected the installed COSMIC greeter without a predefined login')
        config += (f'\n[initial_session]\ncommand = "/usr/bin/start-cosmic"\nuser = "{USER}"\n')
        run(['sudo', 'tee', COSMIC_GREETER_CONFIG], input=config, text=True, stdout=subprocess.DEVNULL)
        prepare_windows()
        run(['sudo', 'install', '-d', '-m', '0755', COSMIC_PROBES])
        names = ('linux_cosmic_windows.py', 'linux_cosmic_desktop.py', 'linux_gui.py',
                 'build_support.py', 'cli_tui.py')
        run(['sudo', 'install', '-m', '0644', *(Path(__file__).with_name(name) for name in names),
             COSMIC_PROBES])
        from linux_cosmic_desktop import AUTH_PASSWORD
        from linux_gui import APP, tun_permissions
        with tempfile.NamedTemporaryFile(mode='w') as temporary:
            temporary.write(password)
            temporary.flush()
            run(['sudo', 'install', '-o', USER, '-g', USER, '-m', '0600', temporary.name,
                 HOME/AUTH_PASSWORD])
        entry = AUTOSTART.with_name('cor-cosmic-authorization.desktop')
        run(as_user('tee', entry), text=True, stdout=subprocess.DEVNULL, input=(
            '[Desktop Entry]\nType=Application\nName=Compatibility authorization probe\n'
            'Exec=/usr/bin/env LC_ALL=C.UTF-8 COR_LINUX_GUEST=true /opt/cor-test-python/bin/python3 '
            f'-B {COSMIC_PROBES}/linux_cosmic_desktop.py --authorization\n'))
        capabilities, _ = tun_permissions()
        if capabilities:
            run(['sudo', 'setcap', '-r', APP/'clash-tun-launcher'])
        run(['sudo', 'chmod', '0755', APP/'libexec/resolvectl'])
    run(['sudo', 'systemctl', 'set-default', 'graphical.target'])
    run(['sudo', 'systemctl', 'add-wants', 'graphical.target', desktop.service])
    if '--background' in MENU.read_text():
        raise RuntimeError('The installed menu must launch in the foreground')
    report.update(menu_sha256=sha256(MENU), boot_id=boot_id())


def boot_id():
    return Path('/proc/sys/kernel/random/boot_id').read_text().strip()


def guest_budget(standard, slow):
    return slow if platform.machine().lower() in ('aarch64', 'arm64') else standard


def active_desktop(uid, desktop, session_type='x11'):
    sessions = output(['loginctl', 'list-sessions', '--no-legend']).splitlines()
    for line in sessions:
        session, owner, *_ = line.split()
        if owner != str(uid):
            continue
        info = dict(field.split('=', 1) for field in output(
            ['loginctl', 'show-session', session, '-p', 'Type', '-p', 'Class', '-p', 'State',
             '-p', 'Desktop', '-p', 'Service', '-p', 'Seat', '-p', 'Remote']).splitlines())
        session_types = (session_type, 'tty') if desktop == COSMIC else (session_type,)
        if (info.get('Type') not in session_types
                or (info.get('Class'), info.get('State'), info.get('Service')) != (
                    'user', 'active', desktop.pam_auto_login)):
            continue
        if desktop == COSMIC and (info.get('Seat'), info.get('Remote')) != ('seat0', 'no'):
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
                if (desktop in (POP_GNOME, ZORIN_GNOME)
                        and values.get('XDG_CURRENT_DESKTOP', '').casefold() != desktop.environment_name.casefold()):
                    continue
                if desktop == COSMIC:
                    socket = values.get('WAYLAND_DISPLAY')
                    current_desktop = values.get('XDG_CURRENT_DESKTOP', '')
                    if ('COSMIC' not in current_desktop.split(':') or not socket
                            or socket not in desktop_wayland_sockets(f'/run/user/{uid}')):
                        continue
                    display.update(WAYLAND_DISPLAY=socket, XDG_SESSION_TYPE='wayland')
                if values.get('XDG_SESSION_ID', session) != session:
                    continue
            except subprocess.CalledProcessError:
                continue
            environment = dict(display, XDG_RUNTIME_DIR=f'/run/user/{uid}',
                               DBUS_SESSION_BUS_ADDRESS=f'unix:path=/run/user/{uid}/bus',
                               XDG_CURRENT_DESKTOP=values.get('XDG_CURRENT_DESKTOP', desktop.environment_name))
            return {'session': session, 'shell_pid': int(pid), 'logind': info,
                    'desktop_environment': environment['XDG_CURRENT_DESKTOP']}, environment
    return None


def desktop_call(environment, *arguments, **options):
    return output(as_user('env', *(f'{key}={value}' for key, value in environment.items()), *arguments),
                  **options)


def x11_windows(environment):
    def listing():
        try:
            return (desktop_call(environment, 'wmctrl', '-l', stderr=subprocess.PIPE),)
        except subprocess.CalledProcessError as error:
            detail = error.stderr or ''
            if 'BadWindow' in detail:
                return None
            raise RuntimeError(f'Could not enumerate X11 windows: {detail.strip()}') from error

    windows, = wait_for('X11 client list kept changing during enumeration', listing,
                        timeout=guest_budget(5, 15))
    return [line.split(maxsplit=1)[0] for line in windows.splitlines()
            if line.endswith(' Clash of Rust')]


def desktop_wayland_sockets(runtime):
    return set(output(as_user('find', runtime, '-maxdepth', '1', '-type', 's',
                              '(', '-name', 'wayland-*', '-o', '-name', 'cor-wayland', ')',
                              '-printf', r'%f\n')).splitlines())


def capture_cosmic(environment, stage):
    directory = Path(output(as_user('mktemp', '-d', HOME/'cor-capture-XXXXXXXX')))
    result = subprocess.check_output(as_user(
        'env', *(f'{key}={value}' for key, value in environment.items()),
        'cosmic-screenshot', '--interactive=false', '--notify=false', '--save-dir', directory,
    ), text=True, timeout=60).strip()
    screenshot = Path(result)
    if screenshot.parent != directory or screenshot.suffix != '.png':
        raise RuntimeError('COSMIC portal did not return a PNG in its capture directory')
    size_limit = 16 * 1024 * 1024
    data = subprocess.check_output(as_user('head', '-c', str(size_limit + 1), screenshot), timeout=15)
    if (not 24 <= len(data) <= size_limit or not data.startswith(b'\x89PNG\r\n\x1a\n')
            or data[12:16] != b'IHDR'):
        raise RuntimeError('COSMIC produced an invalid screenshot')
    width, height = struct.unpack('>II', data[16:24])
    if width < 800 or height < 450:
        raise RuntimeError(f'COSMIC screenshot was too small: {width}x{height}')
    destination = REPORT/f'{stage}.png'
    destination.write_bytes(data)
    return {'filename': destination.name, 'width': width, 'height': height}


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
    session_type = 'wayland' if desktop in (COSMIC, GNOME_WAYLAND) else 'x11'
    login, environment = wait_for(
        f'{desktop.service} did not open a real {desktop.name} {session_type} session',
        lambda: active_desktop(uid, desktop, session_type), timeout=guest_budget(180, 360))
    report['login'] = login
    prior_session = previous.get('startup', {}).get('session_id')
    trace = wait_for(f'{desktop.name} did not autostart the candidate', lambda: startup(
        uid, prior_session, background, not background), timeout=guest_budget(90, 300))
    report['startup'] = trace
    pid = trace['pid']
    try:
        desktop_call(environment, 'busctl', '--user', 'call', 'org.freedesktop.DBus',
                     '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'GetId')
    except subprocess.CalledProcessError as error:
        raise RuntimeError(f'Could not connect to the {desktop.name} user session bus') from error
    report['tray_item'] = wait_for(f'{desktop.name} did not register the candidate tray PID',
                                   lambda: owner_tray(pid, lambda command: desktop_call(environment, *command)),
                                   timeout=guest_budget(45, 180))
    menu_sha256 = sha256(MENU)
    entry = output(as_user('cat', AUTOSTART))
    mode = output(as_user('stat', '--format=%a:%F', AUTOSTART), env=dict(os.environ, LC_ALL='C'))
    if menu_sha256 != previous['menu_sha256'] or mode != '600:regular file' or '--background' not in entry:
        raise RuntimeError('Legacy autostart did not migrate while preserving the installed menu')
    report.update(menu_sha256=menu_sha256, migrated_background_entry=True)

    def windows():
        if desktop == COSMIC:
            from linux_cosmic_windows import windows as inspect_windows
            return inspect_windows(lambda command: desktop_call(environment, *command))
        if desktop == GNOME_WAYLAND:
            from linux_gnome_login import windows as inspect_windows
            return inspect_windows(lambda *command: desktop_call(environment, *command, stderr=subprocess.PIPE))
        return x11_windows(environment)

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
        if desktop == COSMIC:
            report['background_screenshot'] = capture_cosmic(environment, 'background-login')
        service, path = report['tray_item']
        desktop_call(environment, 'gdbus', 'call', '--session', '--dest', service, '--object-path', path,
                     '--method', 'org.kde.StatusNotifierItem.Activate', '0', '0')
        tray = wait_for('The tray did not reopen the existing owner', lambda: window_state(True),
                        timeout=guest_budget(45, 180))
        report['tray_reopen'] = tray
        if desktop == COSMIC:
            from linux_cosmic_windows import close
            close(lambda command: desktop_call(environment, *command))
        elif desktop == GNOME_WAYLAND:
            from linux_gnome_login import call
            for window in tray['windows']:
                call(lambda *command: desktop_call(environment, *command),
                     'CloseWindow', str(window['sequence']))
        else:
            for window in tray['windows']:
                desktop_call(environment, 'wmctrl', '-ic', window)
        report['closed_window'] = wait_for('Closing the window did not return the owner to the tray',
                                           lambda: window_state(False), timeout=guest_budget(45, 180))
        desktop_call(environment, 'gtk-launch', MENU.stem)
        report['menu_reopen'] = wait_for('The application menu did not reopen the existing owner',
                                         lambda: window_state(True), timeout=guest_budget(45, 180))
        if desktop == COSMIC:
            report['menu_screenshot'] = capture_cosmic(environment, 'manual-launch')
        run(['sudo', 'systemctl', 'stop', desktop.service])
        run(['sudo', 'systemctl', 'stop', f'user@{uid}.service'])
    else:
        report['foreground_windows'] = wait_for('The foreground window was not mapped', windows,
                                                timeout=guest_budget(45, 180))
        if desktop == COSMIC:
            from linux_cosmic_desktop import AUTH_RESULT
            from linux_gui import ACTION

            def authorization_result():
                try:
                    value = json.loads(output(as_user('cat', HOME/AUTH_RESULT), stderr=subprocess.DEVNULL))
                except (ValueError, subprocess.CalledProcessError):
                    return None
                if (value.get('session') != login['session'] or value.get('production_action') != ACTION
                        or value.get('cancel_preserved_permissions') is not True
                        or value.get('accept_granted_helper') is not True):
                    raise RuntimeError('COSMIC authorization failed: ' + value.get('error', 'missing checks'))
                return value

            report['authorization'] = wait_for('COSMIC authorization probe did not return',
                                               authorization_result, timeout=guest_budget(250, 450))
            report['foreground_screenshot'] = capture_cosmic(environment, 'first-login')
        if desktop == CINNAMON:
            from linux_mint_desktop import AUTH_LOG, authorization_result
            try:
                report['authorization'] = wait_for('Cinnamon authorization probe did not return',
                                                   lambda: authorization_result(HOME, USER, login['session']),
                                                   timeout=guest_budget(250, 450))
            finally:
                try:
                    report['authorization_log'] = output(as_user('cat', HOME/AUTH_LOG), stderr=subprocess.PIPE)
                except subprocess.SubprocessError:
                    pass


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
            sockets = desktop_wayland_sockets(environment['XDG_RUNTIME_DIR'])
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


def verify_native_cosmic(report):
    from linux_cosmic_desktop import PROXY_RESULT

    previous = json.loads((REPORT/'background-login.json').read_text())
    report['boot_id'] = boot_id()
    if previous['success'] is not True or report['boot_id'] == previous['boot_id']:
        raise RuntimeError('COSMIC Wayland verification requires a fresh login after autostart checks')
    uid = pwd.getpwnam(USER).pw_uid
    login, environment = wait_for('COSMIC Greeter did not open a real Wayland session',
                                  lambda: active_desktop(uid, COSMIC, 'wayland'), timeout=180)
    report['login'] = login
    if existing := core_processes(uid=uid):
        raise RuntimeError(f'COSMIC session has unexpected cores before GUI launch: {existing}')
    environment.update(COR_LINUX_GUEST='true', LC_ALL='C.UTF-8')
    command = as_user('env', *(f'{key}={value}' for key, value in environment.items()),
                      '/opt/cor-test-python/bin/python3', '-B', COSMIC_PROBES/'linux_cosmic_desktop.py')
    source = HOME/'gui-wayland'
    try:
        run(command, timeout=420)
    finally:
        for name in ('wayland.log', 'dbus-probes.log', PROXY_RESULT):
            try:
                log = output(as_user('cat', source/name), stderr=subprocess.DEVNULL)
            except subprocess.CalledProcessError:
                continue
            (REPORT/f'cosmic-{name}').write_text(log+'\n')
    report['gui'] = json.loads(output(as_user('cat', source/'gui.json')))
    run(['sudo', 'systemctl', 'stop', COSMIC.service])
    run(['sudo', 'systemctl', 'stop', f'user@{uid}.service'])


def cosmic_failure_evidence():
    uid = pwd.getpwnam(USER).pw_uid
    runtime = f'/run/user/{uid}'
    commands = {
        'user_manager': ['systemctl', 'show', f'user@{uid}.service',
                         '--property=InvocationID,ActiveState,SubState,MainPID'],
        'autostart_units': as_user('env', f'XDG_RUNTIME_DIR={runtime}',
                                   f'DBUS_SESSION_BUS_ADDRESS=unix:path={runtime}/bus',
                                   'systemctl', '--user', 'list-units', '--all', '--no-pager', '*clash*'),
        'application_journal': ['sudo', 'journalctl', '-b', '--no-pager', '-n', '120', f'_UID={uid}',
                                '--grep=clash|autostart|cosmic-session.target'],
    }
    evidence = {}
    for name, command in commands.items():
        try:
            result = subprocess.run(command, capture_output=True, text=True, timeout=15)
            evidence[name] = {'exit_code': result.returncode, 'stdout': result.stdout[-16000:],
                              'stderr': result.stderr[-4000:]}
        except (OSError, subprocess.SubprocessError) as error:
            evidence[name] = {'error': str(error)}
    return evidence


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    native_stages = ('prepare-x11', 'x11', 'prepare-wayland', 'wayland')
    parser.add_argument('stage', choices=('prepare', 'first-login', 'background-login', *native_stages))
    stage = parser.parse_args().stage
    release = platform.freedesktop_os_release()
    from linux_native_guest import DISTROS
    desktop = {'debian': GNOME, 'linuxmint': CINNAMON,
               'zorin': ZORIN_GNOME}.get(release.get('ID'))
    if release.get('ID') in DISTROS:
        desktop = GNOME_WAYLAND
    if (release.get('ID'), release.get('VERSION_ID')) == ('pop', '22.04'):
        desktop = POP_GNOME
    if (release.get('ID'), release.get('VERSION_ID')) == ('pop', '24.04'):
        desktop = COSMIC
    if (os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('COR_LINUX_GUEST') != 'true'
            or desktop is None or os.geteuid() == 0):
        raise RuntimeError('This check requires a disposable Linux desktop compatibility VM')
    if stage in native_stages and not (desktop == CINNAMON
                                       or (desktop == COSMIC and stage.endswith('wayland'))):
        raise RuntimeError('The native GUI stage does not match the installed desktop')
    report = {'stage': stage, 'success': False}
    try:
        if stage == 'prepare':
            prepare(report, desktop, release)
        elif desktop == COSMIC and stage == 'prepare-wayland':
            run(as_user('unlink', AUTOSTART))
        elif desktop == COSMIC and stage == 'wayland':
            verify_native_cosmic(report)
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
        try:
            report['login_user_journal'] = output(
                ['sudo', 'journalctl', '-b', f'_UID={pwd.getpwnam(USER).pw_uid}',
                 '-n', '80', '--no-pager'])
        except (KeyError, OSError, subprocess.SubprocessError):
            pass
        try:
            report['login_processes'] = '\n'.join(output(
                as_user('ps', '-u', USER, '-o', 'pid,ppid,stat,etimes,comm,args', '--no-headers')
            ).splitlines()[:80])
        except (OSError, subprocess.SubprocessError):
            pass
        if desktop == COSMIC:
            try:
                report['user_failure'] = cosmic_failure_evidence()
            except KeyError:
                pass
        raise
    finally:
        write_json(REPORT/f'{stage}.json', report)


if __name__ == '__main__':
    main()
