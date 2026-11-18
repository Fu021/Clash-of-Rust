"""Measure one selected checkout: isolated API workloads and reproducible GUI scenes."""
import argparse
from collections import Counter
import ctypes as c
import datetime
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import socket
import statistics
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid

from build_support import ROOT, write_json, core_directory, desktop_directory, package_version, desktop_cargo_scope, validate_resources
from memory_desktop import TrayHost, close_window, wait_until
from memory_lifecycle import comparisons, owner_pair, pagination_comparison
import memory_shards


WORKLOADS = {
    'light': {'nodes': 100, 'groups': 5, 'rules': 1000, 'api_rows': 1000, 'profiles': 16},
    'stress': {'nodes': 2000, 'groups': 40, 'rules': 20000, 'api_rows': 50000, 'profiles': 240},
}


def fixture(workload='light'):
    counts = WORKLOADS[workload]
    nodes = ', '.join(f'node-{index}' for index in range(counts['nodes']))
    return ('proxies:\n' + ''.join(
        f'  - {{name: node-{index}, type: http, server: 127.0.0.1, port: 1}}\n' for index in range(counts['nodes']))
        + 'proxy-groups:\n' + ''.join(
            f'  - name: group-{group}\n    type: select\n    proxies: [{nodes}]\n' for group in range(counts['groups']))
        + 'rules:\n' + ''.join(
            f'  - DOMAIN-SUFFIX,example-{index}.test,group-0\n' for index in range(counts['rules']))
        + '  - MATCH,DIRECT\n')


def window_pixels(window, env):
    x = c.CDLL('libX11.so.6')
    x.XOpenDisplay.argtypes = [c.c_char_p]
    x.XOpenDisplay.restype = c.c_void_p
    display = x.XOpenDisplay(env['DISPLAY'].encode())
    if not display:
        raise RuntimeError('Cannot open the benchmark X display')
    class Image(c.Structure):
        _fields_ = [('width', c.c_int), ('height', c.c_int), ('xoffset', c.c_int), ('format', c.c_int),
                    ('data', c.c_void_p), ('byte_order', c.c_int), ('bitmap_unit', c.c_int),
                    ('bitmap_bit_order', c.c_int), ('bitmap_pad', c.c_int), ('depth', c.c_int),
                    ('bytes_per_line', c.c_int), ('bits_per_pixel', c.c_int),
                    ('red_mask', c.c_ulong), ('green_mask', c.c_ulong), ('blue_mask', c.c_ulong)]
    geometry = dict(line.split('=', 1) for line in subprocess.check_output(
        ['xdotool', 'getwindowgeometry', '--shell', window], env=env, text=True).splitlines())
    width, height = int(geometry['WIDTH']), int(geometry['HEIGHT'])
    x.XGetImage.argtypes = [c.c_void_p, c.c_ulong, c.c_int, c.c_int, c.c_uint, c.c_uint, c.c_ulong, c.c_int]
    x.XGetImage.restype = c.POINTER(Image)
    image = x.XGetImage(display, int(window), 0, 0, width, height, c.c_ulong(-1).value, 2)
    x.XDestroyImage.argtypes = [c.POINTER(Image)]
    x.XCloseDisplay.argtypes = [c.c_void_p]
    try:
        if not image or image.contents.bits_per_pixel != 32:
            raise RuntimeError('Expected 32-bit Xvfb pixels')
        data = image.contents
        raw = c.string_at(data.data, data.bytes_per_line*height)
        rows = []
        for y in range(height):
            row = raw[y*data.bytes_per_line:y*data.bytes_per_line+width*4]
            rgb = bytearray(width*3)
            rgb[0::3], rgb[1::3], rgb[2::3] = row[2::4], row[1::4], row[0::4]
            rows.append(rgb)
        return width, height, b''.join(rows)
    finally:
        if image:
            x.XDestroyImage(image)
        x.XCloseDisplay(display)


def search_center(window, env):
    """Locate the legacy search field in memory without producing a screenshot."""
    width, height, raw = window_pixels(window, env)
    stride = width*3
    runs, start = [], None
    # At the default 950 px window, only the search input has this border
    # on the content's left edge. Card/notice borders use other colors.
    for y in range(70, min(height-60, 250)):
        pixel = raw[y*stride+184*3:y*stride+185*3]
        border = all(abs(a-b) <= 3 for a, b in zip(pixel, (100, 116, 139)))
        if border:
            if start is None:
                start = y
        elif start is not None:
            if y-start >= 15:
                runs.append((start, y))
            start = None
    if len(runs) != 1 or width != 950:
        raise RuntimeError('Cannot locate the search field: '+str(runs))
    return 380, sum(runs[0])//2


def client_fixture(data, workload, rule_overrides=None, pagination=None):
    env = dict(os.environ, GSETTINGS_BACKEND='memory', WINIT_UNIX_BACKEND='x11', WINIT_X11_SCALE_FACTOR='1')
    env.pop('WAYLAND_DISPLAY', None)
    (data/'profiles').mkdir()
    profile_id = str(uuid.uuid4())
    with socket.socket() as controller, socket.socket() as mixed:
        controller.bind(('127.0.0.1', 0))
        mixed.bind(('127.0.0.1', 0))
        controller_port, mixed_port = controller.getsockname()[1], mixed.getsockname()[1]
    settings = {'controller_port': controller_port, 'mixed_port': mixed_port,
                'secret': 'memory-benchmark', 'dark': True, 'delay_interval_minutes': 0,
                'update_check_interval_minutes': 0,
                'active_profile': profile_id, 'run_mode': 'rule', 'proxy_mode': 'off'}
    if rule_overrides is not None:
        settings['rule_overrides'] = rule_overrides
    if pagination is not None:
        key, paginated, profiles_list = pagination
        settings.update({key: paginated, 'subscription_update_interval_minutes': 0})
        write_json(data/'window-state.json', {'width': 950, 'height': 700, 'profiles_list': profiles_list})
    write_json(data/'settings.json', settings)
    (data/'profiles'/f'{profile_id}.yaml').write_text(fixture(workload), encoding='utf-8')
    profiles = [{'id': profile_id, 'name': 'Memory workload', 'updated': 0,
                 'source': str(data/'profiles'/f'{profile_id}.yaml')}]
    if pagination is not None and key == 'profiles_paginated':
        for index in range(1, WORKLOADS[workload]['profiles']):
            identifier = str(uuid.UUID(int=index))
            local = data/'profiles'/f'{identifier}.yaml'
            local.write_text('rules:\n  - MATCH,DIRECT\n', encoding='utf-8')
            profiles.append({'id': identifier, 'name': f'Memory profile {index:03}', 'updated': index,
                             'source': str(local)})
    write_json(data/'profiles.json', profiles)
    env['CLASH_OF_RUST_DATA_DIR'] = str(data)
    return env, controller_port


def core_running(port):
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    request = urllib.request.Request(f'http://127.0.0.1:{port}/version',
                                    headers={'Authorization': 'Bearer memory-benchmark'})
    try:
        with opener.open(request, timeout=1) as response:
            return bool(json.load(response).get('version'))
    except (OSError, ValueError):
        return False


def read_scene(path, seconds=None, role=None, frontend=None):
    samples = [row for line in path.read_text().splitlines()
               if 'elapsed_ms' in (row := json.loads(line))]
    if not samples:
        raise ValueError('No process memory samples: '+path.name)
    if seconds is not None and samples[-1]['elapsed_ms'] < max(0, seconds*1000-1000):
        raise ValueError('Measured client exited before the scene completed: '+path.name)
    if role:
        for row in samples:
            roles = {process['role'] for process in row['processes']}
            if role not in roles or 'core' not in roles or (role == 'daemon' and 'gui' in roles):
                raise ValueError('Client/core identity is incomplete: '+path.name)
    if frontend is not None:
        for row in samples:
            matches = [process for process in row['processes']
                       if process['role'] == 'tui' and process['pid'] == frontend]
            identities = [process['pid'] for process in row['processes']]
            if len(matches) != 1 or len(identities) != len(set(identities)):
                raise ValueError('TUI identity is missing or duplicated: '+path.name)
    return samples


def memory_maps(text, executable=None):
    """Aggregate Linux mappings without keeping addresses or user file paths."""
    totals, category = {}, None
    for line in text.splitlines():
        if re.match(r'^[0-9a-f]+-[0-9a-f]+ ', line):
            parts = line.split(maxsplit=5)
            name = parts[5] if len(parts) == 6 else ''
            category = ('heap' if name == '[heap]' else 'stack' if name.startswith('[stack')
                        else 'anonymous' if not name or name.startswith('[anon')
                        else 'shared-memory' if '/memfd:' in name or '/dev/shm/' in name
                        else 'other-special' if name.startswith('[')
                        else 'executable' if executable and name.removesuffix(' (deleted)') == executable
                        else 'font-files' if executable and name.lower().endswith(('.ttf','.otf','.ttc'))
                        else 'shared-libraries' if executable and re.search(r'\.so(?:\.|$)',name)
                        else 'file-backed')
            totals.setdefault(category, {'rss_bytes': 0, 'pss_bytes': 0, 'private_bytes': 0})
        elif category and ':' in line:
            field, value = line.split(':', 1)
            if field in ('Rss', 'Pss', 'Private_Clean', 'Private_Dirty'):
                key = 'private_bytes' if field.startswith('Private_') else field.lower()+'_bytes'
                totals[category][key] += int(value.split()[0])*1024
    return totals


def process_start_ticks(text):
    return int(text[text.rindex(')')+2:].split()[19])


def mapping_snapshot(processes, proc_root=Path('/proc')):
    roles, unavailable = {}, 0
    reasons = Counter()
    try:
        boot_time = next(int(line.split()[1]) for line in (proc_root/'stat').read_text().splitlines()
                         if line.startswith('btime '))
        ticks_per_second = os.sysconf('SC_CLK_TCK')
        if ticks_per_second <= 0:
            raise ValueError('Invalid clock frequency')
    except (OSError, ValueError, IndexError, StopIteration):
        return {'roles': {}, 'unavailable_processes': len(processes),
                'unavailable_reasons': {'clock_unavailable': len(processes)} if processes else {}}
    for process in processes:
        root = proc_root / str(process['pid'])
        try:
            start_ticks = process_start_ticks((root/'stat').read_text())
            if boot_time + start_ticks // ticks_per_second != process['start_time']:
                unavailable += 1
                reasons['process_identity_changed'] += 1
                continue
            categories = memory_maps((root/'smaps').read_text(), os.readlink(root/'exe'))
            if process_start_ticks((root/'stat').read_text()) != start_ticks:
                unavailable += 1
                reasons['process_identity_changed'] += 1
                continue
            if not categories:
                raise ValueError('Empty memory mapping')
        except (OSError, ValueError, IndexError) as error:
            unavailable += 1
            reasons['permission_denied' if isinstance(error, PermissionError)
                    else 'process_gone' if isinstance(error, (FileNotFoundError, ProcessLookupError))
                    else 'proc_read_failed' if isinstance(error, OSError)
                    else 'invalid_proc_data'] += 1
            continue
        for category, values in categories.items():
            totals = roles.setdefault(process['role'], {}).setdefault(category, dict.fromkeys(values, 0))
            for field, value in values.items():
                totals[field] += value
    return {'roles': roles, 'unavailable_processes': unavailable, 'unavailable_reasons': dict(reasons)}


def mapping_peaks(snapshots):
    peaks = {}
    for snapshot in snapshots:
        for role, categories in snapshot['roles'].items():
            for category, values in categories.items():
                peak = peaks.setdefault(role, {}).setdefault(category, dict.fromkeys(values, 0))
                for field, value in values.items():
                    peak[field] = max(peak[field], value)
    return peaks


def mapping_private_peaks(snapshots):
    peaks = {}
    for snapshot in snapshots:
        for role, categories in snapshot['roles'].items():
            value = sum(categories.get(category, {}).get('private_bytes', 0)
                        for category in ('anonymous', 'heap'))
            peaks[role] = max(peaks.get(role, 0), value)
    return peaks


def measure_process(benchmark, pid, name, output, seconds, env, mode='app', frontend=None, on_tick=None, settle_seconds=3):
    if settle_seconds:
        time.sleep(settle_seconds)
    samples_path = output/(name+'.jsonl')
    snapshots = []
    with (output/(name+'.txt')).open('x') as text:
        command = [str(benchmark), mode, str(pid), str(seconds), '250', str(samples_path)]
        if frontend is not None:
            command.append(str(frontend))
        process = subprocess.Popen(command, stdout=text, env=env)
        try:
            while True:
                if on_tick is not None:
                    on_tick()
                try:
                    status = process.wait(timeout=1)
                except subprocess.TimeoutExpired:
                    status = None
                if samples_path.exists():
                    lines = samples_path.read_text().splitlines()
                    for line in reversed(lines):
                        try:
                            row = json.loads(line)
                        except ValueError:
                            continue
                        if 'processes' in row and 'elapsed_ms' in row:
                            if not snapshots or snapshots[-1]['elapsed_ms'] != row['elapsed_ms']:
                                snapshots.append({'elapsed_ms': row['elapsed_ms'],
                                                  **mapping_snapshot(row['processes'])})
                            break
                if status is not None:
                    if status:
                        raise subprocess.CalledProcessError(status, command)
                    break
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
    rows = read_scene(samples_path, seconds, 'daemon' if mode == 'daemon' else 'gui', frontend=frontend)
    write_json(output/(name+'-mapping-peaks.json'), {
        'interval_ms': 1000, 'samples': len(snapshots), 'peak': mapping_peaks(snapshots),
        'anonymous_heap_private_peak': mapping_private_peaks(snapshots),
        'final': snapshots[-1] if snapshots else None,
        'unavailable_samples': sum(row['unavailable_processes'] > 0 for row in snapshots),
        'unavailable_reasons': dict(sum((Counter(row['unavailable_reasons']) for row in snapshots),
                                        Counter())),
    })
    with (output/(name+'-mapping-samples.jsonl')).open('x') as stream:
        for row in snapshots:
            stream.write(json.dumps(row)+'\n')
    final_mapping = mapping_snapshot(rows[-1]['processes'])
    maps = [{'role': role, 'categories': categories}
            for role, categories in final_mapping['roles'].items()]
    write_json(output/(name+'-mappings.json'), maps)
    return rows


def measure_closed(benchmark, pid, name, output, seconds, env, identity, mode='app'):
    rows = measure_process(benchmark, pid, name, output, memory_shards.scene_seconds(name, seconds),
                           env, mode=mode, settle_seconds=0)
    owner_pair(rows, 'gui' if mode == 'app' else 'daemon', identity, closed=True)


def owned_window(window, root_pid, env):
    try:
        pid = int(subprocess.check_output(['xdotool','getwindowpid',window],env=env,text=True).strip())
        seen = set()
        while pid > 1 and pid not in seen:
            if pid == root_pid: return True
            seen.add(pid)
            text = Path(f'/proc/{pid}/stat').read_text()
            pid = int(text[text.rindex(')')+2:].split()[1])
    except (OSError,ValueError,subprocess.CalledProcessError):
        return False
    return False


def renderer_running(pid):
    for task in Path(f'/proc/{pid}/task').iterdir():
        try:
            children = (task/'children').read_text().split()
        except FileNotFoundError:
            continue
        for child in children:
            try:
                if Path(os.readlink(f'/proc/{child}/exe')).name == 'clash-desktop-ui':
                    return True
            except (FileNotFoundError, ProcessLookupError):
                continue
    return False


def subscription_rows_visible(width, pixels):
    for top in (280, 380):
        bright = 0
        for vertical in range(top, top + 64):
            line = pixels[(vertical * width + 260) * 3:(vertical * width + 650) * 3]
            bright += sum(min(red, green, blue) >= 140
                          for red, green, blue in zip(line[0::3], line[1::3], line[2::3]))
            if bright >= 32:
                break
        else:
            return False
    return True


def pagination_navigation(window, env, click, proxies):
    row = 156 if proxies else 194
    click(70, row)
    subprocess.run(['xdotool', 'mousemove', '--window', window, '590', '500'], env=env, check=True)

    def selected():
        width, height, pixels = window_pixels(window, env)
        if (width, height) != (950, 700):
            raise RuntimeError('Unexpected pagination fixture window size')
        offset = (row * width + 195) * 3
        return (pixels[offset:offset+3] == bytes((30, 30, 29))
                and (proxies or subscription_rows_visible(width, pixels)))

    wait_until(selected, 'pagination fixture navigation and visible list content')


def gui_scenes(executable, benchmark, output, seconds, modern_layout=False, slint_layout=False,
               rule_overrides=None, prefix='', workload='light', tray=None, expect_destroy=False,
               pagination=None):
    with tempfile.TemporaryDirectory(prefix='memory-gui-') as directory, (output/(prefix+'gui.log')).open('x') as log:
        data = Path(directory)
        env, controller_port = client_fixture(data, workload, rule_overrides, pagination)
        app = subprocess.Popen([str(executable)], env=env, stdout=log, stderr=log, start_new_session=True)
        try:
            for _ in range(90):
                found = subprocess.run(['xdotool', 'search', '--name', '^Clash of Rust'],
                                       env=env, capture_output=True, text=True)
                windows = [w for w in found.stdout.split() if owned_window(w, app.pid, env)] if found.returncode == 0 else []
                if windows:
                    window = windows[0]
                    break
                if app.poll() is not None:
                    raise RuntimeError('GUI exited; see gui.log')
                time.sleep(1)
            else:
                raise RuntimeError('GUI window did not appear')
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            for _ in range(90):
                try:
                    request = urllib.request.Request(f'http://127.0.0.1:{controller_port}/version',
                              headers={'Authorization': 'Bearer memory-benchmark'})
                    with opener.open(request, timeout=1) as response:
                        json.load(response)
                    break
                except (OSError, ValueError):
                    if app.poll() is not None:
                        raise RuntimeError('GUI exited while waiting for core')
                    time.sleep(1)
            else:
                raise RuntimeError('Synthetic subscription did not start')
            def click(x, y):
                subprocess.run(['xdotool', 'mousemove', '--window', window, str(x), str(y), 'click', '1'],
                               env=env, check=True)
            def measure(name):
                return measure_process(benchmark, app.pid, prefix+name, output, seconds, env)
            if tray:
                wait_until(lambda: tray.registered(app.pid), 'measured application tray registration')

                def visible_windows():
                    if app.poll() is not None:
                        raise RuntimeError('Close exited the GUI instead of retaining the tray')
                    result = subprocess.run(['xdotool', 'search', '--onlyvisible',
                                             '--name', '^Clash of Rust'], env=env, capture_output=True, text=True)
                    if result.returncode not in (0, 1):
                        raise RuntimeError('Could not inspect window visibility')
                    return [item for item in result.stdout.split() if owned_window(item, app.pid, env)] if result.returncode == 0 else []

                def hidden():
                    return not visible_windows() and core_running(controller_port)

                destroyed = []
                def close():
                    current = visible_windows()[0]
                    close_window(current, env)
                    wait_until(hidden, 'window closed with the same client and core still alive')
                    def gone():
                        return subprocess.run(['xdotool', 'getwindowname', current], env=env,
                                              capture_output=True).returncode != 0
                    if expect_destroy:
                        wait_until(gone, 'native window destruction')
                    if (executable.parent/'clash-desktop-ui').is_file():
                        wait_until(lambda: not renderer_running(app.pid), 'renderer exit')
                    destroyed.append(gone())

                reopen_times = []
                def reopen():
                    nonlocal window
                    started = time.monotonic()
                    subprocess.run([str(executable)], env=env, stdout=log, stderr=log,
                                   check=True, timeout=20)
                    wait_until(lambda: visible_windows() and core_running(controller_port), 'restored window')
                    window = visible_windows()[0]
                    reopen_times.append(round((time.monotonic()-started)*1000, 1))

                if pagination is not None:
                    key, paginated, profiles_list = pagination

                    def visit():
                        pagination_navigation(window, env, click, key == 'proxy_groups_paginated')

                    def scroll(button='5'):
                        subprocess.run(['xdotool', 'click', '--repeat', '480', '--delay', '1', button],
                                       env=env, check=True)

                    visit()
                    identity = owner_pair(measure('visible'), 'gui')
                    scroll()
                    owner_pair(measure('scrolled'), 'gui', identity)
                    close()
                    measure_closed(benchmark, app.pid, prefix+'hidden', output, seconds, env, identity)
                    for _ in range(20):
                        reopen()
                        visit()
                        scroll('4')
                        scroll()
                        close()
                    measure_closed(benchmark, app.pid, prefix+'cycles-hidden', output, seconds, env, identity)
                    if not hidden() or not tray.registered(app.pid):
                        raise RuntimeError('Tray or core unavailable after the pagination comparison')
                    if json.loads((data/'settings.json').read_text()).get(key) is not paginated:
                        raise RuntimeError('Pagination fixture setting changed during the measurement')
                    if json.loads((data/'window-state.json').read_text()).get('profiles_list') is not profiles_list:
                        raise RuntimeError('Subscription fixture view changed during the measurement')
                    write_json(output/(prefix+'proof.json'), {
                        'setting': key, 'paginated': paginated, 'profiles_list': profiles_list,
                        'owner_and_core': identity, 'repeat_cycles': 20, 'native_window_destroyed': destroyed,
                        'relaunch_to_visible_ms': reopen_times,
                    })
                    return

                before = measure('tray-open')
                identity = owner_pair(before, 'gui')
                close()
                measure_closed(benchmark, app.pid, prefix+'tray-hidden', output, seconds, env, identity)
                if not hidden():
                    raise RuntimeError('Client/core did not remain in the background during sampling')
                reopen()
                owner_pair(measure('tray-reopened'), 'gui', identity)
                click(70, 156)
                subprocess.run(['xdotool', 'windowfocus', '--sync', window, 'key', '--clearmodifiers', 'ctrl+f'], env=env, check=True)
                time.sleep(.5)
                subprocess.run(['xdotool', 'type', '--delay', '100', '--clearmodifiers', 'node-'], env=env, check=True)
                owner_pair(measure('tray-search'), 'gui', identity)
                close()
                measure_closed(benchmark, app.pid, prefix+'tray-search-hidden', output, seconds, env, identity)
                reopen()
                owner_pair(measure('tray-search-reopened'), 'gui', identity)
                for _ in range(20):
                    close()
                    reopen()
                final = measure('tray-after-cycles')
                owner_pair(final, 'gui', identity)
                close()
                measure_closed(benchmark, app.pid, prefix+'tray-after-cycles-hidden', output, seconds, env, identity)
                if not hidden() or not tray.registered(app.pid):
                    raise RuntimeError('Tray or core unavailable after the final close')
                write_json(output/(prefix+'proof.json'), {
                    'gui_pid': app.pid, 'core_pids': [identity[1][0]], 'owner_and_core': identity,
                    'tray_registered': True, 'close_request': 'WM_DELETE_WINDOW',
                    'hidden_during_measurement': True, 'reopened': True,
                    'repeat_cycles': 20, 'native_window_destroyed': destroyed,
                    'renderer_exit_verified': True,
                    'relaunch_to_visible_ms': reopen_times,
                })
                return
            measure('home')
            click(70, 156 if slint_layout else 126 if modern_layout else 108)
            measure('proxies-collapsed')
            if slint_layout:
                subprocess.run(['xdotool', 'windowfocus', '--sync', window], env=env, check=True)
                subprocess.run(['xdotool', 'key', '--clearmodifiers', 'ctrl+f'], env=env, check=True)
                time.sleep(0.5)
            elif modern_layout:
                click(*search_center(window, env))
            else:
                click(380, 120)
            subprocess.run(['xdotool', 'windowfocus', '--sync', window], env=env, check=True)
            subprocess.run(['xdotool', 'type', '--delay', '100', '--clearmodifiers', 'node-'], env=env, check=True)
            measure('proxies-search')
            click(70, 112 if slint_layout else 83 if modern_layout else 70)
            measure('home-after-proxies')
        finally:
            # All tests use proxy off. Stop this owned process group, including
            # the bundled core, even if a measurement fails.
            try:
                os.killpg(app.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                app.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(app.pid, signal.SIGKILL)
                app.wait()


def headless_scene(console, resources, benchmark, output, seconds, workload, rule_overrides=None, prefix='background-', tui=False, native_settings=False, tui_catalog=False):
    with tempfile.TemporaryDirectory(prefix='memory-cli-') as directory, (output/(prefix+'cor.log')).open('x') as log:
        data = Path(directory)
        env, port = client_fixture(data, workload, rule_overrides)
        for key in ('DISPLAY', 'WAYLAND_DISPLAY', 'DBUS_SESSION_BUS_ADDRESS'):
            env.pop(key, None)
        command = [str(console), '--json', '--yes', '--timeout', '30',
                   '--data-dir', str(data), '--resources', str(resources)]
        pid = identity = None
        terminal_scenes = []
        try:
            subprocess.run([*command, 'start'], env=env, stdout=log, stderr=log, check=True, timeout=90)
            endpoint = json.loads((data/'control.json').read_text())
            if endpoint['owner'] != 'daemon':
                raise RuntimeError('CLI did not create a headless runtime')
            pid = endpoint['pid']
            identity = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[19]
            wait_until(lambda: core_running(port), 'headless core readiness')
            baseline = measure_process(benchmark, pid, prefix+'headless', output, seconds, env, mode='daemon')
            pair = owner_pair(baseline, 'daemon', closed=True)
            if tui:
                from cli_tui import Session
                with Session([console, '--data-dir', data, '--resources', resources, 'ui'], env=env) as terminal:
                    terminal.wait('参考线路')
                    pages = [('home', terminal.page_key('首页'), '参考线路'),
                             ('nodes', terminal.page_key('节点'), '策略组'),
                             ('connections', terminal.page_key('连接'), '活跃连接'),
                             ('network', terminal.page_key('工具', '网络'), '网站检查')]
                    if native_settings:
                        pages += [('settings', terminal.page_key('设置'), '设置 · Enter'),
                                  ('setting-editor', b'/ipv6\r\r', r'设置 · (?:启用 )?IPv6')]
                    if tui_catalog:
                        pages += [('profiles', b'\x1b'+terminal.page_key('订阅'), '订阅 · Enter'),
                                  ('shortcuts', b'', '快捷规则 · 当前订阅')]
                    for page, key, ready in pages:
                        if page == 'shortcuts':
                            terminal.open_shortcuts()
                        else:
                            terminal.send(key)
                        terminal.wait(ready, regex=page == 'setting-editor')
                        if tui_catalog and page == 'shortcuts':
                            terminal.wait('广告拦截：')
                        name = prefix+'tui-'+page
                        terminal.capture(output/'terminal-previews', name)
                        measured = measure_process(benchmark, pid, name, output, seconds, env,
                                                   mode='daemon', frontend=terminal.process.pid,
                                                   on_tick=lambda: terminal.pump(0))
                        owner_pair(measured, 'daemon', pair)
                        terminal_scenes.append(name)
                current = json.loads((data/'control.json').read_text())
                if current['pid'] != pid or not core_running(port):
                    raise RuntimeError('Closing TUI changed the daemon or stopped the core')
                name = prefix+'tui-closed'
                measure_closed(benchmark, pid, name, output, seconds, env, pair, mode='daemon')
                terminal_scenes.append(name)
                if tui_catalog:
                    with Session([console, '--data-dir', data, '--resources', resources, 'ui'], env=env) as terminal:
                        terminal.wait('参考线路')
                        terminal.open_shortcuts()
                        terminal.wait('快捷规则 · 当前订阅')
                        terminal.wait('广告拦截：')
                        name = prefix+'tui-reopened'
                        terminal.capture(output/'terminal-previews', name)
                        reopened = measure_process(benchmark, pid, name, output, seconds, env,
                                                   mode='daemon', frontend=terminal.process.pid,
                                                   on_tick=lambda: terminal.pump(0))
                        owner_pair(reopened, 'daemon', pair)
                        terminal_scenes.append(name)
                    name = prefix+'tui-reclosed'
                    measure_closed(benchmark, pid, name, output, seconds, env, pair, mode='daemon')
                    terminal_scenes.append(name)
                    if json.loads((data/'control.json').read_text())['pid'] != pid:
                        raise RuntimeError('Reopening TUI replaced the daemon')
                for cycle in range(20):
                    with Session([console, '--data-dir', data, '--resources', resources, 'ui'], env=env) as terminal:
                        terminal.wait('参考线路')
                        terminal.send(terminal.page_key('节点'))
                        terminal.wait('group-0')
                        if cycle == 19:
                            name = prefix+'tui-after-cycles'
                            measured = measure_process(benchmark, pid, name, output, seconds, env,
                                                       mode='daemon', frontend=terminal.process.pid,
                                                       on_tick=lambda: terminal.pump(0))
                            owner_pair(measured, 'daemon', pair)
                            terminal_scenes.append(name)
                name = prefix+'tui-after-cycles-closed'
                measure_closed(benchmark, pid, name, output, seconds, env, pair, mode='daemon')
                terminal_scenes.append(name)
            if not core_running(port) or (data/'gui-session.json').exists():
                raise RuntimeError('Headless measurement unexpectedly stopped or created a GUI')
            write_json(output/(prefix+'headless-proof.json'), {
                'daemon_pid': pid, 'owner': endpoint['owner'], 'display': False,
                'gui_session': False, 'core_running': True,
                'tui_scenes': terminal_scenes, 'tui_closed_keeps_daemon': bool(tui),
                'owner_and_core': pair,
            })
            return terminal_scenes
        finally:
            try:
                subprocess.run([*command, 'stop'], env=env, stdout=log, stderr=log, check=True, timeout=45)
            finally:
                # A failed stop must not leak this measurement's detached group.
                # Verify PID birth time and group ownership before fallback cleanup.
                if pid is not None:
                    try:
                        current = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[19]
                        if current == identity and os.getpgid(pid) == pid:
                            os.killpg(pid, signal.SIGTERM)
                    except (FileNotFoundError, ProcessLookupError):
                        pass


def summarize(output, metadata):
    rows = [json.loads(line) for line in (output/'api.jsonl').read_text().splitlines()]
    api = [row for row in rows if row.get('type') == 'api']
    lines = ['# Memory benchmark', '', f"Source: `{metadata['source_commit']}`; version: `{metadata['version']}`; profile: `{metadata['profile']}`.", '',
             'API values are extra live Rust heap, excluding the fixture/runtime baseline. Each cell is the median of independent processes; refresh=5 retains the old snapshot while replacing it.', '',
             '| Workload | Mode | Refreshes | Peak MiB | Retained MiB |', '| --- | --- | ---: | ---: | ---: |']
    for workload in ('rules', 'connections', 'proxies'):
        for mode in ('streamed', 'buffered'):
            for refresh in (1, 5):
                samples = [row for row in api if (row['workload'], row['mode'], row['refreshes']) == (workload, mode, refresh)]
                if len(samples) != metadata['repetitions']:
                    raise ValueError('Benchmark repetition count is incomplete')
                peak, retained = (statistics.median(row[key] for row in samples)/1048576 for key in ('peak_extra_heap_bytes', 'retained_extra_heap_bytes'))
                lines.append(f'| {workload} | {mode} | {refresh} | {peak:.2f} | {retained:.2f} |')
    counts = metadata['gui_fixture']
    lines += ['', f"Workload: `{metadata['workload']}`; API rows: {metadata['rows']:,}. GUI subscription: {counts['nodes']:,} nodes, {counts['groups']:,} groups × {counts['nodes']:,} members, {counts['rules']:,} rules. Proxy off; scheduled delay checks off; no business traffic. Xvfb/X11; snapshots show the scene used."]
    scenes = ['home', 'proxies-collapsed', 'proxies-search', 'home-after-proxies']
    if metadata.get('additional_gui_rules'):
        lines += ['', 'rules-* scenes enable all five subscription overrides with the subscription fallback unchanged in a fresh client/core process.']
        scenes += ['rules-'+scene for scene in scenes.copy()]
    lines += ['', '| Scene | GUI RSS peak MiB | Core RSS peak MiB | Total PSS peak MiB |', '| --- | ---: | ---: | ---: |']
    for scene in scenes:
        rows = [json.loads(line) for line in (output/(scene+'.jsonl')).read_text().splitlines()]
        samples = [row for row in rows if 'elapsed_ms' in row]
        values = []
        for role, field in (('gui', 'rss_bytes'), ('core', 'rss_bytes'), ('total', 'pss_bytes')):
            measured = [row[role][field] for row in samples if row[role][field] is not None]
            values.append(f'{max(measured)/1048576:.2f}' if measured else 'unknown')
        lines.append(f'| {scene} | '+ ' | '.join(values) + ' |')
    if metadata.get('background_scenes'):
        lines += ['', 'Background measurements use separate fresh clients and the same subscription. '
                  'The tray pair has an isolated StatusNotifier host; close sends WM_DELETE_WINDOW, '
                  'then checks that the window is hidden, the original core remains alive, and the app can reopen. '
                  'Headless uses cor start without DISPLAY, Wayland or a session bus. '
                  'Each tray variant also searches nodes and completes 20 close/reopen cycles. '
                  'Proof files record native window destruction and relaunch-to-visible times (not first-paint latency). '
                  'Mapping files separate anonymous, heap and file-backed pages without exposing paths. '
                  'Fixture hosts and the observer are outside the measured process trees.', '',
                  '| Scene | Client | Client RSS peak MiB | Core RSS peak MiB | Total PSS peak MiB |',
                  '| --- | --- | ---: | ---: | ---: |']
        for scene in metadata['background_scenes']:
            role = 'daemon' if scene.endswith('headless') else 'gui'
            samples = read_scene(output/(scene+'.jsonl'), metadata['gui_sample_seconds'], role)
            values = [f"{max(row[key][field] for row in samples)/1048576:.2f}"
                      for key, field in ((role, 'rss_bytes'), ('core', 'rss_bytes'), ('total', 'pss_bytes'))]
            lines.append(f'| {scene} | {"cor" if role == "daemon" else "GUI"} | '+ ' | '.join(values)+' |')
    if metadata.get('terminal_scenes'):
        lines += ['', 'TUI scenes keep the same daemon/core and fixture as their preceding headless baseline. '
                  'The PTY driver is outside the measured trees; the frontend is an additional identified cor process. '
                  'Total PSS sums each process once; RSS may double-count shared executable pages.',
                  '', '| Scene | TUI RSS MiB | Daemon RSS MiB | Core RSS MiB | Total PSS MiB |',
                  '| --- | ---: | ---: | ---: | ---: |']
        for scene in metadata['terminal_scenes']:
            samples = read_scene(output/(scene+'.jsonl'), metadata['gui_sample_seconds'], 'daemon')
            values = [f"{max(row[key][field] for row in samples)/1048576:.2f}"
                      for key, field in (('tui', 'rss_bytes'), ('daemon', 'rss_bytes'),
                                         ('core', 'rss_bytes'), ('total', 'pss_bytes'))]
            lines.append('| '+scene+' | '+' | '.join(values)+' |')
    lines += ['', 'Mapping peaks are sampled separately every second; each category/metric peak can occur at a different time. Missing processes are reported rather than counted as zero.',
              '', '| Scene | Role | Anonymous + heap private peak MiB | Final MiB | Missing samples |',
              '| --- | --- | ---: | ---: | ---: |']
    for path in sorted(output.glob('*-mapping-peaks.json')):
        mapping = json.loads(path.read_text())
        if mapping.get('unavailable_reasons'):
            lines.append(f"| {path.name.removesuffix('-mapping-peaks.json')} | unavailable: "
                         f"{json.dumps(mapping['unavailable_reasons'], sort_keys=True)} | unknown | unknown | "
                         f"{mapping['unavailable_samples']} |")
        for role, categories in mapping['peak'].items():
            peak = mapping['anonymous_heap_private_peak'][role]
            final = (mapping.get('final') or {}).get('roles', {}).get(role)
            retained = (f"{sum(final.get(category, {}).get('private_bytes', 0) for category in ('anonymous', 'heap'))/1048576:.2f}"
                        if final is not None else 'unknown')
            lines.append(f"| {path.name.removesuffix('-mapping-peaks.json')} | {role} | {peak/1048576:.2f} | {retained} | {mapping['unavailable_samples']} |")
    lines += ['', 'Client GUI values include both the native host and all renderer processes when present. GUI peaks are observed at 250 ms intervals, not exact allocator peaks. RSS can count shared pages twice. Compare separate runs only with the same toolchain, build profile, inputs and environment. Core memory also varies independently of the Rust GUI.', '']
    if metadata.get('shards') and metadata.get('capabilities', {}).get('cli') and metadata['capabilities']['slint']:
        prefixes = ['background-'] + (['rules-background-'] if metadata.get('additional_gui_rules') else [])
        deltas = comparisons(output, prefixes, read_scene)
        write_json(output/'closed-residency.json', deltas)
        lines += ['## Closed residency', '',
                  '| Scene | Client PSS change on close (MiB) | Client PSS gap to pure CLI (MiB) | Core PSS gap (MiB) |',
                  '| --- | ---: | ---: | ---: |']
        for prefix, variant in deltas.items():
            for name, pair in variant['pairs'].items():
                change = pair['change_on_close']['client']['pss_bytes']['bytes'] / 1048576
                client_gap = pair['gap_to_cli']['client']['pss_bytes']['bytes'] / 1048576
                core_gap = pair['gap_to_cli']['core']['pss_bytes']['bytes'] / 1048576
                lines.append(f'| {prefix}{name} | {change:+.2f} | {client_gap:+.2f} | {core_gap:+.2f} |')
        lines += ['', 'Full RSS, PSS and USS differences and percentages are in `closed-residency.json`. '
                  'The JSON records samples near 0, 10, 30 and 60 seconds from the start of closed sampling '
                  'and includes final anonymous/heap and executable mappings. GUI and pure CLI run on separate shards.', '']
    pages = memory_shards.pagination_cases(metadata.get('capabilities', {}))
    if pages:
        pagination = {page: pagination_comparison(output, 'pagination-' + page,
                                                  memory_shards.PAGINATION_PHASES, read_scene)
                      for page in pages}
        write_json(output/'pagination-comparison.json', pagination)
        lines += ['## Pagination comparison', '',
                  '| Page | Phase | Paged client PSS (MiB) | All client PSS (MiB) | All minus paged (MiB) |',
                  '| --- | --- | ---: | ---: | ---: |']
        for page, phases in pagination.items():
            for phase, metrics in phases.items():
                paged = metrics['paged']['final']['client']['pss_bytes'] / 1048576
                full = metrics['all']['final']['client']['pss_bytes'] / 1048576
                lines.append(f'| {page} | {phase} | {paged:.2f} | {full:.2f} | {full-paged:+.2f} |')
        lines += ['', 'Each pair uses freshly started clients with identical inputs on the same runner. '
                  'Scrolling uses wheel events on the active page. Group pagination changes the outer group list. '
                  'See `pagination-comparison.json` for final and peak RSS, PSS and USS, mapping deltas and percentages.', '']
    if metadata.get('shards'):
        lines += ['## Measurement durations', '', '| Step | Seconds |', '| --- | ---: |',
                  f"| Prepare and API | {metadata['prepare_seconds']:.1f} |"]
        for shard, receipt in metadata['shards'].items():
            elapsed = datetime.datetime.fromisoformat(receipt['finished_at']) - datetime.datetime.fromisoformat(receipt['started_at'])
            lines.append(f'| {shard} | {elapsed.total_seconds():.1f} |')
        lines += [f"| Verify and combine | {metadata['combine_seconds']:.1f} |", '',
                  'These timings cover the scripts. Use the Actions run and job timestamps for queue and workflow elapsed time.', '']
    report = '\n'.join(lines)
    (output/'README.md').write_text(report, encoding='utf-8')
    if os.environ.get('GITHUB_STEP_SUMMARY'):
        with Path(os.environ['GITHUB_STEP_SUMMARY']).open('a', encoding='utf-8') as summary:
            summary.write(report)


def prepare(args, directory, output):
    started = time.monotonic()
    counts = WORKLOADS[args.workload]
    if args.rows is None:
        args.rows = counts['api_rows']
    if not (1 <= args.rows <= 50000 and 1 <= args.repetitions <= 10 and 1 <= args.seconds <= 120):
        raise ValueError('Bounds: rows 1..50000, repetitions 1..10, GUI seconds 1..120')
    source = args.source.resolve(strict=True)
    bundle = directory / 'payload'
    bundle.mkdir(parents=True, exist_ok=False)
    output.mkdir(parents=True, exist_ok=False)
    benchmark_source = core_directory()/'examples/memory_benchmark.rs'
    destination = core_directory(source)/'examples/memory_benchmark.rs'
    if destination.resolve() != benchmark_source.resolve():
        destination.parent.mkdir(parents=True, exist_ok=True)
        text = benchmark_source.read_text()
        if core_directory(source) == source:
            text = text.replace('clash_client_core::', 'clash_of_rust::')
        destination.write_text(text)
    metadata = {'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=source, text=True).strip(),
                'benchmark_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                'version': package_version(source),
                'profile': args.profile, 'workload': args.workload,
                'gui_fixture': {key: counts[key] for key in ('nodes', 'groups', 'rules')},
                'rows': args.rows, 'repetitions': args.repetitions,
                'gui_sample_seconds': args.seconds, 'interval_ms': 250,
                'os': platform.platform(), 'cpu_count': os.cpu_count(),
                'rustc': subprocess.check_output(['rustc', '-Vv'], text=True).strip(),
                'started_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                'instrumentation': 'The workflow benchmark example replaces only the example in the selected checkout; application sources are unchanged.'}
    desktop = desktop_directory(source)
    slint_layout = (desktop/'src/application/bindings.rs').is_file()
    modern_layout = slint_layout or (desktop/'src/app/mod.rs').is_file() or (desktop/'src/ui_style.rs').is_file()
    metadata['gui_navigation'] = 'current sidebar coordinates and detected search field' if modern_layout else 'legacy window coordinates'
    write_json(output/'metadata.json', metadata)
    (output/'gui-fixture.yaml').write_text(fixture(args.workload), encoding='utf-8')
    command = ['cargo', 'build', '--locked', '--bins', '--example', 'memory_benchmark']
    if desktop != source:
        command.extend(desktop_cargo_scope(source))
    if args.profile == 'release':
        command.append('--release')
    with (output/'build.log').open('x') as log:
        subprocess.run(command, cwd=source, stdout=log, stderr=subprocess.STDOUT, check=True)
    from action_cache import compiler_cache_checkpoint
    compiler_cache_checkpoint(source)
    subprocess.run([sys.executable, source/'scripts/prepare-resources.py', '--system', 'linux', '--arch', 'x64'], cwd=source, check=True)
    resources = source/'bundle/linux-x64/resources'
    validate_resources(resources, 'linux')
    for name in ('core.json', 'geodata.json'):
        shutil.copy2(resources/name, output/name)
    metadata['resources'] = {name: json.loads((resources/name).read_text()) for name in ('core.json', 'geodata.json')}
    write_json(output/'metadata.json', metadata)
    target = source/'target'/('release' if args.profile == 'release' else 'debug')
    shutil.copytree(resources, bundle/'resources')
    for name in ('clash-of-rust', 'clash-tun-launcher', *(['clash-desktop-ui'] if (source/'apps/desktop-ui/Cargo.toml').is_file() else [])):
        shutil.copy2(target/name, bundle/name)
    benchmark = bundle/'memory_benchmark'
    shutil.copy2(target/'examples/memory_benchmark', benchmark)
    with (output/'api.txt').open('x') as text:
        subprocess.run([str(benchmark), 'suite', str(args.rows), str(args.repetitions), str(output/'api.jsonl')],
                       cwd=source, stdout=text, check=True)
    settings_source = (core_directory(source)/'src/config/mod.rs' if desktop != source else source/'src/config.rs')
    tui_root = source/'apps/cli/src/tui'
    capabilities = dict(modern=modern_layout, slint=slint_layout,
                        cli=(source/'apps/cli/Cargo.toml').is_file(), tui=(tui_root/'mod.rs').is_file(),
                        settings=(tui_root/'settings.rs').is_file(), catalog=(tui_root/'catalog.rs').is_file(),
                        overrides='pub rule_overrides:' in settings_source.read_text(encoding='utf-8'),
                        destroy=slint_layout and any(marker in (desktop/'src/application/bindings.rs').read_text()
                                                     for marker in ('fn retire_window(', 'struct Renderer')))
    pages = source/'crates/client-ui/ui/pages'
    for kind, file, marker in (('group_pagination', 'proxies.slint', 'Intent.group-pagination'),
                               ('profile_pagination', 'subscriptions.slint', 'Intent.profile-pagination')):
        page = pages/file
        capabilities[kind] = slint_layout and page.is_file() and marker in page.read_text()
    if capabilities['cli']:
        shutil.copy2(target/'cor', bundle/'cor')
        version = subprocess.check_output([str(bundle/'cor'), '--json', 'version'], text=True)
        if json.loads(version)['data']['version'] != metadata['version']:
            raise ValueError('Headless executable differs from the selected source version')
    metadata['capabilities'] = capabilities
    if capabilities['overrides']:
        metadata['additional_gui_rules'] = {'enabled': [True]*5}
    if capabilities['profile_pagination']:
        metadata['pagination_profiles'] = counts['profiles']
    metadata['prepare_seconds'] = time.monotonic() - started
    digest = memory_shards.freeze(directory, metadata, capabilities)
    shutil.copy2(directory/'manifest.json', output/'manifest.json')
    return digest


def measure_shard(directory, output, shard, digest):
    manifest = memory_shards.manifest_for(directory, digest)
    capabilities, metadata = manifest['capabilities'], manifest['metadata']
    if shard not in manifest['shards']:
        raise ValueError('Unscheduled memory shard')
    memory_shards.check_payload(directory, manifest)
    output.mkdir(parents=True, exist_ok=False)
    bundle = directory/'payload'
    benchmark = bundle/'memory_benchmark'
    prefix = 'rules-' if shard.startswith('rules-') else ''
    overrides = metadata['additional_gui_rules'] if prefix else None
    background = prefix+'background-'
    seconds, workload = metadata['gui_sample_seconds'], metadata['workload']
    started = datetime.datetime.now(datetime.timezone.utc)
    if shard.startswith('pagination-'):
        page = shard.removeprefix('pagination-')
        setting, profiles_list = memory_shards.pagination_cases(capabilities)[page]
        tray_output = output/'tray-hosts'/shard
        tray_output.mkdir(parents=True)
        with TrayHost(tray_output) as tray:
            for mode in ('paged', 'all'):
                gui_scenes(bundle/'clash-of-rust', benchmark, output, seconds, True, True,
                           prefix=f'{shard}-{mode}-', workload=workload, tray=tray,
                           expect_destroy=capabilities['destroy'], pagination=(setting, mode == 'paged', profiles_list))
    elif shard.endswith('-cli'):
        headless_scene(bundle/'cor', bundle/'resources', benchmark, output, seconds, workload, overrides,
                       background, capabilities['tui'], capabilities['settings'], capabilities['catalog'])
    else:
        gui_scenes(bundle/'clash-of-rust', benchmark, output, seconds, capabilities['modern'],
                   capabilities['slint'], overrides, prefix, workload)
        if capabilities['slint']:
            tray_output = output/'tray-hosts'/shard
            tray_output.mkdir(parents=True)
            with TrayHost(tray_output) as tray:
                gui_scenes(bundle/'clash-of-rust', benchmark, output, seconds, capabilities['modern'],
                           True, overrides, background, workload, tray, capabilities['destroy'])
    write_json(output/'receipt.json', dict(shard=shard, digest=digest, scenes=manifest['shards'][shard],
                                           runner=memory_shards.runner(), started_at=started.isoformat(),
                                           finished_at=datetime.datetime.now(datetime.timezone.utc).isoformat()))


def merge(output, shards, digest):
    started = time.monotonic()
    manifest = memory_shards.manifest_for(output, digest)
    metadata = memory_shards.combine(manifest, digest, shards, output, read_scene)
    metadata['combine_seconds'] = time.monotonic() - started
    write_json(output/'metadata.json', metadata)
    summarize(output, metadata)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--stage', choices=('all', 'prepare', 'measure', 'merge'), default='all')
    parser.add_argument('--source', type=Path)
    parser.add_argument('--inputs', type=Path)
    parser.add_argument('--digest')
    parser.add_argument('--shard')
    parser.add_argument('--shards', type=Path)
    parser.add_argument('--profile', choices=('dev', 'release'), default='release')
    parser.add_argument('--workload', choices=WORKLOADS, default='light')
    parser.add_argument('--rows', type=int)
    parser.add_argument('--repetitions', type=int, default=3)
    parser.add_argument('--seconds', type=int, default=10)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    if args.stage in ('all', 'prepare') and not args.source:
        parser.error('--source is required to prepare a benchmark')
    if args.stage == 'all':
        with tempfile.TemporaryDirectory() as scratch:
            directory = Path(scratch)
            digest = prepare(args, directory/'inputs', output)
            manifest = memory_shards.manifest_for(output, digest)
            for shard in manifest['shards']:
                measure_shard(directory/'inputs', directory/'shards'/f'memory-scene-{digest}-{shard}', shard, digest)
            merge(output, directory/'shards', digest)
    elif args.stage == 'prepare' and args.inputs:
        prepare(args, args.inputs.resolve(), output)
    elif args.stage == 'measure' and args.inputs and args.digest and args.shard:
        measure_shard(args.inputs.resolve(), output, args.shard, args.digest)
    elif args.stage == 'merge' and args.shards and args.digest:
        merge(output, args.shards, args.digest)
    else:
        parser.error('Stage requires its input directory, digest and shard selection')


if __name__ == '__main__':
    main()
