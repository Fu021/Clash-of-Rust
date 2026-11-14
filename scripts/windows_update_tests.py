"""Reproduce Windows installer write failures on a disposable native runner."""
from contextlib import contextmanager
import csv
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import urllib.request

from build_support import ROOT, file_version, host_arch, nsis, numeric_version, run, sha256, write_json

REFERENCE_COMMIT = 'ff3cf4d9580d83497605670caf6127911c9a1b0c'
REFERENCE_SHA256 = 'b7bdc912e258f1935150259fe54a32f4ded6c195f6d2908df7a0ea1ea260890d'
LEGACY_WRITE_MESSAGE = '安装文件写入失败，请检查磁盘空间与权限'
CASES = (
    ('locked-owner', 'clash-of-rust.exe', 'locked', 61, 32),
    ('locked-renderer', 'clash-desktop-ui.exe', 'locked', 62, 32),
    ('locked-cli', 'cor.exe', 'locked', 6, 32),
    ('locked-core', 'resources/mihomo.exe', 'locked', 6, 32),
    ('locked-geo', 'resources/GeoIP.dat', 'locked', 6, 32),
    ('locked-uninstaller', 'uninstall.exe', 'locked', 64, 32),
    ('readonly-geo', 'resources/GeoIP.dat', 'readonly', 0, 5),
    ('denied-write', 'resources/default.yaml', 'denied', 6, 5),
)


def reference_source():
    result = subprocess.run(
        ['git', 'show', f'{REFERENCE_COMMIT}:installer/clash-of-rust.nsi'],
        cwd=ROOT, capture_output=True, timeout=30)
    if result.returncode == 0:
        source = result.stdout
    else:
        url = f'https://raw.githubusercontent.com/Fu021/Clash-of-Rust/{REFERENCE_COMMIT}/installer/clash-of-rust.nsi'
        for attempt in range(3):
            try:
                with urllib.request.urlopen(url, timeout=30) as response:
                    source = response.read(64 * 1024 + 1)
                break
            except OSError:
                if attempt == 2:
                    raise
                time.sleep(2)
    if len(source) > 64 * 1024 or hashlib.sha256(source).hexdigest() != REFERENCE_SHA256:
        raise ValueError('Historical installer source identity mismatch')
    return source


def reference_installer(payload, directory):
    source = directory/'installer/reference.nsi'
    source.parent.mkdir(parents=True)
    source.write_bytes(reference_source())
    for name in ('icons/app.ico', 'fonts/LICENSE.txt', 'fonts/LICENSE-NotoEmoji.txt',
                 'countries/LICENSE-Unicode.txt'):
        target = directory/'resources'/name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT/'resources'/name, target)
    package = directory/'reference-setup.exe'
    version = file_version(payload/'clash-of-rust.exe')
    run([nsis(), '/V2', '/INPUTCHARSET', 'UTF8', '/XSetCompress off',
         f'/DAPP_VERSION={version}', f'/DAPP_NUMERIC_VERSION={numeric_version(version)}',
         f'/DAPP_ARCH={host_arch()}', f'/DPAYLOAD={payload}', f'/DOUTPUT={package}', source])
    return package


@contextmanager
def write_barrier(path, kind, directory):
    if kind == 'denied':
        identity = subprocess.check_output(
            ['whoami', '/user', '/fo', 'csv', '/nh'], text=True)
        sid = next(csv.reader(identity.splitlines()))[-1]
        if not sid.startswith('S-1-'):
            raise RuntimeError('Cannot identify the installer test account')
        saved = directory/'file-acl.txt'
        subprocess.run(['icacls', path.name, '/save', str(saved), '/Q'],
                       cwd=path.parent, capture_output=True, check=True)
        try:
            subprocess.run(['icacls', str(path), '/deny', f'*{sid}:(W)', '/Q'],
                           capture_output=True, check=True)
            yield
        finally:
            subprocess.run(['icacls', str(path.parent), '/restore', str(saved), '/Q'],
                           capture_output=True, check=True)
        return
    import ctypes
    from ctypes import wintypes
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    if kind == 'readonly':
        kernel.GetFileAttributesW.argtypes = [wintypes.LPCWSTR]
        kernel.GetFileAttributesW.restype = wintypes.DWORD
        kernel.SetFileAttributesW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD]
        attributes = kernel.GetFileAttributesW(str(path))
        if attributes == 0xffffffff:
            raise ctypes.WinError(ctypes.get_last_error())
        if not kernel.SetFileAttributesW(str(path), attributes | 1):
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            yield
        finally:
            if not kernel.SetFileAttributesW(str(path), attributes):
                raise ctypes.WinError(ctypes.get_last_error())
        return
    if kind != 'locked':
        raise ValueError('Unknown Windows installer write barrier')
    kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                  wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD,
                                  wintypes.HANDLE]
    kernel.CreateFileW.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    handle = kernel.CreateFileW(str(path), 0x80000000, 1, None, 3, 0, None)
    if handle == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        yield
    finally:
        kernel.CloseHandle(handle)


def failure_record(package):
    path = package.parent/'installer-failure.json'
    if not path.exists():
        return None
    source = path.read_bytes()
    if len(source) > 4096:
        raise ValueError('Installer failure evidence is oversized')
    return json.loads(source)


def installed_hashes(installed):
    return {path.relative_to(installed).as_posix(): sha256(path)
            for path in installed.rglob('*') if path.is_file()}


def verify_write_failures(package, installed, payload, execute):
    if os.name != 'nt' or os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('Windows update reproduction requires a disposable Actions runner')
    report_path = ROOT/'target/windows-update-report.json'
    report = {'schema': 1, 'reference_commit': REFERENCE_COMMIT,
              'reference_source_sha256': REFERENCE_SHA256,
              'legacy_code_6_message': LEGACY_WRITE_MESSAGE,
              'free_bytes_before': shutil.disk_usage(installed).free,
              'cases': [], 'completed': False}
    payload_bytes = sum(path.stat().st_size for path in payload.rglob('*') if path.is_file())
    if report['free_bytes_before'] < payload_bytes * 4:
        raise RuntimeError('Insufficient free space for a controlled Windows write-failure test')
    renderer = installed/'clash-desktop-ui.exe'
    original = (payload/renderer.name).read_bytes()
    expected = {name: sha256(payload/name) for name in (
        'clash-of-rust.exe', 'clash-desktop-ui.exe', 'cor.exe', 'clash-tun-launcher.exe',
        'resources/mihomo.exe', 'resources/GeoIP.dat', 'resources/GeoSite.dat')}
    write_json(report_path, report)
    try:
        with tempfile.TemporaryDirectory(prefix='legacy-installer-') as temporary:
            directory = Path(temporary)
            reference = reference_installer(payload, directory)
            for name, relative, kind, legacy_code, system_code in CASES:
                record = {'name': name, 'file': relative, 'barrier': kind,
                          'free_bytes': shutil.disk_usage(installed).free, 'attempts': []}
                report['cases'].append(record)
                for label, installer, expected_code in (
                    ('reference', reference, legacy_code), ('candidate', package, 68)
                ):
                    renderer.write_bytes(original+b'installer-rollback-probe')
                    (installer.parent/'installer-failure.json').unlink(missing_ok=True)
                    before = installed_hashes(installed)
                    with write_barrier(installed/relative, kind, directory):
                        execute(installer, ['/S', '/UPDATE', f'/D={installed}'], expected_code)
                        failure = failure_record(installer)
                        attempt = {'installer': label, 'exit_code': expected_code,
                                   'failure': failure,
                                   'matches_legacy_write_message': expected_code == 6}
                        record['attempts'].append(attempt)
                        write_json(report_path, report)
                        if label == 'candidate':
                            expected_file = relative if '/' in relative else './'+relative
                            if failure != {
                                'schema': 1, 'stage': 'preflight', 'file': expected_file,
                                'win32_error': system_code, 'exit_code': 68
                            }:
                                raise AssertionError(f'{name}: incorrect installer diagnostic: {failure}')
                            if installed_hashes(installed) != before:
                                raise AssertionError(f'{name}: write preflight modified installed files')
                            if list(installed.glob('*.update-backup.exe')):
                                raise AssertionError(f'{name}: write preflight started replacement')
                            attempt['original_files_unchanged'] = True
                    execute(package, ['/S', '/UPDATE', f'/D={installed}'], 0)
                    if any(sha256(installed/path) != digest for path, digest in expected.items()):
                        raise AssertionError(f'{name}: retry did not restore the verified payload')
                    if any(not (installed/'resources/ip-check'/path).is_file()
                           for path in ('LICENSE', 'SOURCE.md')):
                        raise AssertionError('Nested resource directory was not installed')
                    attempt['retry_succeeded'] = True
                    write_json(report_path, report)
                print('Windows update reproduction:', json.dumps(record, ensure_ascii=False), flush=True)
        report['completed'] = True
    finally:
        renderer.write_bytes(original)
        (package.parent/'installer-failure.json').unlink(missing_ok=True)
        write_json(report_path, report)
