"""Test installed TUN authorization in a disposable network namespace (root).

The Rust integration test runs as the specified ordinary user. No host routes,
desktop proxy settings, or existing capability attributes are left changed.
"""
import argparse
import errno
import json
import os
from pathlib import Path
import pwd
import subprocess
import sys


def run(command, **kwargs):
    return subprocess.run([str(item) for item in command], check=True, **kwargs)


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
        run([binary, '--ignored', '--test-threads=1', '--nocapture'],
            env=dict(os.environ, CLASH_TUN_TEST_NAMESPACE='1', GSETTINGS_BACKEND='memory'),
            preexec_fn=ordinary_user, timeout=180)
        return

    executable = '/opt/clash-of-rust/clash-of-rust'
    core = '/opt/clash-of-rust/resources/mihomo'
    try:
        original = os.getxattr(core, 'security.capability')
    except OSError as error:
        if error.errno != errno.ENODATA:
            raise
        original = None
    try:
        invalid = subprocess.run([executable, '--authorize-tun', 'unexpected'], capture_output=True)
        assert invalid.returncode == 1 and '不接受其他参数' in invalid.stderr.decode()
        denied = subprocess.run([executable, '--authorize-tun'], capture_output=True, preexec_fn=ordinary_user)
        assert denied.returncode == 1 and '需要管理员权限' in denied.stderr.decode()
        run([executable, '--authorize-tun'])
        # Confirm native helper wrote exactly the intended network capability set.
        expected = bytes.fromhex('0100000200300000000000000000000000000000')
        assert os.getxattr(core, 'security.capability') == expected
        run(['unshare', '--net', sys.executable, Path(__file__).resolve(),
             '--inside-namespace', '--test-binary', binary, '--uid', args.uid], timeout=200)
        print(json.dumps({'native_authorization': True, 'ordinary_user': args.uid,
                          'tun_switch_restart_cleanup': True, 'network_namespace': True}))
    finally:
        if original is None:
            try:
                os.removexattr(core, 'security.capability')
            except OSError as error:
                if error.errno != errno.ENODATA:
                    raise
        else:
            os.setxattr(core, 'security.capability', original)


if __name__ == '__main__':
    main()
