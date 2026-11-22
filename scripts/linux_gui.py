"""Installed GUI checks using X11 and a real nested/headless GNOME Wayland shell.

Runs once per installed distribution. Authorization uses a disposable identity,
the production polkit action and a real graphical authentication agent.
"""
import argparse
import ast
import contextlib
import csv
import errno
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import secrets
import shutil
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import time

from build_support import ROOT, run, sha256, write_json
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


def respond_authorization(window, password_file, allow):
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
            text=True, timeout=15, input=password, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        run(['xdotool', 'key', 'Return'], timeout=15)
    else:
        run(['xdotool', 'key', 'Escape'], timeout=15)


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
                            respond_authorization(window, password_file, allow)
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


def authentication(update_test=None):
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
            if window and update_test and output(['xdotool', 'getwindowpid', window]) != str(registered.pid):
                raise RuntimeError('Authentication dialog does not belong to the graphical agent')
            return window

        def restart():
            nonlocal registered
            processes.close()
            if agent_log:
                agent_log.seek(0)
                agent_log.truncate()
            registered = launch()

        password_file = Path(os.environ['COR_GUI_AUTH_PASSWORD_FILE'])
        if update_test:
            return update_test(locate, restart, password_file)
        return verify_authorization(log, password_file, locate,
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


def read_object(path):
    try:
        value = json.loads(path.read_text())
        return value if isinstance(value, dict) else {}
    except (FileNotFoundError, ValueError):
        return {}


def recognized_lines(tsv):
    lines = {}
    for row in csv.DictReader(tsv.splitlines(), delimiter='\t', quoting=csv.QUOTE_NONE):
        if row['level'] != '5' or not row['text'].strip():
            continue
        key = tuple(row[name] for name in ('page_num', 'block_num', 'par_num', 'line_num'))
        box = tuple(int(row[name]) for name in ('left', 'top', 'width', 'height'))
        lines.setdefault(key, []).append((row['text'], box))
    return list(lines.values())


def screen_label(lines, label, *, contains=False):
    def normalize(text):
        return ''.join(text.split()).replace('–', '-').replace('－', '-')

    wanted = normalize(label)
    matches = []
    for words in lines:
        text = normalize(''.join(word for word, _ in words))
        if contains:
            if wanted in text:
                return True
        elif wanted == text:
            left = min(box[0] for _, box in words)
            top = min(box[1] for _, box in words)
            right = max(box[0]+box[2] for _, box in words)
            bottom = max(box[1]+box[3] for _, box in words)
            matches.append(((left+right)//2, (top+bottom)//2))
    return min(matches, key=lambda point: point[1], default=None)


def update_screen(report, label, *, click=False, timeout=45, evidence=None):
    image = report/'update-screen.png'

    def locate():
        image.unlink(missing_ok=True)
        run(['scrot', image], timeout=15)
        result = subprocess.run(['tesseract', image, 'stdout', '--psm', '11', '-l', 'eng+chi_sim', 'tsv'],
                                capture_output=True, text=True, check=True, timeout=20)
        return screen_label(recognized_lines(result.stdout), label, contains=not click)

    point = wait_for(f'Graphical update did not display {label!r}', locate, timeout=timeout)
    if click:
        run(['xdotool', 'mousemove', *map(str, point), 'click', '1'], timeout=10)
    if evidence:
        shutil.copyfile(image, report/evidence)


def verified_gui_download(status, published, owner, work, data):
    directory = Path(status['directory'])
    if (directory.parent != work or not directory.name.startswith('clash-of-rust-update-')
            or directory.resolve(strict=True) != directory or directory.stat().st_uid != os.getuid()):
        raise RuntimeError('Unexpected graphical update staging directory')
    plan_path = directory/'plan.json'
    plan = read_object(plan_path) if owned_regular_file(plan_path) else {}
    package = directory/published['package']
    if (package.parent != directory
            or status['version'] != published['version'] or plan.get('version') != published['version']
            or plan.get('parent') != owner or plan.get('restart') is not True
            or plan.get('executable') != str(APP/'clash-of-rust')
            or plan.get('data_dir') != str(data)
            or plan.get('terminal') is not None or plan.get('package') != str(package)
            or plan.get('sha256') != published['sha256'] or not owned_regular_file(package)
            or sha256(package) != published['sha256']):
        raise RuntimeError('GUI handoff did not contain the published package downloaded by this window')
    return package


def owned_regular_file(path):
    info = path.lstat()
    return stat.S_ISREG(info.st_mode) and info.st_uid == os.getuid()


def installer_args(tool):
    return ['-U', '--noconfirm', '--'] if tool == '/usr/bin/pacman' else ['-i']


def pkexec_snapshot(request, tool):
    args = (request/'cmdline').read_bytes().split(b'\0')
    expected = [os.fsencode(arg) for arg in
                ['/usr/bin/pkexec', '--disable-internal-agent', tool, *installer_args(tool)]]
    if args[:-2] != expected or args[-1] != b'':
        return None
    match = re.fullmatch(r'/proc/([1-9][0-9]*)/fd/([0-9]+)', os.fsdecode(args[-2]))
    if not match:
        raise RuntimeError('pkexec did not receive a frozen package from the GUI installer')
    return match.groups()


def process_status(path):
    return dict(line.split(':', 1) for line in (path/'status').read_text().splitlines() if ':' in line)


def verify_gui_pkexec(package, tool, digest, proc=Path('/proc')):
    import fcntl

    requests = []
    for request in proc.glob('[0-9]*'):
        with contextlib.suppress(OSError):
            fields = process_status(request)
            if int(fields['Uid'].split()[0]) != os.getuid():
                continue
            snapshot = pkexec_snapshot(request, tool)
            if snapshot:
                requests.append((request, fields, snapshot))
    if len(requests) != 1:
        raise RuntimeError('Graphical authorization did not accompany the expected pkexec request')
    request, fields, (helper_pid, number) = requests[0]
    helper = proc/helper_pid
    directory = package.parent
    executable = directory/'update-helper'
    helper_args = [os.fsencode(executable), b'--install-update', os.fsencode(directory), b'']
    if (fields['PPid'].strip() != helper_pid or helper.stat().st_uid != os.getuid()
            or (helper/'cmdline').read_bytes().split(b'\0') != helper_args
            or not owned_regular_file(executable) or not (helper/'exe').samefile(executable)
            or sha256(executable) != sha256(APP/'clash-of-rust')):
        raise RuntimeError('pkexec is not a child of the staged GUI installer')
    required = fcntl.F_SEAL_WRITE | fcntl.F_SEAL_GROW | fcntl.F_SEAL_SHRINK | fcntl.F_SEAL_SEAL
    with (helper/'fd'/number).open('rb') as stream:
        if (os.readlink(f'/proc/self/fd/{stream.fileno()}') != '/memfd:cor-update (deleted)'
                or fcntl.fcntl(stream, fcntl.F_GET_SEALS) & required != required):
            raise RuntimeError('The GUI installer did not pass a sealed package snapshot')
        if hashlib.file_digest(stream, 'sha256').hexdigest() != digest:
            raise RuntimeError('The package handed to pkexec did not match the GUI download')
    return int(request.name)


def verify_installed_package(package_format, version):
    if package_format == 'pkg':
        if output(['pacman', '-Q', '--', 'clash-of-rust']) != f'clash-of-rust {version}':
            raise RuntimeError(f'Arch package version differs from {version}')
        audit = output(['pacman', '-Qk', '--quiet', '--', 'clash-of-rust'], stderr=subprocess.STDOUT)
        if audit:
            raise RuntimeError(f'Arch package files are incomplete: {audit}')
    else:
        status = output(['dpkg-query', '--show', '--showformat=${Status}\t${Version}', 'clash-of-rust'])
        audit = output(['dpkg', '--audit', 'clash-of-rust'], stderr=subprocess.STDOUT)
        if status != f'install ok installed\t{version}' or audit:
            raise RuntimeError(f'DEB is not fully installed at {version}: {status}; {audit}')
    for name in ('clash-of-rust', 'cor', 'clash-desktop-ui', 'resources/mihomo'):
        if not (APP/name).is_file() or not os.access(APP/name, os.X_OK):
            raise RuntimeError(f'Installed native package is missing executable {name}')


def stopped_gui_core(owner, *, startup):
    def running():
        status = cli('status')
        if status.get('owner') != 'gui':
            raise RuntimeError('The control endpoint no longer belongs to the GUI')
        return status.get('running')

    if startup:
        wait_for('GUI core startup did not complete',
                 lambda: running() is True and owner in core_processes().values(), timeout=90)
        cli('stop')

    stable_since = None

    def stopped():
        nonlocal stable_since
        if running() is not False or core_processes():
            stable_since = None
            return False
        now = time.monotonic()
        if stable_since is None:
            stable_since = now
        return now-stable_since >= 2

    wait_for('GUI-owned backend did not keep the core stopped', stopped, timeout=15)


def update_gui_environment(work):
    return dict(os.environ, SLINT_BACKEND='winit-software', TMPDIR=str(work),
                LC_ALL='', LC_MESSAGES='zh_CN.UTF-8', LANGUAGE='zh_CN')


def gui_update_session(report, case, checks, locate, restart_agent, password_file):
    published = case['published']
    tool = '/usr/bin/pacman' if case['format'] == 'pkg' else '/usr/bin/dpkg'
    data = report/'profile'
    data.mkdir(mode=0o700)
    work = report/'downloads'
    work.mkdir(mode=0o700)
    os.environ['CLASH_OF_RUST_DATA_DIR'] = str(data)
    env = update_gui_environment(work)
    write_json(data/'settings.json', {'language': 'zh-CN', 'update_prereleases': published['prerelease'],
                                    'update_check_interval_minutes': 0, 'proxy_mode': 'off',
                                    'mixed_port': available_mixed_port()})
    profile = report/'fixture.yaml'
    profile.write_text("proxies: []\nproxy-groups: []\nrules: ['MATCH,DIRECT']\n")

    def ready(original=None):
        def probe():
            owner.poll()
            control = read_object(data/'control.json')
            pid = control.get('pid')
            if control.get('owner') != 'gui' or type(pid) is not int or pid == original:
                return None
            for window in inspect('x11'):
                try:
                    renderer = output(['xdotool', 'getwindowpid', window['id']])
                    if _renderer(Path('/proc')/renderer, pid):
                        return pid, window['id']
                except (OSError, subprocess.SubprocessError):
                    continue
            return None

        return wait_for('GUI-owned backend and visible renderer did not become ready', probe, timeout=90)

    def verify_data(version):
        runtime = cli('status')
        saved = read_object(data/'settings.json')
        after = cli('profile', 'list', '--filter', 'GUI update fixture')
        if (settings.get('update_prereleases') is not published['prerelease']
                or settings.get('proxy_mode') != 'off'
                or runtime.get('owner') != 'gui' or runtime.get('running') is not False
                or runtime.get('version') != version or cli('version')['version'] != version
                or any(key not in saved or saved[key] != value for key, value in settings.items())
                or len(after) != 1 or after[0]['id'] != subscriptions[0]['id']
                or sha256(source) != digest):
            raise RuntimeError('Graphical update did not preserve the expected client, backend or fixture data')

    def request_observation(phase, pid=None):
        write_json(report/'update-observation-request.json', {'phase': phase, 'pid': pid})
        acknowledgement = report.parent/'update-observation-state.json'
        return wait_for(f'Runner did not verify the {phase} phase',
                        lambda: (result if (result := read_object(acknowledgement)).get('phase') == phase
                                 else None))

    status_file = data/'update-install.json'
    with (report/'update.log').open('w') as log, process([APP/'clash-of-rust'], log, env) as owner:
        pid, window = ready()
        stopped_gui_core(pid, startup=True)
        cli('profile', 'add', 'GUI update fixture', str(profile))
        subscriptions = cli('profile', 'list', '--filter', 'GUI update fixture')
        if len(subscriptions) != 1:
            raise RuntimeError('Could not seed a subscription in the GUI-owned backend')
        source = data/'profiles'/f'{subscriptions[0]["id"]}.yaml'
        settings, digest = read_object(data/'settings.json'), sha256(source)
        verify_data(case['previous_version'])
        last_directory = None
        for phase, failure in (('cancel', '授权已取消'), ('locked', '安装失败'), ('retry', None)):
            restart_agent()
            run(['xdotool', 'windowactivate', '--sync', window, 'key', 'Escape'], timeout=15)
            run(['wmctrl', '-ir', window, '-b', 'add,maximized_vert,maximized_horz'], timeout=15)
            update_screen(report, '设置', click=True)
            update_screen(report, '更新', click=True)
            update_screen(report, '检查更新', click=True)
            update_screen(report, published['version'], timeout=240,
                          evidence='update-discovered.png' if phase == 'cancel' else None)
            checks['version_visible_in_gui'] = True
            update_screen(report, '下载安装', click=True)
            if published['prerelease']:
                update_screen(report, '安装预发布版')
                update_screen(report, '继续安装', click=True)
            dialog = wait_for('GUI did not show a graphical installation authorization prompt',
                              locate, timeout=240)
            status = read_object(status_file)
            directory = status.get('directory')
            if not directory or directory == last_directory or status.get('state') != 'installing':
                raise RuntimeError('Authorization prompt has no new pending GUI installation')
            package = verified_gui_download(status, published, pid, work, data)
            request = verify_gui_pkexec(package, tool, published['sha256'])
            request_observation(phase, request)
            checks[phase] = {'download_sha256': published['sha256'], 'graphical_pkexec': tool,
                             'sealed_download': True}
            respond_authorization(dialog, password_file, phase != 'cancel')
            wait_for('Graphical update authorization dialog remained open',
                     lambda: gtk_authorization_window() is None, timeout=30)

            def outcome():
                owner.poll()
                result = read_object(status_file)
                if result.get('directory') == directory and result.get('state') in ('succeeded', 'failed'):
                    return result
                return None

            result = wait_for('No outcome for the graphical installation', outcome, timeout=180)
            last_directory = directory
            expected_state = 'failed' if failure else 'succeeded'
            if (result.get('state') != expected_state or result.get('error') is not bool(failure)
                    or (failure and failure not in result.get('message', ''))):
                raise RuntimeError(f'Unexpected graphical update outcome for {phase}: {result.get("message")}')
            previous_pid = pid
            if failure:
                update_screen(report, failure, timeout=30, evidence=f'update-{phase}.png')
                pid, window = ready()
            else:
                pid, window = ready(original=pid)
            native = case['old_native_version'] if failure else case['new_native_version']
            verify_installed_package(case['format'], native)
            reopened = pid != previous_pid
            stopped_gui_core(pid, startup=reopened)
            verify_data(case['previous_version'] if failure else published['version'])
            checks[phase].update(outcome=expected_state, gui_usable=True, gui_reopened=pid != previous_pid,
                                 installed_native_version=native, native_package_complete=True,
                                 settings_preserved=True, subscription_preserved=True,
                                 core_stopped=True, stopped_core_after_startup=reopened)
            if phase == 'locked':
                checks[phase]['native_lock_conflict'] = request_observation('release')['lock_conflict']


def session():
    if os.geteuid() == 0:
        raise RuntimeError('GUI must run as an ordinary user')
    report = Path(os.environ['COR_GUI_REPORT'])
    result = {'status': 'running', 'checks': {}}
    try:
        require_logind_session()
        result['checks']['login_session'] = True
        with (report/'desktop.log').open('w') as log, contextlib.ExitStack() as stack:
            update_case = read_object(report.parent/'published-update-case.json')
            os.environ['DISPLAY'] = ':91'
            command = ['Xvfb', ':91', '-screen', '0', '1280x900x24', '-nolisten', 'tcp', '-noreset']
            if update_case:
                authority = report/'Xauthority'
                authority.touch(mode=0o600, exist_ok=False)
                os.environ['XAUTHORITY'] = str(authority)
                run(['xauth', 'source', '-'], input=f'add :91 . {secrets.token_hex(16)}\n', text=True)
                command += ['-auth', authority]
            display = stack.enter_context(process(command, log))
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
            if update_case:
                authentication(lambda *agent: gui_update_session(report, update_case, result['checks'], *agent))
                result['status'] = 'passed'
                return
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


def preserve_report(source, *, update_case=False):
    destination = REPORT/('desktop-update' if update_case else 'desktop')
    destination.mkdir(parents=True, exist_ok=True)
    for name in ('gui.json', 'authentication.log', 'desktop.log', 'x11.log', 'wayland.log',
                 'dbus-probes.log', 'update.log'):
        path = source/name
        if path.is_file() and not path.is_symlink():
            shutil.copyfile(path, destination/name)
    if update_case:
        for name in ('update-discovered.png', 'update-cancel.png', 'update-locked.png'):
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


@contextlib.contextmanager
def fixture_signals():
    interruption = None

    def remember_interruption(number, _frame):
        nonlocal interruption
        if interruption is None:
            interruption = KeyboardInterrupt(f'Interrupted by {signal.Signals(number).name}')

    def check_interrupted():
        if interruption is not None:
            raise interruption

    handlers = {number: signal.signal(number, remember_interruption)
                for number in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)}
    try:
        yield check_interrupted
    except BaseException as error:
        if interruption is not None and error is not interruption:
            error.add_note(str(interruption))
        raise
    else:
        check_interrupted()
    finally:
        for number, handler in handlers.items():
            signal.signal(number, handler)


def cleanup_steps(*steps):
    primary = sys.exc_info()[1]
    failures = []
    for name, step in steps:
        try:
            step()
        except BaseException as error:
            failures.append(f'{name}: {error}')
    if failures:
        message = 'Fixture cleanup failed: ' + ' | '.join(failures)
        if primary is None:
            raise RuntimeError(message)
        primary.add_note(message)
        print(message, file=sys.stderr, flush=True)


def stop_desktop_unit(unit):
    result = subprocess.run(['sudo', '-n', '/usr/bin/timeout', '--kill-after=5s', '25s',
                             'systemctl', 'stop', unit], timeout=35)
    if result.returncode:
        state = subprocess.run(['systemctl', 'show', '--property=LoadState', '--property=ActiveState', unit],
                               capture_output=True, text=True, timeout=10)
        if set(state.stdout.splitlines()) != {'LoadState=not-found', 'ActiveState=inactive'}:
            result.check_returncode()


def stop_child(child):
    if child.poll() is None:
        child.terminate()
        try:
            child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait(timeout=10)


def verify_runner_pkexec(pid, uid, tool, proc=Path('/proc')):
    if type(pid) is not int or pid <= 1:
        raise RuntimeError('Invalid graphical authorization PID')
    request = proc/str(pid)
    fields = process_status(request)
    snapshot = pkexec_snapshot(request, tool)
    if (int(fields['Uid'].split()[0]) != uid or not snapshot or fields['PPid'].strip() != snapshot[0]
            or (proc/snapshot[0]).stat().st_uid != uid
            or output(['sudo', '-n', '/usr/bin/readlink', '--', request/'exe']) != '/usr/bin/pkexec'):
        raise RuntimeError('The authorization request is not the native pkexec child of the GUI installer')
    return f'/proc/{snapshot[0]}/fd/{snapshot[1]}'


def native_lock_conflict(trace, tool, snapshot):
    arguments = ', '.join(json.dumps(arg) for arg in (tool, *installer_args(tool), snapshot))
    started = False
    conflict = None
    for line in trace.splitlines():
        if line.startswith(f'execve("{tool}", [{arguments}],') and re.search(r'=\s+0$', line):
            started = True
        elif started and tool == '/usr/bin/pacman':
            if (re.match(r'^(open|openat|creat)\(', line) and '"/var/lib/pacman/db.lck"' in line
                    and 'O_EXCL' in line and re.search(r'=\s+-1 EEXIST\b', line)):
                conflict = 'pacman db.lck EEXIST'
        elif started and re.match(r'^fcntl(?:64)?\(\d+</var/lib/dpkg/lock-frontend>, F_SETLK,', line):
            if re.search(r'=\s+-1 (EAGAIN|EACCES)\b', line):
                conflict = 'dpkg lock-frontend F_SETLK denied'
        if started and conflict and re.match(r'^exit_group\([1-9]\d*\)', line):
            return conflict
    raise RuntimeError('Native installer did not exit after the expected database lock conflict')


@contextlib.contextmanager
def trace_installer(pid, log):
    command = ['sudo', '-n', '/usr/bin/timeout', '--kill-after=5s', '120s',
               '/usr/bin/strace', '-qq', '-y', '-s', '512',
               '-e', 'trace=execve,open,openat,creat,fcntl,exit_group', '-p', str(pid)]
    child = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
    try:
        def attached():
            if child.poll() is not None:
                raise RuntimeError('Native installer syscall observer exited before attaching')
            return int(process_status(Path('/proc')/str(pid))['TracerPid']) != 0

        wait_for('Native installer syscall observer did not attach', attached, timeout=10)
        yield child
    finally:
        cleanup_steps(('stop native installer syscall observer', lambda: stop_child(child)))


def read_observation_request(path, uid):
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    except FileNotFoundError:
        return {}
    with os.fdopen(descriptor, 'rb') as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_uid != uid:
            raise RuntimeError('Graphical observation request is not owned by the disposable GUI user')
        try:
            request = json.loads(stream.read(1024))
        except ValueError:
            return {}
    return request if isinstance(request, dict) else {}


def run_update_desktop(command, home, unit, package_lock, package_format, uid, user, *, check_interrupted):
    request = home/'report'/'update-observation-request.json'
    acknowledgement = home/'update-observation-state.json'
    tool = '/usr/bin/pacman' if package_format == 'pkg' else '/usr/bin/dpkg'
    phases = iter(('cancel', 'locked', 'release', 'retry'))
    expected = next(phases)
    state = None
    with contextlib.ExitStack() as lock, (home/'native-lock.log').open('w+') as log:
        child = subprocess.Popen(command, cwd=home)
        try:
            deadline = time.monotonic()+1050
            while child.poll() is None and time.monotonic() < deadline:
                check_interrupted()
                message = read_observation_request(request, uid)
                phase = message.get('phase')
                if phase is not None and phase != state:
                    if phase != expected:
                        raise RuntimeError(f'Unexpected graphical update observation phase {phase}')
                    response = {'phase': phase}
                    if phase == 'release':
                        observer.wait(timeout=15)
                        log.seek(0)
                        response['lock_conflict'] = native_lock_conflict(log.read(), tool, snapshot)
                        lock.close()
                    else:
                        pid = message.get('pid')
                        snapshot = verify_runner_pkexec(pid, uid, tool)
                        if phase == 'locked':
                            lock.enter_context(package_lock())
                            observer = lock.enter_context(trace_installer(pid, log))
                    check_interrupted()
                    state, expected = phase, next(phases, None)
                    write_json(acknowledgement, response)
                    acknowledgement.chmod(0o644)
                time.sleep(.2)
            if child.returncode is None:
                raise subprocess.TimeoutExpired(command, 1050)
            if child.returncode:
                raise subprocess.CalledProcessError(child.returncode, command)
            if expected is not None:
                raise RuntimeError('Graphical session did not complete the native installation observations')
        finally:
            if sys.exc_info()[1] is not None:
                cleanup_steps(('stop desktop client', lambda: stop_child(child)),
                              ('stop desktop session unit', lambda: stop_desktop_unit(unit)),
                              ('terminate GUI login session', lambda: stop_gui_user(user)),
                              ('kill remaining GUI user processes', lambda: kill_gui_user_processes(user)))


def remove_gui_rule(rule):
    subprocess.run(['sudo', 'rm', '-f', rule], check=False).check_returncode()


def stop_gui_user(user):
    result = run(['loginctl', 'list-users', '--no-legend', '--no-pager'],
                 capture_output=True, text=True, timeout=15)
    if any(row.split()[1:2] == [user] for row in (result.stdout or '').splitlines()):
        run(['sudo', 'loginctl', 'terminate-user', user], timeout=30)


def kill_gui_user_processes(user):
    for identity in ('-U', '-u'):
        result = subprocess.run(['sudo', '-n', 'pkill', '-KILL', identity, user], timeout=15)
        if result.returncode not in (0, 1):
            result.check_returncode()


def delete_gui_user(user, created, user_home):
    import pwd

    if not created:
        try:
            account = pwd.getpwnam(user)
        except KeyError:
            return
        if Path(account.pw_dir) != user_home:
            raise RuntimeError('Refusing to delete an account outside the disposable fixture')
    subprocess.run(['sudo', 'userdel', '--remove', user], check=False).check_returncode()


def installed_tests(*, update_case=None, package_lock=None):
    from linux_ci import require_actions
    require_actions()
    if update_case and (not package_lock or not (
            os.environ.get('COR_LINUX_GUEST') == 'true'
            or os.environ.get('RUNNER_ENVIRONMENT') == 'github-hosted')):
        raise RuntimeError('Graphical update requires a disposable runner or compatibility VM')
    REPORT.mkdir(parents=True, exist_ok=True)
    user = 'corgui'+str(os.getpid())
    password = secrets.token_hex(12)
    with fixture_signals() as check_interrupted:
        temporary = tempfile.TemporaryDirectory(prefix='cor-desktop-')
        home = Path(temporary.name)
        report = home/'report'
        created = False
        user_requested = False
        rule = None
        unit = None
        try:
            home.chmod(0o755)
            script = stage_session(home)
            from linux_native_guest import DISTROS
            native = platform.freedesktop_os_release().get('ID') in DISTROS
            groups = [] if native or update_case else ['--groups', 'sudo']
            check_interrupted()
            user_requested = True
            run(['sudo', 'useradd', '--create-home', *groups,
                 '--home-dir', home/'home', '--shell', '/bin/bash', user])
            created = True
            check_interrupted()
            run(['sudo', 'chpasswd'], input=user+':'+password+'\n', text=True)
            if update_case or native:
                from linux_native_guest import distribution
                if native:
                    distribution()
                rule = Path('/etc/polkit-1/rules.d')/f'10-{user}.rules'
                action = ACTION
                extra = ''
                if update_case:
                    action = 'org.freedesktop.policykit.exec'
                    program = {'deb': '/usr/bin/dpkg', 'pkg': '/usr/bin/pacman'}[update_case['format']]
                    extra = f' && action.lookup("program") === "{program}"'
                    case_file = home/'published-update-case.json'
                    write_json(case_file, update_case)
                    case_file.chmod(0o644)
                contents = ('polkit.addAdminRule(function(action, subject) {'
                            f' if (action.id === "{action}" && subject.user === "{user}"{extra})'
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
            command = ['sudo', 'systemd-run', '--unit='+unit, '--wait', '--collect',
                       '--quiet', '--property=User='+user, '--property=PAMName=login',
                       '--property=TimeoutStopSec=15s',
                       '--property=StandardInput=null', '--property=StandardOutput=journal',
                       '--property=StandardError=journal', '--property=WorkingDirectory='+str(home),
                       *('--setenv='+key+'='+value for key, value in env.items()),
                       sys.executable, '-E', '-s', '-B', script, '--session-launch']
            try:
                check_interrupted()
                if update_case:
                    run_update_desktop(command, home, unit, package_lock,
                                       update_case['format'], report.stat().st_uid, user,
                                       check_interrupted=check_interrupted)
                else:
                    subprocess.run(command, check=True, cwd=home, timeout=360)
                check_interrupted()
            except subprocess.SubprocessError:
                journal = 'desktop-update-session-journal.log' if update_case else 'desktop-session-journal.log'
                with (REPORT/journal).open('w') as log:
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
            steps = []
            if rule:
                steps.append(('remove disposable polkit rule', lambda: remove_gui_rule(rule)))
            if unit:
                steps.append(('stop desktop session unit', lambda: stop_desktop_unit(unit)))
            if user_requested:
                if created:
                    steps.extend((('terminate GUI login session', lambda: stop_gui_user(user)),
                                  ('kill remaining GUI user processes', lambda: kill_gui_user_processes(user))))
                steps.append(('delete disposable GUI user', lambda: delete_gui_user(user, created, home/'home')))
            steps.append(('recover temporary desktop files', lambda: run(
                ['sudo', 'chown', '-R', '-h', f'{os.getuid()}:{os.getgid()}', home], timeout=30)))
            if report.is_dir():
                steps.append(('preserve desktop report', lambda: preserve_report(
                    report, update_case=bool(update_case))))
            if (home/'native-lock.log').is_file():
                steps.append(('preserve native lock evidence', lambda: shutil.copyfile(
                    home/'native-lock.log', REPORT/'desktop-update-native-lock.log')))
            steps.append(('remove desktop fixture directory', temporary.cleanup))
            cleanup_steps(*steps)


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
