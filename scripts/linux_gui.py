"""Installed GUI checks using X11 and a real nested/headless GNOME Wayland shell.

Runs once per installed distribution. Authorization uses a disposable identity,
the production polkit action and a real graphical authentication agent.
"""
import argparse
import ast
import contextlib
import errno
import json
import os
from pathlib import Path
import platform
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time

from build_support import ROOT, run, write_json
from cli_tui import Session

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
    try:
        return output(['gdbus', 'call', '--session', '--dest', destination,
                       '--object-path', path, '--method', method, *arguments],
                      stderr=subprocess.PIPE)
    except subprocess.CalledProcessError as error:
        directory = Path(os.environ.get('COR_GUI_REPORT', REPORT))
        directory.mkdir(parents=True, exist_ok=True)
        with (directory/'dbus-probes.log').open('a') as log:
            log.write(f'{time.monotonic():.3f} {destination} {method}: {error.stderr or error}\n')
        raise


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


def authorization_dialog(request, request_log, locate):
    code = request.poll()
    if code is not None:
        request_log.flush()
        request_log.seek(0)
        text = request_log.read()
        if 'No authentication agent found' in text:
            return 'retry', None
        raise RuntimeError(f'Authorization request exited before the dialog: {code}: {text.strip()}')
    window = locate()
    return ('dialog', window) if window is not None else None


def gtk_authorization_window():
    found = subprocess.run(['xdotool', 'search', '--onlyvisible', '--name', 'Authentication|Authenticate'],
                           capture_output=True, text=True, timeout=10)
    if found.returncode == 0 and found.stdout.strip():
        return found.stdout.splitlines()[0]
    return None


def tun_permissions():
    return (output(['getcap', APP/'clash-tun-launcher']), (APP/'libexec/resolvectl').stat().st_mode & 0o7777)


def verify_authorization(log, password_file, locate, *, respond=None, restart_agent=None):
    helper = APP/'clash-of-rust'
    before = tun_permissions()
    for allow in (False, True):
        phase = 'accept' if allow else 'cancel'
        deadline = time.monotonic()+90
        while True:
            remaining = deadline-time.monotonic()
            if remaining <= 0:
                raise TimeoutError('Production authorization did not show a graphical dialog')
            with tempfile.TemporaryFile(mode='w+t', encoding='utf-8') as request_log:
                try:
                    with process(['pkexec', '--disable-internal-agent', helper, '--authorize-tun'],
                                 request_log) as request:
                        state, window = wait_for(
                            'Production authorization did not show a graphical dialog',
                            lambda: authorization_dialog(request, request_log, locate), timeout=remaining)
                        if state == 'retry':
                            time.sleep(1)
                            continue
                        log.write(f'Graphical authorization {phase}: prompt received\n')
                        log.flush()
                        if respond:
                            respond(allow)
                        else:
                            if window:
                                run(['xdotool', 'windowactivate', '--sync', window], timeout=15)
                            if allow:
                                if window:
                                    run(['xdotool', 'windowfocus', '--sync', window], timeout=15)
                                password = password_file.read_text()
                                if not password.isascii() or not password.isalnum():
                                    raise ValueError('Invalid disposable account password')
                                time.sleep(.3)
                                run(['xdotool', 'type', '--clearmodifiers', '--delay', '10', '--file', '-'],
                                    text=True, timeout=15, input=password,
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                                run(['xdotool', 'key', 'Return'], timeout=15)
                            else:
                                run(['xdotool', 'key', 'Escape'], timeout=15)
                        try:
                            code = request.wait(timeout=100 if respond else 30)
                        except subprocess.TimeoutExpired as error:
                            raise TimeoutError(f'Graphical authorization {phase} did not complete') from error
                        capabilities, mode = tun_permissions()
                        if allow:
                            if code or 'cap_net_admin,cap_net_raw' not in capabilities or mode != 0o4755:
                                raise RuntimeError(
                                    'Accepted graphical authorization did not grant the installed helper '
                                    f'(exit {code}, capabilities={capabilities!r}, DNS helper mode={mode:#o})')
                        elif code == 0 or (capabilities, mode) != before:
                            raise RuntimeError('Cancelled graphical authorization changed network permissions')
                        if not respond:
                            wait_for(f'Graphical authorization {phase} dialog remained visible',
                                     lambda: locate() is None, timeout=15)
                        break
                finally:
                    request_log.seek(0)
                    log.write(request_log.read())
                    log.flush()
        if not allow and restart_agent:
            restart_agent()
    return {'cancel_preserved_permissions': True, 'accept_granted_helper': True,
            'production_action': ACTION, 'graphical_agent': True}


def authentication():
    agent = '/usr/lib/policykit-1-gnome/polkit-gnome-authentication-agent-1'
    distro = platform.freedesktop_os_release().get('ID')
    xfce = distro == 'fedora'
    mate = distro in ('debian', 'opensuse-leap', 'opensuse-tumbleweed', 'arch')
    if xfce:
        agent = '/usr/libexec/xfce-polkit'
    elif mate:
        candidates = (Path('/usr/libexec/polkit-mate-authentication-agent-1'),
                      Path('/usr/lib/polkit-mate/polkit-mate-authentication-agent-1'),
                      Path('/usr/lib/mate-polkit/polkit-mate-authentication-agent-1'))
        agent = next((path for path in candidates if path.is_file()), candidates[0])
    prompt_debug, prompt_marker = {}, None
    if xfce:
        prompt_debug, prompt_marker = {'G_MESSAGES_DEBUG': 'all'}, b'\nVisibility: 0\n'
    elif mate:
        prompt_debug, prompt_marker = {'POLKIT_DEBUG': '1'}, b"read `PAM_PROMPT_ECHO_OFF "
    with (Path(os.environ['COR_GUI_REPORT'])/'authentication.log').open('w') as log, \
            (tempfile.TemporaryFile() if prompt_debug else contextlib.nullcontext()) as agent_log, \
            contextlib.ExitStack() as processes:
        print(f'Graphical authorization agent: {agent}', file=log, flush=True)

        def launch():
            if prompt_debug:
                return processes.enter_context(process([agent], agent_log,
                                                       env={**os.environ, **prompt_debug}))
            return processes.enter_context(process([agent], log))

        registered = launch()

        def locate():
            if registered.poll() is not None:
                raise RuntimeError('Graphical polkit agent exited before authorization')
            window = gtk_authorization_window()
            if window and prompt_marker and prompt_marker not in os.pread(agent_log.fileno(), 65536, 0):
                return None
            return window

        def restart():
            nonlocal registered
            processes.close()
            if agent_log:
                agent_log.seek(0)
                agent_log.truncate()
            registered = launch()

        return verify_authorization(log, Path(os.environ['COR_GUI_AUTH_PASSWORD_FILE']), locate,
                                    restart_agent=restart if mate or xfce else None)


def notifier_host_ready():
    try:
        return dbus('org.kde.StatusNotifierWatcher', '/StatusNotifierWatcher',
                    'org.freedesktop.DBus.Properties.Get', 'org.kde.StatusNotifierWatcher',
                    'IsStatusNotifierHostRegistered') == '(<true>,)'
    except subprocess.SubprocessError:
        return False


def notifier(session_output=output):
    try:
        value = json.loads(session_output(['busctl', '--user', '--json=short', 'get-property',
                                           'org.kde.StatusNotifierWatcher', '/StatusNotifierWatcher',
                                           'org.kde.StatusNotifierWatcher', 'RegisteredStatusNotifierItems']))
        entries = value['data']
        return entries[0] if len(entries) == 1 and isinstance(entries[0], list) else entries
    except (subprocess.SubprocessError, ValueError, KeyError):
        return []


def owner_tray(pid, session_output=output):
    for entry in notifier(session_output):
        service, separator, suffix = entry.partition('/')
        try:
            response = session_output(['gdbus', 'call', '--session', '--dest', 'org.freedesktop.DBus',
                                       '--object-path', '/org/freedesktop/DBus', '--method',
                                       'org.freedesktop.DBus.GetConnectionUnixProcessID', service])
            owner = ast.literal_eval(response.replace('uint32 ', ''))[0]
        except subprocess.SubprocessError:
            continue
        if owner == pid:
            return service, '/'+suffix if separator else '/StatusNotifierItem'
    return None


def ready_tray(data, pid):
    try:
        trace = json.loads((data/'desktop-startup.json').read_text())
        if (trace['schema'] == 1 and trace['kind'] == 'desktop_startup' and trace['pid'] == pid
                and trace['status']['tray_state'] == 'created'):
            return owner_tray(pid)
    except (OSError, ValueError, TypeError, KeyError):
        pass
    return None


def activate_tray(item):
    try:
        dbus(item[0], item[1], 'org.kde.StatusNotifierItem.Activate', '0', '0')
    except subprocess.CalledProcessError:
        return False
    return True


def window_script():
    return ('global.get_window_actors().map(a=>a.meta_window)'
            '.filter(w=>w && (w.get_title()||"").indexOf("Clash of Rust")>=0)'
            '.map(w=>{let r=w.get_frame_rect(); return {pid:w.get_pid(),'
            'sequence:w.get_stable_sequence(),'
            'wayland:w.get_client_type()===imports.gi.Meta.WindowClientType.WAYLAND,'
            'title:w.get_title(),width:r.width,height:r.height};})')


def select_wayland_windows(windows, renderers):
    native = [window for window in windows if window['wayland']]
    owned = [window for window in native if window['pid'] in renderers]
    if owned:
        return owned
    # GNOME 3.36 exposes get_pid(), but it does not return a Wayland client
    # PID. This session is isolated: accept only one window and one renderer.
    if (len(windows) == len(native) == len(renderers) == 1
            and native[0]['pid'] <= 0):
        return native
    return []


def inspect(backend, evaluate_window=evaluate):
    if backend == 'wayland':
        return evaluate_window(window_script())
    tree = output(['xwininfo', '-root', '-tree'])
    import re
    windows = re.findall(r'^\s*(0x[0-9a-fA-F]+)\s+"(Clash of Rust[^"]*)"', tree, re.M)
    result = []
    for window, title in windows:
        info = output(['xwininfo', '-id', window])
        if 'Map State: IsViewable' in info:
            result.append({'id': window, 'title': title,
                           'width': int(re.search(r'Width:\s+(\d+)', info)[1]),
                           'height': int(re.search(r'Height:\s+(\d+)', info)[1])})
    return result


def wait_for_usable_window(backend, probe, timeout=45):
    last_windows = []

    def ready():
        nonlocal last_windows
        windows = probe()
        if windows:
            last_windows = windows
        if any(window['width'] >= 800 and window['height'] >= 450 for window in windows):
            return windows
        return None

    try:
        return wait_for('Installed '+backend+' GUI did not reach its minimum size', ready, timeout)
    except TimeoutError as error:
        raise RuntimeError(
            f'Installed {backend} GUI did not reach 800x450; last windows: {last_windows}'
        ) from error


def verify_x11_minimize(window):
    target = window['id']

    def minimized():
        return '_NET_WM_STATE_HIDDEN' in output(['xprop', '-id', target, '_NET_WM_STATE'])

    run(['xdotool', 'windowactivate', '--sync', target], timeout=5)
    run(['xdotool', 'mousemove', '--window', target, str(window['width']-112),
         '22', 'sleep', '.03', 'click', '1'], timeout=5)
    wait_for('X11 title-bar button did not minimize the window', minimized, timeout=5)
    run(['xdotool', 'windowactivate', target], timeout=5)
    wait_for('The X11 window did not restore after minimizing', lambda: not minimized(), timeout=5)


def gnome_command(help_text, major):
    command = ['gnome-shell', '--wayland']
    if '--wayland-display' in help_text:
        command.append('--wayland-display=cor-wayland')
    if '--headless' in help_text:
        command.extend(['--headless', '--virtual-monitor=1280x900'])
    else:
        command.append('--nested')
    # Mutter hides this supported option from --help; GNOME 3.x uses
    # the development-tools setting for its equivalent test API.
    if major >= 41:
        command.append('--unsafe-mode')
    return command


def wayland_sockets(runtime):
    return {path.name for path in runtime.iterdir()
            if (path.name.startswith('wayland-') or path.name == 'cor-wayland') and path.is_socket()}


def cli(*arguments):
    value = json.loads(output([APP/'cor', '--json', *arguments]))
    return value['data']


def core_processes(proc=Path('/proc'), uid=None):
    uid = os.getuid() if uid is None else uid
    result = {}
    # Capability-bearing processes can have root-owned /proc entries and an
    # unreadable exe link even though their effective UID remains unprivileged.
    for path in proc.glob('[0-9]*/status'):
        try:
            fields = dict(line.split(':', 1) for line in path.read_text().splitlines() if ':' in line)
            identities = fields['Uid'].split()
            if (fields['Name'].strip() == 'mihomo'
                    and len(identities) >= 2
                    and int(identities[0]) == uid
                    and int(identities[1]) == uid):
                result[int(path.parent.name)] = int(fields['PPid'])
        except (OSError, ValueError, KeyError):
            pass
    return result


def available_mixed_port():
    for attempt in range(32):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as tcp, \
                socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
            tcp.bind(('127.0.0.1', 0))
            port = tcp.getsockname()[1]
            try:
                udp.bind(('127.0.0.1', port))
            except OSError as error:
                if error.errno != errno.EADDRINUSE or attempt == 31:
                    raise
                continue
            return port


def gui(backend, report, evaluate_window=evaluate, *, force_wayland=False, verify_proxy=None,
        inspect_window=None, close_window=None):
    data = report/('profile-'+backend)
    data.mkdir()
    env = dict(os.environ, CLASH_OF_RUST_DATA_DIR=str(data), SLINT_BACKEND='winit-software')
    if backend == 'x11':
        env.pop('WAYLAND_DISPLAY', None)
        env.pop('WAYLAND_SOCKET', None)
    elif force_wayland:
        env.pop('DISPLAY', None)
        env.pop('XAUTHORITY', None)
    os.environ['CLASH_OF_RUST_DATA_DIR'] = str(data)
    fixture = report/'local.yaml'
    fixture.write_text("proxies: []\nproxy-groups: []\nrules: ['MATCH,DIRECT']\n")
    mixed_port = available_mixed_port()
    write_json(data/'settings.json', {'mixed_port': mixed_port})
    with (report/(backend+'.log')).open('w') as log, process([APP/'clash-of-rust'], log, env) as owner:
        observed = []
        renderer_pids = []

        def visible():
            nonlocal observed, renderer_pids
            if owner.poll() is not None:
                raise RuntimeError(f'Installed {backend} GUI owner exited: {owner.returncode}')
            windows = inspect_window() if inspect_window else inspect(backend, evaluate_window)
            if backend == 'wayland':
                observed = windows
                renderer_pids = [int(path.name) for path in Path('/proc').glob('[0-9]*')
                                 if _renderer(path, owner.pid)]
                windows = select_wayland_windows(windows, renderer_pids)
            return windows
        try:
            windows = wait_for_usable_window(backend, visible)
        except RuntimeError as error:
            if backend == 'wayland':
                raise RuntimeError(f'{error}; compositor windows: {observed}; '
                                   f'renderer PIDs: {renderer_pids}; '
                                   f'WAYLAND_DISPLAY={env.get("WAYLAND_DISPLAY")}') from error
            raise
        if any(window['title'] != 'Clash of Rust' for window in windows):
            raise RuntimeError(f'Unexpected {backend} window title: {windows}')
        def gui_owner_ready():
            try:
                endpoint = json.loads((data/'control.json').read_text())
                return (isinstance(endpoint, dict)
                        and endpoint.get('owner') == 'gui' and endpoint.get('pid') == owner.pid)
            except (OSError, ValueError):
                return False
        # A visible window can precede asynchronous engine initialization.
        wait_for('GUI control endpoint did not become ready', gui_owner_ready)
        item = wait_for('GUI tray did not become ready and register with the desktop',
                        lambda: ready_tray(data, owner.pid))
        cli('profile', 'add', 'GUI fixture', str(fixture))
        cli('start')
        cli('mode', 'global')
        before = cli('status')
        processes = wait_for('GUI did not own a core process', core_processes)
        if (len(processes) != 1 or next(iter(processes.values())) != owner.pid
                or before.get('owner') != 'gui' or not before.get('running')):
            raise RuntimeError('GUI and CLI did not share exactly one GUI-owned core: '
                               f'GUI PID {owner.pid}, core processes {processes}, '
                               f'CLI owner {before.get("owner")}, running {before.get("running")}')
        proxy_result = {}
        if verify_proxy:
            proxy_result = verify_proxy(mixed_port) or {'desktop_system_proxy_restored': True}
        with Session([APP/'cor', 'ui', '--lang=zh-CN'], env=env) as terminal:
            terminal.wait('CLASH OF RUST')
            terminal.send(terminal.page_key('订阅'))
            terminal.wait('GUI fixture')
            if core_processes() != processes:
                raise RuntimeError('TUI did not share the existing GUI-owned core')
        if backend == 'x11':
            verify_x11_minimize(windows[0])
            proxy_result['title_bar_minimize'] = True
        # One graceful close and one renderer loss exercise different lifecycle
        # paths without repeating the proxy/TUN traffic suite.
        for crash in (False, True):
            renderers = [int(path.name) for path in Path('/proc').glob('[0-9]*')
                         if _renderer(path, owner.pid)]
            if len(renderers) != 1:
                raise RuntimeError('Expected exactly one UI renderer')
            if crash:
                os.kill(renderers[0], signal.SIGKILL)
            elif close_window:
                close_window()
            elif backend == 'wayland':
                evaluate_window('global.get_window_actors().map(a=>a.meta_window)'
                                f'.filter(w=>w && w.get_stable_sequence()==={windows[0]["sequence"]}'
                                ' && (w.get_title()||"").indexOf("Clash of Rust")>=0)'
                                '.forEach(w=>w.delete(global.get_current_time())); ""')
            else:
                run(['wmctrl', '-ic', windows[0]['id']])
            wait_for('Closed renderer survived', lambda: not Path('/proc/'+str(renderers[0])).exists())
            if owner.poll() is not None:
                raise RuntimeError(f'{backend} {"crash" if crash else "close"} terminated '
                                   f'the GUI owner: {owner.returncode}')
            wait_for(f'{backend} tray item {item} did not accept Activate', lambda: activate_tray(item),
                     timeout=15)
            windows = wait_for_usable_window(backend, visible)
            after = cli('status')
            if (core_processes() != processes or not after.get('running')
                    or after.get('owner') != 'gui' or after.get('mode') != before.get('mode')):
                raise RuntimeError('Window reconstruction changed the core or CLI mode')
        cli('stop')
        return {'windows': windows, 'tray': True, 'graceful_close': True, 'renderer_crash': True,
                'core_pid_preserved': True, 'cli_state_preserved': True,
                'tui_reads_gui_profiles': True, **proxy_result}


def _renderer(path, owner):
    try:
        return (path.stat().st_uid == os.getuid()
                and (path/'exe').resolve().name == 'clash-desktop-ui'
                and int((path/'stat').read_text().rsplit(')', 1)[1].split()[1]) == owner)
    except (OSError, ValueError, RuntimeError):
        return False


def require_logind_session():
    session_id = os.environ.get('XDG_SESSION_ID')
    manager = ['busctl', '--system', 'call', 'org.freedesktop.login1', '/org/freedesktop/login1',
               'org.freedesktop.login1.Manager']
    if (not session_id or not os.environ.get('XDG_RUNTIME_DIR')
            or output([*manager, 'GetSessionByPID', 'u', str(os.getpid())])
            != output([*manager, 'GetSession', 's', session_id])):
        raise RuntimeError('GUI user does not belong to the expected logind session')
    return session_id


def appindicator_extension():
    directory = Path('/usr/share/gnome-shell/extensions')
    for name in ('appindicatorsupport@rgcjonas.gmail.com', 'ubuntu-appindicators@ubuntu.com'):
        if (directory/name/'metadata.json').is_file():
            return name
    raise RuntimeError('The AppIndicator extension is not installed')


def session():
    if os.geteuid() == 0:
        raise RuntimeError('GUI must run as an ordinary user')
    report = Path(os.environ['COR_GUI_REPORT'])
    result = {'status': 'running', 'checks': {}}
    try:
        require_logind_session()
        result['checks']['login_session'] = True
        with (report/'desktop.log').open('w') as log, contextlib.ExitStack() as stack:
            display = stack.enter_context(process(['Xvfb', ':91', '-screen', '0', '1280x900x24',
                                                   '-nolisten', 'tcp', '-noreset'], log))
            os.environ['DISPLAY'] = ':91'
            wait_for('X11 display did not start',
                     lambda: subprocess.run(['xdpyinfo'], stdout=subprocess.DEVNULL,
                                            stderr=subprocess.DEVNULL).returncode == 0)
            activation = {name: os.environ[name] for name in
                          ('DBUS_SESSION_BUS_ADDRESS', 'DISPLAY', 'HOME', 'XDG_CONFIG_HOME',
                           'XDG_DATA_HOME', 'XDG_CACHE_HOME', 'XDG_CURRENT_DESKTOP')}
            dbus('org.freedesktop.DBus', '/org/freedesktop/DBus',
                 'org.freedesktop.DBus.UpdateActivationEnvironment', repr(activation))
            window_manager = stack.enter_context(process(['openbox'], log))

            def openbox_ready():
                if window_manager.poll() is not None or display.poll() is not None:
                    raise RuntimeError('Openbox or X11 exited before graphical authorization')
                return subprocess.run(['wmctrl', '-m'], timeout=5, stdout=subprocess.DEVNULL,
                                      stderr=subprocess.DEVNULL).returncode == 0

            wait_for('Openbox did not become ready', openbox_ready, timeout=15)
            result['checks']['authorization'] = authentication()
            extension = appindicator_extension()
            run(['gsettings', 'set', 'org.gnome.shell', 'enabled-extensions', f"['{extension}']"])
            if extension not in output(
                    ['gsettings', 'get', 'org.gnome.shell', 'enabled-extensions']):
                raise RuntimeError('GNOME test extension setting did not persist')
            import re
            version = re.search(r'\b(\d+)\.\d+', output(['gnome-shell', '--version']))
            if not version:
                raise RuntimeError('Could not determine the GNOME Shell version')
            major = int(version[1])
            if major < 41:
                run(['gsettings', 'set', 'org.gnome.shell', 'development-tools', 'true'])
                if output(['gsettings', 'get', 'org.gnome.shell', 'development-tools']) != 'true':
                    raise RuntimeError('GNOME development tools setting did not persist')
            help_text = output(['gnome-shell', '--help'])
            command = gnome_command(help_text, major)
            runtime = Path(os.environ['XDG_RUNTIME_DIR'])
            existing_sockets = wayland_sockets(runtime)
            shell = stack.enter_context(process(command, log))
            last_probe_error = ''

            def ready():
                nonlocal last_probe_error
                if shell.poll() is not None or display.poll() is not None:
                    raise RuntimeError('Desktop compositor exited')
                sockets = wayland_sockets(runtime)-existing_sockets
                if '--wayland-display=cor-wayland' in command:
                    sockets &= {'cor-wayland'}
                if len(sockets) > 1:
                    raise RuntimeError(f'GNOME created multiple Wayland sockets: {sorted(sockets)}')
                if not sockets:
                    return False
                try:
                    if evaluate('Main.layoutManager && Main.layoutManager._startingUp === false') is not True:
                        return False
                except (subprocess.SubprocessError, ValueError, RuntimeError) as error:
                    last_probe_error = getattr(error, 'stderr', None) or str(error)
                    return False
                os.environ['WAYLAND_DISPLAY'] = next(iter(sockets))
                return True

            arm_tcg = (os.environ.get('COR_LINUX_VM_ACCELERATION') == 'tcg'
                       and platform.machine().lower() in ('aarch64', 'arm64'))
            try:
                wait_for('GNOME Wayland compositor did not become ready', ready, timeout=120 if arm_tcg else 60)
            except TimeoutError as error:
                raise RuntimeError(f'{error}; last DBus probe: {last_probe_error or "no error"}') from error
            result['checks']['gnome_startup_complete'] = True
            wait_for('GNOME AppIndicator tray host did not become ready', notifier_host_ready, timeout=90)
            result['checks']['tray_host_ready'] = True
            from linux_native_guest import DISTROS
            native_package = platform.freedesktop_os_release().get('ID') in DISTROS
            proxy_check = None
            if native_package:
                from linux_mint_desktop import verify_system_proxy
                proxy_check = verify_system_proxy
            for backend in ('x11', 'wayland'):
                result['checks'][backend] = gui(backend, report, verify_proxy=proxy_check)
            result['status'] = 'passed'
    except BaseException as error:
        result['status'] = 'failed'
        result['error'] = str(error)
        raise
    finally:
        write_json(report/'gui.json', result)


def stage_session(directory):
    scripts = directory/'scripts'
    scripts.mkdir(mode=0o755)
    scripts.chmod(0o755)
    for name in ('linux_gui.py', 'linux_native_guest.py', 'linux_compatibility_targets.py', 'linux_mint_desktop.py',
                 'build_support.py', 'cli_tui.py'):
        target = scripts/name
        shutil.copyfile(Path(__file__).resolve().parent/name, target)
        target.chmod(0o644)
    return scripts/'linux_gui.py'


def preserve_report(source):
    destination = REPORT/'desktop'
    destination.mkdir(parents=True, exist_ok=True)
    for name in ('gui.json', 'authentication.log', 'desktop.log', 'x11.log', 'wayland.log',
                 'dbus-probes.log'):
        path = source/name
        if path.is_file() and not path.is_symlink():
            shutil.copyfile(path, destination/name)
    for backend in ('x11', 'wayland'):
        profile = source/f'profile-{backend}'
        startup = profile/'desktop-startup.json'
        if not profile.is_symlink() and startup.is_file() and not startup.is_symlink():
            shutil.copyfile(startup, destination/f'{backend}-startup.json')


def launch_session():
    # Retain PAM's session identity and the explicit test settings only. The
    # graphical processes must not inherit credentials from an Actions runner.
    allowed = ('HOME', 'USER', 'LOGNAME', 'PATH', 'SHELL', 'LANG', 'XDG_SESSION_ID',
               'XDG_RUNTIME_DIR', 'XDG_SEAT', 'XDG_VTNR', 'XDG_SESSION_TYPE',
               'XDG_SESSION_CLASS', 'XDG_CONFIG_HOME', 'XDG_CURRENT_DESKTOP',
               'GNOME_SHELL_SESSION_MODE', 'LIBGL_ALWAYS_SOFTWARE', 'GSETTINGS_BACKEND',
               'COR_GUI_REPORT', 'COR_GUI_AUTH_PASSWORD_FILE', 'COR_LINUX_VM_ACCELERATION', 'PYTHONDONTWRITEBYTECODE',
               'GITHUB_ACTIONS', 'NO_AT_BRIDGE')
    env = {key: os.environ[key] for key in allowed if key in os.environ}
    home = Path(env['COR_GUI_REPORT']).parent/'home'
    env.update(HOME=str(home), XDG_CONFIG_HOME=str(home/'.config'),
               XDG_DATA_HOME=str(home/'.local/share'), XDG_CACHE_HOME=str(home/'.cache'))
    command = ['/usr/bin/dbus-run-session', '--', sys.executable, '-E', '-s', '-B',
               str(Path(__file__).resolve()), '--session']
    try:
        os.execve(command[0], command, env)
    except OSError as error:
        write_json(Path(env['COR_GUI_REPORT'])/'gui.json',
                   {'status': 'failed', 'checks': {}, 'error': str(error)})
        raise


def installed_tests():
    from linux_ci import require_actions
    require_actions()
    REPORT.mkdir(parents=True, exist_ok=True)
    user = 'corgui'+str(os.getpid())
    password = secrets.token_hex(12)
    with tempfile.TemporaryDirectory(prefix='cor-desktop-') as temporary:
        home = Path(temporary)
        home.chmod(0o755)
        script = stage_session(home)
        report = home/'report'
        created = False
        rule = None
        try:
            from linux_native_guest import DISTROS
            native = platform.freedesktop_os_release().get('ID') in DISTROS
            groups = [] if native else ['--groups', 'sudo']
            run(['sudo', 'useradd', '--create-home', *groups,
                 '--home-dir', home/'home', '--shell', '/bin/bash', user])
            created = True
            run(['sudo', 'chpasswd'], input=user+':'+password+'\n', text=True)
            if native:
                from linux_native_guest import distribution
                distribution()
                rule = Path('/etc/polkit-1/rules.d')/f'10-{user}.rules'
                contents = ('polkit.addAdminRule(function(action, subject) {'
                            f' if (action.id === "{ACTION}" && subject.user === "{user}")'
                            f' return ["unix-user:{user}"];'
                            ' });\n')
                run(['sudo', 'tee', rule], input=contents, text=True, stdout=subprocess.DEVNULL)
            run(['sudo', 'install', '-d', '-m', '0755', '-o', user, '-g', user, report])
            password_file = home/'password'
            descriptor = os.open(password_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, 'w') as stream:
                stream.write(password)
            run(['sudo', 'chown', f'{user}:{user}', password_file])
            env = {'HOME': str(home/'home'), 'PATH': '/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',
                   'XDG_CONFIG_HOME': str(home/'home/.config'),
                   'XDG_CURRENT_DESKTOP': 'GNOME', 'GNOME_SHELL_SESSION_MODE': 'user',
                   'LIBGL_ALWAYS_SOFTWARE': '1', 'GSETTINGS_BACKEND': 'dconf',
                   'COR_GUI_REPORT': str(report), 'COR_GUI_AUTH_PASSWORD_FILE': str(password_file),
                   'COR_LINUX_VM_ACCELERATION': os.environ.get('COR_LINUX_VM_ACCELERATION', ''),
                   'PYTHONDONTWRITEBYTECODE': '1', 'LANG': 'C.UTF-8',
                   'GITHUB_ACTIONS': 'true', 'NO_AT_BRIDGE': '1'}
            # PAM registers a real logind session for the graphical polkit agent.
            unit = user+'-desktop.service'
            try:
                subprocess.run(['sudo', 'systemd-run', '--unit='+unit, '--wait', '--collect',
                                '--quiet', '--property=User='+user, '--property=PAMName=login',
                                '--property=StandardInput=null', '--property=StandardOutput=journal',
                                '--property=StandardError=journal',
                                '--property=WorkingDirectory='+str(home),
                                *('--setenv='+key+'='+value for key, value in env.items()),
                                sys.executable, '-E', '-s', '-B', script, '--session-launch'],
                               check=True, cwd=home, timeout=360)
            except subprocess.SubprocessError:
                with (REPORT/'desktop-session-journal.log').open('w') as log:
                    with contextlib.suppress(OSError, subprocess.SubprocessError):
                        subprocess.run(['sudo', 'journalctl', '--boot', '--no-pager', '-n', '80',
                                        '--unit', unit, '--unit', 'init.scope',
                                        '--unit', 'systemd-logind.service', '--unit', 'polkit.service',
                                        '--unit', 'polkit-agent-helper@*.service'],
                                       stdout=log, stderr=subprocess.STDOUT, timeout=15)
                    if platform.freedesktop_os_release().get('ID') == 'fedora':
                        with contextlib.suppress(OSError, subprocess.SubprocessError):
                            subprocess.run(['rpm', '-q', 'pam', 'pam-libs', 'liblastlog2',
                                            'polkit', 'polkit-libs', 'selinux-policy-targeted'],
                                           stdout=log, stderr=subprocess.STDOUT, timeout=15)
                        with contextlib.suppress(OSError, subprocess.SubprocessError):
                            subprocess.run(['sudo', 'journalctl', '--boot', '--no-pager', '-n', '40',
                                            '-g', 'avc:.*(polkit|chkpwd|shadow|faillock)'],
                                           stdout=log, stderr=subprocess.STDOUT, timeout=15)
                raise
        finally:
            if rule:
                subprocess.run(['sudo', 'rm', '-f', rule], check=False)
            if created:
                subprocess.run(['sudo', 'loginctl', 'terminate-user', user], check=False)
                subprocess.run(['sudo', 'pkill', '-KILL', '-u', user], check=False)
                subprocess.run(['sudo', 'userdel', '--remove', user], check=False)
            run(['sudo', 'chown', '-R', '-h', f'{os.getuid()}:{os.getgid()}', home])
            if report.is_dir():
                preserve_report(report)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--session', action='store_true')
    parser.add_argument('--session-launch', action='store_true')
    args = parser.parse_args()
    if args.session_launch:
        launch_session()
    elif args.session:
        session()
    else:
        installed_tests()
