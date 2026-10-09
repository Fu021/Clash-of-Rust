"""Test installed TUN authorization in a disposable network namespace (root).

The Rust integration test runs as the specified ordinary user. No host routes,
desktop proxy settings, or existing capability attributes are left changed.
"""
import argparse
import contextlib
import errno
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
    for directory in ('/run/dbus', '/run/systemd/resolve', '/run/systemd/netif'):
        Path(directory).mkdir(parents=True, exist_ok=True)
    daemon = next((p for p in (Path('/usr/lib/systemd/systemd-resolved'),
                              Path('/lib/systemd/systemd-resolved')) if p.is_file()), None)
    assert daemon and Path('/usr/bin/resolvectl').is_file(), 'systemd-resolved is required'
    with tempfile.TemporaryDirectory(prefix='clash-tun-dns-test-') as folder:
        config = Path(folder)/'dbus.conf'
        # No service activation and no PolicyKit authority or authentication
        # agent: DNS writes can succeed only via inherited CAP_NET_ADMIN.
        config.write_text('<busconfig><type>system</type>'
                          '<listen>unix:path=/run/dbus/system_bus_socket</listen>'
                          '<policy context="default"><allow user="*"/><allow own="*"/>'
                          '<allow send_destination="*"/><allow receive_sender="*"/></policy>'
                          '</busconfig>')
        with contextlib.ExitStack() as stack:
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
            log = stack.enter_context((Path(folder)/'daemons.log').open('w+'))
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
            try:
                yield
            finally:
                log.flush()
                log.seek(0)
                print(log.read())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--test-binary', type=Path, required=True)
    parser.add_argument('--uid', type=int, required=True)
    parser.add_argument('--inside-namespace', action='store_true')
    args = parser.parse_args()
    assert os.geteuid() == 0 and args.uid != 0
    binary = args.test_binary.resolve(strict=True)
    user = pwd.getpwuid(args.uid)

    def ordinary_user():
        os.setgroups([])
        os.setgid(user.pw_gid)
        os.setuid(user.pw_uid)

    if args.inside_namespace:
        # Synthetic uplink gives mihomo an auto-detectable default interface;
        # this namespace has no Internet access or connection to host routing.
        run(['ip', 'link', 'set', 'lo', 'up'])
        run(['ip', 'link', 'add', 'uplink', 'type', 'dummy'])
        run(['ip', 'addr', 'add', '192.0.2.1/24', 'dev', 'uplink'])
        run(['ip', 'link', 'set', 'uplink', 'up'])
        run(['ip', 'route', 'add', 'default', 'via', '192.0.2.254', 'dev', 'uplink'])
        with isolated_resolved():
            run([binary, '--ignored', '--test-threads=1', '--nocapture'],
                env=dict(os.environ, CLASH_TUN_TEST_NAMESPACE='1', GSETTINGS_BACKEND='memory'),
                preexec_fn=ordinary_user, timeout=180)
        return

    executable = '/opt/clash-of-rust/clash-of-rust'
    core = '/opt/clash-of-rust/resources/mihomo'
    launcher = '/opt/clash-of-rust/clash-tun-launcher'
    def attribute(path):
        try:
            return os.getxattr(path, 'security.capability')
        except OSError as error:
            if error.errno != errno.ENODATA:
                raise
            return None
    original = {path: attribute(path) for path in (core, launcher)}
    try:
        invalid = subprocess.run([executable, '--authorize-tun', 'unexpected'], capture_output=True)
        assert invalid.returncode == 1 and '不接受其他参数' in invalid.stderr.decode()
        denied = subprocess.run([executable, '--authorize-tun'], capture_output=True, preexec_fn=ordinary_user)
        assert denied.returncode == 1 and '需要管理员权限' in denied.stderr.decode()
        expected = bytes.fromhex('0100000200300000000000000000000000000000')
        # Seed the previous version's grant and verify one-time migration.
        os.setxattr(core, 'security.capability', expected)
        run([executable, '--authorize-tun'])
        run([executable, '--authorize-tun'])  # Idempotent authorization.
        assert attribute(core) is None
        assert attribute(launcher) == expected
        assert attribute(executable) is None, 'The GUI must never receive network capabilities'
        run(['unshare', '--net', '--mount', sys.executable, Path(__file__).resolve(),
             '--inside-namespace', '--test-binary', binary, '--uid', args.uid], timeout=200)
        print(json.dumps({'native_authorization': True, 'ordinary_user': args.uid,
                          'tun_switch_restart_cleanup': True, 'network_namespace': True,
                          'resolved_without_polkit': True, 'legacy_grant_migration': True}))
    finally:
        for path, value in original.items():
            if value is None:
                if attribute(path) is not None:
                    os.removexattr(path, 'security.capability')
            else:
                os.setxattr(path, 'security.capability', value)


if __name__ == '__main__':
    main()
