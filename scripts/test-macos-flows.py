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


def flow(scenario, fixture, package, version, report, prefix):
    before = interfaces()
    system_owned = scenario in ('update-system', 'update-cancel', 'update-root-rollback')
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
                    def completed():
                        if status.exists():
                            record = json.loads(status.read_text())
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
                    elif scenario in ('update-user', 'update-system', 'update-rollback', 'update-root-rollback'):
                        assert record['exiting'], 'Ready update did not request normal GUI shutdown'
                        directory = Path(record['details']['directory'])
                        process.wait(timeout=45)
                        assert process.returncode == 0, process.returncode
                        new_pid = wait_for(lambda: relaunched_pid(directory), scenario+' normal-user relaunch')
                        wait_for(lambda: core_ready(data), scenario+' restarted core')
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
                    if process is not None:
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
    names = ('tun-cancel', 'tun-timeout', 'tun', 'update-user', 'update-system', 'update-cancel',
             'update-version', 'update-signature', 'update-arch', 'update-rollback', 'update-root-rollback', 'update-readonly')
    report = {'schema': 1, 'status': 'running', 'confirmation': 'CI substitute; real dialogs remain manual',
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
