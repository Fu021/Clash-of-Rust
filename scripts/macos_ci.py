"""Native macOS CI/install checks with durable logs, crash reports and step summaries."""
import argparse
import contextlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request

from build_support import ROOT, host_arch, run, validate_binary_arch, validate_resources, write_json, package_version

REPORT = ROOT/'target/macos-report'


def suite(name, operation):
    REPORT.mkdir(parents=True, exist_ok=True)
    path = REPORT/'report.json'
    report = json.loads(path.read_text()) if path.exists() else {'schema': 1, 'status': 'running', 'suites': []}
    entry = {'name': name, 'status': 'running'}
    report['suites'] = [e for e in report['suites'] if e.get('name') != name]
    report['suites'].append(entry)
    write_json(path, report)
    started = time.monotonic()
    print('::group::'+name, flush=True)
    try:
        result = operation()
        entry['status'] = 'passed'
        if result is not None: entry['details'] = result
        return result
    except BaseException as error:
        entry.update(status='failed', error=str(error))
        report['status'] = 'failed'
        raise
    finally:
        entry['duration_seconds'] = round(time.monotonic()-started, 3)
        latest = json.loads(path.read_text())
        latest['suites'] = [entry if e.get('name') == name and e.get('status') == 'running' else e for e in latest['suites']]
        if entry['status'] == 'failed': latest['status'] = 'failed'
        write_json(path, latest)
        print('::endgroup::', flush=True)


def logged(name, command, env=None, timeout=300):
    REPORT.mkdir(parents=True, exist_ok=True)
    import threading
    with (REPORT/(name+'.log')).open('w') as log:
        process = subprocess.Popen([str(v) for v in command], cwd=ROOT, env=env,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   text=True, encoding='utf-8', errors='replace')
        def copy_output():
            for line in process.stdout:
                log.write(line)
                log.flush()
                sys.stdout.write(line)
                sys.stdout.flush()
        reader = threading.Thread(target=copy_output, daemon=True)
        reader.start()
        try:
            result = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
            raise
        finally:
            reader.join(timeout=5)
    if result:
        raise RuntimeError(f'{name} exited {result}; full output: {name}.log')


def native_tests(tests, bundle):
    def launcher_dependencies():
        output = subprocess.check_output(['/usr/bin/otool', '-L', bundle/'clash-tun-launcher'], text=True)
        (REPORT/'launcher-libraries.log').write_text(output)
        if any(name in output for name in ('AppKit.framework', 'Cocoa.framework', 'QuartzCore.framework')):
            raise RuntimeError('TUN helper unexpectedly links GUI frameworks')
    suite('TUN helper excludes GUI frameworks', launcher_dependencies)
    suite('Real mihomo integration', lambda: logged('core-integration', [tests['core_integration'], '--ignored', '--test-threads=1'], timeout=300))
    suite('Native network service discovery', lambda: logged('native-network', [tests['library'], 'native_network_services_can_be_read', '--ignored', '--test-threads=1']))
    suite('Per-user autostart round trip', lambda: logged('native-autostart', [tests['library'], 'native_autostart_round_trip', '--ignored', '--test-threads=1']))
    suite('Native authorization quoting and cancellation', lambda: logged('native-authorization', [tests['library'], 'native_authorization_script', '--ignored', '--test-threads=1']))
    suite('Proxy restore and later-owner protection', lambda: logged('native-proxy', ['sudo', '-n', 'env', 'GITHUB_ACTIONS=true', tests['library'], 'native_proxy_restore_round_trip', '--ignored', '--test-threads=1']))
    suite('Authorized core lease lifecycle', lambda: logged('native-tun', [tests['library'], 'native_tun_lease', '--ignored', '--test-threads=1'], env=dict(os.environ, MIHOMO_TEST_RESOURCES=str(bundle/'resources')), timeout=180))
    capture = suite('All pages in both themes', rendered_ui)
    tests['macos_ui_fixture'] = capture['executable']
    run(['cargo', 'build', '--release', '--locked', '--features', 'ui-preview', '--bin', 'clash-tun-launcher'])
    fixture_helper = REPORT/'macos-helper-fixture'
    shutil.copy2(ROOT/'target/release/clash-tun-launcher', fixture_helper)
    tests['macos_helper_fixture'] = str(fixture_helper)
    suite('Signed GUI authorization and update flows', lambda: logged('native-flows',
        [sys.executable, ROOT/'scripts/test-macos-flows.py', '--bundle', bundle,
         '--ui', capture['executable'], '--helper', fixture_helper, '--version', package_version(),
         '--arch', host_arch(), '--report', REPORT], timeout=1200))


def rendered_ui():
    import struct, zlib
    # Production binaries were frozen before this optional feature compilation.
    command = ['cargo', 'test', '--release', '--locked', '--features', 'ui-preview',
               '--test', 'ui_preview', '--no-run', '--message-format=json']
    messages = subprocess.check_output(command, cwd=ROOT, text=True, timeout=600)
    executables = [entry['executable'] for line in messages.splitlines()
                   if (entry := json.loads(line)).get('reason') == 'compiler-artifact'
                   and entry.get('executable') and entry['target']['name'] == 'ui_preview']
    if len(executables) != 1: raise RuntimeError('UI capture executable is missing')
    output = REPORT/'ui'
    output.mkdir(exist_ok=True)
    def chunk(kind, data): return struct.pack('>I', len(data))+kind+data+struct.pack('>I', zlib.crc32(kind+data)&0xffffffff)
    for scene in ('home', 'proxies', 'profiles', 'connections', 'rules', 'logs', 'diagnostics', 'ip', 'settings'):
        for suffix in ('', '-light'):
            name = scene+suffix
            destination = output/name
            env = dict(os.environ, CLASH_UI_PREVIEW_SCENE=name, CLASH_UI_PREVIEW_OUTPUT=str(destination))
            logged('ui-'+name, [executables[0]], env=env, timeout=60)
            width, height = json.loads(destination.with_suffix('.json').read_text())
            pixels = destination.with_suffix('.rgba').read_bytes()
            if len(pixels) != width*height*4 or width < 800 or height < 450 or len(set(pixels)) < 16:
                raise RuntimeError('UI rendered an empty/invalid screenshot: '+name)
            stride = width*4
            rows = b''.join(b'\x00'+pixels[y*stride:(y+1)*stride] for y in range(height))
            image = b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0))+chunk(b'IDAT', zlib.compress(rows))+chunk(b'IEND', b'')
            destination.with_suffix('.png').write_bytes(image)
            destination.with_suffix('.rgba').unlink()
    return {'screenshots': 18, 'source': 'iced rendered pixels', 'data': 'isolated fixtures, not live network results', 'executable': executables[0]}


@contextlib.contextmanager
def mounted(package):
    with tempfile.TemporaryDirectory(prefix='clash-dmg-') as temporary:
        mount = Path(temporary)/'volume'
        run(['/usr/bin/hdiutil', 'attach', package, '-readonly', '-nobrowse', '-noautoopen', '-mountpoint', mount], timeout=60)
        try: yield mount
        finally: run(['/usr/bin/hdiutil', 'detach', mount], timeout=60)


def gui_smoke(app, label):
    # Private data directory avoids existing profiles and proxy journals.
    with tempfile.TemporaryDirectory(prefix='clash-gui-') as temporary:
        env = dict(os.environ, CLASH_OF_RUST_DATA_DIR=temporary)
        binary = app/'Contents/MacOS/clash-of-rust'
        with (REPORT/(label+'-gui.log')).open('w') as log:
            process = subprocess.Popen([binary], env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            version = None
            try:
                deadline = time.monotonic()+45
                proxy = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                version = None
                while time.monotonic() < deadline:
                    if process.poll() is not None: raise RuntimeError(f'GUI exited early: {process.returncode}')
                    try:
                        settings = next(Path(temporary).rglob('settings.json'))
                        value = json.loads(settings.read_text())
                        request = urllib.request.Request(f"http://127.0.0.1:{value['controller_port']}/version", headers={'Authorization': 'Bearer '+value['secret']})
                        with proxy.open(request, timeout=1) as response: version = json.load(response)
                        break
                    except (OSError, KeyError, StopIteration, json.JSONDecodeError): time.sleep(.2)
                if not version: raise RuntimeError('GUI did not start a healthy core within 45 seconds')
                # Core readiness alone cannot prove that a GUI window was created.
                swift = REPORT/'windows.swift'
                swift.write_text('''import Foundation
import CoreGraphics
let pid = Int(CommandLine.arguments[1])!
let windows = CGWindowListCopyWindowInfo(.optionAll, kCGNullWindowID) as? [[String: Any]] ?? []
let matching = windows.filter {
    guard ($0[kCGWindowOwnerPID as String] as? Int) == pid,
          ($0[kCGWindowLayer as String] as? Int) == 0,
          ($0[kCGWindowIsOnscreen as String] as? Bool) == true,
          let bounds = $0[kCGWindowBounds as String] as? [String: Any],
          let width = bounds["Width"] as? NSNumber,
          let height = bounds["Height"] as? NSNumber else { return false }
    return width.doubleValue >= 800 && height.doubleValue >= 450
}
let data = try JSONSerialization.data(withJSONObject: matching, options: [.prettyPrinted, .sortedKeys])
FileHandle.standardOutput.write(data)
''')
                output = subprocess.check_output(['/usr/bin/swift', swift, str(process.pid)], timeout=90)
                windows = json.loads(output)
                write_json(REPORT/(label+'-windows.json'), windows)
                if not windows: raise RuntimeError('Core is healthy but macOS reports no application window')
                window = windows[0]['kCGWindowNumber']
                capture = subprocess.run(['/usr/sbin/screencapture', '-x', '-l', str(window), REPORT/(label+'.png')], capture_output=True, timeout=15)
                return {'core': version, 'window_count': len(windows), 'screenshot': 'captured' if capture.returncode == 0 else 'unavailable: runner screen recording permission', 'manual_checks': ['permission dialog interaction', 'sleep/wake', 'physical multi-monitor/DPI', 'real login after reboot']}
            finally:
                # GUI shutdown by SIGTERM is also a useful abnormal-exit check:
                # its ordinary core must not remain listening afterwards.
                process.terminate()
                try: process.wait(timeout=10)
                except subprocess.TimeoutExpired: process.kill(); process.wait()
                # macOS user processes do not get Linux parent-death signals.
                # Reap only this isolated GUI's child core, identified by parent.
                if version:
                    import socket, signal
                    deadline = time.monotonic()+5
                    while time.monotonic() < deadline:
                        try:
                            with socket.create_connection(('127.0.0.1', value['controller_port']), timeout=.2): pass
                        except OSError: break
                        time.sleep(.1)
                    else:
                        try: os.killpg(process.pid, signal.SIGTERM)
                        except ProcessLookupError: pass
                        raise RuntimeError('GUI termination left its core listening')


def package_tests(package, manifest, build):
    suite('Installed package verification', lambda: installed_package_tests(package, manifest, build))
    path = REPORT/'report.json'
    report = json.loads(path.read_text())
    report['status'] = 'passed'
    write_json(path, report)


def installed_package_tests(package, manifest, build):
    from macos_package import APP_NAME, BUNDLE_ID
    with mounted(package) as volume:
        app = volume/APP_NAME
        info = plistlib.loads((app/'Contents/Info.plist').read_bytes())
        if (info['CFBundleIdentifier'], info['ClashOfRustVersion'], info['LSMinimumSystemVersion']) != (BUNDLE_ID, manifest['version'], '14.0'):
            raise ValueError('Installed app metadata differs from CI')
        for name in ('clash-of-rust', 'clash-tun-launcher'):
            validate_binary_arch(app/'Contents/MacOS'/name, 'macos', manifest['arch'])
        validate_binary_arch(app/'Contents/Resources/resources/mihomo', 'macos', manifest['arch'])
        validate_resources(app/'Contents/Resources/resources', 'macos')
        suite('Sealed app signature', lambda: logged('codesign', ['/usr/bin/codesign', '--verify', '--deep', '--strict', app]))
        with tempfile.TemporaryDirectory(prefix='clash-install-') as temporary:
            installed = Path(temporary)/APP_NAME
            run(['/usr/bin/ditto', app, installed])
            suite('Installed GUI, window and bundled core', lambda: gui_smoke(installed, 'installed'))
            # Use the same already-built integration binary with installed resources.
            env = dict(os.environ, MIHOMO_TEST_RESOURCES=str(installed/'Contents/Resources/resources'))
            suite('Installed core rules/subscriptions', lambda: logged('installed-core', [build/manifest['tests']['core_integration'], '--ignored', '--test-threads=1'], env=env))
    # The updater must attach this same DMG itself. Finish inspection and detach
    # the outer volume before exercising actual update transactions.
    suite('Installed DMG authorization and update flows', lambda: logged('installed-flows',
        [sys.executable, ROOT/'scripts/test-macos-flows.py', '--bundle', build/'payload',
         '--ui', build/manifest['tests']['macos_ui_fixture'],
         '--helper', build/manifest['tests']['macos_helper_fixture'],
         '--version', manifest['version'], '--arch', manifest['arch'],
         '--report', REPORT, '--label', 'installed', '--package', package], timeout=1200))


def diagnostics(arch):
    REPORT.mkdir(parents=True, exist_ok=True)
    commands = {'system': ['/usr/bin/sw_vers'], 'kernel': ['/usr/bin/uname', '-a'],
                'hardware': ['/usr/sbin/sysctl', 'hw.ncpu', 'hw.memsize', 'hw.model'],
                'proxy': ['/usr/sbin/scutil', '--proxy'], 'dns': ['/usr/sbin/scutil', '--dns'],
                'routes': ['/usr/sbin/netstat', '-rn'], 'interfaces': ['/sbin/ifconfig'],
                'session': ['/bin/launchctl', 'print', f'gui/{os.getuid()}'],
                'processes': ['/bin/ps', '-axo', 'pid,ppid,uid,stat,comm'],
                'unified-log': ['/usr/bin/log', 'show', '--last', '15m', '--style', 'compact', '--predicate', 'process == "clash-of-rust" OR process == "clash-tun-launcher" OR process == "mihomo"']}
    for name, command in commands.items():
        try: logged('diagnostic-'+name, command, timeout=30)
        except Exception as error: (REPORT/(name+'-collection-error.txt')).write_text(str(error))
    for crash_dir in (Path.home()/'Library/Logs/DiagnosticReports', Path('/Library/Logs/DiagnosticReports')):
        for path in crash_dir.glob('*'):
            if path.name.startswith(('clash', 'mihomo')) and path.is_file():
                try: shutil.copy2(path, REPORT/path.name)
                except OSError: pass
    for pid in subprocess.check_output(['/usr/bin/pgrep', '-x', 'clash-of-rust'], text=True).split() if subprocess.run(['/usr/bin/pgrep', '-x', 'clash-of-rust'], capture_output=True).returncode == 0 else []:
        try: logged('thread-sample-'+pid, ['/usr/bin/sample', pid, '2'], timeout=10)
        except Exception: pass
    report_path = REPORT/'report.json'
    report = json.loads(report_path.read_text()) if report_path.exists() else {'schema': 1, 'status': 'not_run', 'suites': []}
    report['arch'] = arch
    report['manual_checks'] = ['Administrator authorization dialogs require manual interaction', 'Reboot/login, sleep/wake and monitor scaling require a physical or interactive remote Mac']
    write_json(report_path, report)
    summary = os.environ.get('GITHUB_STEP_SUMMARY')
    if summary:
        with open(summary, 'a') as stream:
            stream.write(f'### macOS {arch} diagnostics\n\n| Check | Result | Seconds |\n|---|---|---|\n')
            for entry in report['suites']: stream.write(f"| {entry['name']} | {entry['status']} | {entry.get('duration_seconds', '')} |\n")
            for path in sorted(REPORT.glob('*-flows.json')):
                flows = json.loads(path.read_text())
                stream.write(f'\n#### {path.stem}\n\n| Flow | Result | Seconds |\n|---|---|---|\n')
                for case in flows['cases']:
                    stream.write(f"| {case['name']} | {case['status']} | {case.get('duration_seconds', '')} |\n")
            stream.write('\nFull logs, window metadata, available screenshots and crash reports are in the macOS report artifact. Authorization dialogs, reboot/login and physical display tests remain manual.\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('ci', 'diagnostics'))
    parser.add_argument('--arch', choices=('x64', 'arm64'), required=True)
    args = parser.parse_args()
    if sys.platform != 'darwin' or host_arch() != args.arch: raise ValueError('A matching native macOS runner is required')
    if args.operation == 'diagnostics': diagnostics(args.arch); return
    # All compilation, including failures, has a durable full-output transcript.
    REPORT.mkdir(parents=True, exist_ok=True)
    planned = ['Compile, Clippy, unit tests and native integration', 'Real mihomo integration',
               'TUN helper excludes GUI frameworks', 'Native network service discovery', 'Per-user autostart round trip',
               'Native authorization quoting and cancellation', 'Proxy restore and later-owner protection',
               'Authorized core lease lifecycle', 'All pages in both themes', 'Signed GUI authorization and update flows']
    write_json(REPORT/'report.json', {'schema': 1, 'arch': args.arch, 'status': 'running',
               'suites': [{'name': name, 'status': 'not_run'} for name in planned]})
    for name, command in [('proxy', ['/usr/sbin/scutil', '--proxy']),
                          ('routes', ['/usr/sbin/netstat', '-rn']), ('interfaces', ['/sbin/ifconfig'])]:
        logged('baseline-'+name, command)
    suite('Compile, Clippy, unit tests and native integration', lambda: logged('ci', [sys.executable, ROOT/'scripts/run-ci.py', '--system', 'macos', '--arch', args.arch], timeout=3300))
    report = json.loads((REPORT/'report.json').read_text()); report['status'] = 'passed'; write_json(REPORT/'report.json', report)


if __name__ == '__main__': main()
