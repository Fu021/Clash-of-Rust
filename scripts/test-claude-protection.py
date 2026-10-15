"""Temporary disposable-runner test, controlled HTTPS evidence and UI captures."""
import argparse
import base64
import contextlib
import hashlib
import http.server
import importlib.util
import json
import os
from pathlib import Path
import shutil
import ssl
import socket
import socketserver
import struct
import subprocess
import sys
import tempfile
import threading
import time
import uuid
import urllib.parse

from build_support import ROOT, run, write_json, host_system
import workflow_artifacts


def serve(cert, key):
    metrics = {'doh': 0, 'hosts': {}}
    original_dns = json.loads(os.environ.get('CLASH_CLAUDE_ORIGINAL_DNS', '[]'))
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
            if self.path.startswith('/dns-query'):
                encoded = urllib.parse.parse_qs(urllib.parse.urlsplit(self.path).query).get('dns', [''])[0]
                self.dns_response(base64.urlsafe_b64decode(encoded + '='*(-len(encoded)%4)))
            elif self.path.startswith('/metrics'):
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
                self.dns_response(body)
            else:
                request = json.loads(body)
                response={'ip': request.get('q', '1.2.3.4'), 'company': 'Controlled test fixture', 'asn': 'AS64500', 'country': 'United States'}
                if request.get('key'):
                    response.update({field:False for field in ('is_datacenter','is_mobile','is_proxy','is_vpn','is_tor','is_crawler','is_abuser')})
                self.answer(response)

        def dns_response(self, body):
            with lock:
                metrics['doh'] += 1
            position = 12
            labels = []
            while position < len(body) and body[position]:
                labels.append(body[position+1:position+1+body[position]].decode('ascii').lower())
                position += 1 + body[position]
            name = '.'.join(labels)
            if original_dns and not (name.endswith('.bash.ws') or name == 'dns-check.example'):
                # The native Mac system-resolver test changes DNS for this
                # disposable VM. Preserve runner/control-plane resolution by
                # forwarding unrelated questions to the original LAN resolver.
                # It is intentionally outside the scoped TUN routes.
                for server in original_dns:
                    if server in ('1.1.1.1', '8.8.8.8'):
                        continue
                    family = socket.AF_INET6 if ':' in server else socket.AF_INET
                    try:
                        with socket.socket(family, socket.SOCK_DGRAM) as upstream:
                            upstream.settimeout(2)
                            upstream.sendto(body, (server, 53))
                            reply = upstream.recv(65535)
                        return self.answer(reply, 'application/dns-message')
                    except OSError:
                        continue
                raise RuntimeError('Original resolver unavailable for runner DNS')
            end = position + 5
            qtype = struct.unpack('!H', body[position + 1:position + 3])[0]
            address = bytes([203, 0, 113, 2]) if qtype == 1 else bytes.fromhex('20010db8000000000000000000000002')
            if qtype not in (1, 28):
                response = body[:2] + struct.pack('!5H', 0x8180, 1, 0, 0, 0) + body[12:end]
            else:
                response = body[:2] + struct.pack('!5H', 0x8180, 1, 1, 0, 0) + body[12:end]
                response += b'\xc0\x0c' + struct.pack('!HHIH', qtype, 1, 1, len(address)) + address
            self.answer(response, 'application/dns-message')

    class LocalServer(http.server.ThreadingHTTPServer):
        def server_bind(self):
            # HTTPServer's getfqdn performs blocking reverse DNS on macOS;
            # this fixture serves loopback only and never needs that lookup.
            socketserver.TCPServer.server_bind(self)
            self.server_name = 'localhost'
            self.server_port = self.server_address[1]

    plain = LocalServer(('127.0.0.1', 0), Handler)
    secure = LocalServer(('127.0.0.1', 0), Handler)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(cert, key)
    secure.socket = context.wrap_socket(secure.socket, server_side=True)
    threading.Thread(target=plain.serve_forever, daemon=True).start()
    print(json.dumps({'http': plain.server_port, 'https': secure.server_port}), flush=True)
    secure.serve_forever()


def route_probe(mixed, target, port, host, tls):
    """Keep a controlled SOCKS connection open for native route assertions."""
    assert os.environ.get('GITHUB_ACTIONS') == 'true'
    def receive(stream, size):
        result = b''
        while len(result) < size:
            chunk = stream.recv(size-len(result))
            assert chunk, 'Controlled SOCKS connection closed'
            result += chunk
        return result
    stream = socket.create_connection(('127.0.0.1', mixed), timeout=8)
    stream.sendall(b'\x05\x01\x00')
    assert receive(stream, 2) == b'\x05\x00'
    if target == '203.0.113.77':
        address = b'\x01'+socket.inet_aton(target)
    else:
        address = b'\x03'+bytes([len(target)])+target.encode('ascii')
    stream.sendall(b'\x05\x01\x00'+address+struct.pack('!H', port))
    reply = receive(stream, 4)
    assert reply[:2] == b'\x05\x00', reply
    size = {1:4, 4:16}.get(reply[3])
    if reply[3] == 3:
        size = receive(stream, 1)[0]
    receive(stream, size+2)
    if tls:
        context = ssl.create_default_context(cafile=os.environ['CLASH_CLAUDE_TEST_CA'])
        stream = context.wrap_socket(stream, server_hostname=host)
    stream.sendall(f'GET /hold HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n'.encode('ascii'))
    assert b'200' in stream.recv(4096), 'Controlled route did not reach fixture'
    print(json.dumps({'ready': True}), flush=True)
    sys.stdin.read()
    stream.close()


@contextlib.contextmanager
def trusted_fixture(directory):
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('Native tests and temporary CA trust require a disposable Actions runner')
    cert, key = directory/'fixture.crt', directory/'fixture.key'
    ca, ca_key = directory/'ca.crt', directory/'ca.key'
    ca_config = directory/'ca.cnf'
    ca_config.write_text('[req]\nprompt=no\ndistinguished_name=dn\nx509_extensions=v3_ca\n[dn]\nCN=Clash of Rust disposable test root\n[v3_ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\nsubjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid:always\n')
    names = ['api.ipify.org', 'api6.ipify.org', 'api.ip.sb', 'api.ipapi.is', 'www.cloudflare.com',
             'rdap.org', 'rdap.arin.net', 'api.globalping.io', 'speed.cloudflare.com',
             'claude.ai', 'claude.com', 'api.anthropic.com', 'platform.claude.com',
             's-cdn.anthropic.com', 'outside.ech.test']
    run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '2',
         '-sha256', '-config', ca_config,
         '-keyout', ca_key, '-out', ca])
    csr, extensions = directory/'fixture.csr', directory/'server.ext'
    extensions.write_text('basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectKeyIdentifier=hash\nauthorityKeyIdentifier=keyid,issuer\nsubjectAltName=' + ','.join(['IP:127.0.0.1', 'IP:1.1.1.1', 'IP:8.8.8.8', *('DNS:'+name for name in names)])+'\n')
    run(['openssl','req','-new','-newkey','rsa:2048','-nodes','-subj','/CN=Clash of Rust controlled service','-keyout',key,'-out',csr])
    run(['openssl','x509','-req','-in',csr,'-CA',ca,'-CAkey',ca_key,'-CAcreateserial','-days','2','-extfile',extensions,'-out',cert])
    run(['openssl','verify','-CAfile',ca,cert])
    digest = hashlib.sha1(ssl.PEM_cert_to_DER_cert(ca.read_text())).hexdigest()
    system = host_system()
    installed = None
    try:
        if system == 'linux':
            installed = Path('/usr/local/share/ca-certificates')/('cor-test-'+uuid.uuid4().hex+'.crt')
            run(['sudo', 'cp', ca, installed])
            run(['sudo', 'update-ca-certificates'])
        elif system == 'macos':
            run(['sudo', 'security', 'add-trusted-cert', '-d', '-r', 'trustRoot', '-k', '/Library/Keychains/System.keychain', ca])
        else:
            run(['certutil', '-addstore', 'Root', ca])
        yield cert, key, ca
    finally:
        if installed:
            run(['sudo', 'rm', '-f', installed])
            run(['sudo', 'update-ca-certificates'])
        elif system == 'macos':
            subprocess.run(['sudo', 'security', 'delete-certificate', '-Z', digest, '/Library/Keychains/System.keychain'], check=False)
        else:
            subprocess.run(['certutil', '-delstore', 'Root', digest], check=False)


@contextlib.contextmanager
def desktop_session(output):
    if host_system() != 'linux':
        yield
        return
    # Filesystem Unix sockets remain reachable from the isolated net namespace;
    # the default abstract session-bus socket does not.
    original = os.environ.get('DBUS_SESSION_BUS_ADDRESS')
    with tempfile.TemporaryDirectory(prefix='cor-notify-bus-') as directory, (output/'desktop-session.log').open('w') as log:
        address = 'unix:path='+str(Path(directory)/'bus')
        bus = subprocess.Popen(['dbus-daemon','--session','--nofork','--address='+address],stdout=log,stderr=log)
        os.environ['DBUS_SESSION_BUS_ADDRESS'] = address
        try:
            deadline = time.monotonic()+10
            while not (Path(directory)/'bus').exists():
                assert bus.poll() is None and time.monotonic()<deadline, 'Filesystem notification bus did not start'
                time.sleep(.05)
            # Deliberately run without a notification daemon: accepting Notify
            # is insufficient evidence that a native warning is visible.
            ready = subprocess.run(['gdbus','call','--session','--dest','org.freedesktop.DBus','--object-path','/org/freedesktop/DBus','--method','org.freedesktop.DBus.NameHasOwner','org.freedesktop.Notifications'],capture_output=True,text=True,check=True)
            assert 'false' in ready.stdout, 'Unexpected desktop notification daemon'
            yield
        finally:
            for process in (bus,):
                if process:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill();process.wait()
            if original is None:
                os.environ.pop('DBUS_SESSION_BUS_ADDRESS',None)
            else:
                os.environ['DBUS_SESSION_BUS_ADDRESS']=original


@contextlib.contextmanager
def ipv6_uplink(output):
    # Mihomo checks physical-interface IPv6 at process startup. These VMs are
    # IPv4-only; add only a documentation address, no IPv6 default gateway.
    system=host_system()
    if system=='linux':
        yield  # The existing isolated namespace supplies its dummy IPv6 uplink.
        return
    address='2001:db8:1::1'
    interface=None
    original_dhcp=None
    if system=='macos':
        route=subprocess.check_output(['/sbin/route','-n','get','default'],text=True)
        interface=next(line.split(':',1)[1].strip() for line in route.splitlines() if line.strip().startswith('interface:'))
        run(['sudo','/sbin/ifconfig',interface,'inet6',address,'prefixlen','64','alias'])
    else:
        command="$ErrorActionPreference='Stop'; $i=(Get-NetRoute -AddressFamily IPv4 -DestinationPrefix '0.0.0.0/0' | Sort-Object RouteMetric | Select-Object -First 1).InterfaceIndex; $d=(Get-NetIPInterface -InterfaceIndex $i -AddressFamily IPv6).Dhcp.ToString(); New-NetIPAddress -InterfaceIndex $i -AddressFamily IPv6 -IPAddress '2001:db8:1::1' -PrefixLength 64 -PolicyStore ActiveStore | Out-Null; @{index=$i;dhcp=$d} | ConvertTo-Json -Compress"
        previous=json.loads(subprocess.check_output(['powershell','-NoProfile','-NonInteractive','-Command',command],text=True))
        interface=str(int(previous['index']))
        original_dhcp=previous['dhcp']
        assert original_dhcp in ('Enabled','Disabled')
    try:
        time.sleep(2)  # Complete duplicate-address detection before core startup.
        write_json(output/'ipv6-test-uplink.json',{'interface':interface,'address':address,'scope':'temporary documentation address; no IPv6 default route'})
        yield
    finally:
        if system=='macos':
            subprocess.run(['sudo','/sbin/ifconfig',interface,'inet6',address,'delete'],check=True)
        else:
            command="Remove-NetIPAddress -InterfaceIndex "+interface+" -IPAddress '2001:db8:1::1' -Confirm:$false -ErrorAction Stop; Set-NetIPInterface -InterfaceIndex "+interface+" -AddressFamily IPv6 -Dhcp "+original_dhcp+" -PolicyStore ActiveStore -ErrorAction Stop"
            subprocess.run(['powershell','-NoProfile','-NonInteractive','-Command',command],check=True)


def check(build, output):
    manifest = json.loads((build/'manifest.json').read_text())
    report = output/'results.json'
    output.mkdir(parents=True, exist_ok=True)
    with desktop_session(output), ipv6_uplink(output), tempfile.TemporaryDirectory(prefix='cor-claude-ca-') as temporary:
        with trusted_fixture(Path(temporary)) as (cert, key, ca):
            env = {'CLASH_CLAUDE_TEST_REPORT': str(report.resolve()), 'CLASH_CLAUDE_TEST_CERT': str(cert),
                   'CLASH_CLAUDE_TEST_CA': str(ca),
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


def clean():
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('Only disposable workflow scratch output may be cleaned')
    if (ROOT/'target').is_symlink():
        raise RuntimeError('Unexpected symlink in workflow scratch root')
    for name in ('claude-build','claude-validation','macos-report'):
        path=ROOT/'target'/name
        if path.is_symlink():
            path.unlink()
        elif path.is_dir():
            shutil.rmtree(path)
        elif path.exists():
            raise RuntimeError('Unexpected file in workflow scratch output')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    sub.add_parser('clean')
    server = sub.add_parser('serve')
    server.add_argument('--cert', type=Path, required=True)
    server.add_argument('--key', type=Path, required=True)
    probe = sub.add_parser('route-probe')
    probe.add_argument('--mixed', type=int, required=True)
    probe.add_argument('--target', choices=['203.0.113.77', 'claude.ai', 's-cdn.anthropic.com'], required=True)
    probe.add_argument('--port', type=int, required=True)
    probe.add_argument('--host', choices=['claude.ai', 's-cdn.anthropic.com', 'outside.ech.test'], required=True)
    probe.add_argument('--tls', action='store_true')
    test = sub.add_parser('check')
    test.add_argument('--build', type=Path, required=True)
    test.add_argument('--output', type=Path, required=True)
    choose = sub.add_parser('resolve')
    choose.add_argument('--head', required=True)
    choose.add_argument('--ci-run-id')
    args = parser.parse_args()
    if args.command == 'serve':
        serve(args.cert, args.key)
    elif args.command == 'route-probe':
        route_probe(args.mixed, args.target, args.port, args.host, args.tls)
    elif args.command == 'check':
        check(args.build.resolve(strict=True), args.output.resolve())
    elif args.command == 'clean':
        clean()
    else:
        run_id, commit = resolve(args.head, args.ci_run_id)
        with open(os.environ['GITHUB_OUTPUT'], 'a') as stream:
            stream.write(f'ci_run_id={run_id}\ncommit={commit}\n')


if __name__ == '__main__':
    main()
