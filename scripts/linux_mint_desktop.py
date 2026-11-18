"""Native Cinnamon GUI checks for the disposable Mint login fixture."""
import argparse
import ast
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile

from build_support import run, write_json
from linux_gui import (ACTION, APP, cli, gtk_authorization_window, gui, output, require_logind_session,
                       tun_permissions, verify_authorization)

AUTH_RESULT = 'authorization.json'
PROBE_DIR = 'cor-desktop-probes'
PASSWORD_FILE = '.cor-authorization-password'
CINNAMON_PROMPT = (
    '(()=>{for(let actor=global.stage.get_key_focus(); actor; actor=actor.get_parent()){'
    'if(actor.has_style_class_name && actor.has_style_class_name("prompt-dialog-password-entry")) '
    'return actor.is_mapped() && actor.reactive;}return false;})()'
)


def cinnamon_eval(call, expression):
    value = call(['gdbus', 'call', '--session', '--dest', 'org.Cinnamon', '--object-path',
                  '/org/Cinnamon', '--method', 'org.Cinnamon.Eval', expression])
    success, result = ast.literal_eval(value.replace('(true,', '(True,', 1).replace('(false,', '(False,', 1))
    if not success:
        raise RuntimeError('Cinnamon rejected the desktop test probe')
    return json.loads(result)


def stage_authorization(home, user, password):
    probe = home/PROBE_DIR
    run(['sudo', 'install', '-d', '-m', '0755', probe])
    for name in ('linux_mint_desktop.py', 'linux_gui.py', 'build_support.py', 'cli_tui.py'):
        run(['sudo', 'install', '-m', '0644', Path(__file__).resolve().with_name(name), probe/name])
    with tempfile.NamedTemporaryFile(mode='w') as source:
        source.write(password)
        source.flush()
        run(['sudo', 'install', '-o', user, '-g', user, '-m', '0600', source.name, home/PASSWORD_FILE])
    entry = home/'.config/autostart/cor-authorization-probe.desktop'
    run(['sudo', '-u', user, 'tee', entry], text=True, stdout=subprocess.DEVNULL, input=(
        '[Desktop Entry]\nType=Application\nName=Compatibility authorization probe\n'
        f'Exec=/usr/bin/env LC_ALL=C.UTF-8 /opt/cor-test-python/bin/python3 -B {probe}/linux_mint_desktop.py\n'))
    capabilities, _ = tun_permissions()
    if capabilities:
        run(['sudo', 'setcap', '-r', APP/'clash-tun-launcher'])
    run(['sudo', 'chmod', '0755', APP/'libexec/resolvectl'])


def authorization_result(home, user, session):
    path = home/AUTH_RESULT
    try:
        value = json.loads(output(['sudo', '-u', user, 'cat', path], stderr=subprocess.DEVNULL))
    except (ValueError, subprocess.CalledProcessError):
        return None
    if not isinstance(value, dict) or value.get('session') != session:
        raise RuntimeError('Cinnamon authorization came from a different desktop session')
    if (value.get('production_action') != ACTION or value.get('cancel_preserved_permissions') is not True
            or value.get('accept_granted_helper') is not True):
        raise RuntimeError('Native Cinnamon authorization failed: ' + value.get('error', 'missing checks'))
    return value


def authorization():
    home = Path.home()
    password = home/PASSWORD_FILE
    if not password.is_file():
        return
    release = platform.freedesktop_os_release()
    if (os.geteuid() == 0 or release.get('ID') != 'linuxmint' or not os.environ.get('DISPLAY')
            or os.environ.get('XDG_SESSION_TYPE') != 'x11'):
        raise RuntimeError('Mint authorization requires the disposable ordinary-user Cinnamon X11 login')
    result = {'session': os.environ.get('XDG_SESSION_ID')}
    try:
        result['session'] = require_logind_session()
        if tun_permissions() != ('', 0o755):
            raise RuntimeError('Mint authorization fixture expected an unprivileged helper')
        use_cinnamon_prompt = release['VERSION_ID'] == '22.3'
        result['agent'] = 'cinnamon' if use_cinnamon_prompt else 'gtk'

        def locate():
            if not use_cinnamon_prompt:
                return gtk_authorization_window()
            try:
                return '' if cinnamon_eval(output, CINNAMON_PROMPT) else None
            except subprocess.CalledProcessError:
                return None

        with (home/'authorization.log').open('w') as log:
            result.update(verify_authorization(log, password, locate))
    except Exception as error:
        result['error'] = str(error)
        raise
    finally:
        password.unlink(missing_ok=True)
        write_json(home/AUTH_RESULT, result)


def verify_system_proxy():
    schema = 'org.gnome.system.proxy'

    def settings():
        return tuple(output(['gsettings', 'list-recursively', schema+suffix])
                     for suffix in ('', '.http', '.https', '.socks'))

    before = settings()
    try:
        cli('proxy', 'system')
        if (output(['gsettings', 'get', schema, 'mode']) != "'manual'"
                or output(['gsettings', 'get', schema+'.http', 'host']) != "'127.0.0.1'"):
            raise RuntimeError('The installed owner did not apply the Cinnamon system proxy')
    finally:
        cli('proxy', 'off')
    if settings() != before:
        raise RuntimeError('Turning off the Cinnamon system proxy did not restore the previous settings')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('backend', nargs='?', choices=('x11', 'wayland'))
    backend = parser.parse_args().backend
    if backend is None:
        authorization()
        return
    release = platform.freedesktop_os_release()
    display = 'WAYLAND_DISPLAY' if backend == 'wayland' else 'DISPLAY'
    if (os.geteuid() == 0 or release.get('ID') != 'linuxmint' or not os.environ.get(display)
            or os.environ.get('COR_LINUX_GUEST') != 'true'):
        raise RuntimeError('The GUI checks require the disposable Mint desktop guest')
    report = Path.home()/f'gui-{backend}'
    report.mkdir(mode=0o700)
    os.environ['COR_GUI_REPORT'] = str(report)
    result = gui(backend, report, lambda expression: cinnamon_eval(output, expression),
                 force_wayland=backend == 'wayland', verify_proxy=verify_system_proxy)
    write_json(report/'gui.json', result)


if __name__ == '__main__':
    main()
