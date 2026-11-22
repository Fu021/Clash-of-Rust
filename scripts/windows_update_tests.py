"""Reproduce Windows installer write failures on a disposable native runner."""
from contextlib import ExitStack, contextmanager
import csv
import hashlib
from http.client import HTTPConnection
from http.server import ThreadingHTTPServer
import json
import os
from pathlib import Path
import random
import shutil
import socket
import struct
import subprocess
import tempfile
import threading
import time
import urllib.request
import uuid

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
    payload = payload.resolve()
    directory = directory.resolve()
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


class IndependentHTTPServer(ThreadingHTTPServer):
    allow_reuse_address = False
    allow_reuse_port = False

    def server_bind(self):
        if os.name == 'nt':
            self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
        super().server_bind()


@contextmanager
def reserve_mixed_port():
    candidates = [0, *random.sample(range(1024, 65536), 128)]
    for attempt, candidate in enumerate(candidates):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as tcp, \
                socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
            if os.name == 'nt':
                for reservation in (tcp, udp):
                    reservation.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
            try:
                udp.bind(('127.0.0.1', candidate))
                port = udp.getsockname()[1]
                tcp.bind(('127.0.0.1', port))
            except OSError as failure:
                if attempt == len(candidates)-1:
                    raise OSError('No available TCP/UDP port for independent Mihomo') from failure
                continue
            yield port
            return


class IndependentCoreNotReady(OSError):
    pass


def windows_loopback_port_owners(port):
    import ctypes

    api = ctypes.WinDLL('iphlpapi')
    tcp_table_owner_pid_listener = 3
    udp_table_owner_pid = 1
    insufficient_buffer = 122
    dword = ctypes.c_uint32
    entry_count = struct.Struct('=I')
    loopback, = entry_count.unpack(socket.inet_aton('127.0.0.1'))
    owners = {'TCP': set(), 'UDP': set()}
    for protocol, function, table_class, row in (
        ('TCP', 'GetExtendedTcpTable', tcp_table_owner_pid_listener, struct.Struct('=4xII8xI')),
        ('UDP', 'GetExtendedUdpTable', udp_table_owner_pid, struct.Struct('=III')),
    ):
        query = getattr(api, function)
        query.argtypes = (ctypes.c_void_p, ctypes.POINTER(dword), ctypes.c_int,
                          dword, ctypes.c_int, dword)
        query.restype = dword
        size = dword()
        table = None
        for _ in range(5):
            status = query(table, ctypes.byref(size), False, socket.AF_INET, table_class, 0)
            if status == 0:
                break
            if status != insufficient_buffer:
                raise RuntimeError(f'{function} failed with Windows error {status}')
            table = ctypes.create_string_buffer(size.value)
        else:
            raise RuntimeError(f'{function} could not size the Windows endpoint table')

        count, = entry_count.unpack_from(table)
        for index in range(count):
            address, local_port, pid = row.unpack_from(table, entry_count.size + index * row.size)
            if address == loopback and socket.ntohs(local_port & 0xffff) == port:
                owners[protocol].add(pid)
    return owners


def probe_independent_core(process, proxy_port, server):
    from proxy_benchmark import BLOCK
    if process.poll() is not None:
        raise RuntimeError('Independent Mihomo exited during the update test')
    owners = windows_loopback_port_owners(proxy_port)
    if owners['TCP'] != {process.pid} or owners['UDP'] != {process.pid}:
        raise IndependentCoreNotReady(
            f'Independent Mihomo PID {process.pid} does not own both listeners at 127.0.0.1:{proxy_port} '
            f'(TCP owner PIDs={sorted(owners["TCP"])}, UDP owner PIDs={sorted(owners["UDP"])})')
    connection = HTTPConnection('127.0.0.1', proxy_port, timeout=2)
    try:
        connection.request(
            'GET', f'http://127.0.0.1:{server.server_port}/small',
            headers={'Connection': 'close'})
        response = connection.getresponse()
        body = response.read(len(BLOCK)+1)
        digest = hashlib.sha256(body).hexdigest()
        marker = response.getheader('X-Cor-Update-Fixture')
        identity = marker == server.fixture_id
        if response.status != 200 or body != BLOCK or not identity:
            failure = (
                'Independent Mihomo did not forward the fixture payload: '
                f'HTTP {response.status}, bytes={len(body)}, sha256={digest}, '
                f'identity={identity}, proxy_port={proxy_port}, '
                f'origin_port={server.server_port}, origin_path={server.last_path!r}')
            if response.status == 502 and not body and marker is None and server.last_path is None:
                raise IndependentCoreNotReady(failure)
            raise AssertionError(failure)
        return digest
    finally:
        connection.close()


def wait_independent_core(process, probe, record):
    started = time.monotonic()
    record.update({'completed': False, 'attempts': 0, 'transient_errors': []})
    while True:
        record['attempts'] += 1
        try:
            digest = probe()
        except OSError as failure:
            record['last_error'] = str(failure)[:2048]
            if len(record['transient_errors']) < 8:
                record['transient_errors'].append(record['last_error'])
            if process.poll() is not None or time.monotonic()-started >= 20:
                raise
            time.sleep(.1)
        else:
            record.update({'completed': True, 'probe_sha256': digest,
                           'elapsed_ms': round((time.monotonic()-started)*1000)})
            return


@contextmanager
def independent_core(payload, directory, readiness=None):
    from proxy_benchmark import Payload

    class FixturePayload(Payload):
        def do_GET(self):
            self.server.last_path = self.path[:256]
            super().do_GET()

        def end_headers(self):
            self.send_header('X-Cor-Update-Fixture', self.server.fixture_id)
            super().end_headers()

    external = directory/'独立 Mihomo with spaces'
    external.mkdir()
    executable = external/'mihomo.exe'
    shutil.copyfile(payload/'resources/mihomo.exe', executable)
    for name in ('GeoIP.dat', 'GeoSite.dat', 'Country.mmdb', 'ASN.mmdb'):
        shutil.copyfile(payload/'resources'/name, external/name)
    with ExitStack() as cleanup:
        with reserve_mixed_port() as proxy_port:
            server = cleanup.enter_context(
                IndependentHTTPServer(('127.0.0.1', 0), FixturePayload))
        if server.server_port == proxy_port:
            raise AssertionError('Independent Mihomo and origin fixture must use distinct ports')
        server.fixture_id = uuid.uuid4().hex
        server.last_path = None
        config = external/'config.yaml'
        write_json(config, {
            'mixed-port': proxy_port, 'bind-address': '127.0.0.1', 'allow-lan': False,
            'mode': 'rule', 'log-level': 'info', 'ipv6': False,
            'dns': {'enable': False}, 'tun': {'enable': False},
            'proxies': [], 'rules': ['MATCH,DIRECT'],
        })
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        cleanup.callback(worker.join, timeout=5)
        cleanup.callback(server.shutdown)
        with (directory/'independent-core.log').open('wb') as log:
            with subprocess.Popen(
                [str(executable), '-d', str(external), '-f', str(config)],
                cwd=external, stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                creationflags=subprocess.CREATE_NO_WINDOW,
            ) as process:
                def probe():
                    return probe_independent_core(process, proxy_port, server)
                try:
                    wait_independent_core(process, probe, readiness if readiness is not None else {})
                    yield process, executable, config, probe
                finally:
                    if process.poll() is None:
                        process.terminate()
                        try:
                            process.wait(timeout=10)
                        except subprocess.TimeoutExpired:
                            process.kill()
                            process.wait(timeout=10)


def verify_independent_core(package, installed, payload, directory, execute, record):
    record.update({'completed': False, 'expected_exit_code': 0})
    try:
        (package.parent/'installer-failure.json').unlink(missing_ok=True)
        with independent_core(payload, directory, readiness=record.setdefault('readiness', {})) \
                as (process, executable, config, probe):
            if executable.resolve().is_relative_to(installed.resolve()):
                raise AssertionError('Independent Mihomo must live outside the installation')
            original = {path: sha256(path) for path in (executable, config)}
            record.update({
                'pid': process.pid, 'executable': str(executable),
                'separate_installation': True, 'proxy_before': probe(),
            })
            renderer = installed/'clash-desktop-ui.exe'
            renderer.write_bytes((payload/renderer.name).read_bytes()+b'independent-core-update-probe')
            execute(package, ['/S', '/UPDATE', f'/D={installed}'], 0)
            record['proxy_after'] = probe()
            if record['proxy_before'] != record['proxy_after'] or process.poll() is not None:
                raise AssertionError('Update disrupted the independent Mihomo process or traffic')
            for relative in (
                'clash-of-rust.exe', 'clash-desktop-ui.exe', 'cor.exe',
                'clash-tun-launcher.exe', 'resources/mihomo.exe', 'resources/GeoIP.dat',
                'resources/GeoSite.dat',
            ):
                if sha256(installed/relative) != sha256(payload/relative):
                    raise AssertionError('Independent-core update did not install the verified payload')
            if any(sha256(path) != digest for path, digest in original.items()):
                raise AssertionError('Update modified the independent Mihomo installation')
            if failure_record(package) is not None or list(installed.glob('*.update-backup.exe')):
                raise AssertionError('Independent-core update left failure evidence or backup files')
            record.update({'external_files_unchanged': True, 'payload_verified': True,
                           'same_process_alive': True, 'completed': True})
    except Exception as failure:
        record['error'] = str(failure)[:2048]
        log = directory/'independent-core.log'
        if log.is_file():
            with log.open('rb') as stream:
                stream.seek(max(0, log.stat().st_size-4096))
                record['core_log_tail'] = stream.read(4096).decode('utf-8', errors='replace')
        raise
    finally:
        record['failure'] = failure_record(package)


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
                            if list(installed.glob('*.update-backup.exe')):
                                raise AssertionError(f'{name}: write preflight started replacement')
                    if label == 'candidate':
                        if installed_hashes(installed) != before:
                            raise AssertionError(f'{name}: write preflight modified installed files')
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
            report['independent_core'] = {}
            verify_independent_core(
                package, installed, payload, directory, execute, report['independent_core'])
            write_json(report_path, report)
            print('Windows independent-core update:',
                  json.dumps(report['independent_core'], ensure_ascii=False), flush=True)
        report['completed'] = True
    finally:
        renderer.write_bytes(original)
        (package.parent/'installer-failure.json').unlink(missing_ok=True)
        write_json(report_path, report)
