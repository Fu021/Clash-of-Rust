"""Render actual application views using the optional, isolated Rust test harness."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import time


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
