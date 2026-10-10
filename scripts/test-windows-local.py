"""Test a Windows payload in the current interactive, non-elevated session.

Uses disposable data, does not change proxy/TUN or the user's startup entry.
Unlike the CI harness, no accounts or desktop/session ACLs are changed.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import urllib.request


def startup(executable, root, background):
    # Allocate unused ports without reading the user's personal configuration.
    import socket
    with socket.socket() as first, socket.socket() as second:
        first.bind(('127.0.0.1', 0))
        second.bind(('127.0.0.1', 0))
        ports = first.getsockname()[1], second.getsockname()[1]
    settings_path = root/'settings.json'
    if not settings_path.exists():
        import uuid
        settings = dict(controller_port=ports[0], mixed_port=ports[1], proxy_mode='off',
                        secret=str(uuid.uuid4()))
        settings_path.write_text(json.dumps(settings), encoding='utf-8')
    log = root/'launch.log'
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with log.open('wb') as output:
        process = subprocess.Popen([str(executable), *(['--background'] if background else [])],
                                   env=dict(os.environ, CLASH_OF_RUST_DATA_DIR=str(root)),
                                   stdout=output, stderr=output)
        try:
            deadline = time.monotonic() + 45
            failure = 'settings not created'
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise AssertionError(f'GUI exited early: {process.returncode}; ' + log.read_text('utf-8', errors='replace'))
                try:
                    settings = json.loads(settings_path.read_text('utf-8'))
                    assert settings['proxy_mode'] == 'off'
                    request = urllib.request.Request(f"http://127.0.0.1:{settings['controller_port']}/configs",
                                                     headers={'Authorization': 'Bearer ' + settings['secret']})
                    with opener.open(request, timeout=1) as response:
                        config = json.load(response)
                    assert not config.get('tun', {}).get('enable', False)
                    assert config['mixed-port'] == settings['mixed_port']
                    break
                except (OSError, ValueError, KeyError) as error:
                    failure = str(error)
                time.sleep(.2)
            else:
                raise AssertionError('Core startup failed: ' + failure)
            for name in ('settings.json', 'profiles.json', 'runtime/config.yaml', 'runtime/candidate.yaml'):
                assert (root/name).is_file(), name
            assert json.loads((root/'profiles.json').read_text('utf-8'))
            kernel = c.WinDLL('kernel32', use_last_error=True)
            kernel.OpenEventW.argtypes = [w.DWORD, w.BOOL, w.LPCWSTR]
            kernel.OpenEventW.restype = w.HANDLE
            kernel.SetEvent.argtypes = [w.HANDLE]
            kernel.CloseHandle.argtypes = [w.HANDLE]
            event = kernel.OpenEventW(2, False, 'Local\\ClashOfRust.Exit')
            if not event:
                raise c.WinError(c.get_last_error())
            try:
                assert kernel.SetEvent(event)
            finally:
                kernel.CloseHandle(event)
            assert process.wait(timeout=20) == 0
            print('PASS: GUI/core/save/clean shutdown; background=' + str(background), flush=True)
        finally:
            if process.poll() is None:
                subprocess.run(['taskkill', '/F', '/PID', str(process.pid), '/T'], capture_output=True)
                process.wait(timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('executable', type=Path)
    parser.add_argument('--test-binary', type=Path)
    parser.add_argument('--transaction-recovery', action='store_true',
                        help='For repaired payloads with startup recovery support')
    args = parser.parse_args()
    assert os.name == 'nt'
    assert not c.windll.shell32.IsUserAnAdmin(), 'Run from an ordinary interactive session'
    executable = args.executable.resolve(strict=True)
    if args.test_binary:
        subprocess.run([str(args.test_binary.resolve(strict=True)), '--test-threads=1'], check=True)
    # The session-wide single-instance guard intentionally covers all payloads.
    if subprocess.run(['tasklist', '/FI', 'IMAGENAME eq clash-of-rust.exe', '/FO', 'CSV'],
                      capture_output=True).stdout.find(b'clash-of-rust.exe') >= 0:
        raise RuntimeError('Close the existing Clash of Rust GUI before running this test')
    with tempfile.TemporaryDirectory(prefix='clash-local-中文-') as directory:
        root = Path(directory)
        startup(executable, root, False)
        startup(executable, root, True)
        old = json.loads((root/'settings.json').read_text('utf-8'))
        for name in ('node_sort', 'rule_overrides'):
            old.pop(name, None)
        (root/'settings.json').write_text(json.dumps(old), encoding='utf-8')
        startup(executable, root, False)
        print('PASS: old settings upgrade under ordinary Windows permissions', flush=True)
        if args.transaction_recovery:
            import uuid
            original = (root/'settings.json').read_bytes()
            transaction = root/('.profile-transaction-' + uuid.uuid4().hex)
            transaction.mkdir()
            (transaction/'0.old').write_bytes(original)
            (root/'settings.json').write_text('{partial settings', encoding='utf-8')
            (root/'profile-transaction.json').write_text(json.dumps({
                'directory': transaction.name,
                'entries': [{'target': 'settings.json', 'existed': True, 'remove': False}],
            }), encoding='utf-8')
            startup(executable, root, False)
            assert (root/'settings.json').read_bytes() == original
            assert not transaction.exists() and not (root/'profile-transaction.json').exists()
            print('PASS: interrupted transaction recovered on real ordinary GUI startup', flush=True)


if __name__ == '__main__':
    main()
