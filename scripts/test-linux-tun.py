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
import re
import shutil
import signal
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


@contextlib.contextmanager
def isolated_accounts():
    """Account/sudo policy changes exist only in this private mount namespace."""
    with tempfile.TemporaryDirectory(prefix='cor-account-matrix-') as temporary:
        root = Path(temporary)
        root.chmod(0o755)
        identities = []
        used = {entry.pw_uid for entry in pwd.getpwall()}
        for uid in range(60001, 60101):
            if uid in used:
                continue
            name = 'cor-matrix-'+str(len(identities))
            home = root/name
            home.mkdir(mode=0o700)
            os.chown(home, uid, uid)
            identities.append({'name': name, 'uid': uid, 'home': home})
            if len(identities) == 2:
                break
        assert len(identities) == 2
        for name in ('passwd', 'group', 'shadow', 'gshadow'):
            source = Path('/etc')/name
            copy = root/name
            shutil.copy2(source, copy)
            with copy.open('a') as stream:
                for user in identities:
                    username, uid, home = user['name'], user['uid'], user['home']
                    stream.write({
                        'passwd': f'{username}:x:{uid}:{uid}::{home}:/bin/sh\n',
                        'group': f'{username}:x:{uid}:\n',
                        'shadow': f'{username}:!:20000:0:99999:7:::\n',
                        'gshadow': f'{username}:!::\n',
                    }[name])
            run(['mount', '--bind', copy, source])
        sudoers = root/'sudoers.d'
        shutil.copytree('/etc/sudoers.d', sudoers)
        rule = sudoers/'cor-matrix'
        rule.write_text(''.join(f'{user["name"]} ALL=(root) NOPASSWD: /opt/clash-of-rust/cor\n'
                                for user in identities))
        rule.chmod(0o440)
        run(['visudo', '-cf', rule], stdout=subprocess.DEVNULL)
        run(['mount', '--bind', sudoers, '/etc/sudoers.d'])
        try:
            yield root, identities
        finally:
            # The enclosing namespace will disappear too; unmount now so getent
            # and sudo never retain paths in the removed temporary directory.
            for path in ('/etc/sudoers.d', '/etc/gshadow', '/etc/shadow', '/etc/group', '/etc/passwd'):
                run(['umount', path])


def account_cli_matrix():
    """Exercise installed commands as a user, sudo caller, selected user and root."""
    cli = '/opt/clash-of-rust/cor'
    catalogue = set(re.findall(r'command!\(\s*"([^"]+)"',
                              (Path(__file__).resolve().parents[1]/'apps/cli/src/args.rs').read_text()))
    with isolated_accounts() as (temporary, users):
        root_home = temporary/'root'
        root_home.mkdir(mode=0o700)
        base_env = dict(os.environ, HOME=str(root_home), USER='root', LOGNAME='root')
        for key in ('DISPLAY', 'WAYLAND_DISPLAY', 'WAYLAND_SOCKET', 'DBUS_SESSION_BUS_ADDRESS',
                    'CLASH_OF_RUST_DATA_DIR', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME',
                    'XDG_CACHE_HOME', 'XDG_STATE_HOME', 'XDG_RUNTIME_DIR',
                    'SUDO_UID', 'SUDO_GID', 'SUDO_USER', 'SUDO_COMMAND'):
            base_env.pop(key, None)

        def invoke(user, mode, *arguments, success=True, json_output=True, json_lines=False, text=None):
            env = dict(base_env)
            kwargs = {}
            prefix = []
            if user is not None:
                env.update(HOME=str(user['home']), USER=user['name'], LOGNAME=user['name'])
            if mode in ('user', 'sudo'):
                def drop():
                    os.setgroups([])
                    os.setgid(user['uid'])
                    os.setuid(user['uid'])
                kwargs['preexec_fn'] = drop
            if mode == 'sudo':
                prefix = ['/usr/bin/sudo', '-n', '--']
            command = [*prefix, cli, '--timeout', '15']
            if json_output:
                command += ['--jsonl' if json_lines else '--json']
            if mode == 'selected':
                command += ['--user', user['name']]
            output = subprocess.run([*command, *map(str, arguments)], env=env, input=text,
                                    capture_output=True, text=True, timeout=35, **kwargs)
            assert (output.returncode == 0) == success, (
                mode, user and user['name'], arguments, output.returncode, output.stdout, output.stderr)
            if success and json_output:
                try:
                    if json_lines:
                        return [json.loads(line)['data'] for line in output.stdout.splitlines()]
                    return json.loads(output.stdout)['data']
                except (ValueError, KeyError) as error:
                    raise AssertionError((mode, arguments, output.stdout, output.stderr)) from error
            return output

        def data(user):
            return user['home']/'.local/share/clashofrust'

        def snapshot(user):
            directory = data(user)
            return {str(p.relative_to(directory)): (p.read_bytes(), p.stat().st_uid)
                    for p in directory.rglob('*') if p.is_file() and p.name not in ('daemon.log', 'control.json')}

        try:
            for user in users:
                # Every catalogue entry has a usable help entry in both contexts;
                # help must not create a data directory or require a display.
                for path in sorted(catalogue):
                    for mode in ('root', 'user'):
                        output = invoke(user, mode, 'help', *path.split(), json_output=False)
                        assert '用法：cor ' in output.stdout
                assert not data(user).exists()
                invoke(user, 'user', 'profile', 'list', '--all-users', success=False)
                invoke(user, 'user', '--user', users[1]['name'], 'status', success=False)
                raw = ('proxies: []\nproxy-groups: [{name: Route, type: select, proxies: [DIRECT, REJECT]}]\n'
                       'rules: ["MATCH,Route"]\n'
                       'dns: {enable: true, enhanced-mode: fake-ip, '
                       'nameserver: ["https://192.0.2.254/dns-query#DIRECT"]}\n'
                       f'tun: {{device: CorMatrix{user["uid"]}, route-address: [203.0.113.0/24], dns-hijack: []}}\n')
                source = user['home']/'source.yaml'
                source.write_text(raw)
                source.chmod(0o600)
                os.chown(source, user['uid'], user['uid'])
                invoke(user, 'user', 'profile', 'import', source, '--name', 'Same name')
                invoke(user, 'user', 'profile', 'use', 'Same name')
                # Avoid contention between independent users' mixed listeners.
                import socket
                with socket.socket() as listener:
                    listener.bind(('127.0.0.1', 0))
                    port = listener.getsockname()[1]
                invoke(user, 'user', 'config', 'set', 'mixed_port', port)
                invoke(user, 'user', 'config', 'set', 'dns_override', 'false')
                invoke(user, 'user', 'config', 'set', 'subscription_update_interval_minutes', '0')

            alice, bob = users
            alice_before, bob_before = snapshot(alice), snapshot(bob)
            invoke(alice, 'user', '--data-dir', data(bob), 'profile', 'list', success=False)
            assert snapshot(bob) == bob_before
            invoke(None, 'root', 'profile', 'list', '--all-users', json_output=False)
            overview = invoke(None, 'root', 'profile', 'list', '--all-users')
            same_name = [p for p in overview if p['name'] == 'Same name']
            assert {p['owner'] for p in same_name} == {alice['name'], bob['name']}
            assert snapshot(alice) == alice_before and snapshot(bob) == bob_before
            assert not (root_home/'.local/share/clashofrust').exists(), 'Read-only inventory created a root backend'
            filtered = invoke(None, 'root', 'profile', 'list', '--all-users', '--filter', bob['name'])
            assert filtered and all(p['owner'] == bob['name'] for p in filtered)

            first_pid = json.loads((data(alice)/'control.json').read_text())['pid']
            for mode in ('user', 'sudo', 'selected'):
                entries = invoke(alice, mode, 'profile', 'list')
                assert any(p['name'] == 'Same name' and p['active'] for p in entries)
                assert json.loads((data(alice)/'control.json').read_text())['pid'] == first_pid
                assert invoke(alice, mode, 'profile', 'directory') == str(data(alice)/'profiles')
                invoke(alice, mode, 'config', 'get')
                invoke(alice, mode, 'config', 'set', 'text_scale', '100')
                invoke(alice, mode, 'config', 'set', 'text_scale', '1', success=False)
                assert invoke(alice, mode, 'config', 'get', 'text_scale')['text_scale'] == 100
                invoke(alice, mode, 'config', 'check')
                invoke(alice, mode, 'config', 'effective', json_output=False)
                invoke(alice, mode, 'env', '--shell', 'bash')
                invoke(alice, mode, 'geo', 'status')
                invoke(alice, mode, 'ip-check', 'list')
                invoke(alice, mode, 'claude', 'audit', '--offline')
                invoke(alice, mode, 'claude', 'protect')
                invoke(alice, mode, 'service', 'status')
                invoke(alice, mode, 'rule', 'add', 'DOMAIN,matrix.test,REJECT')
                rules = invoke(alice, mode, 'rule', 'list')['items']
                identifier = next(p['id'] for p in rules if p['rule'] == 'DOMAIN,matrix.test,REJECT')
                invoke(alice, mode, 'rule', 'edit', identifier, 'DOMAIN,matrix-edited.test,REJECT')
                invoke(alice, mode, 'rule', 'move', identifier, '2')
                invoke(alice, mode, 'rule', 'enable', identifier, 'off')
                invoke(alice, mode, 'rule', 'shortcut')
                invoke(alice, mode, 'rule', 'remove', identifier, '--yes')
                exported = alice['home']/f'{mode}-profile.yaml'
                invoke(alice, mode, 'profile', 'export', 'Same name', '--output', exported,
                       json_output=False)
                assert exported.stat().st_uid == alice['uid'] and exported.stat().st_mode & 0o777 == 0o600
                invoke(alice, mode, 'profile', 'update', 'Same name')
                invoke(alice, mode, 'profile', 'edit', 'Same name', '--name', 'Renamed')
                invoke(alice, mode, 'profile', 'edit', 'Renamed', '--name', 'Same name')
                invoke(alice, mode, 'version')
                invoke(alice, mode, 'completion', 'bash', json_output=False)
                # Admin network commands grant only the fixed helper. The core
                # and all persistent files must still belong to Alice.
                if mode != 'user':
                    invoke(alice, mode, 'start')
                    invoke(alice, mode, 'proxy', 'tun')
                    assert invoke(alice, mode, 'status')['proxy'] == 'tun'
                    pid = json.loads((data(alice)/'control.json').read_text())['pid']
                    uids = next(line for line in Path(f'/proc/{pid}/status').read_text().splitlines()
                                if line.startswith('Uid:')).split()[1:]
                    assert set(uids) == {str(alice['uid'])}, 'A user backend retained root'
                    invoke(alice, mode, 'node', 'list')
                    invoke(alice, mode, 'node', 'select', 'Route', 'DIRECT')
                    invoke(alice, mode, 'mode', 'global')
                    invoke(alice, mode, 'mode', 'rule')
                    invoke(alice, mode, 'connection', 'list')
                    invoke(alice, mode, 'connection', 'show', 'missing', success=False)
                    invoke(alice, mode, 'connection', 'close', '--all', '--yes')
                    invoke(alice, mode, 'logs', json_lines=True)
                    invoke(alice, mode, 'logs', 'export', '--output', alice['home']/f'{mode}-logs.txt',
                           json_output=False)
                    invoke(alice, mode, 'logs', 'clear', '--yes')
                    invoke(alice, mode, 'restart')
                    assert invoke(alice, mode, 'proxy')['proxy'] == 'tun'
                    invoke(alice, mode, 'proxy', 'off')
                for path in data(alice).rglob('*'):
                    assert path.stat().st_uid == alice['uid'], f'User data became root-owned: {path}'
            assert snapshot(bob) == bob_before, 'Other users were modified'
            bob_entries = invoke(bob, 'sudo', 'profile', 'list')
            assert any(p['name'] == 'Same name' and p['active'] for p in bob_entries)
            assert snapshot(bob) == bob_before, 'Sudo selected or modified the wrong user data'
            # stdin and option-like names survive privilege delegation unchanged.
            invoke(alice, 'sudo', 'profile', 'import', '-', '--name', '--user=literal',
                   text='proxies: []\nrules: ["MATCH,DIRECT"]\n')
            invoke(alice, 'sudo', 'profile', 'remove', '--yes', '--', '--user=literal')
            for user in users:
                invoke(user, 'user', 'stop')
                assert not (data(user)/'control.json').exists()
            # Foreground mode also retains the selected account and cleans up
            # after SIGTERM; no extra graphical session is involved.
            process = subprocess.Popen([cli, '--user', alice['name'], 'run', '--no-core', '--quiet'],
                                       env=dict(base_env, COR_MATRIX_ADMIN_TOKEN='synthetic-root-only',
                                                CLASH_OF_RUST_DATA_DIR=str(root_home/'forbidden')),
                                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                                       text=True, start_new_session=True)
            try:
                deadline = time.monotonic()+15
                while True:
                    assert process.poll() is None, process.stderr.read()
                    assert time.monotonic() < deadline, 'Selected foreground owner did not start'
                    control = data(alice)/'control.json'
                    if control.exists():
                        pid = json.loads(control.read_text())['pid']
                        status = Path(f'/proc/{pid}/status').read_text()
                        caught = next(line for line in status.splitlines() if line.startswith('SigCgt:'))
                        if int(caught.split()[1], 16) & (1 << (signal.SIGTERM - 1)):
                            environment = Path(f'/proc/{pid}/environ').read_bytes().split(b'\0')
                            assert not any(v.startswith(b'COR_MATRIX_ADMIN_TOKEN=') for v in environment)
                            assert b'HOME='+os.fsencode(alice['home']) in environment
                            break
                    time.sleep(0.05)
            finally:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGTERM)
                process.communicate(timeout=15)
            assert not (data(alice)/'control.json').exists()
            assert not (root_home/'forbidden').exists()
            print(json.dumps({'account_matrix': True, 'help_commands': len(catalogue),
                              'contexts': ['user', 'sudo', 'selected-user', 'direct-root'],
                              'read_only_inventory': True, 'cross_user_denied': True,
                              'owner_and_backend_reuse': True,
                              'admin_environment_filtered': True,
                              'not_exercised': ['public downloads/install', 'interactive polkit agent',
                                                'real login/autostart', 'public DNS/IP/Claude probes']}),
                  flush=True)
        finally:
            for user in users:
                invoke(user, 'user', 'stop')


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
            account_cli_matrix()
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
             '--inside-namespace', '--test-binary', binary, '--uid', args.uid, '--test-case', args.test_case], timeout=520)
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
