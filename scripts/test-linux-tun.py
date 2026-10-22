"""Test installed TUN authorization in a disposable network namespace (root).

The installed CLI is checked as root without a display or a polkit authority;
the Rust integration test then runs as the specified ordinary user. No host
routes, desktop proxy settings, or existing capability attributes are left changed.
"""
import argparse
import contextlib
import errno
from http.client import HTTPConnection
import json
import os
from pathlib import Path
import pwd
import subprocess
import sys
import tempfile
import time


def run(command, **kwargs):
    return subprocess.run([str(item) for item in command], check=True, **kwargs)


@contextlib.contextmanager
def isolated_resolved():
    # Called only after unshare --mount --net. No sockets, DNS files, routes,
    # or daemons from the host namespace are used or changed.
    run(['mount', '--make-rprivate', '/'])
    run(['mount', '-t', 'tmpfs', '-o', 'mode=0755', 'tmpfs', '/run'])
    # resolved creates its own runtime directory with systemd-resolve ownership
    # before dropping root. A precreated root-owned directory makes it exit.
    for directory in ('/run/dbus', '/run/systemd/netif/links'):
        Path(directory).mkdir(parents=True, exist_ok=True)
    daemon = next((p for p in (Path('/usr/lib/systemd/systemd-resolved'),
                              Path('/lib/systemd/systemd-resolved')) if p.is_file()), None)
    assert daemon and Path('/usr/bin/resolvectl').is_file(), 'systemd-resolved is required'
    with tempfile.TemporaryDirectory(prefix='clash-tun-dns-test-') as folder:
        config = Path(folder)/'dbus.conf'
        # No service activation and no PolicyKit authority or authentication
        # agent: DNS writes must use the authorized native DNS bridge.
        config.write_text('<busconfig><type>system</type>'
                          '<listen>unix:path=/run/dbus/system_bus_socket</listen>'
                          '<policy context="default"><allow user="*"/><allow own="*"/>'
                          '<allow send_destination="*"/><allow receive_sender="*"/></policy>'
                          '</busconfig>')
        with contextlib.ExitStack() as stack:
            log = stack.enter_context((Path(folder)/'daemons.log').open('w+'))
            def print_log():
                log.flush()
                log.seek(0)
                print(log.read())
            # Dump diagnostics even if a daemon exits before reaching yield.
            stack.callback(print_log)
            processes = []
            def terminate():
                for process in reversed(processes):
                    if process.poll() is None:
                        process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
            stack.callback(terminate)
            processes.append(subprocess.Popen(['dbus-daemon', '--nofork', '--config-file='+str(config)],
                                              stdout=log, stderr=log))
            for _ in range(100):
                if Path('/run/dbus/system_bus_socket').exists():
                    break
                assert processes[0].poll() is None, 'Isolated D-Bus exited'
                time.sleep(0.05)
            else:
                raise RuntimeError('Isolated D-Bus did not start')
            processes.append(subprocess.Popen([str(daemon)], stdout=log, stderr=log,
                                               env=dict(os.environ, SYSTEMD_LOG_TARGET='console')))
            for _ in range(100):
                ready = subprocess.run(['busctl', '--system', '--timeout=1', 'status',
                                        'org.freedesktop.resolve1'], capture_output=True)
                if ready.returncode == 0:
                    break
                assert processes[-1].poll() is None, 'Isolated systemd-resolved exited'
                time.sleep(0.05)
            else:
                raise RuntimeError('Isolated systemd-resolved did not start')
            denied = subprocess.run(['busctl', '--system', '--timeout=1', 'status',
                                     'org.freedesktop.PolicyKit1'], capture_output=True)
            assert denied.returncode != 0, 'A PolicyKit authority must not be available'
            yield


@contextlib.contextmanager
def isolated_uplink():
    """Real dual-stack peer, encrypted DNS and node, outside the client's TUN.

    Already inside private mount/network namespaces. Resolver IPs and CA trust
    exist only here; neither the host nor the Internet is involved.
    """
    from tun_dns_fixture import BOOTSTRAP, PUBLIC, NODE, WEBSITE
    namespace = 'cor-upstream'
    with tempfile.TemporaryDirectory(prefix='cor-upstream-') as temporary:
        directory = Path(temporary)
        directory.chmod(0o755)
        ca, key = directory/'root.crt', directory/'root.key'
        run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
             '-subj', '/CN=Disposable TUN test root', '-addext', 'basicConstraints=critical,CA:TRUE',
             '-keyout', key, '-out', ca], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        run(['openssl', 'req', '-new', '-newkey', 'rsa:2048', '-nodes', '-subj', '/CN=Disposable DNS',
             '-keyout', directory/'server.key', '-out', directory/'server.csr'],
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        addresses = (*BOOTSTRAP, *PUBLIC)
        certificate_addresses = (*addresses, '192.0.2.254')
        extensions = directory/'server.ext'
        extensions.write_text('basicConstraints=critical,CA:FALSE\nextendedKeyUsage=serverAuth\n'
                              'subjectAltName='+','.join('IP:'+ip for ip in certificate_addresses)+',DNS:resolver.fixture.test\n')
        run(['openssl', 'x509', '-req', '-in', directory/'server.csr', '-CA', ca, '-CAkey', key,
             '-CAcreateserial', '-days', '1', '-extfile', extensions, '-out', directory/'server.crt'],
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        trust = directory/'trust.pem'
        trust.write_bytes(Path('/etc/ssl/certs/ca-certificates.crt').read_bytes()+b'\n'+ca.read_bytes())
        run(['mount', '--bind', trust, '/etc/ssl/certs/ca-certificates.crt'])
        run(['ip', 'netns', 'add', namespace])
        process = None
        try:
            run(['ip', 'link', 'add', 'uplink', 'type', 'veth', 'peer', 'name', 'peer'])
            run(['ip', 'link', 'set', 'peer', 'netns', namespace])
            for prefix, interface, v4, v6 in [
                ([], 'uplink', '192.0.2.1/24', '2001:db8:1::1/64'),
                (['ip', 'netns', 'exec', namespace], 'peer', '192.0.2.254/24', '2001:db8:1::fe/64'),
            ]:
                run([*prefix, 'ip', 'link', 'set', 'lo', 'up'])
                run([*prefix, 'ip', 'addr', 'add', v4, 'dev', interface])
                run([*prefix, 'ip', '-6', 'addr', 'add', v6, 'dev', interface, 'nodad'])
                run([*prefix, 'ip', 'link', 'set', interface, 'up'])
            for address in (*addresses, *NODE, *WEBSITE):
                family, length = ('-6', 128) if ':' in address else ('-4', 32)
                run(['ip', '-n', namespace, family, 'addr', 'add', f'{address}/{length}', 'dev', 'lo'])
            run(['ip', 'route', 'add', 'default', 'via', '192.0.2.254', 'dev', 'uplink'])
            run(['ip', '-6', 'route', 'add', 'default', 'via', '2001:db8:1::fe', 'dev', 'uplink'])
            with (directory/'server.log').open('w+') as log:
                process = subprocess.Popen(['ip', 'netns', 'exec', namespace, sys.executable,
                                            Path(__file__).with_name('tun_dns_fixture.py'), directory],
                                           stdout=log, stderr=log)
                try:
                    for _ in range(100):
                        assert process.poll() is None, 'Upstream fixture exited'
                        if (directory/'ready').exists():
                            break
                        time.sleep(0.05)
                    else:
                        raise RuntimeError('Upstream fixture did not become ready')
                    yield
                finally:
                    process.terminate()
                    process.wait(timeout=5)
                    log.seek(0)
                    print(log.read())
        finally:
            # Print tables on failure too, so a route regression is actionable.
            for family in ('-4', '-6'):
                for args in (['rule'], ['route', 'show', 'table', 'all']):
                    subprocess.run(['ip', family, *args], check=False)
            if process is not None and process.poll() is None:
                process.kill()
                process.wait()
            subprocess.run(['ip', 'netns', 'delete', namespace], check=False)
            run(['umount', '/etc/ssl/certs/ca-certificates.crt'])


def root_cli_smoke():
    """Exercise the installed headless root path before granting user TUN access."""
    cli = '/opt/clash-of-rust/cor'
    launcher = '/opt/clash-of-rust/clash-tun-launcher'
    rejected = subprocess.run([launcher, '-v'], capture_output=True, text=True, timeout=10)
    assert rejected.returncode != 0 and '不能以 root 运行' in rejected.stderr
    env = dict(os.environ)
    for key in ('DISPLAY', 'WAYLAND_DISPLAY', 'WAYLAND_SOCKET', 'DBUS_SESSION_BUS_ADDRESS'):
        env.pop(key, None)
    with tempfile.TemporaryDirectory(prefix='cor-root-cli-') as temporary:
        directory = Path(temporary)
        command = [cli, '--data-dir', str(directory/'data'), '--json', '--timeout', '30']

        def invoke(*arguments, success=True):
            output = subprocess.run([*command, *map(str, arguments)], env=env,
                                    capture_output=True, text=True, timeout=45)
            if success:
                assert output.returncode == 0, f'root cor {arguments}: {output.stderr}'
                return json.loads(output.stdout)['data']
            assert output.returncode != 0, f'Invalid configuration was accepted: {arguments}'
            return output.stderr

        def http(host, port, path, secret=None):
            connection = HTTPConnection(host, port, timeout=10)
            try:
                headers = {'Authorization': 'Bearer '+secret} if secret else {}
                connection.request('GET', path, headers=headers)
                response = connection.getresponse()
                assert response.status == 200, f'HTTP {host}:{port}: {response.status}'
                return response.read()
            finally:
                connection.close()

        try:
            invoke('config', 'check')
            assert invoke('start')['running']
            settings = json.loads((directory/'data/settings.json').read_text())
            mixed, controller = settings['mixed_port'], settings['controller_port']
            secret = settings['secret']

            def api(path):
                return json.loads(http('127.0.0.1', controller, '/'+path, secret))

            assert http('127.0.0.1', mixed, 'http://203.0.113.20/auto-check') == b'protected-ok'
            invalid = directory/'invalid.yaml'
            invalid.write_text('proxies: [{name: InvalidRootNode, type: COR_INVALID_PROXY_TYPE}]\n'
                               'rules: ["MATCH,DIRECT"]\n')
            error = invoke('profile', 'import', invalid, '--name', 'invalid', success=False)
            assert 'mihomo 配置校验失败' in error and 'COR_INVALID_PROXY_TYPE'.lower() in error.lower(), error
            assert secret not in error, 'Validation errors exposed the controller secret'

            # Use only the isolated upstream; this test never needs public DNS.
            profile = directory/'root.yaml'
            profile.write_text('proxies: []\nrules: ["MATCH,DIRECT"]\n'
                               'dns: {enable: true, enhanced-mode: fake-ip, '
                               'nameserver: ["https://192.0.2.254/dns-query#DIRECT"]}\n'
                               'tun: {device: CorRootTest, route-address: [203.0.113.0/24], dns-hijack: []}\n')
            invoke('config', 'set', 'dns_override', 'false')
            invoke('profile', 'import', profile, '--name', 'root-network')
            invoke('profile', 'use', 'root-network')
            invoke('proxy', 'tun')
            assert api('configs')['tun']['enable']
            before = api('connections')['downloadTotal']
            assert http('203.0.113.20', 80, '/auto-check') == b'protected-ok'
            assert api('connections')['downloadTotal'] > before, 'Root traffic bypassed the TUN'
            invoke('stop')
            # A fresh root daemon must also restore a persisted TUN mode.
            assert invoke('start')['running'] and api('configs')['tun']['enable']
            assert http('203.0.113.20', 80, '/auto-check') == b'protected-ok'
            invoke('proxy', 'off')
            assert not api('configs')['tun']['enable']
        except Exception:
            diagnostics = subprocess.run([*command, 'logs', '--limit', '40'], env=env,
                                         capture_output=True, text=True, timeout=45)
            print(diagnostics.stdout+diagnostics.stderr, flush=True)
            raise
        finally:
            invoke('stop')
        assert not (directory/'data/control.json').exists(), 'Root daemon survived cor stop'
        links = json.loads(subprocess.check_output(['ip', '-json', 'link', 'show'], text=True))
        assert not any(link['ifname'] == 'CorRootTest' for link in links), 'Root TUN was not removed'
    print('PASS: installed root CLI validation, startup, proxy traffic, TUN, restart and cleanup', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--test-binary', type=Path, required=True)
    parser.add_argument('--uid', type=int, required=True)
    parser.add_argument('--inside-namespace', action='store_true')
    parser.add_argument('--test-case', default='basic_system_proxy_and_tun', choices=('basic_system_proxy_and_tun',))
    args = parser.parse_args()
    assert os.geteuid() == 0 and args.uid != 0
    binary = args.test_binary.resolve(strict=True)
    user = pwd.getpwuid(args.uid)

    def ordinary_user():
        os.setgroups([])
        os.setgid(user.pw_gid)
        os.setuid(user.pw_uid)

    if args.inside_namespace:
        with isolated_resolved(), isolated_uplink():
            root_cli_smoke()
            run(['/opt/clash-of-rust/clash-of-rust', '--authorize-tun'])
            run([binary, args.test_case, '--exact',
                 '--ignored', '--test-threads=1', '--nocapture'],
                env=dict(os.environ, CLASH_TUN_TEST_NAMESPACE='1', CLASH_PROXY_SMOKE='1',
                         MIHOMO_TEST_RESOURCES='/opt/clash-of-rust/resources', MIHOMO_TEST_CLI='/opt/clash-of-rust/cor', GSETTINGS_BACKEND='memory'),
                preexec_fn=ordinary_user, timeout=240)
        return

    core = '/opt/clash-of-rust/resources/mihomo'
    launcher = '/opt/clash-of-rust/clash-tun-launcher'
    dns_helper = '/opt/clash-of-rust/libexec/resolvectl'
    def attribute(path):
        try:
            return os.getxattr(path, 'security.capability')
        except OSError as error:
            if error.errno != errno.ENODATA:
                raise
            return None
    original = {path: attribute(path) for path in (core, launcher)}
    original_dns_mode = os.stat(dns_helper).st_mode & 0o7777
    try:
        run(['unshare', '--net', '--mount', sys.executable, Path(__file__).resolve(),
             '--inside-namespace', '--test-binary', binary, '--uid', args.uid, '--test-case', args.test_case], timeout=260)
        print(json.dumps({'basic': True, 'native_authorization': True, 'root_cli': True, 'ordinary_user': args.uid,
                          'tun_traffic_cleanup': True, 'network_namespace': True,
                          'resolved_without_polkit': True}))
    finally:
        os.chmod(dns_helper, original_dns_mode)
        for path, value in original.items():
            if value is None:
                if attribute(path) is not None:
                    os.removexattr(path, 'security.capability')
            else:
                os.setxattr(path, 'security.capability', value)


if __name__ == '__main__':
    main()
