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
from linux_update_retry import RATE_LIMIT_MESSAGES, ReleaseRateLimit, retry_release_check

APP = Path('/opt/clash-of-rust')
REPORT = ROOT/'target/linux-report'
ACTION = 'org.clashofrust.tun.authorize'
DESKTOP_TIMEOUT = 360
DESKTOP_UPDATE_TIMEOUT = 1050
PRERELEASE_CONFIRMATION_SECONDS = 3
UPDATE_FIXTURE_ACCENT = ('teal', '#c6613f')
UPDATE_LABELS = {
    'home': ('首页', 'Home'),
    'settings': ('设置', 'Settings'),
    'updates': ('更新', 'Updates'),
    'check': ('检查更新', 'Check for updates'),
    'download': ('下载安装', 'Download and install'),
    'prerelease': ('安装预发布版', 'Install pre-release'),
    'continue': ('继续安装', 'Continue'),
    'cancelled': ('授权已取消', 'Installation authorization cancelled'),
    'failed': ('安装失败', 'Installation failed'),
    'close': ('关闭', 'Close'),
}


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


class FixtureProcess(subprocess.Popen):
    """Observe exit without releasing the session leader PID before group cleanup."""

    def poll(self):
        if self.returncode is not None:
            return self.returncode
        status = os.waitid(os.P_PID, self.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
        if status is not None:
            return status.si_status if status.si_code == os.CLD_EXITED else -status.si_status

    def wait(self, timeout=None):
        deadline = None if timeout is None else time.monotonic()+timeout
        while (status := self.poll()) is None:
            if deadline is not None and time.monotonic() >= deadline:
                raise subprocess.TimeoutExpired(self.args, timeout)
            time.sleep(.02)
        return status

    def reap(self):
        return super().wait(timeout=5)


@contextlib.contextmanager
def process(command, log, env=None, *, reap_for_restart=False):
    launcher, stop = ((subprocess.Popen, stop_child) if reap_for_restart
                      else (FixtureProcess, stop_process_group))
    child = launcher(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT,
                     env=env, start_new_session=True)
    try:
        yield child
    finally:
        cleanup_steps(('stop desktop fixture process', lambda: stop(child)))


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
    def require_focus():
        if window and int(output(['xdotool', 'getactivewindow'])) != int(window, 0):
            raise RuntimeError('Graphical authorization prompt lost focus')

    if window:
        run(['xdotool', 'windowactivate', '--sync', window], timeout=15)
    if allow:
        if window:
            run(['xdotool', 'windowfocus', '--sync', window], timeout=15)
        password = password_file.read_text()
        if not password.isascii() or not password.isalnum():
            raise ValueError('Invalid disposable account password')
        time.sleep(.3)
        require_focus()
        run(['xdotool', 'type', '--clearmodifiers', '--delay', '10', '--file', '-'],
            text=True, timeout=15, input=password, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        require_focus()
        run(['xdotool', 'key', 'Return'], timeout=15)
    else:
        require_focus()
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
            if (status := owner.poll()) is not None:
                raise RuntimeError(f'Installed {backend} GUI owner exited: {status}')
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


def recognized_lines(tsv, scale=1):
    lines = {}
    for row in csv.DictReader(tsv.splitlines(), delimiter='\t', quoting=csv.QUOTE_NONE):
        if row['level'] != '5' or not row['text'].strip():
            continue
        key = tuple(row[name] for name in ('page_num', 'block_num', 'par_num', 'line_num'))
        box = tuple(int(row[name])//scale for name in ('left', 'top', 'width', 'height'))
        lines.setdefault(key, []).append((row['text'], box))
    return list(lines.values())


def normalize_screen_text(text):
    return ''.join(text.casefold().split()).replace('–', '-').replace('－', '-')


def screen_groups(words):
    group = []
    for word, box in sorted(words, key=lambda entry: entry[1][0]):
        if group:
            previous = group[-1][1]
            if box[0] - (previous[0]+previous[2]) > max(previous[3], box[3]):
                yield group
                group = []
        group.append((word, box))
    if group:
        yield group


def screen_bounds(words):
    left = min(box[0] for _, box in words)
    top = min(box[1] for _, box in words)
    right = max(box[0]+box[2] for _, box in words)
    bottom = max(box[1]+box[3] for _, box in words)
    return left, top, right-left, bottom-top


def screen_label(lines, labels, *, contains=False):
    wanted = tuple(map(normalize_screen_text, labels))
    matches = []
    for words in lines:
        if contains:
            text = normalize_screen_text(''.join(word for word, _ in words))
            if any(label in text for label in wanted):
                return True
            continue
        for group in screen_groups(words):
            if normalize_screen_text(''.join(word for word, _ in group)) in wanted:
                left, top, width, height = screen_bounds(group)
                matches.append((left+width//2, top+height//2))
    return min(matches, key=lambda point: point[1], default=None)


def screen_version(lines, version):
    parts = re.split(r'([.+-])', version)
    expected = r'\s*'.join(re.escape(part) for part in parts)
    pattern = re.compile(rf'(?<![0-9A-Za-z.+-])(?:v\s*)?{expected}(?![0-9A-Za-z+-]|\s*[.+-]\s*[0-9A-Za-z])')
    continuation = re.compile(r'(?:[0-9]\s*|[0-9A-Za-z])[.+-]\s+$')
    for words in lines:
        line = ' '.join(word for word, _ in words).replace('–', '-').replace('－', '-')
        if any(not continuation.search(line[:match.start()]) for match in pattern.finditer(line)):
            return True
    return False


def update_window_geometry(window):
    info = output(['xwininfo', '-id', window])
    return tuple(int(re.search(rf'^\s*{field}:\s+(-?\d+)', info, re.M)[1])
                 for field in ('Absolute upper-left X', 'Absolute upper-left Y', 'Width', 'Height'))


def recognize_update_screen(image):
    code = (
        'from PIL import Image\n'
        'import sys\n'
        'image = Image.open(sys.argv[1]).convert("L")\n'
        'image.resize((image.width*2, image.height*2)).save(sys.stdout.buffer, format="PNG")\n'
    )
    enlarged = subprocess.check_output(['/usr/bin/python3', '-c', code, image], timeout=10)
    result = subprocess.run(['tesseract', 'stdin', 'stdout', '--psm', '11', '-l', 'chi_sim+eng', 'tsv'],
                            input=enlarged, capture_output=True, check=True, timeout=20,
                            env=dict(os.environ, OMP_THREAD_LIMIT='1'))
    return recognized_lines(result.stdout.decode('utf-8'), scale=2)


def recognize_primary_update_buttons(image, labels):
    prepare = """
from PIL import Image, ImageChops, ImageOps
from itertools import groupby
import json
import sys

def bands(projection):
    for marked, group in groupby(enumerate(projection), key=lambda entry: entry[1]):
        if marked:
            first = last = next(group)[0]
            for last, _ in group:
                pass
            yield first, last + 1

image = Image.open(sys.argv[1]).convert('RGB')
red, green, blue = ImageChops.difference(image, Image.new('RGB', image.size, sys.argv[2])).split()
mask = ImageChops.lighter(ImageChops.lighter(red, green), blue).point(lambda value: 255 if value <= 4 else 0)
buttons = []
for top, bottom in bands(mask.getprojection()[1]):
    if not 26 <= bottom - top <= 60:
        continue
    for left, right in bands(mask.crop((0, top, image.width, bottom)).getprojection()[0]):
        if not 42 <= right - left <= 300:
            continue
        text = image.crop((left + 7, top + 4, right - 7, bottom - 4)).convert('L')
        text = text.resize((text.width * 3, text.height * 3))
        text = text.point(lambda value: 0 if value >= 180 else 255)
        filename = f'{sys.argv[1]}.{len(buttons)}.png'
        ImageOps.expand(text, border=18, fill=255).save(filename)
        buttons.append((filename, (left, top, right - left, bottom - top)))
print(json.dumps(buttons))
"""
    buttons = json.loads(subprocess.check_output(
        ['/usr/bin/python3', '-c', prepare, image, UPDATE_FIXTURE_ACCENT[1]], timeout=10))
    wanted = set(map(normalize_screen_text, labels))
    matches = []
    for filename, bounds in buttons:
        text = subprocess.check_output(
            ['tesseract', filename, 'stdout', '--psm', '7', '-l', 'chi_sim+eng'],
            text=True, timeout=10, env=dict(os.environ, OMP_THREAD_LIMIT='1')).strip()
        if normalize_screen_text(text) in wanted:
            matches.append([(text, tuple(bounds))])
    return matches


def redact_update_words(lines, version):
    labels = [label for translations in UPDATE_LABELS.values() for label in translations]
    allowed = {normalize_screen_text(label): label for label in (*labels, version)}
    return [
        {'text': allowed.get(normalize_screen_text(''.join(word for word, _ in group)), '<redacted>'),
         'box': screen_bounds(group)}
        for words in lines for group in screen_groups(words)
    ]


def redact_update_image(size, destination, words):
    code = (
        'from PIL import Image, ImageDraw\n'
        'import json, sys\n'
        'result = Image.new("RGB", tuple(map(int, sys.argv[1:3])), "#dddddd")\n'
        'draw = ImageDraw.Draw(result)\n'
        'words = json.load(sys.stdin)\n'
        'for word in sorted(words, key=lambda word: word["text"] == "<redacted>"):\n'
        '    x, y, width, height = word["box"]\n'
        '    if 0 <= x < x+width <= result.width and 0 <= y < y+height <= result.height:\n'
        '        fill = "#888888" if word["text"] == "<redacted>" else "#265e94"\n'
        '        draw.rectangle((x, y, x+width-1, y+height-1), fill=fill)\n'
        'result.save(sys.argv[3])\n'
    )
    run(['/usr/bin/python3', '-c', code, *size, destination], input=json.dumps(words),
        text=True, timeout=10)


class UpdateNavigation:
    def __init__(self, report, version):
        self.report = report
        self.version = version
        self.events = []
        self.lines = []
        self.phase = None
        self.step = 'window'
        self.window = None
        self.geometry = None

    def record(self, step, **details):
        self.events.append({'phase': self.phase, 'step': step, **details})
        write_json(self.report/'update-navigation.json', self.events)

    def focus(self):
        try:
            return int(output(['xdotool', 'getactivewindow']))
        except (ValueError, subprocess.SubprocessError):
            return None

    def attach(self, window, *, dismiss_notice=True):
        self.step = 'window'
        self.window = int(window, 0)
        self.geometry = None
        self.lines = []
        run(['xdotool', 'windowactivate', '--sync', window], timeout=15)
        if dismiss_notice:
            run(['xdotool', 'key', 'Escape'], timeout=10)
        run(['wmctrl', '-ir', window, '-b', 'add,maximized_vert,maximized_horz'], timeout=15)

        def ready():
            geometry = update_window_geometry(window)
            if self.focus() == self.window and geometry[2] >= 1100 and geometry[3] >= 800:
                return geometry

        self.geometry = wait_for('Graphical update window did not maximize and gain focus', ready, timeout=15)
        self.record('window', window=self.window, geometry=self.geometry)

    def capture(self, *, primary_labels=()):
        if self.focus() != self.window:
            return False
        self.lines = []
        with tempfile.TemporaryDirectory(prefix='cor-update-frame-', dir=self.report) as temporary:
            image = Path(temporary)/'update-screen.png'
            run(['scrot', '-a', ','.join(map(str, self.geometry)), image], timeout=15)
            if self.focus() != self.window:
                return False
            self.lines = recognize_update_screen(image)
            if primary_labels and screen_label(self.lines, primary_labels) is None:
                self.lines.extend(recognize_primary_update_buttons(image, primary_labels))
        return True

    def save_frame(self, name, words):
        if self.lines and self.geometry:
            redact_update_image(self.geometry[2:], self.report/name, words)

    @contextlib.contextmanager
    def diagnose_failure(self):
        try:
            yield
        except (RuntimeError, OSError, ValueError, subprocess.SubprocessError) as original:
            try:
                words = redact_update_words(self.lines, self.version)
                self.record(self.step, result='failed', window=self.window, geometry=self.geometry,
                            focused=self.focus(), ocr=words)
                self.save_frame('update-navigation-failure.png', words)
            except (OSError, ValueError, subprocess.SubprocessError) as error:
                original.add_note(f'Could not preserve graphical navigation evidence: {type(error).__name__}')
            raise

    def input(self, step, point, button=1, repeat=1):
        self.step = step
        focused = self.focus()
        width, height = self.geometry[2:]
        if focused != self.window or not (0 <= point[0] < width and 0 <= point[1] < height):
            self.record(step, window=self.window, focused=focused, point=point, result='rejected')
            raise RuntimeError('Graphical update input target is outside the focused application window')
        result = 'failed'
        try:
            run(['xdotool', 'mousemove', '--window', self.window, *map(str, point),
                 'click', '--repeat', repeat, '--delay', '70', button], timeout=10)
            result = 'sent'
        finally:
            self.record(step, window=self.window, focused=focused, point=point,
                        button=button, repeat=repeat, result=result)

    def locate(self, labels, *, region=None, contains=False):
        primary = not contains and labels in (UPDATE_LABELS['download'], UPDATE_LABELS['continue'])
        if not (self.capture(primary_labels=labels) if primary else self.capture()):
            return None
        lines = self.lines
        if region:
            left, top, right, bottom = region
            lines = [[(word, box) for word, box in words if
                      left <= box[0]+box[2]//2 < right and top <= box[1]+box[3]//2 < bottom]
                     for words in lines]
        return screen_label(lines, labels, contains=contains)

    def wait(self, step, labels, *, click=False, region=None, timeout=45, evidence=None, scroll=False):
        self.step = step
        scrolls = 0

        def locate():
            nonlocal scrolls
            point = self.locate(labels, region=region, contains=not click)
            if point is None and scroll and scrolls < 10 and self.focus() == self.window:
                width, height = self.geometry[2:]
                self.input('scroll-settings', (width*3//4, height//2), button=5, repeat=3)
                self.step = step
                scrolls += 1
                time.sleep(.35)
            return point

        point = wait_for(f'Graphical update did not display {labels!r}', locate, timeout=timeout)
        if click:
            self.input(step, point)
        else:
            self.record(step, result='visible', window=self.window)
        if evidence:
            self.save_frame(evidence, redact_update_words(self.lines, self.version))

    def open_settings(self):
        width, height = self.geometry[2:]
        header = (60, 44, width, 102)
        self.input('open-settings', (56, height-62))
        try:
            self.wait('settings-page', UPDATE_LABELS['settings'], region=header, timeout=6)
        except TimeoutError:
            self.input('open-compact-settings', (30, height-35))
            self.wait('settings-page', UPDATE_LABELS['settings'], region=header)

    def verify_responsive(self):
        self.open_settings()
        width = self.geometry[2]
        home = self.locate(UPDATE_LABELS['home'], region=(0, 104, 216, 171))
        self.input('return-home', home or (30, 124))
        self.wait('home-page', UPDATE_LABELS['home'], region=(60, 44, width, 102))

    def check(self):
        width, height = self.geometry[2:]
        header = (60, 44, width, 102)
        self.open_settings()
        tab = self.locate(UPDATE_LABELS['updates'], region=header)
        if tab:
            self.input('open-updates', tab)
        else:
            self.input('reset-settings-scroll', (width*3//4, height//2), button=4, repeat=30)
        self.wait('check-for-updates', UPDATE_LABELS['check'], click=True, scroll=True)

    def wait_for_published_version(self):
        def probe():
            if not self.capture():
                return False
            if screen_version(self.lines, self.version):
                return True
            close = screen_label(self.lines, UPDATE_LABELS['close'])
            if close is not None and screen_label(self.lines, RATE_LIMIT_MESSAGES, contains=True):
                self.record('published-version', result='rate-limited', window=self.window)
                self.input('close-rate-limit-notice', close)
                wait_for('Graphical rate limit notice did not close',
                         lambda: self.capture() and screen_label(self.lines, UPDATE_LABELS['close']) is None,
                         timeout=15)
                raise ReleaseRateLimit('Graphical Release check was rate limited by GitHub')
            return False

        recheck = False

        def attempt():
            nonlocal recheck
            if recheck:
                self.wait('check-for-updates', UPDATE_LABELS['check'], click=True, scroll=True)
            recheck = True
            self.step = 'published-version'
            wait_for('Graphical update did not display the exact published version', probe, timeout=240)

        retry_release_check(attempt, 'graphical update discovery', lambda: None)
        self.record(self.step, result='visible', window=self.window)
        self.save_frame('update-discovered.png', redact_update_words(self.lines, self.version))

    def dismiss_failure(self, status_file, result):
        self.step = 'dismiss-failure-notice'
        if self.focus() != self.window:
            raise RuntimeError('Graphical update notice is outside the focused application window')
        self.wait('close-failure-notice', UPDATE_LABELS['close'], click=True, timeout=15)
        if result.get('acknowledged') is False:
            def acknowledged():
                status = read_object(status_file)
                return (status.get('acknowledged') is True
                        and all(status.get(field) == result.get(field)
                                for field in ('directory', 'version', 'state', 'updated_at')))

            wait_for('Graphical update failure was not acknowledged', acknowledged, timeout=15)
        self.record(self.step, result='dismissed', window=self.window)


def pending_gui_authorization(status_file, completed_attempts, locate, *, version):
    status = read_object(status_file)
    directory = status.get('directory')
    if not directory or status.get('version') != version:
        return None
    if previous := completed_attempts.get(directory):
        updated_at, last_update = status.get('updated_at'), previous.get('updated_at')
        if type(updated_at) is not int or type(last_update) is not int or updated_at <= last_update:
            return None
    if status.get('state') in ('succeeded', 'failed'):
        raise RuntimeError('Graphical installation ended before the expected authorization prompt')
    if status.get('state') == 'installing' and (dialog := locate()):
        return dialog, status
    return None


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


def process_identity(pid):
    fields = (Path('/proc')/str(pid)/'stat').read_text().rsplit(')', 1)[1].split()
    return pid, int(fields[19])


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
    return dict(os.environ, SLINT_BACKEND='winit-software', SLINT_SCALE_FACTOR='1', TMPDIR=str(work),
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
    write_json(data/'settings.json', {'language': 'zh-CN', 'accent_color': UPDATE_FIXTURE_ACCENT[0],
                                    'update_prereleases': published['prerelease'],
                                    'update_check_interval_minutes': 0, 'proxy_mode': 'off',
                                    'theme_mode': 'light', 'dark': False,
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
            try:
                windows = inspect('x11')
            except subprocess.CalledProcessError:
                return None
            for window in windows:
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
    navigation = UpdateNavigation(report, published['version'])
    with (report/'update.log').open('w') as log, \
            process([APP/'clash-of-rust'], log, env, reap_for_restart=True) as owner:
        navigation.phase = 'startup'
        with navigation.diagnose_failure():
            pid, window = ready()
            stopped_gui_core(pid, startup=True)
            navigation.step = 'seed-fixture'
            cli('profile', 'add', 'GUI update fixture', str(profile))
            subscriptions = cli('profile', 'list', '--filter', 'GUI update fixture')
            if len(subscriptions) != 1:
                raise RuntimeError('Could not seed a subscription in the GUI-owned backend')
            source = data/'profiles'/f'{subscriptions[0]["id"]}.yaml'
            settings, digest = read_object(data/'settings.json'), sha256(source)
            verify_data(case['previous_version'])
        previous_result = None
        completed_attempts = {}
        seen_requests = set()
        for phase, failure in (('cancel', '授权已取消'), ('locked', '安装失败'), ('retry', None)):
            navigation.phase = phase
            with navigation.diagnose_failure():
                navigation.step = 'authorization-agent'
                restart_agent()
                navigation.attach(window)
                if phase == 'cancel':
                    navigation.check()
                    navigation.wait_for_published_version()
                    checks['version_visible_in_gui'] = True
                if previous_result and type(previous_result.get('updated_at')) is int:
                    wait_for('Clock did not advance past the previous installation record',
                             lambda: int(time.time()) > previous_result['updated_at'], timeout=5)
                navigation.wait('download-and-install', UPDATE_LABELS['download'], click=True, scroll=True)
                if published['prerelease']:
                    navigation.wait('prerelease-confirmation', UPDATE_LABELS['prerelease'])
                    time.sleep(PRERELEASE_CONFIRMATION_SECONDS)
                    navigation.wait('confirm-prerelease', UPDATE_LABELS['continue'], click=True)
                navigation.step = 'authorization-prompt'
                dialog, status = wait_for(
                    'GUI did not show a graphical installation authorization prompt',
                    lambda: pending_gui_authorization(status_file, completed_attempts, locate,
                                                      version=published['version']), timeout=240)
                directory = status['directory']
                navigation.step = 'verify-authorization-request'
                package = verified_gui_download(status, published, pid, work, data)
                request = verify_gui_pkexec(package, tool, published['sha256'])
                identity = process_identity(request)
                if identity in seen_requests:
                    raise RuntimeError('Graphical authorization reused a previous pkexec request')
                seen_requests.add(identity)
                request_observation(phase, request)
                checks[phase] = {'download_sha256': published['sha256'], 'graphical_pkexec': tool,
                                 'sealed_download': True}
                navigation.step = 'respond-to-authorization'
                if (locate() != dialog or process_identity(request) != identity
                        or verify_gui_pkexec(package, tool, published['sha256']) != request):
                    raise RuntimeError('The verified graphical authorization prompt changed before the response')
                respond_authorization(dialog, password_file, phase != 'cancel')
                wait_for('Graphical update authorization dialog remained open',
                         lambda: locate() != dialog, timeout=30)

                def outcome():
                    owner.poll()
                    result = read_object(status_file)
                    started, completed = status.get('updated_at'), result.get('updated_at')
                    if (result.get('directory') == directory and result.get('version') == published['version']
                            and result.get('state') in ('succeeded', 'failed')
                            and (type(started) is not int or (type(completed) is int and completed >= started))):
                        return result
                    return None

                navigation.step = 'installation-outcome'
                result = wait_for('No outcome for the graphical installation', outcome, timeout=180)
                previous_result = result
                completed_attempts[directory] = result
                expected_state = 'failed' if failure else 'succeeded'
                if (result.get('state') != expected_state or result.get('error') is not bool(failure)
                        or result.get('version') != published['version']
                        or (failure and failure not in result.get('message', ''))):
                    raise RuntimeError(f'Unexpected graphical update outcome for {phase}')
                if phase == 'locked':
                    navigation.step = 'release-native-lock'
                    checks[phase]['native_lock_conflict'] = request_observation('release')['lock_conflict']

                previous_pid = pid
                navigation.step = 'recover-gui'
                pid, window = ready(original=None if failure else pid)
                if failure:
                    if pid != previous_pid:
                        raise RuntimeError('Graphical update failure replaced the original GUI owner')
                    navigation.attach(window, dismiss_notice=False)
                    labels = UPDATE_LABELS['cancelled' if phase == 'cancel' else 'failed']
                    navigation.wait('failure-notice', labels, timeout=30, evidence=f'update-{phase}.png')
                    navigation.dismiss_failure(status_file, result)
                else:
                    navigation.step = 'verify-reopened-window'
                    navigation.attach(window, dismiss_notice=False)
                    navigation.verify_responsive()
                navigation.step = 'verify-installed-state'
                native = case['old_native_version'] if failure else case['new_native_version']
                verify_installed_package(case['format'], native)
                reopened = pid != previous_pid
                stopped_gui_core(pid, startup=reopened)
                verify_data(case['previous_version'] if failure else published['version'])
                checks[phase].update(outcome=expected_state, gui_usable=True, gui_reopened=reopened,
                                     installed_native_version=native, native_package_complete=True,
                                     settings_preserved=True, subscription_preserved=True)
                if reopened:
                    checks[phase]['core_started_and_stopped_by_fixture_after_reopen'] = True
                else:
                    checks[phase]['original_gui_owner_retained'] = True
                    checks[phase]['core_still_stopped_after_failure'] = True


def session():
    if os.geteuid() == 0:
        raise RuntimeError('GUI must run as an ordinary user')
    report = Path(os.environ['COR_GUI_REPORT'])
    result = {'status': 'running', 'checks': {}}
    case_file = report.parent/'published-update-case.json'
    try:
        require_logind_session()
        result['checks']['login_session'] = True
        with (report/'desktop.log').open('w') as log, contextlib.ExitStack() as stack:
            update_case = read_object(case_file)
            if case_file.exists():
                if not update_case.get('exercise_id'):
                    raise RuntimeError('Missing graphical update exercise identity')
                for key in ('exercise_id', 'fixture_sha256'):
                    if key in update_case:
                        result[key] = update_case[key]
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
        result['error'] = type(error).__name__ if case_file.is_file() else str(error)
        raise
    finally:
        write_json(report/'gui.json', result)


def stage_session(directory):
    scripts = directory/'scripts'
    scripts.mkdir(mode=0o755)
    scripts.chmod(0o755)
    for name in ('linux_gui.py', 'linux_native_guest.py', 'linux_compatibility_targets.py', 'linux_mint_desktop.py',
                 'build_support.py', 'cli_tui.py', 'linux_update_retry.py'):
        target = scripts/name
        shutil.copyfile(Path(__file__).resolve().parent/name, target)
        target.chmod(0o644)
    return scripts/'linux_gui.py'


def preserve_report(source, *, update_report=None):
    destination = update_report if update_report is not None else REPORT/'desktop'
    destination.mkdir(parents=True, exist_ok=True)
    if update_report is not None:
        names = ('gui.json', 'update-discovered.png', 'update-cancel.png', 'update-locked.png',
                 'update-navigation.json', 'update-navigation-failure.png')
    else:
        names = ('gui.json', 'authentication.log', 'desktop.log', 'x11.log', 'wayland.log',
                 'dbus-probes.log', 'update.log')
    for name in names:
        path = source/name
        if path.is_file() and not path.is_symlink():
            shutil.copyfile(path, destination/name)
    if update_report is not None:
        return
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
        report = Path(env['COR_GUI_REPORT'])
        reason = type(error).__name__ if (report.parent/'published-update-case.json').is_file() else str(error)
        write_json(report/'gui.json', {'status': 'failed', 'checks': {}, 'error': reason})
        raise


@contextlib.contextmanager
def fixture_signals(on_interrupt=None):
    interruption = None

    def remember_interruption(number, _frame):
        nonlocal interruption
        if interruption is None:
            interruption = KeyboardInterrupt(f'Interrupted by {signal.Signals(number).name}')

    def check_interrupted():
        if interruption is not None:
            raise interruption

    numbers = (signal.SIGINT, signal.SIGTERM, getattr(signal, 'SIGHUP', None))
    handlers = {number: signal.signal(number, remember_interruption)
                for number in numbers if number is not None}
    primary = None
    try:
        yield check_interrupted
    except BaseException as error:
        primary = error
        if interruption is None and isinstance(error, KeyboardInterrupt):
            interruption = error
        raise
    finally:
        mask = signal.pthread_sigmask(signal.SIG_BLOCK, handlers)
        try:
            for number, handler in handlers.items():
                signal.signal(number, handler)
            for number in sorted(signal.sigpending() & (handlers.keys() - mask)):
                signal.sigwait({number})
                remember_interruption(number, None)
            if interruption is not None:
                try:
                    if primary is None:
                        raise interruption
                    if primary is not interruption:
                        primary.add_note(str(interruption))
                finally:
                    if on_interrupt is not None:
                        cleanup_steps(('record interrupted update', lambda: on_interrupt(interruption)))
        finally:
            signal.pthread_sigmask(signal.SIG_SETMASK, mask)


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


def fixture_sudo(*arguments, timeout=30, **options):
    if 'input' not in options:
        options['stdin'] = subprocess.DEVNULL
    return run(['sudo', '-n', '/usr/bin/timeout', '--kill-after=5s', f'{timeout}s', *arguments],
               timeout=timeout+15, start_new_session=True, **options)


def stop_desktop_unit(unit):
    result = subprocess.run(['sudo', '-n', '/usr/bin/timeout', '--kill-after=5s', '25s',
                             'systemctl', 'stop', unit], timeout=35, start_new_session=True)
    if result.returncode:
        state = subprocess.run(['systemctl', 'show', '--property=LoadState', '--property=ActiveState', unit],
                               capture_output=True, text=True, timeout=10, start_new_session=True)
        if set(state.stdout.splitlines()) != {'LoadState=not-found', 'ActiveState=inactive'}:
            result.check_returncode()


def stop_fixture_unit(unit, **properties):
    expected = {**properties, 'Id': unit, 'Transient': 'yes'}
    fields = ('LoadState', 'ActiveState', *expected)
    result = subprocess.run(['systemctl', 'show', '--no-pager', '--property='+','.join(fields), unit],
                            capture_output=True, text=True, timeout=10, start_new_session=True)
    state = dict(line.split('=', 1) for line in result.stdout.splitlines() if '=' in line)
    if state.get('LoadState') == 'not-found' and state.get('ActiveState') == 'inactive':
        return
    result.check_returncode()
    if any(state.get(key) != value for key, value in expected.items()):
        raise RuntimeError(f'Refusing to stop an unverified fixture unit {unit}')
    stop_desktop_unit(unit)


def stop_child(child):
    if child.poll() is None:
        child.terminate()
        try:
            child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait(timeout=10)


def stop_process_group(child):
    if child.returncode is not None:
        return
    try:
        os.waitid(os.P_PID, child.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
        if os.getpgid(child.pid) != child.pid or os.getsid(child.pid) != child.pid:
            raise RuntimeError('The fixture child is not the leader of its own session')
        with contextlib.suppress(ProcessLookupError):
            os.killpg(child.pid, signal.SIGTERM)
            time.sleep(.5)
            os.killpg(child.pid, signal.SIGKILL)
    except ChildProcessError as error:
        raise RuntimeError('The fixture group leader was reaped before cleanup') from error
    child.reap()


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
    child = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
                             start_new_session=True)
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


def run_update_desktop(command, home, package_lock, package_format, uid, *, stop_session, check_interrupted):
    request = home/'report'/'update-observation-request.json'
    acknowledgement = home/'update-observation-state.json'
    tool = '/usr/bin/pacman' if package_format == 'pkg' else '/usr/bin/dpkg'
    phases = iter(('cancel', 'locked', 'release', 'retry'))
    expected = next(phases)
    state = None
    with contextlib.ExitStack() as lock, (home/'native-lock.log').open('w+') as log:
        child = subprocess.Popen(command, cwd=home, start_new_session=True)
        try:
            deadline = time.monotonic()+DESKTOP_UPDATE_TIMEOUT
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
                raise subprocess.TimeoutExpired(command, DESKTOP_UPDATE_TIMEOUT)
            if child.returncode:
                raise subprocess.CalledProcessError(child.returncode, command)
            if expected is not None:
                raise RuntimeError('Graphical session did not complete the native installation observations')
        finally:
            if sys.exc_info()[1] is not None:
                cleanup_steps(('stop desktop client', lambda: stop_child(child)),
                              ('stop verified desktop session', stop_session))


def remove_gui_rule(rule):
    fixture_sudo('rm', '-f', rule, timeout=15)


def stop_gui_user(user, user_home, uid):
    if gui_account(user, user_home, uid) is None:
        return
    result = run(['loginctl', 'list-users', '--no-legend', '--no-pager'],
                 capture_output=True, text=True, timeout=15, start_new_session=True)
    if any(row.split()[:2] == [str(uid), user] for row in (result.stdout or '').splitlines()):
        run(['sudo', 'loginctl', 'terminate-user', user], timeout=30, start_new_session=True)


def gui_account(user, user_home, uid=None):
    import pwd

    try:
        account = pwd.getpwnam(user)
    except KeyError:
        return None
    if (Path(account.pw_dir) != user_home or account.pw_uid == 0
            or (uid is not None and account.pw_uid != uid)):
        raise RuntimeError('Refusing to clean up an account outside the disposable fixture')
    return account


def delete_gui_user(user, user_home, uid):
    def remove():
        if gui_account(user, user_home, uid) is None:
            return True
        try:
            fixture_sudo('userdel', '--remove', user)
        except subprocess.CalledProcessError as error:
            if error.returncode != 8:
                raise
            return False
        return True

    if remove():
        return
    if gui_account(user, user_home, uid) is not None:
        try:
            fixture_sudo('/usr/bin/pkill', '--signal', 'KILL', '--uid', str(uid))
        except subprocess.CalledProcessError as error:
            if error.returncode != 1:
                raise
    wait_for('Disposable GUI user still has running processes', remove, timeout=10)


def installed_tests(*, update_case=None, package_lock=None, update_report=None, check_interrupted=None):
    from linux_ci import require_actions
    require_actions()
    if update_case and (not package_lock or update_report is None or not (
            os.environ.get('COR_LINUX_GUEST') == 'true'
            or os.environ.get('RUNNER_ENVIRONMENT') == 'github-hosted')):
        raise RuntimeError('Graphical update requires a disposable runner or compatibility VM')
    REPORT.mkdir(parents=True, exist_ok=True)
    fixture_id = secrets.token_hex(12)
    user = 'corgui'+fixture_id
    password = secrets.token_hex(12)
    signals = fixture_signals() if check_interrupted is None else contextlib.nullcontext(check_interrupted)
    with signals as check_interrupted:
        temporary = tempfile.TemporaryDirectory(prefix='cor-desktop-')
        home = Path(temporary.name)
        report = home/'report'
        uid = None
        rule = None
        unit = None
        session_cleanup = contextlib.ExitStack()
        try:
            home.chmod(0o755)
            script = stage_session(home)
            from linux_native_guest import DISTROS
            native = platform.freedesktop_os_release().get('ID') in DISTROS
            groups = [] if native or update_case else ['--groups', 'sudo']
            check_interrupted()
            fixture_sudo('useradd', '--create-home', *groups,
                         '--home-dir', home/'home', '--shell', '/bin/bash', user)
            account = gui_account(user, home/'home')
            if account is None:
                raise RuntimeError('The disposable GUI user was not created')
            uid = account.pw_uid
            check_interrupted()
            fixture_sudo('chpasswd', input=user+':'+password+'\n', text=True)
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
                fixture_sudo('tee', rule, input=contents, text=True, stdout=subprocess.DEVNULL, timeout=15)
            run(['sudo', 'install', '-d', '-m', '0755', '-o', user, '-g', user, report], timeout=15)
            password_file = home/'password'
            descriptor = os.open(password_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, 'w') as stream:
                stream.write(password)
            run(['sudo', 'chown', f'{user}:{user}', password_file], timeout=15)
            env = {'HOME': str(home/'home'), 'PATH': '/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',
                   'XDG_CONFIG_HOME': str(home/'home/.config'),
                   'XDG_CURRENT_DESKTOP': 'GNOME', 'GNOME_SHELL_SESSION_MODE': 'user',
                   'LIBGL_ALWAYS_SOFTWARE': '1', 'GSETTINGS_BACKEND': 'dconf',
                   'COR_GUI_REPORT': str(report), 'COR_GUI_AUTH_PASSWORD_FILE': str(password_file),
                   'COR_LINUX_VM_ACCELERATION': os.environ.get('COR_LINUX_VM_ACCELERATION', ''),
                   'PYTHONDONTWRITEBYTECODE': '1', 'LANG': 'C.UTF-8',
                   'GITHUB_ACTIONS': 'true', 'NO_AT_BRIDGE': '1'}
            # PAM registers a real logind session for the graphical polkit agent.
            unit = 'cor-desktop-'+fixture_id+'.service'
            description = f'Clash of Rust desktop fixture {home}'
            timeout = DESKTOP_UPDATE_TIMEOUT if update_case else DESKTOP_TIMEOUT
            session_cleanup.callback(
                lambda: cleanup_steps(
                    ('stop verified desktop unit', lambda: stop_fixture_unit(
                        unit, Description=description, User=user, WorkingDirectory=str(home))),
                    ('terminate verified GUI login session', lambda: stop_gui_user(user, home/'home', uid))))
            command = ['sudo', 'systemd-run', '--unit='+unit, '--wait', '--collect',
                       '--quiet', '--description='+description, '--property=User='+user,
                       '--property=PAMName=login', '--property=KillMode=control-group',
                       f'--property=RuntimeMaxSec={timeout+120}s',
                       '--property=TimeoutStopSec=15s',
                       '--property=StandardInput=null', '--property=StandardOutput=journal',
                       '--property=StandardError=journal', '--property=WorkingDirectory='+str(home),
                       *('--setenv='+key+'='+value for key, value in env.items()),
                       sys.executable, '-E', '-s', '-B', script, '--session-launch']
            try:
                check_interrupted()
                if update_case:
                    run_update_desktop(command, home, package_lock, update_case['format'], uid,
                                       stop_session=session_cleanup.close,
                                       check_interrupted=check_interrupted)
                else:
                    subprocess.run(command, check=True, cwd=home, timeout=timeout, start_new_session=True)
                check_interrupted()
            except subprocess.SubprocessError:
                if update_case:
                    raise
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
            steps = []
            if rule:
                steps.append(('remove disposable polkit rule', lambda: remove_gui_rule(rule)))
            if unit:
                steps.append(('stop verified desktop session', session_cleanup.close))
            if uid is not None:
                steps.append(('delete disposable GUI user', lambda: delete_gui_user(user, home/'home', uid)))
            steps.append(('recover temporary desktop files', lambda: run(
                ['sudo', 'chown', '-R', '-h', f'{os.getuid()}:{os.getgid()}', home],
                timeout=30, start_new_session=True)))
            if report.is_dir():
                steps.append(('preserve desktop report', lambda: preserve_report(
                    report, update_report=update_report)))
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
