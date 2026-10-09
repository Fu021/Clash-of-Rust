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
import struct
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid
import zlib

from build_support import ROOT, write_json


def fixture():
    nodes = ', '.join(f'node-{index}' for index in range(2000))
    return ('proxies:\n' + ''.join(
        f'  - {{name: node-{index}, type: http, server: 127.0.0.1, port: 1}}\n' for index in range(2000))
        + 'proxy-groups:\n' + ''.join(
            f'  - name: group-{group}\n    type: select\n    proxies: [{nodes}]\n' for group in range(40))
        + 'rules:\n' + ''.join(
            f'  - DOMAIN-SUFFIX,example-{index}.test,group-0\n' for index in range(20000))
        + '  - MATCH,DIRECT\n')


def screenshot(window, output, env):
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
            raise RuntimeError('Expected a 32-bit Xvfb screenshot')
        data = image.contents
        raw = c.string_at(data.data, data.bytes_per_line*height)
        rows = []
        for y in range(height):
            row = raw[y*data.bytes_per_line:y*data.bytes_per_line+width*4]
            rgb = bytearray(width*3)
            rgb[0::3], rgb[1::3], rgb[2::3] = row[2::4], row[1::4], row[0::4]
            rows.append(b'\0'+rgb)
        def chunk(kind, value):
            return struct.pack('>I', len(value))+kind+value+struct.pack('>I', zlib.crc32(kind+value))
        output.write_bytes(b'\x89PNG\r\n\x1a\n'
                           + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
                           + chunk(b'IDAT', zlib.compress(b''.join(rows))) + chunk(b'IEND', b''))
    finally:
        if image:
            x.XDestroyImage(image)
        x.XCloseDisplay(display)


def gui_scenes(executable, benchmark, output, seconds):
    env = dict(os.environ, GSETTINGS_BACKEND='memory', WINIT_UNIX_BACKEND='x11', WINIT_X11_SCALE_FACTOR='1')
    env.pop('WAYLAND_DISPLAY', None)
    with tempfile.TemporaryDirectory(prefix='memory-gui-') as directory, (output/'gui.log').open('x') as log:
        data = Path(directory)
        (data/'profiles').mkdir()
        profile_id = str(uuid.uuid4())
        with socket.socket() as controller, socket.socket() as mixed:
            controller.bind(('127.0.0.1', 0))
            mixed.bind(('127.0.0.1', 0))
            controller_port, mixed_port = controller.getsockname()[1], mixed.getsockname()[1]
        write_json(data/'settings.json', {'controller_port': controller_port, 'mixed_port': mixed_port,
                   'secret': 'memory-benchmark', 'dark': True, 'delay_interval_minutes': 0,
                   'active_profile': profile_id, 'run_mode': 'rule', 'proxy_mode': 'off'})
        (data/'profiles'/f'{profile_id}.yaml').write_text(fixture(), encoding='utf-8')
        write_json(data/'profiles.json', [{'id': profile_id, 'name': 'Memory workload', 'updated': 0,
                   'source': str(data/'profiles'/f'{profile_id}.yaml')}])
        env['CLASH_OF_RUST_DATA_DIR'] = str(data)
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
                time.sleep(3)
                screenshot(window, output/(name+'.png'), env)
                with (output/(name+'.txt')).open('x') as text:
                    subprocess.run([str(benchmark), 'app', str(app.pid), str(seconds), '250',
                                    str(output/(name+'.jsonl'))], stdout=text, check=True, env=env)
            measure('home')
            click(70, 108)
            measure('proxies-collapsed')
            click(380, 120)
            subprocess.run(['xdotool', 'windowfocus', '--sync', window], env=env, check=True)
            subprocess.run(['xdotool', 'type', '--delay', '100', '--clearmodifiers', 'node-'], env=env, check=True)
            measure('proxies-search')
            click(70, 70)
            measure('home-after-proxies')
        finally:
            # All tests use proxy off. Stop this owned process group, including
            # the bundled core, even if screenshot/measurement fails.
            try:
                os.killpg(app.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                app.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(app.pid, signal.SIGKILL)
                app.wait()


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
    lines += ['', 'GUI subscription: 2,000 nodes, 40 groups × 2,000 members, 20,000 rules. Proxy off; scheduled delay checks off; no business traffic. Xvfb/X11; snapshots show the scene used.', '',
              '| Scene | GUI RSS peak MiB | Core RSS peak MiB | Total PSS peak MiB |', '| --- | ---: | ---: | ---: |']
    for scene in ('home', 'proxies-collapsed', 'proxies-search', 'home-after-proxies'):
        rows = [json.loads(line) for line in (output/(scene+'.jsonl')).read_text().splitlines()]
        samples = [row for row in rows if 'elapsed_ms' in row]
        values = []
        for role, field in (('gui', 'rss_bytes'), ('core', 'rss_bytes'), ('total', 'pss_bytes')):
            measured = [row[role][field] for row in samples if row[role][field] is not None]
            values.append(f'{max(measured)/1048576:.2f}' if measured else 'unknown')
        lines.append(f'| {scene} | '+ ' | '.join(values) + ' |')
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
    parser.add_argument('--rows', type=int, default=50000)
    parser.add_argument('--repetitions', type=int, default=3)
    parser.add_argument('--seconds', type=int, default=10)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not (1 <= args.rows <= 50000 and 1 <= args.repetitions <= 10 and 1 <= args.seconds <= 120):
        raise ValueError('Bounds: rows 1..50000, repetitions 1..10, GUI seconds 1..120')
    source, output = args.source.resolve(strict=True), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    benchmark_source = ROOT/'examples/memory_benchmark.rs'
    destination = source/'examples/memory_benchmark.rs'
    if destination.resolve() != benchmark_source.resolve():
        shutil.copy2(benchmark_source, destination)
    import tomllib
    metadata = {'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=source, text=True).strip(),
                'benchmark_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                'version': tomllib.loads((source/'Cargo.toml').read_text())['package']['version'],
                'profile': args.profile, 'rows': args.rows, 'repetitions': args.repetitions,
                'gui_sample_seconds': args.seconds, 'interval_ms': 250,
                'os': platform.platform(), 'cpu_count': os.cpu_count(),
                'rustc': subprocess.check_output(['rustc', '-Vv'], text=True).strip(),
                'started_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                'instrumentation': 'The workflow benchmark example replaces only the example in the selected checkout; application sources are unchanged.'}
    write_json(output/'metadata.json', metadata)
    (output/'gui-fixture.yaml').write_text(fixture(), encoding='utf-8')
    command = ['cargo', 'build', '--locked', '--bins', '--example', 'memory_benchmark']
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
    gui_scenes(bundle/'clash-of-rust', benchmark, output, args.seconds)
    summarize(output, metadata)


if __name__ == '__main__':
    main()
