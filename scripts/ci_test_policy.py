"""Keep routine CI focused on proxy/TUN; select relevant optional regressions."""
import json
import os
from pathlib import Path
import subprocess

from build_support import ROOT


def enabled(name):
    value = os.environ.get(name, 'false').lower()
    if value not in ('true', 'false', '1', '0'):
        raise ValueError('Invalid test selection: '+name)
    return value in ('true', '1')


def extended_tests():
    return enabled('COR_CI_EXTENDED_TESTS')


def android_cases():
    if extended_tests():
        return ('localProxyAndRealTunCarryTrafficAndDisconnectCleanly',
                'applicationRoutingPersistsAndKeepsOtherAppsOnline',
                'quickTileRestoresColdCoreWithoutOpeningTheActivity')
    cases = ('basicSystemProxyAndTun',)
    if enabled('COR_CI_ANDROID_ROUTING_TESTS'):
        cases += ('applicationRoutingPersistsAndKeepsOtherAppsOnline',)
    return cases


def linux_user_tests():
    return extended_tests() or enabled('COR_CI_LINUX_USER_TESTS')


def select(paths, extended=False):
    android = any(
        path.startswith(('apps/android/app/src/main/java/org/clashofrust/android/ApplicationRouting',
                         'apps/android/app/src/main/java/org/clashofrust/android/TunnelService.java',
                         'apps/android/app/src/androidTest/java/org/clashofrust/android/ProxyTunTest.java'))
        or path in ('scripts/android.py', 'scripts/ci_test_policy.py')
        for path in paths
    )
    linux = any(
        path.startswith(('apps/cli/src/users.rs', 'apps/cli/src/host_network.rs', 'apps/cli/src/backend.rs',
                         'apps/cli/src/service.rs', 'crates/client-runtime/src/platform/linux/tun/',
                         'crates/client-runtime/src/platform/host_network'))
        or path == 'scripts/test-linux-tun.py'
        for path in paths
    )
    return {'extended_tests': extended, 'android_routing_tests': extended or android,
            'linux_user_tests': extended or linux}


def changed_paths():
    event_path = os.environ.get('GITHUB_EVENT_PATH')
    event = json.loads(Path(event_path).read_text()) if event_path else {}
    base = event.get('before')
    if not base or set(base) == {'0'}:
        base = (event.get('pull_request') or {}).get('base', {}).get('sha') or 'HEAD^'
    result = subprocess.run(['git', 'diff', '--name-only', '-z', base, 'HEAD', '--'],
                            cwd=ROOT, capture_output=True, check=True)
    return result.stdout.decode().rstrip('\0').split('\0')


def main():
    try:
        plan = select(changed_paths(), extended_tests())
    except (OSError, ValueError, subprocess.CalledProcessError):
        plan = select([], True)
        plan['selection_fallback'] = 'changed_source_unavailable'
    print(json.dumps(plan))
    if output := os.environ.get('GITHUB_OUTPUT'):
        with open(output, 'a') as stream:
            for key in ('extended_tests', 'android_routing_tests', 'linux_user_tests'):
                stream.write(f'{key}={str(plan[key]).lower()}\n')
    if summary := os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(summary, 'a') as stream:
            stream.write('\nCI test selection: '+json.dumps(plan)+'\n')


if __name__ == '__main__':
    main()
