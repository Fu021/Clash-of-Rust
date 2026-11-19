"""Run the native Wayland checks in the disposable Pop!_OS login."""
import argparse
import json
import os
from pathlib import Path
import platform

from build_support import write_json
from linux_cosmic_windows import close, windows
from linux_gui import cli, gui, output, tun_permissions, verify_authorization

AUTH_MARKER = '.cor-cosmic-authorization'
AUTH_PASSWORD = '.cor-cosmic-password'
AUTH_RESULT = 'cosmic-authorization.json'
PROXY_RESULT = 'system-proxy.json'


def authorize(home):
    password = home/AUTH_PASSWORD
    if not password.is_file():
        return
    result = {}
    try:
        session = output(['loginctl', 'show-user', str(os.getuid()), '-p', 'Display', '--value'])
        if not session:
            raise RuntimeError('The COSMIC user has no real display login')
        result['session'] = session
        if tun_permissions() != ('', 0o755):
            raise RuntimeError('COSMIC authorization requires an unprivileged installed helper')

        def request_input(allow):
            (home/AUTH_MARKER).write_text('accept' if allow else 'cancel')

        with (home/'cosmic-authorization.log').open('w') as log:
            result.update(verify_authorization(log, password, lambda: '', respond=request_input))
    except Exception as error:
        result['error'] = str(error)
        raise
    finally:
        password.unlink(missing_ok=True)
        write_json(home/AUTH_RESULT, result)


def verify_system_proxy(mixed_port):
    program = (
        'import json; from gi.repository import Gio; '
        'resolver = Gio.ProxyResolver.get_default(); '
        'print(json.dumps({"resolver": resolver.__gtype__.name, '
        '"routes": resolver.lookup("https://cor-proxy-probe.invalid/", None)}))'
    )

    def resolve():
        return json.loads(output(['/usr/bin/python3', '-c', program]))

    result = {'desktop': os.environ.get('XDG_CURRENT_DESKTOP'),
              'expected_proxy': f'http://127.0.0.1:{mixed_port}'}
    try:
        result['before'] = resolve()
        try:
            cli('proxy', 'system')
            result['enabled'] = resolve()
        finally:
            cli('proxy', 'off')
            result['after'] = resolve()
        if result['after'] != result['before']:
            raise RuntimeError('The native GIO proxy resolver did not return to its previous routing')
        if result['expected_proxy'] not in result['enabled']['routes']:
            raise RuntimeError('The native GIO proxy resolver did not select the installed proxy')
    finally:
        write_json(Path(os.environ['COR_GUI_REPORT'])/PROXY_RESULT, result)


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
