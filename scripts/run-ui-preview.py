"""Render actual application views using the optional, isolated Rust test harness."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import signal
import struct
import subprocess
import time
import zlib


def pixels(path):
    """Read our screenshot writer's RGB PNGs without a new image dependency."""
    data = path.read_bytes()
    if data[:8] != b'\x89PNG\r\n\x1a\n':
        raise ValueError('Expected a screenshot PNG')
    offset, compressed = 8, bytearray()
    while offset < len(data):
        length = struct.unpack('>I', data[offset:offset+4])[0]
        kind, chunk = data[offset+4:offset+8], data[offset+8:offset+8+length]
        if kind == b'IHDR':
            width, height, depth, color, _, _, interlace = struct.unpack('>IIBBBBB', chunk)
            if (depth, color, interlace) != (8, 2, 0):
                raise ValueError('Expected the screenshot writer RGB format')
        elif kind == b'IDAT':
            compressed.extend(chunk)
        offset += length+12
    raw, stride = zlib.decompress(compressed), width*3+1
    if len(raw) != stride*height or any(raw[y*stride] for y in range(height)):
        raise ValueError('Expected unfiltered screenshot rows')
    return width, height, [raw[y*stride+1:(y+1)*stride] for y in range(height)]


def selection_regression(window, scene, output, env, runner):
    """Compare a real click redraw with a full redraw at the same window size."""
    def click(x, y):
        subprocess.run(['xdotool', 'mousemove', '--window', window, str(x), str(y),
                        'click', '1', 'mousemove', '--window', window, '10', '10'],
                       env=env, check=True)
        time.sleep(0.5)
    light = scene.endswith('light')
    for name, x, y, expected in [
        ('ip-filter-click', 215, 273, 'SiteSummaryFilter(Group("IP信息"), Status(Identified))'),
        ('ip-filter-category', 580, 251, 'SiteSummaryFilter(Group("AI"), All)'),
        ('ip-filter-status', 598, 273, 'SiteSummaryFilter(Group("AI"), Status(Available))'),
        ('ip-filter-reset', 642, 385, 'SiteResetFilters'),
    ]:
        click(x, y)
        if expected not in (output/(scene+'.log')).read_text():
            raise RuntimeError('Expected real UI interaction: '+expected)
        stem = name+('-light' if light else '')
        partial, full = output/(stem+'.png'), output/(stem+'-full.png')
        runner.screenshot(window, partial, env)
        subprocess.run(['xdotool', 'windowsize', window, '970', '720'], env=env, check=True)
        time.sleep(0.3)
        subprocess.run(['xdotool', 'windowsize', window, '950', '700'], env=env, check=True)
        time.sleep(0.5)
        runner.screenshot(window, full, env)
        a, b = pixels(partial), pixels(full)
        if a[:2] != b[:2] or any(a[2][y][172*3:593*3] != b[2][y][172*3:593*3]
                                  for y in range(350, 440)):
            raise RuntimeError('Selection redraw differs from full repaint: '+stem)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--profile', choices=('dev', 'release'), default='release')
    args = parser.parse_args()
    source = args.source.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    command = ['cargo', 'test', '--locked', '--features', 'ui-preview', '--test', 'ui_preview',
               '--no-run', '--message-format=json']
    if args.profile == 'release':
        command.append('--release')
    with (output/'build.log').open('x') as log:
        messages = subprocess.check_output(command, cwd=source, text=True, stderr=log)
    executables = [entry['executable'] for line in messages.splitlines()
                   if (entry := json.loads(line)).get('reason') == 'compiler-artifact'
                   and entry['target']['name'] == 'ui_preview' and entry.get('executable')]
    if len(executables) != 1:
        raise ValueError('Expected exactly one UI preview test executable')
    spec = importlib.util.spec_from_file_location('memory_runner', source/'scripts/run-memory-benchmark.py')
    runner = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(runner)
    scenes = ['proxies-ascending', 'proxies-descending', 'proxies-name', 'ip-summary',
              'ip-ai', 'ip-restricted', 'ip-details', 'ip-partial', 'ip-compact', 'ip-summary-light']
    for scene in scenes:
        env = dict(os.environ, CLASH_UI_PREVIEW_SCENE=scene, WINIT_UNIX_BACKEND='x11',
                   WINIT_X11_SCALE_FACTOR='1', GSETTINGS_BACKEND='memory')
        env.pop('WAYLAND_DISPLAY', None)
        with (output/(scene+'.log')).open('x') as log:
            process = subprocess.Popen(executables, env=env, cwd=source, stdout=log, stderr=log,
                                       start_new_session=True)
            try:
                for _ in range(45):
                    found = subprocess.run(['xdotool', 'search', '--pid', str(process.pid), '--name',
                                            '^Clash of Rust'], env=env, capture_output=True, text=True)
                    if found.returncode == 0 and found.stdout.split():
                        window = found.stdout.split()[0]
                        break
                    if process.poll() is not None:
                        raise RuntimeError('Preview exited; see '+str(output/(scene+'.log')))
                    time.sleep(1)
                else:
                    raise RuntimeError('Preview window did not appear')
                time.sleep(2)
                runner.screenshot(window, output/(scene+'.png'), env)
                if scene in ('ip-summary', 'ip-summary-light'):
                    selection_regression(window, scene, output, env, runner)
            finally:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
    (output/'metadata.json').write_text(json.dumps({
        'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=source, text=True).strip(),
        'profile': args.profile, 'scenes': scenes,
        'data': 'Deterministic demonstration results, not real network measurements',
        'rendering': 'Actual application views; optional harness excluded from installers',
    }, indent=2)+'\n', encoding='utf-8')


if __name__ == '__main__':
    main()
