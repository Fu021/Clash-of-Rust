"""Temporary disposable-runner test, controlled HTTPS evidence and UI captures."""
import argparse
import contextlib
import hashlib
import http.server
import importlib.util
import json
import os
from pathlib import Path
import shutil
import ssl
import struct
import subprocess
import sys
import tempfile
import threading
import time
import uuid

from build_support import ROOT, run, write_json, host_system
import workflow_artifacts


def serve(cert, key):
    metrics = {'doh': 0, 'hosts': {}}
    lock = threading.Lock()

    class Handler(http.server.BaseHTTPRequestHandler):
        protocol_version = 'HTTP/1.1'

        def log_message(self, *args):
            pass  # Never retain headers, bodies or API keys.

        def answer(self, value, content_type='application/json'):
            payload = value if isinstance(value, bytes) else json.dumps(value).encode()
            self.send_response(200)
            self.send_header('Content-Type', content_type)
            self.send_header('Content-Length', str(len(payload)))
            self.send_header('Connection', 'close')
            self.end_headers()
            if self.command != 'HEAD':
                self.wfile.write(payload)

        def do_HEAD(self):
            self.do_GET()

        def do_GET(self):
            host = self.headers.get('Host', '').split(':')[0]
            with lock:
                metrics['hosts'][host] = metrics['hosts'].get(host, 0) + 1
            if self.path.startswith('/metrics'):
                with lock:
                    self.answer(dict(metrics))
            elif self.path.startswith('/hold'):
                self.send_response(200)
                self.send_header('Transfer-Encoding', 'chunked')
                self.end_headers()
                try:
                    for _ in range(1200):
                        self.wfile.write(b'4\r\nheld\r\n')
                        self.wfile.flush()
                        time.sleep(0.1)
                except (OSError, ssl.SSLError):
                    pass
            elif self.path.startswith('/cdn-cgi/trace'):
                self.answer(f'h={host}\nip=1.2.3.4\nloc=US\ncolo=IAD\nhttp=http/2\ntls=TLSv1.3\nwarp=off\n'.encode(), 'text/plain')
            elif host == 'api6.ipify.org':
                self.answer({'ip': '2606:4700::1111'})
            elif host == 'api.ipify.org':
                self.answer({'ip': '1.2.3.4'})
            elif host == 'api.ip.sb':
                self.answer({'ip': self.path.rsplit('/', 1)[-1], 'country_code': 'US', 'asn': 64500, 'asn_organization': 'Controlled test fixture'})
            elif host == 'rdap.org':
                self.send_response(302)
                self.send_header('Location', 'https://rdap.arin.net/registry/ip/1.2.3.4')
                self.send_header('Content-Length', '0')
                self.end_headers()
            elif host == 'rdap.arin.net':
                self.answer({'startAddress': '1.0.0.0', 'endAddress': '1.255.255.255', 'name': 'Controlled registry', 'country': 'US'})
            elif host == 'api.globalping.io':
                self.answer({'status': 'finished', 'results': [{'probe': {'country': 'US', 'city': f'Fixture {index}'}, 'result': {'stats': {'min': 10, 'avg': 12, 'max': 15, 'loss': 0}}} for index in range(20)]})
            elif host == 'speed.cloudflare.com':
                self.send_response(200)
                self.send_header('Content-Length', str(10*1024*1024))
                self.send_header('Connection', 'close')
                self.end_headers()
                if self.command != 'HEAD':
                    for _ in range(160):
                        self.wfile.write(bytes(64*1024))
            else:
                self.answer({'controlled_fixture': True})

        def do_POST(self):
            length = int(self.headers.get('Content-Length', '0'))
            host = self.headers.get('Host', '').split(':')[0]
            if host == 'speed.cloudflare.com':
                if length != 2*1024*1024:
                    self.send_error(400)
                    return
                remaining = length
                while remaining:
                    block = self.rfile.read(min(remaining, 64*1024))
                    if not block:
                        return
                    remaining -= len(block)
                self.answer({'controlled_upload_bytes': length})
                return
            if length > 65536:
                self.send_error(413)
                return
            body = self.rfile.read(length)
            if host == 'api.globalping.io':
                request = json.loads(body)
                if request.get('limit') != 20 or request.get('target') != '1.2.3.4':
                    self.send_error(400)
                    return
                self.answer({'id': 'controlled-fixture'})
                return
            if self.path.startswith('/dns-query'):
                with lock:
                    metrics['doh'] += 1
                position = 12
                while position < len(body) and body[position]:
                    position += 1 + body[position]
                end = position + 5
                qtype = struct.unpack('!H', body[position + 1:position + 3])[0]
                address = bytes([203, 0, 113, 2]) if qtype == 1 else bytes.fromhex('20010db8000000000000000000000002')
                if qtype not in (1, 28):
                    response = body[:2] + struct.pack('!5H', 0x8180, 1, 0, 0, 0) + body[12:end]
                else:
                    response = body[:2] + struct.pack('!5H', 0x8180, 1, 1, 0, 0) + body[12:end]
                    response += b'\xc0\x0c' + struct.pack('!HHIH', qtype, 1, 1, len(address)) + address
                self.answer(response, 'application/dns-message')
            else:
                request = json.loads(body)
                response={'ip': request.get('q', '1.2.3.4'), 'company': 'Controlled test fixture', 'asn': 'AS64500', 'country': 'United States'}
                if request.get('key'):
                    response.update({field:False for field in ('is_datacenter','is_mobile','is_proxy','is_vpn','is_tor','is_crawler','is_abuser')})
                self.answer(response)

    plain = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    secure = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(cert, key)
    secure.socket = context.wrap_socket(secure.socket, server_side=True)
    threading.Thread(target=plain.serve_forever, daemon=True).start()
    print(json.dumps({'http': plain.server_port, 'https': secure.server_port}), flush=True)
    secure.serve_forever()


@contextlib.contextmanager
def trusted_fixture(directory):
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('Native tests and temporary CA trust require a disposable Actions runner')
    cert, key = directory/'fixture.crt', directory/'fixture.key'
    names = ['api.ipify.org', 'api6.ipify.org', 'api.ip.sb', 'api.ipapi.is', 'www.cloudflare.com',
             'rdap.org', 'rdap.arin.net', 'api.globalping.io', 'speed.cloudflare.com',
             'claude.ai', 'claude.com', 'api.anthropic.com', 'platform.claude.com']
    run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '2',
         '-subj', '/CN=Clash of Rust disposable test fixture',
         '-addext', 'subjectAltName=' + ','.join(['IP:127.0.0.1', 'IP:1.1.1.1', 'IP:8.8.8.8', *('DNS:'+name for name in names)]),
         '-keyout', key, '-out', cert])
    digest = hashlib.sha1(ssl.PEM_cert_to_DER_cert(cert.read_text())).hexdigest()
    system = host_system()
    installed = None
    try:
        if system == 'linux':
            installed = Path('/usr/local/share/ca-certificates')/('cor-test-'+uuid.uuid4().hex+'.crt')
            run(['sudo', 'cp', cert, installed])
            run(['sudo', 'update-ca-certificates'])
        elif system == 'macos':
            run(['sudo', 'security', 'add-trusted-cert', '-d', '-r', 'trustRoot', '-k', '/Library/Keychains/System.keychain', cert])
        else:
            run(['certutil', '-addstore', 'Root', cert])
        yield cert, key
    finally:
        if installed:
            run(['sudo', 'rm', '-f', installed])
            run(['sudo', 'update-ca-certificates'])
        elif system == 'macos':
            subprocess.run(['sudo', 'security', 'delete-certificate', '-Z', digest, '/Library/Keychains/System.keychain'], check=False)
        else:
            subprocess.run(['certutil', '-delstore', 'Root', digest], check=False)


def check(build, output):
    manifest = json.loads((build/'manifest.json').read_text())
    report = output/'results.json'
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='cor-claude-ca-') as temporary:
        with trusted_fixture(Path(temporary)) as (cert, key):
            env = {'CLASH_CLAUDE_TEST_REPORT': str(report.resolve()), 'CLASH_CLAUDE_TEST_CERT': str(cert),
                   'CLASH_CLAUDE_TEST_KEY': str(key), 'CLASH_CLAUDE_TEST_SCRIPT': str(Path(__file__).resolve()),
                   'CLASH_CLAUDE_TEST_PYTHON': sys.executable}
            original = {name: os.environ.get(name) for name in env}
            os.environ.update(env)
            try:
                spec = importlib.util.spec_from_file_location('native_proxy_tests', ROOT/'scripts/test-proxy-modes.py')
                module = importlib.util.module_from_spec(spec)
                spec.loader.exec_module(module)
                module.check(build/manifest['tests']['core_integration'], build/'payload', test_case='claude_protection_round_trip_and_guard')
            finally:
                for name, value in original.items():
                    if value is None:
                        os.environ.pop(name, None)
                    else:
                        os.environ[name] = value
    if not report.exists():
        raise RuntimeError('Native test did not produce its measured result report')


def resolve(head, run_id):
    release = workflow_artifacts.release_module()
    repo = release.repository()
    client = release.GitHub(repo, release.credential(repo))
    deadline = time.monotonic() + 2700
    while time.monotonic() < deadline:
        runs = client('/actions/workflows/ci.yml/runs?per_page=30')['workflow_runs']
        candidates = [r for r in runs if r['head_sha'] == head and r['event'] in ('pull_request', 'push')]
        if run_id:
            candidates = [client(f'/actions/runs/{int(run_id)}')]
        if candidates:
            selected = max(candidates, key=lambda r:r['id'])
            if selected['head_sha'] != head or selected['path'].split('@')[0] != '.github/workflows/ci.yml':
                raise ValueError('CI source does not match the temporary test source')
            if selected['status'] == 'completed':
                if selected['conclusion'] != 'success':
                    raise RuntimeError('Build CI did not pass: '+selected['html_url'])
                return selected['id'], selected['head_sha']
        time.sleep(15)
    raise TimeoutError('Build CI did not finish within 45 minutes')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    server = sub.add_parser('serve')
    server.add_argument('--cert', type=Path, required=True)
    server.add_argument('--key', type=Path, required=True)
    test = sub.add_parser('check')
    test.add_argument('--build', type=Path, required=True)
    test.add_argument('--output', type=Path, required=True)
    choose = sub.add_parser('resolve')
    choose.add_argument('--head', required=True)
    choose.add_argument('--ci-run-id')
    args = parser.parse_args()
    if args.command == 'serve':
        serve(args.cert, args.key)
    elif args.command == 'check':
        check(args.build.resolve(strict=True), args.output.resolve())
    else:
        run_id, commit = resolve(args.head, args.ci_run_id)
        with open(os.environ['GITHUB_OUTPUT'], 'a') as stream:
            stream.write(f'ci_run_id={run_id}\ncommit={commit}\n')


if __name__ == '__main__':
    main()
