"""Opt-in X11/WSLg smoke test using isolated data and native window events.

Run under dbus-run-session so no desktop tray host exists. Requires Linux,
libX11, a graphical DISPLAY, and a prepared application with bundled resources.
"""
import argparse
import ctypes as c
import os
import subprocess
import tempfile
import time
import json
import urllib.request
import signal
import socket
import atexit
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
inputs = parser.add_mutually_exclusive_group()
inputs.add_argument('--executable', type=Path)
inputs.add_argument('--deb', type=Path, help='Unpack and test without installing')
parser.add_argument('--output', type=Path)
parser.add_argument('--scales', type=float, nargs='+', default=[1.0],
                    help='X11 scale factors to test; fractional values require a display with DPI support')
args = parser.parse_args()
assert args.scales and all(0 < scale <= 4 for scale in args.scales), 'Invalid scale factors'
assert os.environ.get('DISPLAY'), 'A graphical X11 DISPLAY is required'
assert os.environ.get('DBUS_SESSION_BUS_ADDRESS'), 'Run under dbus-run-session'
if args.deb:
    package_directory = tempfile.TemporaryDirectory(prefix='clash-gui-package-')
    atexit.register(package_directory.cleanup)
    subprocess.run(['dpkg-deb', '-x', str(args.deb.resolve(strict=True)), package_directory.name], check=True)
    executable = Path(package_directory.name, 'opt/clash-of-rust/clash-of-rust')
else:
    executable = (args.executable or Path('/usr/bin/clash-of-rust')).resolve(strict=True)
tray_owner = subprocess.check_output([
    'gdbus', 'call', '--session', '--dest', 'org.freedesktop.DBus',
    '--object-path', '/org/freedesktop/DBus', '--method',
    'org.freedesktop.DBus.NameHasOwner', 'org.kde.StatusNotifierWatcher',
], text=True)
assert 'false' in tray_owner, 'Use dbus-run-session to isolate the tray host'

X = c.CDLL('libX11.so.6')
X.XOpenDisplay.argtypes = [c.c_char_p]
X.XOpenDisplay.restype = c.c_void_p
display = X.XOpenDisplay(None)
assert display, 'X11 display unavailable'
X.XDefaultRootWindow.argtypes = [c.c_void_p]
X.XDefaultRootWindow.restype = c.c_ulong
root = X.XDefaultRootWindow(display)
X.XQueryTree.argtypes = [c.c_void_p, c.c_ulong, c.POINTER(c.c_ulong), c.POINTER(c.c_ulong), c.POINTER(c.POINTER(c.c_ulong)), c.POINTER(c.c_uint)]
X.XFetchName.argtypes = [c.c_void_p, c.c_ulong, c.POINTER(c.c_void_p)]
X.XFree.argtypes = [c.c_void_p]
X.XInternAtom.argtypes = [c.c_void_p, c.c_char_p, c.c_int]
X.XInternAtom.restype = c.c_ulong
X.XFlush.argtypes = [c.c_void_p]
X.XSendEvent.argtypes = [c.c_void_p, c.c_ulong, c.c_int, c.c_long, c.c_void_p]
X.XGetWindowProperty.argtypes = [c.c_void_p, c.c_ulong, c.c_ulong, c.c_long, c.c_long, c.c_int, c.c_ulong, c.POINTER(c.c_ulong), c.POINTER(c.c_int), c.POINTER(c.c_ulong), c.POINTER(c.c_ulong), c.POINTER(c.c_void_p)]

class ClassHint(c.Structure):
    _fields_ = [('res_name', c.c_void_p), ('res_class', c.c_void_p)]
X.XGetClassHint.argtypes = [c.c_void_p, c.c_ulong, c.POINTER(ClassHint)]

def window_class(window):
    hint = ClassHint()
    assert X.XGetClassHint(display, window, c.byref(hint)), 'Window has no WM_CLASS'
    try:
        return tuple(c.string_at(value).decode() for value in (hint.res_name, hint.res_class))
    finally:
        if hint.res_name: X.XFree(hint.res_name)
        if hint.res_class: X.XFree(hint.res_class)

def window_pid(window):
    kind, format_, count, remaining, data = c.c_ulong(), c.c_int(), c.c_ulong(), c.c_ulong(), c.c_void_p()
    status = X.XGetWindowProperty(display, window, X.XInternAtom(display, b'_NET_WM_PID', 0), 0, 1, 0, 6, c.byref(kind), c.byref(format_), c.byref(count), c.byref(remaining), c.byref(data))
    try:
        if status == 0 and kind.value == 6 and format_.value == 32 and count.value == 1 and data:
            return c.cast(data, c.POINTER(c.c_ulong))[0]
    finally:
        if data:
            X.XFree(data)
class Attributes(c.Structure):
    _fields_ = [(k, c.c_int) for k in ('x', 'y', 'width', 'height', 'border_width', 'depth')] + [('visual', c.c_void_p), ('root', c.c_ulong)] + [(k, c.c_int) for k in ('kind', 'bit_gravity', 'win_gravity', 'backing_store')] + [('backing_planes', c.c_ulong), ('backing_pixel', c.c_ulong), ('save_under', c.c_int), ('colormap', c.c_ulong), ('map_installed', c.c_int), ('map_state', c.c_int), ('all_event_masks', c.c_long), ('your_event_mask', c.c_long), ('do_not_propagate_mask', c.c_long), ('override_redirect', c.c_int), ('screen', c.c_void_p)]
X.XGetWindowAttributes.argtypes = [c.c_void_p, c.c_ulong, c.POINTER(Attributes)]
class Aspect(c.Structure):
    _fields_ = [('x', c.c_int), ('y', c.c_int)]
class SizeHints(c.Structure):
    _fields_ = [('flags', c.c_long)] + [(k, c.c_int) for k in (
        'x', 'y', 'width', 'height', 'min_width', 'min_height',
        'max_width', 'max_height', 'width_inc', 'height_inc')]
    _fields_ += [('min_aspect', Aspect), ('max_aspect', Aspect)]
    _fields_ += [(k, c.c_int) for k in ('base_width', 'base_height', 'win_gravity')]
X.XGetWMNormalHints.argtypes = [c.c_void_p, c.c_ulong, c.POINTER(SizeHints), c.POINTER(c.c_long)]
X.XResizeWindow.argtypes = [c.c_void_p, c.c_ulong, c.c_uint, c.c_uint]
class ClientMessage(c.Structure):
    _fields_ = [('kind', c.c_int), ('serial', c.c_ulong), ('send_event', c.c_int), ('display', c.c_void_p), ('window', c.c_ulong), ('message_type', c.c_ulong), ('format', c.c_int), ('data', c.c_long * 5)]
class Event(c.Union):
    _fields_ = [('client', ClientMessage), ('padding', c.c_long * 24)]

def windows(parent=root):
    r, p, children, count = c.c_ulong(), c.c_ulong(), c.POINTER(c.c_ulong)(), c.c_uint()
    if not X.XQueryTree(display, parent, c.byref(r), c.byref(p), c.byref(children), c.byref(count)):
        return []
    result = list(children[:count.value])
    if children:
        X.XFree(children)
    all_windows = list(result)
    for child in result:
        all_windows.extend(windows(child))
    return all_windows

def app_window(pid):
    for win in windows():
        if window_pid(win) != pid:
            continue
        name = c.c_void_p()
        if X.XFetchName(display, win, c.byref(name)) and name.value:
            title = c.string_at(name).decode(errors='replace')
            X.XFree(name)
            if title.startswith('Clash of Rust'):
                attrs = Attributes()
                X.XGetWindowAttributes(display, win, c.byref(attrs))
                if attrs.map_state == 2:
                    return win, title, attrs.width, attrs.height
    return None

def check_window_size(pid, window, scale):
    hints, supplied = SizeHints(), c.c_long()
    assert X.XGetWMNormalHints(display, window, c.byref(hints), c.byref(supplied))
    minimum = (round(800 * scale), round(450 * scale))
    assert hints.flags & (1 << 4), 'Native minimum size is missing'
    assert (hints.min_width, hints.min_height) == minimum, (
        scale, (hints.min_width, hints.min_height), minimum, app_window(pid))
    # Physical widths must convert back to 800/950 logical pixels at either
    # scale. In particular, a stale 1200-pixel minimum must not block 950 at 1x.
    current = app_window(pid)
    widths = []
    for logical_width in (1100, 800, 950):
        requested = round(logical_width * scale)
        X.XResizeWindow(display, window, requested, current[3])
        X.XFlush(display)
        for _ in range(100):
            current = app_window(pid)
            if current and current[2] == requested:
                break
            time.sleep(0.05)
        assert current and current[2] == requested, (scale, requested, current)
        widths.append(current[2])
    return {'scale': scale, 'native_minimum': minimum, 'resized_widths': widths}


results = []
for scale, background in ((scale, background) for scale in args.scales for background in (False, True)):
    with tempfile.TemporaryDirectory(prefix='clash-wsl-gui-test-') as tmp:
        with socket.socket() as mixed, socket.socket() as controller:
            mixed.bind(('127.0.0.1', 0))
            controller.bind(('127.0.0.1', 0))
            mixed_port, controller_port = mixed.getsockname()[1], controller.getsockname()[1]
        Path(tmp, 'settings.json').write_text(json.dumps({'mixed_port': mixed_port, 'controller_port': controller_port, 'proxy_mode': 'off', 'secret': 'diagnostic-only'}))
        env = dict(os.environ, CLASH_OF_RUST_DATA_DIR=tmp, GSETTINGS_BACKEND='memory',
                   WINIT_X11_SCALE_FACTOR=str(scale))
        command = [str(executable)] + (['--background'] if background else [])
        proc = subprocess.Popen(command, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        try:
            window = None
            for _ in range(100):
                window = app_window(proc.pid)
                if window or proc.poll() is not None:
                    break
                time.sleep(0.1)
            assert window, 'No mapped window (background=%s)' % background
            app_class = window_class(window[0])
            assert app_class == ('clash-of-rust', 'clash-of-rust'), app_class
            time.sleep(3)
            assert proc.poll() is None, 'App exited before close'
            geometry = check_window_size(proc.pid, window[0], scale)
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            # The engine may persist its effective settings during startup.
            settings = json.loads(Path(tmp, 'settings.json').read_text())
            request = urllib.request.Request(f'http://127.0.0.1:{controller_port}/version', headers={'Authorization': 'Bearer ' + settings['secret']})
            with opener.open(request, timeout=3) as response:
                core_version = json.load(response)
            event = Event()
            event.client = ClientMessage(33, 0, 1, display, window[0], X.XInternAtom(display, b'WM_PROTOCOLS', 0), 32, (c.c_long * 5)(X.XInternAtom(display, b'WM_DELETE_WINDOW', 0), 0, 0, 0, 0))
            assert X.XSendEvent(display, window[0], 0, 0, c.byref(event))
            X.XFlush(display)
            if background:
                # Login startup keeps retrying instead of silently disappearing
                # when the tray host has not appeared. A second close confirms exit.
                time.sleep(1)
                assert proc.poll() is None, 'Background app exited while awaiting the tray host'
                assert app_window(proc.pid), 'No tray host: the fallback window must stay accessible'
                assert X.XSendEvent(display, window[0], 0, 0, c.byref(event))
                X.XFlush(display)
            out, err = proc.communicate(timeout=15)
            assert proc.returncode == 0, (proc.returncode, err)
            assert not err, err.decode(errors='replace')
            results.append({'background': background, 'geometry': geometry, 'mapped_window': window[1:], 'wm_class': app_class, 'core_version': core_version, 'close_exits': True, 'pending_tray_close_requires_confirmation': background, 'returncode': proc.returncode, 'stderr': err.decode()})
        finally:
            if proc.poll() is None:
                os.killpg(proc.pid, signal.SIGTERM)
                proc.communicate(timeout=10)
print(json.dumps(results, ensure_ascii=False, indent=2))
if args.output:
    args.output.write_text(json.dumps(results, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
