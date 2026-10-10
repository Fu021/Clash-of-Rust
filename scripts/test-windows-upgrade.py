"""Compare published Windows versions in the same ordinary-user environment.

Run only on disposable Actions VMs. Downloads verified official installers and
extracts their payloads without installing them or replacing the runner's app.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

from build_support import (ROOT, download_asset, file_version, host_arch, read_json,
                           sha256, validate_binary_arch, write_json)

spec = importlib.util.spec_from_file_location('startup', ROOT/'scripts/test-windows-startup.py')
startup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(startup)


def released_payload(version, destination):
    name = f'Clash-of-Rust-{version}-windows-{host_arch()}-setup.exe'
    release = read_json('https://api.github.com/repos/Fu021/Clash-of-Rust/releases/tags/v' + version)
    asset = next(item for item in release['assets'] if item['name'] == name)
    installer = destination/name
    download_asset(asset, installer)
    payload = destination/version
    seven_zip = shutil.which('7z') or str(Path(os.environ['ProgramFiles'])/'7-Zip/7z.exe')
    subprocess.run([seven_zip, 'x', '-y', '-bso0', '-bsp0', '-o' + str(payload), str(installer)],
                   check=True)
    executable = payload/'clash-of-rust.exe'
    validate_binary_arch(executable, 'windows', host_arch())
    assert file_version(executable) == version
    assert (payload/'resources/mihomo.exe').is_file()
    return payload, {'version': version, 'installer_sha256': sha256(installer),
                     'application_sha256': sha256(executable),
                     'core_sha256': sha256(payload/'resources/mihomo.exe')}


def kill_test_apps(api):
    subprocess.run(['taskkill', '/F', '/FI',
                    'USERNAME eq ' + os.environ['COMPUTERNAME'] + '\\' + api.username,
                    '/IM', 'clash-of-rust.exe', '/T'],
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def cli(api, executable, token, arguments):
    process = api.launch(executable, token, dict(os.environ), arguments)
    try:
        result = api.kernel.WaitForSingleObject(process.process, 30_000)
        assert result == 0, f'CLI timed out: {arguments}'
        code = startup.w.DWORD()
        startup.checked(api.kernel.GetExitCodeProcess(process.process, startup.c.byref(code)))
        return {'exit_code': code.value, 'output': api.output(process).strip()}
    finally:
        api.cleanup(process)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT/'dist/windows-upgrade')
    parser.add_argument('--candidate', type=Path, help='Optional already-built repaired payload')
    args = parser.parse_args()
    assert os.name == 'nt' and os.environ.get('GITHUB_ACTIONS') == 'true'
    args.output.mkdir(parents=True, exist_ok=True)
    api = startup.Windows()
    assert api.shell.IsUserAnAdmin()
    token = startup.w.HANDLE()
    report = {'architecture': host_arch(), 'payloads': [], 'cases': []}

    def case(name, operation):
        try:
            result = operation()
            report['cases'].append({'name': name, 'completed': True, 'result': result})
            print(name + ': ' + json.dumps(result, ensure_ascii=False), flush=True)
        except Exception as error:
            report['cases'].append({'name': name, 'completed': False, 'error': str(error)})
            print(name + ': FAILED: ' + str(error), flush=True)
        finally:
            write_json(args.output/'report.json', report)
            kill_test_apps(api)

    try:
        token, sid = api.ordinary_user()
        api.create_test_desktop(sid)
        api.allow_test_namespace(sid)
        with tempfile.TemporaryDirectory(prefix='clash upgrade 中文 ') as temporary:
            root = Path(temporary)
            api.user_temp = str(root)
            api.grant(root, sid, '(OI)(CI)M')
            startup.check_launcher(api, token, sid)
            payloads = []
            for version in ('0.4.12', '0.4.13', '0.5.0'):
                payload, metadata = released_payload(version, root)
                payloads.append((version, payload))
                report['payloads'].append(metadata)
                write_json(args.output/'report.json', report)
            if args.candidate:
                payloads.append(('repaired', args.candidate.resolve(strict=True)))
            # Keep executable path constant across upgrades, as the task's
            # ownership check correctly refuses a different installation path.
            installed = root/'same installation'
            installed.mkdir()
            executable = installed/'clash-of-rust.exe'
            shared = root/'upgraded data'
            shared.mkdir()
            for version, payload in payloads:
                shutil.copy2(payload/'clash-of-rust.exe', executable)
                shutil.copytree(payload/'resources', installed/'resources', dirs_exist_ok=True)
                fresh = root/('fresh ' + version)
                fresh.mkdir()
                case(version + ' ordinary fresh GUI',
                     lambda: startup.check_startup(api, executable, token, fresh))
                case(version + ' ordinary upgraded GUI',
                     lambda: startup.check_startup(api, executable, token, shared))
                case(version + ' ordinary background restart',
                     lambda: startup.check_startup(api, executable, token, shared, background=True))
                case(version + ' ordinary autostart enable',
                     lambda: cli(api, executable, token, ['--autostart-enable']))
                case(version + ' ordinary autostart status',
                     lambda: cli(api, executable, token, ['--autostart-status']))
                # Re-register the same task/XML through the old admin installer
                # path: empty credentials and security descriptor, LUA principal.
                # The temporary user's marker and original principal are retained.
                task = 'ClashOfRust-' + sid
                script = root/'rebuild-task.ps1'
                script.write_text(
                    "$ErrorActionPreference = 'Stop'\n"
                    "$scheduler = New-Object -ComObject Schedule.Service\n"
                    "$scheduler.Connect()\n$folder = $scheduler.GetFolder('\\')\n"
                    f"$name = '{task}'\n$xml = $folder.GetTask($name).Xml\n"
                    "$folder.DeleteTask($name, 0)\n"
                    "$task = $folder.RegisterTask($name, $xml, 6, $null, $null, 3, $null)\n"
                    "$task.GetSecurityDescriptor(7)\n", encoding='utf-8')
                rebuilt = {}

                def rebuild():
                    rebuilt['output'] = subprocess.check_output(
                        ['powershell', '-NoProfile', '-NonInteractive', '-File', str(script)],
                        text=True, stderr=subprocess.STDOUT).strip()
                    return rebuilt

                case(version + ' admin installer task recreation', rebuild)
                case(version + ' ordinary status after admin recreation',
                     lambda: cli(api, executable, token, ['--autostart-status']))
                case(version + ' ordinary disable after admin recreation',
                     lambda: cli(api, executable, token, ['--autostart-remove']))
                # Cleanup is independent of whether ordinary deletion failed.
                cleanup = root/'delete-task.ps1'
                cleanup.write_text(
                    "$scheduler = New-Object -ComObject Schedule.Service\n"
                    "$scheduler.Connect()\n$folder = $scheduler.GetFolder('\\')\n"
                    f"try {{$folder.DeleteTask('{task}', 0)}} catch {{exit 0}}\n", encoding='utf-8')
                subprocess.run(['powershell', '-NoProfile', '-NonInteractive', '-File', str(cleanup)],
                               check=True, stdout=subprocess.DEVNULL)
                cli(api, executable, token, ['--autostart-remove'])
                kill_test_apps(api)
            # A 0.4.13 interrupted commit must recover before loading settings.
            import uuid
            original = (shared/'settings.json').read_bytes()
            backup = shared/('.profile-transaction-' + uuid.uuid4().hex)
            backup.mkdir()
            (backup/'0.old').write_bytes(original)
            (shared/'settings.json').write_text('{partial', encoding='utf-8')
            (shared/'profile-transaction.json').write_text(json.dumps({
                'directory': backup.name,
                'entries': [{'target': 'settings.json', 'existed': True, 'remove': False}],
            }), encoding='utf-8')
            case(payloads[-1][0] + ' interrupted transaction recovery',
                 lambda: startup.check_startup(api, executable, token, shared))
            report['recovered_original_settings'] = (shared/'settings.json').read_bytes() == original
            write_json(args.output/'report.json', report)
    finally:
        try:
            kill_test_apps(api) if api.username else None
            api.close_test_desktop()
        finally:
            try:
                api.restore_test_namespace()
            finally:
                if token:
                    api.kernel.CloseHandle(token)
                api.delete_test_user()


if __name__ == '__main__':
    main()
