"""Exercise signed Mac GUI, authorization, lease restart and DMG update transactions.

The UI fixture uses the real App state machine. A test-only executor substitutes
sudo for human confirmation, retaining osascript and the real helper entrypoint.
Fault injection exists only in feature-gated fixture executables.
"""
import argparse
import contextlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import signal
import socket
import subprocess
import tempfile
import time
import urllib.request

from build_support import ROOT, run, sha256, write_json
from macos_package import APP_NAME, build_app, disk_image


def free_port():
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        return listener.getsockname()[1]


def interfaces():
    return set(subprocess.check_output(['/sbin/ifconfig', '-l'], text=True).split())


def core_ready(data):
    try:
        settings = json.loads((data/'settings.json').read_text())
        request = urllib.request.Request(f"http://127.0.0.1:{settings['controller_port']}/version",
                                         headers={'Authorization': 'Bearer '+settings['secret']})
        with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request, timeout=1) as response:
            return bool(json.load(response).get('version'))
    except (OSError, KeyError, json.JSONDecodeError):
        return False


def wait_for(predicate, label, seconds=90):
    deadline = time.monotonic()+seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(.1)
    raise RuntimeError('Timed out: '+label)


def window_info(pid, report, label):
    swift = report/'flow-windows.swift'
    swift.write_text('''import Foundation
import CoreGraphics
let pid = Int(CommandLine.arguments[1])!
let list = CGWindowListCopyWindowInfo(.optionAll, kCGNullWindowID) as? [[String: Any]] ?? []
let windows = list.filter { item in
    guard (item[kCGWindowOwnerPID as String] as? Int) == pid,
          (item[kCGWindowLayer as String] as? Int) == 0,
          (item[kCGWindowIsOnscreen as String] as? Bool) == true,
          let bounds = item[kCGWindowBounds as String] as? [String: Any],
          let width = bounds["Width"] as? NSNumber, let height = bounds["Height"] as? NSNumber else { return false }
    return width.doubleValue >= 800 && height.doubleValue >= 450
}
FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject: windows, options: [.prettyPrinted, .sortedKeys]))
''')
    result = json.loads(subprocess.check_output(['/usr/bin/swift', swift, str(pid)], timeout=90))
    write_json(report/(label+'-windows.json'), result)
    if not result:
        raise RuntimeError('Restarted core is healthy but the GUI window is missing')
    return len(result)


def relaunched_pid(directory):
    marker = '--update-result '+str(directory)
    for line in subprocess.check_output(['/bin/ps', '-axo', 'pid=,uid=,command='], text=True).splitlines():
        fields = line.strip().split(None, 2)
        if len(fields) == 3 and marker in fields[2] and 'clash-of-rust' in fields[2]:
            if int(fields[1]) != os.getuid():
                raise RuntimeError('Updated GUI was relaunched as a different user')
            return int(fields[0])
    return None


@contextlib.contextmanager
def mount(package, destination):
    disk_image(['attach', package, '-readonly', '-nobrowse', '-noautoopen', '-mountpoint', destination], timeout=60)
    try:
        yield destination/APP_NAME
    finally:
        disk_image(['detach', destination], timeout=60)


def dmg(app, destination):
    disk_image(['create', '-volname', 'Clash native CI', '-srcfolder', app.parent,
                '-fs', 'HFS+', '-format', 'UDZO', '-ov', destination], timeout=120)
    return destination


@contextlib.contextmanager
def network_service_for_ci(report, name):
    uid = os.getuid()
    label = f'org.clashofrust.network.{uid}'
    directory = Path('/Library/PrivilegedHelperTools')/label
    agent = Path('/Library/LaunchDaemons')/(label+'.plist')
    assert not directory.exists() and not agent.exists(), 'Do not overwrite an existing network service'
    try:
        yield
    finally:
        if agent.exists() or directory.exists():
            state = subprocess.run(['/usr/bin/sudo', '-n', '/bin/launchctl', 'print', 'system/'+label], capture_output=True, text=True, timeout=15)
            (report/(name+'-network-service-launchd.log')).write_text(state.stdout+state.stderr)
            service_log = directory/'network-service.log'
            if service_log.exists():
                output = subprocess.run(['/usr/bin/sudo', '-n', '/bin/cat', service_log], capture_output=True, text=True, timeout=15)
                (report/(name+'-network-service.log')).write_text(output.stdout+output.stderr)
            subprocess.run(['/usr/bin/sudo', '-n', '/bin/launchctl', 'bootout', 'system/'+label], capture_output=True, timeout=20)
            subprocess.run(['/usr/bin/sudo', '-n', '/bin/rm', '-rf', directory, agent], check=True, timeout=20)


def system_lifecycle(fixture, report, prefix):
    with tempfile.TemporaryDirectory(prefix='clash-system-login-') as temporary:
        root = Path(temporary)
        app = root/APP_NAME
        shutil.copytree(fixture, app)
        data = root/'data'
        data.mkdir()
        controller, mixed = free_port(), free_port()
        while mixed == controller: mixed = free_port()
        write_json(data/'settings.json', {'controller_port': controller, 'mixed_port': mixed,
                                         'secret': 'isolated-system-login-ci'})
        records = []
        label = f'org.clashofrust.network.{os.getuid()}'
        for attempt in range(3):
            if attempt == 2:
                subprocess.run(['/usr/bin/sudo', '-n', '/bin/launchctl', 'kickstart', '-k', 'system/'+label], check=True, timeout=20)
            status = report/(prefix+f'-system-lifecycle-{attempt}.json')
            env = dict(os.environ, CLASH_CI_FLOW='system-lifecycle', CLASH_CI_FLOW_REPORT=str(status),
                       CLASH_OF_RUST_DATA_DIR=str(data), CLASH_CI_AUTHORIZATION='sudo' if attempt == 0 else 'cancel')
            with (report/(prefix+f'-system-lifecycle-{attempt}.log')).open('w') as log:
                command = [app/'Contents/MacOS/clash-of-rust'] + ([] if attempt == 0 else ['--background'])
                process = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                try:
                    record = wait_for(lambda: json.loads(status.read_text()) if status.exists() and json.loads(status.read_text())['phase'] == 10 else None,
                                      'system proxy login/quit cycle')
                    assert not record['error'] and record['details']['restored_system_mode'], record
                    assert record['administrator_requests'] == (1 if attempt == 0 else 0), record
                    assert record['system_authorization_sessions'] == 0, record
                    process.wait(timeout=30)
                    assert process.returncode == 0
                    assert not core_ready(data), 'System proxy quit left the controller alive'
                    assert not (data/'system-proxy.json').exists(), 'System proxy quit failed to restore its journal'
                    records.append(record)
                finally:
                    if process.poll() is None:
                        os.killpg(process.pid, signal.SIGTERM)
                        process.wait(timeout=15)
        return {'cycles': records, 'helper_restarted_by_launchd': True, 'later_authorization_forbidden': True}


def autostart_flow(fixture, report, prefix):
    label = 'org.clashofrust.desktop'
    target = f'gui/{os.getuid()}/{label}'
    agent = Path.home()/'Library/LaunchAgents'/f'{label}.plist'
    assert not agent.exists(), 'Do not overwrite an existing login agent'
    with tempfile.TemporaryDirectory(prefix='clash-login-flow-') as temporary:
        root = Path(temporary)
        app = root/APP_NAME
        shutil.copytree(fixture, app)
        data = root/'data'
        data.mkdir()
        controller, mixed = free_port(), free_port()
        while mixed == controller:
            mixed = free_port()
        write_json(data/'settings.json', {'controller_port': controller, 'mixed_port': mixed,
                                         'secret': 'isolated-login-ci'})
        status = report/(prefix+'-autostart.json')
        log = report/(prefix+'-autostart.log')
        env = {'GITHUB_ACTIONS': 'true', 'CLASH_CI_FLOW': 'autostart',
               'CLASH_CI_FLOW_REPORT': str(status), 'CLASH_OF_RUST_DATA_DIR': str(data)}
        value = {'Label': label, 'ProgramArguments': [str(app/'Contents/MacOS/clash-of-rust'), '--background'],
                 'RunAtLoad': True, 'LimitLoadToSessionType': 'Aqua', 'ProcessType': 'Interactive',
                 'EnvironmentVariables': env, 'StandardOutPath': str(log), 'StandardErrorPath': str(log)}
        agent.parent.mkdir(parents=True, exist_ok=True)
        agent.write_bytes(plistlib.dumps(value))
        try:
            subprocess.run(['/bin/launchctl', 'bootstrap', f'gui/{os.getuid()}', agent], check=True, timeout=15)
            def completed():
                if status.exists():
                    record = json.loads(status.read_text())
                    if record['phase'] == 10:
                        return record
                job = subprocess.run(['/bin/launchctl', 'print', target], capture_output=True, text=True, timeout=5)
                if job.returncode or not re.search(r'^\s*pid = \d+', job.stdout, re.M):
                    raise RuntimeError('Changing autostart terminated its running LaunchAgent')
            record = wait_for(completed, 'login agent autostart toggle')
            assert not record['error'] and record['running'] and not record['exiting'], record
            assert not agent.exists(), 'Autostart disable left the plist behind'
            assert record['system_authorization_sessions'] == 0 and record['administrator_requests'] == 0, record
            job = subprocess.check_output(['/bin/launchctl', 'print', target], text=True, timeout=5)
            assert re.search(r'^\s*pid = \d+', job, re.M), 'Autostart toggle unloaded the running GUI service'
            assert core_ready(data), 'Autostart toggle interrupted the running core'
            record['launch_method'] = 'actual per-user LaunchAgent'
            return record
        finally:
            subprocess.run(['/bin/launchctl', 'bootout', target], capture_output=True, timeout=15)
            agent.unlink(missing_ok=True)
            wait_for(lambda: not core_ready(data), 'login agent core cleanup', 15)


def flow(scenario, fixture, package, version, report, prefix):
    if scenario == 'autostart':
        return autostart_flow(fixture, report, prefix)
    if scenario == 'system-lifecycle':
        return system_lifecycle(fixture, report, prefix)
    before = interfaces()
    system_owned = scenario in ('update-system', 'update-tun-system', 'update-cancel', 'update-root-rollback') or (scenario.startswith('update') and scenario.endswith('system'))
    with tempfile.TemporaryDirectory(prefix="clash Mac ' 空格 ") as temporary:
        root = Path(temporary)
        install = root/'Applications'
        install.mkdir()
        app = install/APP_NAME
        shutil.copytree(fixture, app)
        old_hash = sha256(app/'Contents/MacOS/clash-of-rust')
        data = root/'data'
        data.mkdir()
        controller = free_port()
        mixed = free_port()
        while mixed == controller:
            mixed = free_port()
        write_json(data/'settings.json', {'controller_port': controller, 'mixed_port': mixed,
                                         'secret': 'isolated-native-ci', 'dark': True})
        status = report/(prefix+'-'+scenario+'.json')
        env = dict(os.environ, CLASH_OF_RUST_DATA_DIR=str(data), CLASH_CI_FLOW=scenario,
                   CLASH_CI_FLOW_REPORT=str(status), CLASH_CI_AUTHORIZATION='sudo',
                   CLASH_CI_SHORT_IDLE='true')
        if scenario in ('tun-cancel', 'update-cancel'):
            env['CLASH_CI_AUTHORIZATION'] = 'cancel'
        if scenario == 'tun-timeout':
            env['CLASH_CI_AUTHORIZATION'] = 'timeout'
        if scenario in ('update-rollback', 'update-root-rollback'):
            env['CLASH_CI_FAIL_SWAP'] = 'true'
        if scenario.startswith('update'):
            env.update(CLASH_CI_UPDATE_PACKAGE=str(package), CLASH_CI_UPDATE_VERSION=version)
        if system_owned:
            install.chmod(0o755)
            run(['/usr/bin/sudo', '-n', 'chown', '-R', '0', install])
        with contextlib.ExitStack() as contexts:
            if scenario == 'update-readonly':
                image_source = root/'readonly-image'
                image_source.mkdir()
                shutil.copytree(fixture, image_source/APP_NAME)
                image = dmg(image_source/APP_NAME, root/'readonly.dmg')
                app = contexts.enter_context(mount(image, root/'readonly-volume'))
            process = None
            with (report/(prefix+'-'+scenario+'.log')).open('w') as log:
                try:
                    process = subprocess.Popen([app/'Contents/MacOS/clash-of-rust'], env=env,
                                               stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                    reopened_phases = set()
                    def completed():
                        if status.exists():
                            record = json.loads(status.read_text())
                            if scenario == 'window-lifecycle' and record['phase'] in (2, 5) and record['phase'] not in reopened_phases:
                                # Real LaunchServices delivery to an already running
                                # signed app exercises the same path as Dock reopen.
                                reopened_phases.add(record['phase'])
                                subprocess.run(['/usr/bin/open', '-a', app], check=True, timeout=15)
                            if record['phase'] == 10:
                                return record
                        if process.poll() is not None:
                            raise RuntimeError('GUI exited before reporting its handoff/action result')
                    record = wait_for(completed, scenario+' GUI action')
                    if scenario == 'tun':
                        assert not record['error'] and record['running'] and not record['tun'], record
                        assert record['details']['lease_reused_after_long_run'], record
                        process.wait(timeout=30)
                        assert process.returncode == 0, process.returncode
                    elif scenario == 'proxy-switches':
                        assert not record['error'] and record['running'] and not record['tun'], record
                        assert record['system_authorization_sessions'] == 0, record
                        assert record['administrator_requests'] == 1, record
                        assert core_ready(data), 'Repeated mode switches interrupted the core'
                    elif scenario == 'file-picker':
                        assert not record['error'] and record['running'], record
                        assert record['details']['native_panel_cancelled'] and not record['details']['pending'], record
                        assert core_ready(data), 'The asynchronous file chooser interrupted the core'
                    elif scenario == 'native-quit':
                        assert record['exiting'] and not record['error'], record
                        assert record['details']['native_quit'] and record['details']['core_stopped'], record
                        process.wait(timeout=30)
                        assert process.returncode == 0, process.returncode
                        assert not core_ready(data), 'Native Quit left the old controller alive'
                    elif scenario == 'window-lifecycle':
                        assert not record['error'] and record['running'] and not record['exiting'], record
                        assert all(record['details'][key] for key in ('close_dock_reopen', 'reopen_without_tray', 'minimize_dock_reopen')), record
                        assert reopened_phases == {2, 5}, 'Both hidden-window cases must reopen through LaunchServices'
                        assert record['administrator_requests'] == 0 and record['system_authorization_sessions'] == 0, record
                        assert core_ready(data), 'Closing/reopening the window interrupted the core'
                        record['reopened_window_count'] = window_info(process.pid, report, prefix+'-'+scenario)
                    elif scenario in ('update-user', 'update-system', 'update-tun-user', 'update-tun-system', 'update-rollback', 'update-root-rollback') or 'online' in scenario:
                        assert record['exiting'], 'Ready update did not request normal GUI shutdown'
                        directory = Path(record['details']['directory'])
                        process.wait(timeout=45)
                        assert process.returncode == 0, process.returncode
                        new_pid = wait_for(lambda: relaunched_pid(directory), scenario+' normal-user relaunch')
                        wait_for(lambda: core_ready(data), scenario+' restarted core')
                        assert json.loads((data/'settings.json').read_text())['controller_port'] == controller, 'Update changed the controller port'
                        if scenario.startswith('update-tun-'):
                            assert record['details']['tun_to_system_before_update'], record
                            assert json.loads((data/'settings.json').read_text())['proxy_mode'] == 'system', 'Update lost system proxy mode'
                        record['restarted_window_count'] = window_info(new_pid, report, prefix+'-'+scenario)
                        rollback = scenario.endswith('rollback')
                        assert (sha256(app/'Contents/MacOS/clash-of-rust') == old_hash) == rollback, 'App replacement/rollback did not match outcome'
                        assert not list(install.glob('.clash-*.app')), 'Update left staged apps or backups behind'
                    else:
                        assert process.poll() is None and record['running'] and not record['exiting'], record
                        assert core_ready(data), 'Rejected authorization/update stopped the old core'
                        assert sha256(app/'Contents/MacOS/clash-of-rust') == old_hash, 'Rejected update changed the original app'
                        error = record.get('failure') or record.get('notice') or ''
                        expected = {'tun-cancel': '取消', 'tun-timeout': '超时', 'update-cancel': '取消',
                                    'update-version': '版本', 'update-signature': '签名',
                                    'update-arch': '架构', 'update-readonly': '只读'}[scenario]
                        assert expected in error, (scenario, error)
                    record['authorization_confirmation'] = 'noninteractive CI substitute; actual dialogs require manual validation'
                    write_json(status, record)
                    return record
                finally:
                    proxy_cleanup_error = None
                    if process is not None:
                        # New update/mode cases can leave System enabled in the
                        # restarted GUI. Restore before SIGTERM and before its
                        # isolated journal disappears, keeping runner traffic
                        # away from a core we are about to stop.
                        journal = data/'system-proxy.json'
                        if journal.exists():
                            try:
                                subprocess.run([fixture/'Contents/MacOS/clash-of-rust'],
                                               env=dict(os.environ, CLASH_CI_RESTORE_PROXY=str(journal)),
                                               check=True, timeout=15)
                            except Exception as error:
                                proxy_cleanup_error = error
                        if status.exists():
                            recorded = json.loads(status.read_text())
                            update_directory = recorded.get('details', {}).get('directory')
                            if update_directory and Path(update_directory).exists():
                                (Path(update_directory)/'cancel').touch()
                        # The updater's restart remains in this isolated process group.
                        try:
                            os.killpg(process.pid, signal.SIGTERM)
                        except (ProcessLookupError, PermissionError):
                            pass
                        try:
                            process.wait(timeout=10)
                        except subprocess.TimeoutExpired:
                            os.killpg(process.pid, signal.SIGKILL)
                            process.wait()
                        wait_for(lambda: not core_ready(data), scenario+' core cleanup', 15)
                        wait_for(lambda: not {v for v in interfaces()-before if v.startswith('utun')}, scenario+' TUN interface cleanup', 15)
                    if system_owned:
                        run(['/usr/bin/sudo', '-n', 'chown', '-R', str(os.getuid()), install])
                    if proxy_cleanup_error:
                        raise RuntimeError('System proxy cleanup failed') from proxy_cleanup_error


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--ui', type=Path, required=True)
    parser.add_argument('--helper', type=Path, required=True)
    parser.add_argument('--version', required=True)
    parser.add_argument('--arch', choices=('x64', 'arm64'), required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--label', default='ci')
    parser.add_argument('--package', type=Path)
    args = parser.parse_args()
    if os.uname().sysname != 'Darwin' or os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('Native disposable macOS Actions runner required')
    args.report.mkdir(parents=True, exist_ok=True)
    report_path = args.report/(args.label+'-flows.json')
    names = ('file-picker', 'native-quit', 'window-lifecycle', 'tun-cancel', 'tun-timeout', 'tun', 'proxy-switches', 'autostart', 'system-lifecycle', 'update-user', 'update-system', 'update-tun-user', 'update-tun-system', 'update-cancel',
             'update-version', 'update-signature', 'update-arch', 'update-rollback', 'update-root-rollback', 'update-readonly')
    report = {'schema': 1, 'status': 'running', 'confirmation': 'CI substitute; real dialogs remain manual',
              'network_authorization': 'Authorization database unchanged; only initial helper installation substitutes sudo for human confirmation',
              'cases': [{'name': name, 'status': 'not_run'} for name in names]}
    write_json(report_path, report)
    with tempfile.TemporaryDirectory(prefix='clash-native-flow-build-') as temporary:
        root = Path(temporary)
        payload = root/'payload'
        shutil.copytree(args.bundle, payload)
        shutil.copy2(args.ui, payload/'clash-of-rust')
        shutil.copy2(args.helper, payload/'clash-tun-launcher')
        fixture = build_app(payload, root/'fixture'/APP_NAME, args.version, args.arch)
        with contextlib.ExitStack() as contexts:
            if args.package:
                production = root/'production'/APP_NAME
                with mount(args.package, root/'production-volume') as mounted_app:
                    shutil.copytree(mounted_app, production)
                valid = args.package
            else:
                production = build_app(args.bundle, root/'production'/APP_NAME, args.version, args.arch)
                valid = dmg(production, root/'valid.dmg')
            invalid = {}
            for kind in ('signature', 'arch'):
                copied = root/kind/APP_NAME
                shutil.copytree(production, copied)
                if kind == 'signature':
                    with (copied/'Contents/Resources/LICENSE').open('a') as license_file:
                        license_file.write('\nInjected sealed-resource modification.\n')
                else:
                    path = copied/'Contents/Resources/resources/core.json'
                    record = json.loads(path.read_text())
                    record['arch'] = 'x64' if args.arch == 'arm64' else 'arm64'
                    write_json(path, record)
                    run(['/usr/bin/codesign', '--force', '--sign', '-', copied])
                invalid[kind] = dmg(copied, root/(kind+'.dmg'))
            for entry in report['cases']:
                entry['status'] = 'running'
                write_json(report_path, report)
                started = time.monotonic()
                name = entry['name']
                try:
                    package = invalid.get(name.removeprefix('update-'), valid)
                    version = '0.0.0-ci-mismatch' if name == 'update-version' else args.version
                    with network_service_for_ci(args.report, args.label+'-'+name):
                        entry['details'] = flow(name, fixture, package, version, args.report, args.label)
                    entry['status'] = 'passed'
                except Exception as error:
                    entry.update(status='failed', error=str(error))
                    report['status'] = 'failed'
                finally:
                    entry['duration_seconds'] = round(time.monotonic()-started, 3)
                    write_json(report_path, report)
            failed = [entry for entry in report['cases'] if entry['status'] == 'failed']
            report['status'] = 'failed' if failed else 'passed'
            write_json(report_path, report)
    if failed:
        raise RuntimeError('Native flows failed: '+', '.join(entry['name'] for entry in failed)+'; '+str(report_path))
    print('PASS: signed GUI authorization, lease restart, DMG handoff, relaunch and rollback; '+str(report_path), flush=True)


if __name__ == '__main__':
    main()
