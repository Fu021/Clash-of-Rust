"""Installed GUI checks using X11 and a real nested/headless GNOME Wayland shell.

Runs once per installed distribution. Authorization uses a disposable identity,
the production polkit action and a real graphical authentication agent.
"""
import argparse
import ast
import contextlib
import json
import os
from pathlib import Path
import secrets
import signal
import subprocess
import sys
import tempfile
import time

from build_support import ROOT, run, write_json

APP = Path('/opt/clash-of-rust')
REPORT = ROOT/'target/linux-report'
ACTION = 'org.clashofrust.tun.authorize'


def wait_for(description, probe, timeout=45):
    deadline = time.monotonic()+timeout
    while time.monotonic() < deadline:
        result = probe()
        if result:
            return result
        time.sleep(.2)
    raise TimeoutError(description)


def output(command, **kwargs):
    return subprocess.check_output(list(map(str, command)), text=True, timeout=15, **kwargs).strip()


def dbus(destination, path, method, *arguments):
    return output(['gdbus', 'call', '--session', '--dest', destination,
                   '--object-path', path, '--method', method, *arguments])


def evaluate(script):
    result = dbus('org.gnome.Shell', '/org/gnome/Shell', 'org.gnome.Shell.Eval', script)
    ok, value = ast.literal_eval(result.replace('(true,', '(True,').replace('(false,', '(False,'))
    if not ok:
        raise RuntimeError('GNOME rejected window inspection: '+value)
    return json.loads(value) if value else None


@contextlib.contextmanager
def process(command, log, env=None):
    child = subprocess.Popen(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT,
                             env=env, start_new_session=True)
    try:
        yield child
    finally:
        with contextlib.suppress(ProcessLookupError):
            os.killpg(child.pid, signal.SIGTERM)
        try:
            child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(child.pid, signal.SIGKILL)
            child.wait(timeout=5)


def authentication():
    helper = APP/'clash-of-rust'
    launcher = APP/'clash-tun-launcher'
    dns_helper = APP/'libexec/resolvectl'
    before = (output(['getcap', launcher]), dns_helper.stat().st_mode & 0o7777)
    agent = '/usr/lib/policykit-1-gnome/polkit-gnome-authentication-agent-1'
    with (Path(os.environ['COR_GUI_REPORT'])/'authentication.log').open('w') as log:
        with process([agent], log) as registered:
            time.sleep(1)
            if registered.poll() is not None:
                raise RuntimeError('Graphical polkit agent did not register')
            for allow in (False, True):
                with process(['pkexec', '--disable-internal-agent', helper, '--authorize-tun'], log) as request:
                    def dialog():
                        found = subprocess.run(['xdotool', 'search', '--onlyvisible', '--name',
                                                'Authentication|Authenticate'],
                                               capture_output=True, text=True)
                        return found.stdout.splitlines()[0] if found.returncode == 0 and found.stdout.strip() else None
                    window = wait_for('Production authorization did not show a graphical dialog', dialog)
                    run(['xdotool', 'windowactivate', '--sync', window])
                    if allow:
                        run(['xdotool', 'type', '--clearmodifiers', '--delay', '10',
                             os.environ['COR_GUI_AUTH_PASSWORD']])
                        run(['xdotool', 'key', 'Return'])
                    else:
                        run(['xdotool', 'key', 'Escape'])
                    code = request.wait(timeout=30)
                    if allow:
                        if (code or 'cap_net_admin,cap_net_raw' not in output(['getcap', launcher])
                                or dns_helper.stat().st_mode & 0o7777 != 0o4755):
                            raise RuntimeError('Accepted graphical authorization did not grant the installed helper')
                    elif code == 0 or (output(['getcap', launcher]), dns_helper.stat().st_mode & 0o7777) != before:
                        raise RuntimeError('Cancelled graphical authorization changed network permissions')
    return {'cancel_preserved_permissions': True, 'accept_granted_helper': True,
            'production_action': ACTION, 'graphical_agent': True}


def notifier():
    try:
        value = json.loads(output(['busctl', '--user', '--json=short', 'get-property',
                                  'org.kde.StatusNotifierWatcher', '/StatusNotifierWatcher',
                                  'org.kde.StatusNotifierWatcher', 'RegisteredStatusNotifierItems']))
        entries = value['data']
        return entries[0] if len(entries) == 1 and isinstance(entries[0], list) else entries
    except (subprocess.SubprocessError, ValueError, KeyError):
        return []


def window_script():
    return ('JSON.stringify(global.get_window_actors().map(a=>a.meta_window)'
            '.filter(w=>w.get_title().indexOf("Clash of Rust")>=0)'
            '.map(w=>{let r=w.get_frame_rect(); return {pid:w.get_pid(),'
            'title:w.get_title(),width:r.width,height:r.height};}))')


def inspect(backend):
    if backend == 'wayland':
        return evaluate(window_script())
    tree = output(['xwininfo', '-root', '-tree'])
    import re
    windows = re.findall(r'^\s*(0x[0-9a-fA-F]+)\s+"Clash of Rust', tree, re.M)
    result = []
    for window in windows:
        info = output(['xwininfo', '-id', window])
        if 'Map State: IsViewable' in info:
            result.append({'id': window, 'width': int(re.search(r'Width:\s+(\d+)', info)[1]),
                           'height': int(re.search(r'Height:\s+(\d+)', info)[1])})
    return result


def cli(*arguments):
    value = json.loads(output([APP/'cor', '--json', *arguments]))
    return value['data']


def core_pids():
    result = []
    for path in Path('/proc').glob('[0-9]*/exe'):
        try:
            if path.parent.stat().st_uid == os.getuid() and path.resolve().name == 'mihomo':
                result.append(int(path.parent.name))
        except (OSError, RuntimeError):
            pass
    return sorted(result)


def gui(backend, report):
    data = report/('profile-'+backend)
    data.mkdir()
    env = dict(os.environ, CLASH_OF_RUST_DATA_DIR=str(data), SLINT_BACKEND='winit-software')
    if backend == 'x11':
        env.pop('WAYLAND_DISPLAY', None)
        env.pop('WAYLAND_SOCKET', None)
    os.environ['CLASH_OF_RUST_DATA_DIR'] = str(data)
    fixture = report/'local.yaml'
    fixture.write_text("proxies: []\nproxy-groups: []\nrules: ['MATCH,DIRECT']\n")
    with (report/(backend+'.log')).open('w') as log, process([APP/'clash-of-rust'], log, env) as owner:
        def visible():
            if owner.poll() is not None:
                raise RuntimeError('Installed GUI owner exited')
            return inspect(backend)
        windows = wait_for('Installed '+backend+' GUI did not show a window', visible)
        if not any(window['width'] >= 800 and window['height'] >= 450 for window in windows):
            raise RuntimeError('Installed GUI window is smaller than the supported minimum')
        cli('profile', 'add', 'GUI fixture', str(fixture))
        cli('start')
        cli('mode', 'global')
        before = cli('status')
        pids = wait_for('GUI did not own a core process', core_pids)
        if len(pids) != 1:
            raise RuntimeError('GUI and CLI created duplicate core processes')
        def owner_tray():
            for entry in notifier():
                service, separator, suffix = entry.partition('/')
                try:
                    pid = ast.literal_eval(dbus('org.freedesktop.DBus', '/org/freedesktop/DBus',
                                               'org.freedesktop.DBus.GetConnectionUnixProcessID', service)
                                           .replace('uint32 ', ''))[0]
                except subprocess.SubprocessError:
                    continue
                if pid == owner.pid:
                    return service, '/'+suffix if separator else '/StatusNotifierItem'
            return None
        item = wait_for('GNOME did not register the GUI owner tray', owner_tray)
        # One graceful close and one renderer loss exercise different lifecycle
        # paths without repeating the proxy/TUN traffic suite.
        for crash in (False, True):
            renderers = [int(path.name) for path in Path('/proc').glob('[0-9]*')
                         if _renderer(path, owner.pid)]
            if len(renderers) != 1:
                raise RuntimeError('Expected exactly one UI renderer')
            if crash:
                os.kill(renderers[0], signal.SIGKILL)
            elif backend == 'wayland':
                evaluate('global.get_window_actors().map(a=>a.meta_window)'
                         '.filter(w=>w.get_title().indexOf("Clash of Rust")>=0)'
                         '.forEach(w=>w.delete(global.get_current_time())); ""')
            else:
                run(['wmctrl', '-ic', windows[0]['id']])
            wait_for('Closed renderer survived', lambda: not Path('/proc/'+str(renderers[0])).exists())
            if owner.poll() is not None:
                raise RuntimeError('Closing the window terminated its owner')
            dbus(item[0], item[1], 'org.kde.StatusNotifierItem.Activate', '0', '0')
            windows = wait_for('Tray activation did not reopen the GUI', visible)
            after = cli('status')
            if core_pids() != pids or after.get('mode') != before.get('mode'):
                raise RuntimeError('Window reconstruction changed the core or CLI mode')
        cli('stop')
        return {'windows': windows, 'tray': True, 'graceful_close': True, 'renderer_crash': True,
                'core_pid_preserved': True, 'cli_state_preserved': True}


def _renderer(path, owner):
    try:
        return (path.stat().st_uid == os.getuid()
                and (path/'exe').resolve().name == 'clash-desktop-ui'
                and int((path/'stat').read_text().rsplit(')', 1)[1].split()[1]) == owner)
    except (OSError, ValueError, RuntimeError):
        return False


def session():
    if os.geteuid() == 0:
        raise RuntimeError('GUI must run as an ordinary user')
    report = Path(os.environ['COR_GUI_REPORT'])
    result = {'status': 'running', 'checks': {}}
    try:
        with (report/'desktop.log').open('w') as log, contextlib.ExitStack() as stack:
            display = stack.enter_context(process(['Xvfb', ':91', '-screen', '0', '1280x900x24',
                                                   '-nolisten', 'tcp'], log))
            os.environ['DISPLAY'] = ':91'
            wait_for('X11 display did not start',
                     lambda: subprocess.run(['xdpyinfo'], stdout=subprocess.DEVNULL,
                                            stderr=subprocess.DEVNULL).returncode == 0)
            stack.enter_context(process(['openbox'], log))
            result['checks']['authorization'] = authentication()
            run(['gsettings', 'set', 'org.gnome.shell', 'enabled-extensions',
                 "['ubuntu-appindicators@ubuntu.com']"])
            help_text = output(['gnome-shell', '--help'])
            command = ['gnome-shell', '--wayland', '--wayland-display=cor-wayland']
            if '--headless' in help_text:
                command.extend(['--headless', '--virtual-monitor=1280x900'])
            else:
                command.append('--nested')
            if '--unsafe-mode' in help_text:
                command.append('--unsafe-mode')
            shell = stack.enter_context(process(command, log))
            os.environ['WAYLAND_DISPLAY'] = 'cor-wayland'
            def ready():
                if shell.poll() is not None or display.poll() is not None:
                    raise RuntimeError('Desktop compositor exited')
                try:
                    return evaluate('JSON.stringify(true)')
                except (subprocess.SubprocessError, ValueError, RuntimeError):
                    return False
            wait_for('GNOME Wayland compositor did not become ready', ready, timeout=60)
            for backend in ('x11', 'wayland'):
                result['checks'][backend] = gui(backend, report)
            result['status'] = 'passed'
    except BaseException as error:
        result['status'] = 'failed'
        result['error'] = str(error)
        raise
    finally:
        write_json(report/'gui.json', result)


def installed_tests():
    from linux_ci import require_actions
    require_actions()
    REPORT.mkdir(parents=True, exist_ok=True)
    user = 'corgui'+str(os.getpid())
    password = secrets.token_hex(12)
    with tempfile.TemporaryDirectory(prefix='cor-desktop-') as temporary:
        home = Path(temporary)
        home.chmod(0o755)
        created = False
        try:
            run(['sudo', 'useradd', '--create-home', '--groups', 'sudo',
                 '--home-dir', home/'home', '--shell', '/bin/bash', user])
            created = True
            run(['sudo', 'chpasswd'], input=user+':'+password+'\n', text=True)
            report = REPORT/'desktop'
            run(['sudo', 'install', '-d', '-m', '0755', '-o', user, '-g', user, report])
            runtime = home/'runtime'
            run(['sudo', 'install', '-d', '-m', '0700', '-o', user, '-g', user, runtime])
            env = {'HOME': str(home/'home'), 'PATH': os.environ['PATH'],
                   'XDG_RUNTIME_DIR': str(runtime), 'XDG_CONFIG_HOME': str(home/'home/.config'),
                   'XDG_CURRENT_DESKTOP': 'GNOME', 'GNOME_SHELL_SESSION_MODE': 'user',
                   'LIBGL_ALWAYS_SOFTWARE': '1', 'GSETTINGS_BACKEND': 'dconf',
                   'COR_GUI_REPORT': str(report), 'COR_GUI_AUTH_PASSWORD': password,
                   'PYTHONDONTWRITEBYTECODE': '1', 'LANG': 'C.UTF-8',
                   'GITHUB_ACTIONS': 'true', 'NO_AT_BRIDGE': '1'}
            # Clear the environment: neither the guest nor GUI sees Actions tokens.
            run(['sudo', '-u', user, 'env', '-i',
                 *(key+'='+value for key, value in env.items()),
                 'dbus-run-session', '--', sys.executable, __file__, '--session'], timeout=360)
        finally:
            if created:
                subprocess.run(['sudo', 'pkill', '-KILL', '-u', user], check=False)
                subprocess.run(['sudo', 'userdel', '--remove', user], check=False)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--session', action='store_true')
    if parser.parse_args().session:
        session()
    else:
        installed_tests()
