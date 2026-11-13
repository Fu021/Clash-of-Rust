"""Bounded loopback HTTP/TLS fixture for disposable native proxy CI."""
import contextlib
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import ssl
import subprocess
import tempfile
import threading
import time

from build_support import ROOT, host_arch, host_system

BLOCK = bytes(range(256)) * 256
BODY_SIZE = 16 * 1024 * 1024
LATENCY_SECONDS = 0.025


def openssl_executable():
    executable = shutil.which('openssl')
    if executable:
        return executable
    candidates = [
        Path(os.environ.get('ProgramFiles', 'C:/Program Files')) / 'Git/usr/bin/openssl.exe',
        Path(os.environ.get('ProgramFiles', 'C:/Program Files')) / 'Git/mingw64/bin/openssl.exe',
    ]
    for candidate in candidates:
        if candidate.is_file():
            return str(candidate)
    raise RuntimeError('The proxy throughput TLS fixture requires OpenSSL')


class Payload(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def do_HEAD(self):
        if self.path != '/latency':
            self.send_error(404)
            return
        started = time.monotonic()
        time.sleep(LATENCY_SECONDS)
        waited_ms = (time.monotonic() - started) * 1000
        self.send_response(204)
        self.send_header('Content-Length', '0')
        self.send_header('X-Cor-Fixture-Wait-Ms', f'{waited_ms:.6f}')
        self.end_headers()

    def do_GET(self):
        if self.path not in ('/payload', '/small'):
            self.send_error(404)
            return
        size = len(BLOCK) if self.path == '/small' else BODY_SIZE
        self.connection.settimeout(15)
        self.connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.send_response(200)
        self.send_header('Content-Length', str(size))
        self.send_header('Content-Type', 'application/octet-stream')
        if self.path != '/small':
            self.send_header('Connection', 'close')
            self.close_connection = True
        self.end_headers()
        try:
            for _ in range(size // len(BLOCK)):
                self.wfile.write(BLOCK)
        except (BrokenPipeError, ConnectionResetError, TimeoutError):
            pass

    def log_message(self, *args):
        pass


class TlsServer(ThreadingHTTPServer):
    def get_request(self):
        connection, address = super().get_request()
        connection.settimeout(5)
        try:
            return self.tls.wrap_socket(connection, server_side=True), address
        except BaseException:
            connection.close()
            raise


def save_report(path):
    if not path.is_file():
        return
    report = json.loads(path.read_text(encoding='utf-8'))
    destination = ROOT / 'target/proxy-benchmark-report'
    destination.mkdir(parents=True, exist_ok=True)
    destination.joinpath('loopback.json').write_text(
        json.dumps(report, indent=2) + '\n', encoding='utf-8')
    rows = []
    for sample in report['summaries']:
        rows.append(
            f"| {sample['protocol']} | {sample['route']} | {sample['sink']} | "
            f"{sample['median_mib_per_second']:.2f} | "
            f"{sample['median_first_byte_ms']:.2f} |")
    text = (
        '\n### Local proxy throughput\n\n'
        f"Host: {report['system']} / {report['arch']}; {report['host']['logical_cpus']} logical CPUs. "
        f"Complete: {str(report['complete']).lower()}. "
        '16 MiB per sample, three rounds; median shown. '
        'System proxy endpoints are read back from native preferences and used explicitly '
        'so the loopback bypass does not turn proxy samples into direct samples. '
        'IPv4 HTTP/1.1 and trusted local HTTPS only; no public DNS, remote node or GitHub CDN. '
        'Speeds include hashing; file samples also include flush/sync. '
        'Runner hardware differs; compare proxy/direct ratios within each runner first.\n\n'
        '| Protocol | Route | Sink | MiB/s | First byte (ms) |\n'
        '| --- | --- | --- | ---: | ---: |\n'
        + '\n'.join(rows) + '\n')
    if report.get('error'):
        text += '\nFailure: '+str(report['error'])+'\n'
    sequences = report.get('pooled_sequences', [])
    if sequences:
        text += '\n## Reused HTTPS client: 29 small files, then one large file\n\n'
        text += '| Route | Round | Requests | Final file (MiB/s) |\n| --- | ---: | ---: | ---: |\n'
        for sample in sequences:
            text += (f'| {sample["route"]} | {sample["round"]} | {sample["requests"]} | '
                     f'{sample["final_mib_per_second"]:.2f} |\n')
    delays = report.get('latency_summaries', [])
    if delays:
        text += (
            '\n### Local node latency control\n\n'
            'The local endpoint delays each HEAD response by 25 ms. '
            'Controller measurements use the real mihomo delay API for DIRECT and a '
            'synthetic SOCKS node that forwards through the same mixed listener. '
            'Mihomo unified delay reports the repeated HEAD rather than the initial '
            'handshake when the second response succeeds; client timings include a new '
            'connection. These are not public node or DNS measurements.\n\n'
            'The measured server sleep is reported separately; scheduler delays are '
            'not silently attributed to the proxy.\n\n'
            '| Route | Median wall time (ms) | Median reported delay (ms) | Server sleep (ms) |\n'
            '| --- | ---: | ---: | ---: |\n')
        for sample in delays:
            measured = sample['median_reported_delay_ms']
            server_wait = sample.get('median_server_wait_ms')
            wait_label = '—' if server_wait is None else f'{server_wait:.2f}'
            text += (f"| {sample['route']} | {sample['median_elapsed_ms']:.2f} | "
                     f"{measured:.2f} | {wait_label} |\n" if measured is not None else
                     f"| {sample['route']} | {sample['median_elapsed_ms']:.2f} | — | {wait_label} |\n")
    destination.joinpath('summary.md').write_text(text, encoding='utf-8')
    print(text, flush=True)
    if summary := os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(summary, 'a', encoding='utf-8') as stream:
            stream.write(text)


@contextlib.contextmanager
def fixture(*, uid=None, gid=None):
    with tempfile.TemporaryDirectory(prefix='cor-proxy-benchmark-') as folder:
        root = Path(folder)
        if host_system() == 'windows':
            from windows_ci import allow_ordinary_user_directory
            allow_ordinary_user_directory(root)
        elif uid is not None:
            os.chown(root, uid, gid)
        certificate = root / 'certificate.pem'
        configuration = root / 'openssl.cnf'
        configuration.write_text(
            '[req]\ndistinguished_name = dn\n[dn]\n'
            '[ca_ext]\nbasicConstraints = critical,CA:TRUE,pathlen:0\n'
            'keyUsage = critical,keyCertSign,cRLSign\n'
            'subjectKeyIdentifier = hash\nauthorityKeyIdentifier = keyid:always\n'
            '[server_ext]\nsubjectAltName = IP:127.0.0.1\n'
            'basicConstraints = critical,CA:FALSE\n'
            'keyUsage = critical,digitalSignature,keyEncipherment\n'
            'subjectKeyIdentifier = hash\nauthorityKeyIdentifier = keyid,issuer\n'
            'extendedKeyUsage = serverAuth\n', encoding='ascii')
        openssl = openssl_executable()
        commands = [
            [openssl, 'req', '-x509', '-newkey', 'rsa:2048', '-sha256', '-nodes', '-days', '1',
             '-subj', '/CN=cor-loopback-test-ca', '-config', str(configuration),
             '-extensions', 'ca_ext', '-keyout', str(root / 'ca-key.pem'), '-out', str(certificate)],
            [openssl, 'req', '-new', '-newkey', 'rsa:2048', '-sha256', '-nodes',
             '-subj', '/CN=cor-loopback-fixture', '-config', str(configuration),
             '-keyout', str(root / 'key.pem'), '-out', str(root / 'request.pem')],
            [openssl, 'x509', '-req', '-sha256', '-days', '1', '-in', str(root / 'request.pem'),
             '-CA', str(certificate), '-CAkey', str(root / 'ca-key.pem'), '-CAcreateserial',
             '-extfile', str(configuration), '-extensions', 'server_ext',
             '-out', str(root / 'server.pem')],
        ]
        for command in commands:
            subprocess.run(command, check=True, capture_output=True, timeout=20)
        if host_system() != 'windows':
            (root / 'ca-key.pem').chmod(0o600)
            (root / 'key.pem').chmod(0o600)
            certificate.chmod(0o644)
        tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls.minimum_version = ssl.TLSVersion.TLSv1_2
        tls.set_alpn_protocols(['http/1.1'])
        tls.load_cert_chain(root / 'server.pem', root / 'key.pem')
        report = root / 'report.json'
        manifest = root / 'fixture.json'
        with contextlib.ExitStack() as stack:
            servers = []
            for scheme in ('http', 'https'):
                server = (TlsServer if scheme == 'https' else ThreadingHTTPServer)(
                    ('127.0.0.1', 0), Payload)
                server.daemon_threads = True
                stack.callback(server.server_close)
                if scheme == 'https':
                    server.tls = tls
                thread = threading.Thread(target=server.serve_forever, daemon=True)
                thread.start()
                stack.callback(thread.join, 3)
                stack.callback(server.shutdown)
                servers.append(f'{scheme}://127.0.0.1:{server.server_port}/payload')
            digest = hashlib.sha256()
            for _ in range(BODY_SIZE // len(BLOCK)):
                digest.update(BLOCK)
            manifest.write_text(json.dumps({
                'urls': servers, 'size': BODY_SIZE, 'sha256': digest.hexdigest(),
                'latency_url': servers[0].removesuffix('/payload') + '/latency',
                'certificate': str(certificate), 'report': str(report),
                'system': host_system(), 'arch': host_arch(),
                'host': {'release': platform.release(), 'logical_cpus': os.cpu_count(),
                         'processor': platform.processor()},
            }), encoding='utf-8')
            if host_system() != 'windows':
                manifest.chmod(0o644)
            try:
                yield {'CLASH_PROXY_BENCHMARK_FIXTURE': str(manifest)}
            except BaseException:
                try:
                    save_report(report)
                except (OSError, ValueError, KeyError) as error:
                    print(f'Could not preserve partial proxy benchmark report: {error}', flush=True)
                raise
            else:
                if not report.is_file():
                    raise RuntimeError('Proxy throughput test did not produce a report')
                save_report(report)
                if json.loads(report.read_text(encoding='utf-8')).get('complete') is not True:
                    raise RuntimeError('Proxy throughput test did not complete all samples')
