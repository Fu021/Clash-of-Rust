"""Run the native Wayland checks in the disposable Pop!_OS login."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess

from build_support import write_json
from linux_cosmic_windows import close, windows
from linux_gui import APP, gui, output, tun_permissions, verify_authorization, wait_for

AUTH_MARKER = '.cor-cosmic-authorization'
AUTH_PASSWORD = '.cor-cosmic-password'
AUTH_RESULT = 'cosmic-authorization.json'
AUTH_LOG = 'cosmic-authorization.log'
PROXY_RESULT = 'system-proxy.json'


def authorize(home):
    password = home/AUTH_PASSWORD
    if not password.is_file():
        return
    result = {}
    try:
        def active_session():
            try:
                session = output(['loginctl', 'show-user', str(os.getuid()), '-p', 'Display', '--value'])
                if session and output(['loginctl', 'show-session', session, '-p', 'Active', '--value']) == 'yes':
                    return session
            except subprocess.TimeoutExpired:
                return None
            return None

        result['session'] = wait_for('The COSMIC user did not reach an active display login',
                                     active_session, timeout=45)
        if tun_permissions() != ('', 0o755):
            raise RuntimeError('COSMIC authorization requires an unprivileged installed helper')

        marker = home/AUTH_MARKER
        marker.write_text(f'login {result["session"]}')
        wait_for('COSMIC authorization agent was not ready for the display login',
                 lambda: marker.read_text() == 'ready', timeout=120)

        def request_input(allow):
            marker.write_text('accept' if allow else 'cancel')

        with (home/AUTH_LOG).open('w') as log:
            result.update(verify_authorization(log, password, lambda: '', respond=request_input))
    except Exception as error:
        result['error'] = str(error)
        raise
    finally:
        password.unlink(missing_ok=True)
        write_json(home/AUTH_RESULT, result)


def verify_system_proxy(_mixed_port):
    program = (
        'import json; from gi.repository import Gio; '
        'resolver = Gio.ProxyResolver.get_default(); '
        'print(json.dumps({"resolver": resolver.__gtype__.name, '
        '"routes": resolver.lookup("https://cor-proxy-probe.invalid/", None)}))'
    )

    def resolve():
        return json.loads(output(['/usr/bin/python3', '-c', program]))

    data = Path(os.environ['CLASH_OF_RUST_DATA_DIR'])
    settings = data/'settings.json'
    journal = data/'system-proxy.json'
    result = {'desktop': os.environ.get('XDG_CURRENT_DESKTOP')}
    try:
        result['before'] = resolve()
        original = settings.read_bytes()
        command = subprocess.run([APP/'cor', 'proxy', 'system'], text=True, stdout=subprocess.PIPE,
                                 stderr=subprocess.STDOUT, timeout=30)
        result['after'] = resolve()
        result['refused'] = bool(command.returncode and 'COSMIC 桌面暂不支持系统代理' in command.stdout)
        result['settings_unchanged'] = original == settings.read_bytes()
        result['journal_absent'] = not journal.exists()
        if (not all(result[key] for key in ('refused', 'settings_unchanged', 'journal_absent'))
                or result['after'] != result['before']):
            raise RuntimeError('COSMIC system proxy rejection changed the installed routing or settings')
    finally:
        write_json(Path(os.environ['COR_GUI_REPORT'])/PROXY_RESULT, result)
    return {'desktop_system_proxy_refused': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--authorization', action='store_true')
    arguments = parser.parse_args()
    release = platform.freedesktop_os_release()
    if (os.geteuid() == 0 or (release.get('ID'), release.get('VERSION_ID')) != ('pop', '24.04')
            or os.environ.get('COR_LINUX_GUEST') != 'true' or not os.environ.get('WAYLAND_DISPLAY')):
        raise RuntimeError('The probe requires a disposable ordinary-user Pop!_OS COSMIC login')
    home = Path.home()
    if arguments.authorization:
        authorize(home)
        return
    report = home/'gui-wayland'
    report.mkdir()
    os.environ['COR_GUI_REPORT'] = str(report)
    value = gui('wayland', report, force_wayland=True, verify_proxy=verify_system_proxy,
                inspect_window=windows, close_window=close)
    write_json(report/'gui.json', value)


if __name__ == '__main__':
    main()
