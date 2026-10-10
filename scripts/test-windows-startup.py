"""Run the real Windows GUI as a temporary, ordinary local user in CI.

Uses isolated directories and no system proxy/TUN. The test account is deleted
in finally; its random password never enters a command line or test output.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import json
import os
from pathlib import Path
import tempfile
import time
import urllib.request


class SidAndAttributes(c.Structure):
    _fields_ = [('sid', c.c_void_p), ('attributes', w.DWORD)]


class StartupInfo(c.Structure):
    _fields_ = [('cb', w.DWORD), ('reserved', w.LPWSTR), ('desktop', w.LPWSTR),
                ('title', w.LPWSTR), ('x', w.DWORD), ('y', w.DWORD),
                ('x_size', w.DWORD), ('y_size', w.DWORD), ('x_count', w.DWORD),
                ('y_count', w.DWORD), ('fill', w.DWORD), ('flags', w.DWORD),
                ('show', w.WORD), ('reserved_size', w.WORD),
                ('reserved_bytes', c.c_void_p), ('stdin', w.HANDLE),
                ('stdout', w.HANDLE), ('stderr', w.HANDLE)]


class ProcessInfo(c.Structure):
    _fields_ = [('process', w.HANDLE), ('thread', w.HANDLE),
                ('pid', w.DWORD), ('tid', w.DWORD)]


class SecurityAttributes(c.Structure):
    _fields_ = [('length', w.DWORD), ('descriptor', c.c_void_p), ('inherit', w.BOOL)]


class UserInfo(c.Structure):
    _fields_ = [('name', w.LPWSTR), ('password', w.LPWSTR), ('password_age', w.DWORD),
                ('privilege', w.DWORD), ('home', w.LPWSTR), ('comment', w.LPWSTR),
                ('flags', w.DWORD), ('script', w.LPWSTR)]


def checked(result):
    if not result:
        raise c.WinError(c.get_last_error())
    return result


class Windows:
    def __init__(self):
        self.kernel = c.WinDLL('kernel32', use_last_error=True)
        self.security = c.WinDLL('advapi32', use_last_error=True)
        self.shell = c.WinDLL('shell32', use_last_error=True)
        self.user = c.WinDLL('user32', use_last_error=True)
        self.network = c.WinDLL('netapi32', use_last_error=True)
        self.station = self.desktop_handle = None
        self.desktop_name = None
        self.username = self.password = self.user_temp = None
        signatures = [
            (self.kernel, 'GetCurrentProcess', [], w.HANDLE),
            (self.kernel, 'CloseHandle', [w.HANDLE], w.BOOL),
            (self.kernel, 'LocalFree', [c.c_void_p], c.c_void_p),
            (self.kernel, 'WaitForSingleObject', [w.HANDLE, w.DWORD], w.DWORD),
            (self.kernel, 'GetExitCodeProcess', [w.HANDLE, c.POINTER(w.DWORD)], w.BOOL),
            (self.kernel, 'TerminateProcess', [w.HANDLE, w.UINT], w.BOOL),
            (self.kernel, 'OpenEventW', [w.DWORD, w.BOOL, w.LPCWSTR], w.HANDLE),
            (self.kernel, 'SetEvent', [w.HANDLE], w.BOOL),
            (self.security, 'OpenProcessToken', [w.HANDLE, w.DWORD, c.POINTER(w.HANDLE)], w.BOOL),
            (self.security, 'ConvertStringSidToSidW', [w.LPCWSTR, c.POINTER(c.c_void_p)], w.BOOL),
            (self.security, 'ConvertSidToStringSidW', [c.c_void_p, c.POINTER(w.LPWSTR)], w.BOOL),
            (self.security, 'GetTokenInformation',
             [w.HANDLE, c.c_int, c.c_void_p, w.DWORD, c.POINTER(w.DWORD)], w.BOOL),
            (self.security, 'LogonUserW',
             [w.LPCWSTR, w.LPCWSTR, w.LPCWSTR, w.DWORD, w.DWORD, c.POINTER(w.HANDLE)], w.BOOL),
            (self.security, 'LookupAccountSidW',
             [w.LPCWSTR, c.c_void_p, w.LPWSTR, c.POINTER(w.DWORD),
              w.LPWSTR, c.POINTER(w.DWORD), c.POINTER(w.DWORD)], w.BOOL),
            (self.security, 'ImpersonateLoggedOnUser', [w.HANDLE], w.BOOL),
            (self.security, 'RevertToSelf', [], w.BOOL),
            (self.security, 'CreateProcessAsUserW',
             [w.HANDLE, w.LPCWSTR, w.LPWSTR, c.c_void_p, c.c_void_p, w.BOOL,
              w.DWORD, c.c_void_p, w.LPCWSTR, c.POINTER(StartupInfo), c.POINTER(ProcessInfo)], w.BOOL),
            (self.security, 'CreateProcessWithLogonW',
             [w.LPCWSTR, w.LPCWSTR, w.LPCWSTR, w.DWORD, w.LPCWSTR, w.LPWSTR, w.DWORD, c.c_void_p,
              w.LPCWSTR, c.POINTER(StartupInfo), c.POINTER(ProcessInfo)], w.BOOL),
            (self.network, 'NetUserAdd', [w.LPCWSTR, w.DWORD, c.c_void_p, c.POINTER(w.DWORD)], w.DWORD),
            (self.network, 'NetUserDel', [w.LPCWSTR, w.LPCWSTR], w.DWORD),
            (self.network, 'NetLocalGroupAddMembers',
             [w.LPCWSTR, w.LPCWSTR, w.DWORD, c.c_void_p, w.DWORD], w.DWORD),
            (self.shell, 'IsUserAnAdmin', [], w.BOOL),
            (self.security, 'ConvertStringSecurityDescriptorToSecurityDescriptorW',
             [w.LPCWSTR, w.DWORD, c.POINTER(c.c_void_p), c.c_void_p], w.BOOL),
            (self.user, 'GetProcessWindowStation', [], w.HANDLE),
            (self.user, 'SetProcessWindowStation', [w.HANDLE], w.BOOL),
            (self.user, 'CreateWindowStationW',
             [w.LPCWSTR, w.DWORD, w.DWORD, c.POINTER(SecurityAttributes)], w.HANDLE),
            (self.user, 'CloseWindowStation', [w.HANDLE], w.BOOL),
            (self.user, 'CreateDesktopW',
             [w.LPCWSTR, w.LPCWSTR, c.c_void_p, w.DWORD, w.DWORD,
              c.POINTER(SecurityAttributes)], w.HANDLE),
            (self.user, 'CloseDesktop', [w.HANDLE], w.BOOL),
        ]
        for library, name, args, result in signatures:
            function = getattr(library, name)
            function.argtypes, function.restype = args, result

    def create_test_desktop(self, sid):
        descriptor = c.c_void_p()
        # Give the ordinary user an isolated medium-integrity desktop, while
        # keeping the existing runner desktop and its permissions unchanged.
        sddl = f'D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})S:(ML;;NW;;;ME)'
        checked(self.security.ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl, 1, c.byref(descriptor), None))
        previous = self.user.GetProcessWindowStation()
        name = 'ClashStartupCI' + str(os.getpid())
        try:
            attributes = SecurityAttributes(c.sizeof(SecurityAttributes), descriptor.value, False)
            self.station = checked(self.user.CreateWindowStationW(name, 0, 0xF037F, c.byref(attributes)))
            checked(self.user.SetProcessWindowStation(self.station))
            try:
                self.desktop_handle = checked(self.user.CreateDesktopW(
                    'Startup', None, None, 0, 0xF01FF, c.byref(attributes)))
            finally:
                checked(self.user.SetProcessWindowStation(previous))
            self.desktop_name = name + '\\Startup'
        finally:
            self.kernel.LocalFree(descriptor)

    def close_test_desktop(self):
        if self.desktop_handle:
            checked(self.user.CloseDesktop(self.desktop_handle))
        if self.station:
            checked(self.user.CloseWindowStation(self.station))

    def ordinary_user(self):
        import secrets
        name = 'ClashCI' + secrets.token_hex(5)
        password = 'Aa1!' + secrets.token_urlsafe(24)
        user = UserInfo(name, password, 0, 1, None, 'Temporary Clash startup CI user', 0x201, None)
        parameter = w.DWORD()
        status = self.network.NetUserAdd(None, 1, c.byref(user), c.byref(parameter))
        if status:
            raise c.WinError(status)
        self.username, self.password = name, password
        users_sid = c.c_void_p()
        token = w.HANDLE()
        try:
            # Resolve the built-in Users group by SID, also on localized hosts.
            checked(self.security.ConvertStringSidToSidW('S-1-5-32-545', c.byref(users_sid)))
            group, domain = c.create_unicode_buffer(256), c.create_unicode_buffer(256)
            group_length, domain_length, kind = w.DWORD(256), w.DWORD(256), w.DWORD()
            checked(self.security.LookupAccountSidW(None, users_sid, group, c.byref(group_length),
                                                   domain, c.byref(domain_length), c.byref(kind)))
            member = w.LPWSTR(name)
            status = self.network.NetLocalGroupAddMembers(None, group.value, 3, c.byref(member), 1)
            if status not in (0, 1378):  # already a member is also fine
                raise c.WinError(status)
            checked(self.security.LogonUserW(name, '.', password, 2, 0, c.byref(token)))
            checked(self.security.ImpersonateLoggedOnUser(token))
            try:
                assert not self.shell.IsUserAnAdmin(), 'Startup test must really use non-admin permissions'
            finally:
                checked(self.security.RevertToSelf())
            length = w.DWORD()
            self.security.GetTokenInformation(token, 1, None, 0, c.byref(length))
            buffer = c.create_string_buffer(length.value)
            checked(self.security.GetTokenInformation(token, 1, buffer, length, c.byref(length)))
            sid = SidAndAttributes.from_buffer(buffer).sid
            string = w.LPWSTR()
            checked(self.security.ConvertSidToStringSidW(sid, c.byref(string)))
            try:
                return token, string.value
            finally:
                self.kernel.LocalFree(c.cast(string, c.c_void_p))
        except BaseException:
            if token:
                self.kernel.CloseHandle(token)
            raise
        finally:
            if users_sid:
                self.kernel.LocalFree(users_sid)

    def delete_test_user(self):
        if self.username:
            status = self.network.NetUserDel(None, self.username)
            if status:
                raise c.WinError(status)

    def grant(self, path, sid, rights):
        import subprocess
        subprocess.run(['icacls', str(path), '/grant', f'*{sid}:{rights}'],
                       check=True, stdout=subprocess.DEVNULL)

    def launch(self, executable, token, environment, arguments=()):
        import subprocess
        if token is not None:
            profile = str(Path(os.environ['SystemDrive'] + '\\Users')/self.username)
            environment = dict(environment, TEMP=self.user_temp, TMP=self.user_temp,
                               USERPROFILE=profile, LOCALAPPDATA=profile+'\\AppData\\Local',
                               APPDATA=profile+'\\AppData\\Roaming', USERNAME=self.username)
        # cmd opens the redirected output inside the ordinary user's process;
        # this avoids inheriting runner-service pipe handles across accounts.
        import uuid
        output = Path(self.user_temp)/('process-' + uuid.uuid4().hex + '.log')
        shell = Path(os.environ['SystemRoot'])/'System32/cmd.exe'
        command = c.create_unicode_buffer(
            '"' + str(shell) + '" /D /S /C "' +
            subprocess.list2cmdline([str(executable), *arguments]) +
            ' > "' + str(output) + '" 2>&1"')
        env = c.create_unicode_buffer('\0'.join(f'{k}={v}' for k, v in sorted(environment.items())) + '\0\0')
        startup, process = StartupInfo(), ProcessInfo()
        startup.cb = c.sizeof(startup)
        startup.desktop = self.desktop_name
        original = w.HANDLE()
        try:
            if token is None:
                checked(self.security.OpenProcessToken(self.kernel.GetCurrentProcess(), 0x8B,
                                                       c.byref(original)))
                token = original
                checked(self.security.CreateProcessAsUserW(token, str(shell), command,
                                                          None, None, False, 0x08000400, env,
                                                          str(executable.parent), c.byref(startup), c.byref(process)))
            else:
                checked(self.security.CreateProcessWithLogonW(self.username, '.', self.password, 1,
                                                             str(shell), command, 0x08000400,
                                                             env, str(executable.parent),
                                                             c.byref(startup), c.byref(process)))
        finally:
            if original:
                self.kernel.CloseHandle(original)
        self.kernel.CloseHandle(process.thread)
        process.output = output
        return process

    def output(self, process):
        if process.output.exists():
            return process.output.read_text(encoding='utf-8', errors='replace')[-6000:]
        return '(no process output)'

    def wait(self, process, seconds, expected=0):
        result = self.kernel.WaitForSingleObject(process.process, seconds * 1000)
        if result != 0:
            raise TimeoutError(f'Process {process.pid} did not finish: wait={result}')
        code = w.DWORD()
        checked(self.kernel.GetExitCodeProcess(process.process, c.byref(code)))
        assert code.value == expected, (
            f'Process {process.pid} failed: 0x{code.value:08x}, expected={expected}\n' + self.output(process))

    def stop(self, process, token):
        if token is not None:
            checked(self.security.ImpersonateLoggedOnUser(token))
        try:
            event = self.kernel.OpenEventW(2, False, 'Local\\ClashOfRust.Exit')
        finally:
            if token is not None:
                checked(self.security.RevertToSelf())
        if event:
            try:
                checked(self.kernel.SetEvent(event))
            finally:
                self.kernel.CloseHandle(event)
        self.wait(process, 20)

    def cleanup(self, process):
        if self.kernel.WaitForSingleObject(process.process, 0) == 258:
            import subprocess
            subprocess.run(['taskkill', '/F', '/PID', str(process.pid), '/T'],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            self.kernel.WaitForSingleObject(process.process, 10000)
        self.kernel.CloseHandle(process.process)


def check_startup(api, executable, token, directory, background=False):
    environment = dict(os.environ, CLASH_OF_RUST_DATA_DIR=str(directory))
    process = api.launch(executable, token, environment, ['--background'] if background else [])
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    deadline = time.monotonic() + 45
    failure = 'settings.json was not created'
    try:
        while time.monotonic() < deadline:
            if api.kernel.WaitForSingleObject(process.process, 0) != 258:
                api.wait(process, 0)
                raise AssertionError('GUI exited before the core became ready:\n' + api.output(process))
            try:
                settings = json.loads((directory/'settings.json').read_text(encoding='utf-8'))
                request = urllib.request.Request(f"http://127.0.0.1:{settings['controller_port']}/configs",
                                                 headers={'Authorization': 'Bearer ' + settings['secret']})
                with opener.open(request, timeout=1) as response:
                    config = json.load(response)
                if config.get('mixed-port') == settings['mixed_port']:
                    break
            except (OSError, ValueError, KeyError) as error:
                failure = str(error)
            time.sleep(0.2)
        else:
            raise AssertionError(f'GUI/core startup failed in {directory}: {failure}')
        for name in ('settings.json', 'profiles.json', 'runtime/config.yaml', 'runtime/candidate.yaml'):
            assert (directory/name).is_file(), f'Startup failed to save {name}'
        profiles = json.loads((directory/'profiles.json').read_text(encoding='utf-8'))
        assert profiles and settings['active_profile'], 'First-run default profile was not saved'
        assert not config.get('tun', {}).get('enable', False), 'Startup test must not enable TUN'
        api.stop(process, token)
        print('PASS: real GUI, default profile, offline Geo, configuration saves and core startup;',
              'background=' + str(background), flush=True)
    finally:
        api.cleanup(process)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('executable', type=Path)
    parser.add_argument('--test-binary', type=Path, required=True)
    args = parser.parse_args()
    assert os.name == 'nt', 'This is a native Windows test'
    executable = args.executable.resolve(strict=True)
    test_binary = args.test_binary.resolve(strict=True)
    api = Windows()
    assert api.shell.IsUserAnAdmin(), 'Expected an elevated CI runner so both contexts can be tested'
    limited = w.HANDLE()
    try:
        limited, sid = api.ordinary_user()
        api.create_test_desktop(sid)
        api.grant(executable.parent, sid, '(OI)(CI)RX')
        api.grant(test_binary.parent, sid, 'RX')
        api.grant(test_binary, sid, 'RX')
        with tempfile.TemporaryDirectory(prefix='clash startup 中文 ') as work:
            root = Path(work)
            api.grant(root, sid, '(OI)(CI)M')
            api.user_temp = str(root)
            # Use a real local Users account, rather than a filtered service
            # token that Windows may reject before application initialization.
            for arguments in (['config::tests::', '--test-threads=1'],
                              ['profile_transaction::tests::', '--test-threads=1'],
                              ['platform::windows::show_event_tests::', '--test-threads=1']):
                process = api.launch(test_binary, limited, dict(os.environ), arguments)
                try:
                    api.wait(process, 60)
                finally:
                    api.cleanup(process)
                print('PASS: ordinary user ' + arguments[0], flush=True)
            fresh = root/'ordinary first run'
            fresh.mkdir()
            check_startup(api, executable, limited, fresh)
            check_startup(api, executable, limited, fresh, background=True)
            # Simulate a 0.4.12 data directory: new preferences must default
            # correctly without requiring users to discard their settings.
            old_settings = json.loads((fresh/'settings.json').read_text(encoding='utf-8'))
            for name in ('node_sort', 'rule_overrides'):
                old_settings.pop(name, None)
            (fresh/'settings.json').write_text(json.dumps(old_settings), encoding='utf-8')
            check_startup(api, executable, limited, fresh)
            # 0.4.13 first added the disk-backed transaction recovery path.
            # Leave a durable journal and a partially written live settings
            # file; the next ordinary startup must roll it back before loading.
            import uuid
            transaction = fresh/('.profile-transaction-' + uuid.uuid4().hex)
            transaction.mkdir()
            original_settings = (fresh/'settings.json').read_bytes()
            (transaction/'0.old').write_bytes(original_settings)
            (fresh/'settings.json').write_text('{partial settings', encoding='utf-8')
            (fresh/'profile-transaction.json').write_text(json.dumps({
                'directory': transaction.name,
                'entries': [{'target': 'settings.json', 'existed': True, 'remove': False}],
            }), encoding='utf-8')
            check_startup(api, executable, limited, fresh)
            assert (fresh/'settings.json').read_bytes() == original_settings
            assert not transaction.exists() and not (fresh/'profile-transaction.json').exists()
            print('PASS: 0.4.12 settings and 0.4.13 interrupted transactions recover under ordinary permissions',
                  flush=True)
            migrated = root/'elevated then ordinary'
            migrated.mkdir()
            check_startup(api, executable, None, migrated)
            check_startup(api, executable, limited, migrated)
            # The same early CLI handler is used by the UAC repair assistant.
            # A new ordinary account must enable/disable startup successfully,
            # and an assistant running as a different account must refuse it.
            for arguments, expected in ((['--autostart-enable'], 0),
                                        (['--autostart-status'], 0),
                                        (['--autostart-remove'], 0),
                                        (['--autostart-status'], 1),
                                        (['--autostart-remove', '--autostart-user', sid], 3)):
                process = api.launch(executable, None if expected == 3 else limited,
                                     dict(os.environ), arguments)
                try:
                    api.wait(process, 30, expected)
                finally:
                    api.cleanup(process)
            print('PASS: ordinary user startup toggle and repair-account validation', flush=True)
    finally:
        try:
            api.close_test_desktop()
        finally:
            if limited:
                api.kernel.CloseHandle(limited)
            # Registering a logon task can start the real background GUI as
            # these logon sessions are created. Clean up only this test user's
            # application process tree, before deleting the temporary account.
            if api.username:
                import subprocess
                subprocess.run(['taskkill', '/F', '/FI',
                                'USERNAME eq ' + os.environ['COMPUTERNAME'] + '\\' + api.username,
                                '/IM', 'clash-of-rust.exe', '/T'],
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            api.delete_test_user()
    print('PASS: Windows startup checks use a verified ordinary local Users account', flush=True)


if __name__ == '__main__':
    main()
