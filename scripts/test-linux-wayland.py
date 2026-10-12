"""Real Wayland login/foreground window checks on an isolated headless Weston.

Run under dbus-run-session. No real desktop, proxy settings, or user data is used.
"""
import argparse
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile
import time
import urllib.request
import socket


def stop(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)


def port():
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        return listener.getsockname()[1]


def root_buffer(trace):
    surfaces = re.findall(r'get_xdg_surface\([^\n]*?wl_surface[@#](\d+)', trace)
    if not surfaces:
        return None
    buffers = {identifier: (int(width), int(height)) for identifier, width, height in
               re.findall(r'create_buffer\([^,]*wl_buffer[@#](\d+),\s*\d+,\s*(\d+),\s*(\d+),', trace)}
    attached = re.findall(r'wl_surface[@#]' + surfaces[-1] + r'\.attach\(wl_buffer[@#](\d+)', trace)
    return buffers.get(attached[-1]) if attached else None


def check(executable, output, scale, background):
    name = ('background' if background else 'foreground') + '-' + str(scale)
    logs = output / name
    logs.mkdir()
    with tempfile.TemporaryDirectory(prefix='clash-wayland-') as directory:
        root = Path(directory)
        runtime = root / 'runtime'
        runtime.mkdir(mode=0o700)
        data = root / 'data'
        data.mkdir()
        controller, mixed = port(), port()
        while controller == mixed:
            mixed = port()
        (data / 'settings.json').write_text(json.dumps({
            'controller_port': controller, 'mixed_port': mixed,
            'secret': 'wayland-fixture', 'proxy_mode': 'off',
            'delay_interval_minutes': 0,
        }))
        env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime),
                   WAYLAND_DISPLAY='clash-wayland-test', GSETTINGS_BACKEND='memory')
        for key in ('DISPLAY', 'WAYLAND_SOCKET', 'WINIT_X11_SCALE_FACTOR', 'WAYLAND_DEBUG'):
            env.pop(key, None)
        help_text = subprocess.check_output(['weston', '--help'], text=True)
        renderer = '--renderer=pixman' if '--renderer=' in help_text else '--use-pixman'
        command = ['weston', '--backend=headless-backend.so', renderer,
                   '--width=3200', '--height=2400', '--scale=' + str(scale),
                   '--idle-time=0', '--no-config', '--socket=clash-wayland-test']
        with (logs / 'compositor.log').open('w') as compositor_log:
            compositor = subprocess.Popen(command, env=env, stdout=compositor_log,
                                          stderr=compositor_log, start_new_session=True)
            try:
                for _ in range(100):
                    if (runtime / env['WAYLAND_DISPLAY']).exists():
                        break
                    if compositor.poll() is not None:
                        raise RuntimeError('Weston exited; see ' + str(logs / 'compositor.log'))
                    time.sleep(0.1)
                else:
                    raise RuntimeError('Wayland compositor did not become ready')
                app_env = dict(env, CLASH_OF_RUST_DATA_DIR=str(data), WAYLAND_DEBUG='client')
                trace_file = logs / 'wayland.log'
                with trace_file.open('w') as trace_log, (logs / 'app.log').open('w') as app_log:
                    app = subprocess.Popen([str(executable)] + (['--background'] if background else []),
                                           env=app_env, stdout=app_log, stderr=trace_log,
                                           start_new_session=True)
                    try:
                        started = time.monotonic()
                        expected = (950 * scale, 700 * scale)
                        for _ in range(200):
                            trace = trace_file.read_text(errors='replace')
                            if background and time.monotonic() - started < 2:
                                assert 'get_xdg_surface(' not in trace, 'Login created a native window before display setup settled'
                            if root_buffer(trace) == expected:
                                break
                            if app.poll() is not None:
                                raise RuntimeError('Application exited; see ' + str(trace_file))
                            time.sleep(0.1)
                        else:
                            raise AssertionError((name, expected, root_buffer(trace), str(trace_file)))
                        # A later Configure must not leave a differently scaled frame.
                        time.sleep(0.5)
                        trace = trace_file.read_text(errors='replace')
                        assert root_buffer(trace) == expected, (name, expected, root_buffer(trace))
                        settings = json.loads((data / 'settings.json').read_text())
                        request = urllib.request.Request(f'http://127.0.0.1:{controller}/version',
                                  headers={'Authorization': 'Bearer ' + settings['secret']})
                        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                        for _ in range(150):
                            try:
                                with opener.open(request, timeout=1) as response:
                                    core = json.load(response)
                                break
                            except (OSError, ValueError):
                                if app.poll() is not None:
                                    raise RuntimeError('Application exited while waiting for its core')
                                time.sleep(0.1)
                        else:
                            raise RuntimeError('Core did not become ready; see ' + str(trace_file))
                        native_windows = len(set(re.findall(r'get_xdg_surface\([^\n]*?wl_surface[@#](\d+)', trace)))
                        assert native_windows == 1, (name, native_windows)
                        return {'mode': name, 'status': 'passed', 'scale': scale,
                                'rendered_physical_size': expected, 'core_version': core,
                                'native_windows': native_windows}
                    finally:
                        stop(app)
            finally:
                stop(compositor)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--executable', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    assert os.environ.get('DBUS_SESSION_BUS_ADDRESS'), 'Run under dbus-run-session'
    args.output.mkdir(parents=True, exist_ok=True)
    report = {'status': 'running', 'cases': []}
    try:
        for scale in (1, 2):
            for background in (False, True):
                report['cases'].append(check(args.executable.resolve(strict=True), args.output, scale, background))
        report['status'] = 'passed'
    except Exception as error:
        report.update(status='failed', error=str(error))
        raise
    finally:
        (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
