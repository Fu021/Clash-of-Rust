"""Measure one selected checkout: isolated API workloads and reproducible GUI scenes."""
import argparse
import ctypes as c
import datetime
import json
import os
from pathlib import Path
import platform
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

from build_support import ROOT, write_json, core_directory, desktop_directory, package_version
from memory_desktop import TrayHost, close_window, wait_until


WORKLOADS = {
    'light': {'nodes': 100, 'groups': 5, 'rules': 1000, 'api_rows': 1000},
    'stress': {'nodes': 2000, 'groups': 40, 'rules': 20000, 'api_rows': 50000},
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


def client_fixture(data, workload, rule_overrides=None):
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
                'active_profile': profile_id, 'run_mode': 'rule', 'proxy_mode': 'off'}
    if rule_overrides is not None:
        settings['rule_overrides'] = rule_overrides
    write_json(data/'settings.json', settings)
    (data/'profiles'/f'{profile_id}.yaml').write_text(fixture(workload), encoding='utf-8')
    write_json(data/'profiles.json', [{'id': profile_id, 'name': 'Memory workload', 'updated': 0,
               'source': str(data/'profiles'/f'{profile_id}.yaml')}])
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


def read_scene(path, seconds=None, role=None):
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
    return samples


def measure_process(benchmark, pid, name, output, seconds, env, mode='app'):
    time.sleep(3)
    with (output/(name+'.txt')).open('x') as text:
        subprocess.run([str(benchmark), mode, str(pid), str(seconds), '250',
                        str(output/(name+'.jsonl'))], stdout=text, check=True, env=env)
    return read_scene(output/(name+'.jsonl'), seconds, 'daemon' if mode == 'daemon' else 'gui')


def gui_scenes(executable, benchmark, output, seconds, modern_layout=False, slint_layout=False, rule_overrides=None, prefix='', workload='light', tray=None):
    with tempfile.TemporaryDirectory(prefix='memory-gui-') as directory, (output/(prefix+'gui.log')).open('x') as log:
        data = Path(directory)
        env, controller_port = client_fixture(data, workload, rule_overrides)
        app = subprocess.Popen([str(executable)], env=env, stdout=log, stderr=log, start_new_session=True)
        try:
            for _ in range(90):
                found = subprocess.run(['xdotool', 'search', '--pid', str(app.pid), '--name', '^Clash of Rust'],
                                       env=env, capture_output=True, text=True)
                if found.returncode == 0 and found.stdout.split():
                    window = found.stdout.split()[0]
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
                before = measure('tray-open')
                close_window(window, env)

                def visible():
                    if app.poll() is not None:
                        raise RuntimeError('Close exited the GUI instead of retaining the tray')
                    result = subprocess.run(['xdotool', 'search', '--onlyvisible', '--pid', str(app.pid),
                                             '--name', '^Clash of Rust'], env=env, capture_output=True)
                    if result.returncode not in (0, 1):
                        raise RuntimeError('Could not inspect window visibility')
                    return result.returncode == 0

                def hidden():
                    return not visible() and core_running(controller_port)

                wait_until(hidden, 'window hidden with the same GUI and core still alive')
                after = measure('tray-hidden')
                if not hidden():
                    raise RuntimeError('GUI/core did not remain in the background during sampling')
                # A second launch uses the same real show-existing-window path
                # as opening the installed application again.
                subprocess.run([str(executable)], env=env, stdout=log, stderr=log,
                               check=True, timeout=20)
                wait_until(lambda: visible() and core_running(controller_port), 'restored window')
                core_pids = lambda rows: {p['pid'] for row in rows for p in row['processes']
                                          if p['role'] == 'core'}
                if core_pids(before) != core_pids(after):
                    raise RuntimeError('Core restarted while closing the window')
                write_json(output/(prefix+'proof.json'), {
                    'gui_pid': app.pid, 'core_pids': sorted(core_pids(after)),
                    'tray_registered': True, 'close_request': 'WM_DELETE_WINDOW',
                    'hidden_during_measurement': True, 'reopened': True,
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


def headless_scene(console, resources, benchmark, output, seconds, workload, rule_overrides=None, prefix='background-'):
    with tempfile.TemporaryDirectory(prefix='memory-cli-') as directory, (output/(prefix+'cor.log')).open('x') as log:
        data = Path(directory)
        env, port = client_fixture(data, workload, rule_overrides)
        for key in ('DISPLAY', 'WAYLAND_DISPLAY', 'DBUS_SESSION_BUS_ADDRESS'):
            env.pop(key, None)
        command = [str(console), '--json', '--yes', '--timeout', '30',
                   '--data-dir', str(data), '--resources', str(resources)]
        pid = identity = None
        try:
            subprocess.run([*command, 'start'], env=env, stdout=log, stderr=log, check=True, timeout=90)
            endpoint = json.loads((data/'control.json').read_text())
            if endpoint['owner'] != 'daemon':
                raise RuntimeError('CLI did not create a headless runtime')
            pid = endpoint['pid']
            identity = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[19]
            wait_until(lambda: core_running(port), 'headless core readiness')
            measure_process(benchmark, pid, prefix+'headless', output, seconds, env, mode='daemon')
            if not core_running(port) or (data/'gui-session.json').exists():
                raise RuntimeError('Headless measurement unexpectedly stopped or created a GUI')
            write_json(output/(prefix+'headless-proof.json'), {
                'daemon_pid': pid, 'owner': endpoint['owner'], 'display': False,
                'gui_session': False, 'core_running': True,
            })
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
                  'Fixture hosts and the observer are outside the measured process trees.', '',
                  '| Scene | Client | Client RSS peak MiB | Core RSS peak MiB | Total PSS peak MiB |',
                  '| --- | --- | ---: | ---: | ---: |']
        for scene in metadata['background_scenes']:
            role = 'daemon' if scene.endswith('headless') else 'gui'
            samples = read_scene(output/(scene+'.jsonl'), metadata['gui_sample_seconds'], role)
            values = [f"{max(row[key][field] for row in samples)/1048576:.2f}"
                      for key, field in ((role, 'rss_bytes'), ('core', 'rss_bytes'), ('total', 'pss_bytes'))]
            lines.append(f'| {scene} | {"cor" if role == "daemon" else "GUI"} | '+ ' | '.join(values)+' |')
    lines += ['', 'GUI peaks are observed at 250 ms intervals, not exact allocator peaks. RSS can count shared pages twice. Compare separate runs only with the same toolchain, build profile, inputs and environment. Core memory also varies independently of the Rust GUI.', '']
    report = '\n'.join(lines)
    (output/'README.md').write_text(report, encoding='utf-8')
    if os.environ.get('GITHUB_STEP_SUMMARY'):
        with Path(os.environ['GITHUB_STEP_SUMMARY']).open('a', encoding='utf-8') as summary:
            summary.write(report)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--profile', choices=('dev', 'release'), default='release')
    parser.add_argument('--workload', choices=WORKLOADS, default='light',
                        help='light: small subscription (default); stress: the original large fixture')
    parser.add_argument('--rows', type=int, help='Override the selected workload\'s API row count')
    parser.add_argument('--repetitions', type=int, default=3)
    parser.add_argument('--seconds', type=int, default=10)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    counts = WORKLOADS[args.workload]
    if args.rows is None:
        args.rows = counts['api_rows']
    if not (1 <= args.rows <= 50000 and 1 <= args.repetitions <= 10 and 1 <= args.seconds <= 120):
        raise ValueError('Bounds: rows 1..50000, repetitions 1..10, GUI seconds 1..120')
    source, output = args.source.resolve(strict=True), args.output.resolve()
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
        command.append('--workspace')
    if args.profile == 'release':
        command.append('--release')
    with (output/'build.log').open('x') as log:
        subprocess.run(command, cwd=source, stdout=log, stderr=subprocess.STDOUT, check=True)
    subprocess.run([sys.executable, source/'scripts/prepare-resources.py', '--system', 'linux', '--arch', 'x64'], cwd=source, check=True)
    resources = source/'bundle/linux-x64/resources'
    for name in ('core.json', 'geodata.json'):
        shutil.copy2(resources/name, output/name)
    metadata['resources'] = {name: json.loads((resources/name).read_text()) for name in ('core.json', 'geodata.json')}
    write_json(output/'metadata.json', metadata)
    target = source/'target'/('release' if args.profile == 'release' else 'debug')
    bundle = source/'bundle/linux-x64'
    for name in ('clash-of-rust', 'clash-tun-launcher'):
        shutil.copy2(target/name, bundle/name)
    benchmark = target/'examples/memory_benchmark'
    with (output/'api.txt').open('x') as text:
        subprocess.run([str(benchmark), 'suite', str(args.rows), str(args.repetitions), str(output/'api.jsonl')],
                       cwd=source, stdout=text, check=True)
    gui_scenes(bundle/'clash-of-rust', benchmark, output, args.seconds, modern_layout, slint_layout=slint_layout, workload=args.workload)
    settings_source = (core_directory(source)/'src/config/mod.rs' if desktop != source else source/'src/config.rs')
    if 'pub rule_overrides:' in settings_source.read_text(encoding='utf-8'):
        overrides = {'enabled': [True]*5}
        gui_scenes(bundle/'clash-of-rust', benchmark, output, args.seconds, modern_layout, slint_layout=slint_layout,
                   rule_overrides=overrides, prefix='rules-', workload=args.workload)
        metadata['additional_gui_rules'] = overrides
        write_json(output/'metadata.json', metadata)
    if slint_layout:
        # Preserve the original visible-window scenarios without a tray host,
        # so old baselines remain comparable. Measure normal tray use separately.
        metadata['background_scenes'] = []
        console = target/'cor'
        has_cli = (source/'apps/cli/Cargo.toml').is_file()
        if has_cli:
            version = subprocess.check_output([str(console), '--json', 'version'], text=True)
            if json.loads(version)['data']['version'] != metadata['version']:
                raise ValueError('Headless executable differs from the selected source version')
            shutil.copy2(console, bundle/'cor')
        with TrayHost(output) as tray:
            variants = [('background-', None)]
            if metadata.get('additional_gui_rules'):
                variants.append(('rules-background-', metadata['additional_gui_rules']))
            for prefix, overrides in variants:
                gui_scenes(bundle/'clash-of-rust', benchmark, output, args.seconds,
                           modern_layout, slint_layout=True, rule_overrides=overrides,
                           prefix=prefix, workload=args.workload, tray=tray)
                metadata['background_scenes'] += [prefix+'tray-open', prefix+'tray-hidden']
                if has_cli:
                    headless_scene(bundle/'cor', resources, benchmark, output, args.seconds,
                                   args.workload, overrides, prefix)
                    metadata['background_scenes'].append(prefix+'headless')
        write_json(output/'metadata.json', metadata)
    summarize(output, metadata)


if __name__ == '__main__':
    main()
