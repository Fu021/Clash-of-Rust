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
                'quickTileRestoresColdCoreWithoutOpeningTheActivity',
                'measurementDiagnosticsDistinguishFailuresAndExport')
    cases = ('basicSystemProxyAndTun',)
    if enabled('COR_CI_ANDROID_ROUTING_TESTS'):
        cases += ('applicationRoutingPersistsAndKeepsOtherAppsOnline',)
    if enabled('COR_CI_ANDROID_DIAGNOSTICS_TESTS'):
        cases += ('measurementDiagnosticsDistinguishFailuresAndExport',)
    return cases


def linux_user_tests():
    return extended_tests() or enabled('COR_CI_LINUX_USER_TESTS')


def desktop_ui_tests():
    return extended_tests() or enabled('COR_CI_DESKTOP_UI_TESTS')


def mobile_ui_tests():
    return extended_tests() or enabled('COR_CI_MOBILE_UI_TESTS')


def update_tests():
    return extended_tests() or enabled('COR_CI_UPDATE_TESTS')


def proxy_benchmark():
    return enabled('COR_CI_PROXY_BENCHMARK')


def select(paths, extended=False):
    android = any(
        path.startswith(('apps/android/app/src/main/java/org/clashofrust/android/ApplicationRouting',
                         'apps/android/app/src/main/java/org/clashofrust/android/ApplicationOwners.java',
                         'apps/android/core/',
                         'apps/android/native/src/application_policies',
                         'apps/android/app/src/main/java/org/clashofrust/android/ApplicationCatalog.java',
                         'apps/android/native/src/application_routing',
                         'crates/client-ui/ui/mobile/android/applications.slint',
                         'apps/android/app/src/main/java/org/clashofrust/android/TunnelService.java',
                         'apps/android/app/src/main/java/org/clashofrust/android/IncidentNotifications.java',
                         'apps/android/app/src/androidTest/java/org/clashofrust/android/ProxyTunTest.java'))
        or path.startswith(('crates/client-core/src/config/',
                             'crates/client-core/src/protection/',
                             'crates/mobile-ffi/src/',
                             'crates/client-core/src/application_rules.rs',
                             'crates/client-core/src/application_rules/',
                             'crates/client-ui/ui/mobile/android/application-policies.slint'))
        or path in ('scripts/android.py', 'scripts/ci_test_policy.py')
        for path in paths
    )
    linux = any(
        path.startswith(('apps/cli/src/',
                         'crates/client-runtime/src/platform/linux/tun/',
                         'crates/client-runtime/src/platform/linux/cli_service.rs',
                         'crates/client-runtime/src/platform/linux/startup.rs',
                         'crates/client-runtime/src/platform/host_network'))
        or path in ('scripts/test-linux-tun.py', 'scripts/tun_ipv6_lifecycle.py',
                    'scripts/cli_tui.py', 'scripts/cli_tui_preview.py', 'scripts/test_cli_tui.py',
                    'scripts/ci_test_policy.py', 'apps/desktop/tests/core_integration.rs',
                    'apps/desktop/tests/support/protected_dns.rs',
                    'apps/desktop/tests/support/proxy_benchmark.rs',
                    'scripts/proxy_benchmark.py')
        or path.startswith('crates/client-runtime/src/platform/network/')
        or path in ('crates/client-runtime/src/control/network.rs',
                    'crates/client-runtime/src/engine/recovery.rs')
        for path in paths
    )
    diagnostics = any(path.startswith(('crates/client-core/src/controller/',
                                      'crates/client-core/src/support.rs',
                                      'apps/android/native/',
                                      'apps/android/app/src/main/java/org/clashofrust/android/NetworkOverview.java',
                                      'apps/android/app/src/main/java/org/clashofrust/android/MainActivity.java',
                                      'apps/android/app/src/main/java/org/clashofrust/android/IncidentNotifications.java',
                                      'apps/android/app/src/androidTest/'))
                      or path in ('scripts/android.py', 'scripts/ci_test_policy.py')
                      for path in paths)
    desktop_ui = any(
        (path.startswith('crates/client-ui/') and not path.startswith('crates/client-ui/ui/mobile/'))
        or path.startswith(('apps/desktop-ui/', 'crates/desktop-protocol/', 'vendor/winit/',
                         'apps/desktop/src/application/', 'crates/client-core/src/config/',
                         'crates/client-core/src/protection/', 'crates/client-runtime/src/control/',
                         'crates/client-core/src/error_message.rs',
                         'crates/client-core/src/detection/',
                         'resources/fonts/',
                         'crates/client-core/src/application_rules.rs',
                         'crates/client-core/src/application_rules/'))
        or path in ('Cargo.toml', 'Cargo.lock', 'apps/desktop/Cargo.toml',
                    'apps/desktop-ui/Cargo.toml', 'crates/desktop-shell/Cargo.toml',
                    'scripts/run-ci.py', 'scripts/macos_ci.py', 'scripts/ci_test_policy.py')
        for path in paths
    )
    mobile_ui = any(
        (path.startswith('crates/client-ui/ui/components/')
         and not path.startswith('crates/client-ui/ui/components/desktop-'))
        or path.startswith(('crates/client-ui/ui/mobile/', 'crates/client-ui/ui/theme/',
                         'crates/client-ui/ui/icons/', 'apps/android/native/', 'resources/fonts/',
                         'crates/mobile-ffi/src/', 'crates/client-core/src/config/',
                         'crates/client-core/src/protection/', 'crates/client-core/src/detection/',
                         'crates/client-core/src/error_message.rs',
                         'crates/client-core/src/application_rules.rs',
                         'crates/client-core/src/application_rules/'))
        or path in ('crates/client-ui/ui/models.slint', 'crates/client-ui/ui/preview-reports.slint',
                    'crates/client-ui/ui/preview-network.slint',
                    'Cargo.toml', 'Cargo.lock', 'scripts/android.py', 'scripts/ci_test_policy.py')
        for path in paths
    )
    benchmark = any(
        path.startswith(('apps/desktop/tests/support/proxy_benchmark.rs',
                         'crates/client-core/src/controller/',
                         'crates/client-runtime/src/platform/windows/mod.rs',
                         'crates/client-runtime/src/platform/macos/cf.rs',
                         'crates/client-runtime/src/platform/macos/native_proxy.rs',
                         'crates/client-runtime/src/platform/macos/proxy_preferences.rs',
                         'crates/client-runtime/src/platform/macos/mod.rs',
                         'crates/client-runtime/src/platform/macos/service.rs',
                         'crates/client-runtime/src/platform/macos/tun.rs',
                         'crates/client-runtime/src/platform/linux/mod.rs'))
        or path in ('scripts/proxy_benchmark.py', 'apps/desktop/tests/core_integration.rs',
                    'crates/client-core/src/download.rs',
                    'crates/client-core/src/subscription/routes.rs')
        for path in paths
    )
    update = any(
        path.startswith(('crates/client-core/src/update', 'crates/client-core/src/config/',
                         'crates/client-core/src/download.rs', 'crates/client-core/src/geodata/',
                         'crates/client-runtime/src/update/', 'apps/cli/src/commands/maintenance.rs',
                         'apps/cli/src/tui/',
                         'crates/client-runtime/src/control/', 'apps/cli/src/backend.rs',
                         'apps/cli/src/service', 'apps/cli/src/commands/inspection',
                         'crates/client-runtime/src/platform/linux/cli_service.rs',
                         'installer/', 'crates/client-runtime/src/platform/windows/'))
        or path in ('apps/cli/tests/console.rs', 'scripts/test-cli-update.py', 'scripts/ci_test_policy.py',
                    'scripts/windows_ci.py', 'scripts/windows_update_tests.py',
                    'scripts/build-installer.py')
        for path in paths
    )
    return {'extended_tests': extended, 'android_routing_tests': extended or android,
            'android_diagnostics_tests': extended or diagnostics,
            'linux_user_tests': extended or linux, 'desktop_ui_tests': extended or desktop_ui,
            'mobile_ui_tests': extended or mobile_ui,
            'proxy_benchmark': benchmark, 'update_tests': extended or update}


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
    plan['proxy_benchmark'] |= proxy_benchmark()
    print(json.dumps(plan))
    if output := os.environ.get('GITHUB_OUTPUT'):
        with open(output, 'a') as stream:
            for key in ('extended_tests', 'android_routing_tests', 'android_diagnostics_tests', 'linux_user_tests',
                        'desktop_ui_tests', 'mobile_ui_tests', 'proxy_benchmark', 'update_tests'):
                stream.write(f'{key}={str(plan[key]).lower()}\n')
    if summary := os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(summary, 'a') as stream:
            stream.write('\nCI test selection: '+json.dumps(plan)+'\n')


if __name__ == '__main__':
    main()
