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
import sys

# Hosted Windows stdout may use cp1252 even when the app emits UTF-8.
sys.stdout.reconfigure(encoding='utf-8', errors='backslashreplace')
sys.stderr.reconfigure(encoding='utf-8', errors='backslashreplace')


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


class UnicodeString(c.Structure):
    _fields_ = [('length', w.USHORT), ('maximum', w.USHORT), ('buffer', w.LPWSTR)]


class ObjectAttributes(c.Structure):
    _fields_ = [('length', w.ULONG), ('root', w.HANDLE),
                ('name', c.POINTER(UnicodeString)), ('attributes', w.ULONG),
                ('descriptor', c.c_void_p), ('quality', c.c_void_p)]


class Trustee(c.Structure):
    _fields_ = [('multiple', c.c_void_p), ('operation', c.c_int),
                ('form', c.c_int), ('kind', c.c_int), ('name', c.c_void_p)]


class ExplicitAccess(c.Structure):
    _fields_ = [('permissions', w.DWORD), ('mode', c.c_int),
                ('inheritance', w.DWORD), ('trustee', Trustee)]


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
        self.nt = c.WinDLL('ntdll')
        self.station = self.desktop_handle = None
        self.desktop_name = None
        self.username = self.password = self.user_temp = None
        self.namespace = self.namespace_descriptor = self.namespace_dacl = None
        signatures = [
            (self.kernel, 'GetCurrentProcess', [], w.HANDLE),
            (self.kernel, 'GetCurrentProcessId', [], w.DWORD),
            (self.kernel, 'ProcessIdToSessionId', [w.DWORD, c.POINTER(w.DWORD)], w.BOOL),
            (self.kernel, 'CreateEventW', [c.c_void_p, w.BOOL, w.BOOL, w.LPCWSTR], w.HANDLE),
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
            (self.security, 'GetSecurityInfo',
             [w.HANDLE, c.c_int, w.DWORD, c.c_void_p, c.c_void_p,
              c.POINTER(c.c_void_p), c.c_void_p, c.POINTER(c.c_void_p)], w.DWORD),
            (self.security, 'SetSecurityInfo',
             [w.HANDLE, c.c_int, w.DWORD, c.c_void_p, c.c_void_p, c.c_void_p, c.c_void_p], w.DWORD),
            (self.security, 'SetEntriesInAclW',
             [w.ULONG, c.POINTER(ExplicitAccess), c.c_void_p, c.POINTER(c.c_void_p)], w.DWORD),
            (self.nt, 'NtOpenDirectoryObject',
             [c.POINTER(w.HANDLE), w.DWORD, c.POINTER(ObjectAttributes)], c.c_long),
            (self.nt, 'RtlNtStatusToDosError', [c.c_long], w.DWORD),
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

    def allow_test_namespace(self, sid):
        # A run-as account shares the runner's session, rather than owning a
        # normal interactive session. Permit only this temporary SID to create
        # named objects there, then restore the original DACL during cleanup.
        session = w.DWORD()
        checked(self.kernel.ProcessIdToSessionId(self.kernel.GetCurrentProcessId(), c.byref(session)))
        path = (f'\\Sessions\\{session.value}\\BaseNamedObjects'
                if session.value else '\\BaseNamedObjects')
        buffer = c.create_unicode_buffer(path)
        name = UnicodeString(len(path) * 2, c.sizeof(buffer), c.cast(buffer, w.LPWSTR))
        attributes = ObjectAttributes(c.sizeof(ObjectAttributes), None, c.pointer(name), 0x40, None, None)
        handle = w.HANDLE()
        status = self.nt.NtOpenDirectoryObject(c.byref(handle), 0x60001, c.byref(attributes))
        if status < 0:
            raise c.WinError(self.nt.RtlNtStatusToDosError(status))
        self.namespace = handle
        descriptor, dacl = c.c_void_p(), c.c_void_p()
        status = self.security.GetSecurityInfo(handle, 6, 4, None, None,
                                               c.byref(dacl), None, c.byref(descriptor))
        if status:
            raise c.WinError(status)
        self.namespace_descriptor, self.namespace_dacl = descriptor, dacl
        user, updated = c.c_void_p(), c.c_void_p()
        try:
            checked(self.security.ConvertStringSidToSidW(sid, c.byref(user)))
            # Win32 named-object initialization also needs to create namespace
            # subdirectories. Query/traverse/create-object (0x7) alone lets
            # NtOpenDirectoryObject succeed but CreateEventW fails with error 5.
            directory_rights = 0x1 | 0x2 | 0x4 | 0x8
            entry = ExplicitAccess(directory_rights, 1, 0, Trustee(None, 0, 0, 1, user.value))
            status = self.security.SetEntriesInAclW(1, c.byref(entry), dacl, c.byref(updated))
            if status:
                raise c.WinError(status)
            status = self.security.SetSecurityInfo(handle, 6, 4, None, None, updated, None)
            if status:
                raise c.WinError(status)
        finally:
            for allocation in (user, updated):
                if allocation:
                    self.kernel.LocalFree(allocation)

    def restore_test_namespace(self):
        if self.namespace:
            try:
                if self.namespace_descriptor:
                    status = self.security.SetSecurityInfo(
                        self.namespace, 6, 4, None, None, self.namespace_dacl, None)
                    if status:
                        raise c.WinError(status)
            finally:
                if self.namespace_descriptor:
                    self.kernel.LocalFree(self.namespace_descriptor)
                self.kernel.CloseHandle(self.namespace)

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
        batch = output.with_suffix('.cmd')
        # Batch execution waits for GUI programs and preserves their exit code.
        # START through /C previously reported success for a failed Rust test.
        batch.write_text('@echo off\nchcp 65001 >nul\n' + subprocess.list2cmdline([str(executable), *arguments]) +
                         ' > "' + str(output) + '" 2>&1\nexit /b %errorlevel%\n', encoding='utf-8')
        shell = Path(os.environ['SystemRoot'])/'System32/cmd.exe'
        command = c.create_unicode_buffer(
            '"' + str(shell) + '" /D /S /C ""' + str(batch) + '""')
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


def check_startup(api, executable, token, directory, background=False, geometry=None):
    if not (directory/'settings.json').exists():
        # CI and local proxies may already occupy the application's defaults.
        # Use two distinct OS-selected ports while retaining all other defaults.
        import socket
        import uuid
        with socket.socket() as controller, socket.socket() as mixed:
            controller.bind(('127.0.0.1', 0))
            mixed.bind(('127.0.0.1', 0))
            settings = {'controller_port': controller.getsockname()[1],
                        'mixed_port': mixed.getsockname()[1],
                        'secret': str(uuid.uuid4()), 'proxy_mode': 'off'}
        (directory/'settings.json').write_text(json.dumps(settings), encoding='utf-8')
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
                if (config.get('mixed-port') == settings['mixed_port']
                        and settings.get('active_profile')):
                    break
            except (OSError, ValueError, KeyError) as error:
                failure = str(error)
            time.sleep(0.2)
        else:
            raise AssertionError(f'GUI/core startup failed in {directory}: {failure}\n' +
                                 api.output(process))
        for name in ('settings.json', 'profiles.json', 'runtime/config.yaml', 'runtime/candidate.yaml'):
            assert (directory/name).is_file(), f'Startup failed to save {name}'
        profiles = json.loads((directory/'profiles.json').read_text(encoding='utf-8'))
        assert profiles and settings['active_profile'], 'First-run default profile was not saved'
        assert not config.get('tun', {}).get('enable', False), 'Startup test must not enable TUN'
        if geometry is not None:
            check_window_geometry(api, directory, geometry)
        api.stop(process, token)
        print('PASS: real GUI, default profile, offline Geo, configuration saves and core startup;',
              'background=' + str(background), flush=True)
    finally:
        api.cleanup(process)


def check_window_geometry(api, directory, scenario):
    """Resize the real ordinary-user HWND, then verify saved logical dimensions."""
    callback_type = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    for name, arguments, result in [
        ('EnumDesktopWindows', [w.HANDLE, callback_type, w.LPARAM], w.BOOL),
        ('GetWindowTextW', [w.HWND, w.LPWSTR, c.c_int], c.c_int),
        ('GetClientRect', [w.HWND, c.POINTER(w.RECT)], w.BOOL),
        ('GetWindowRect', [w.HWND, c.POINTER(w.RECT)], w.BOOL),
        ('GetDpiForWindow', [w.HWND], w.UINT),
        ('SetWindowPos', [w.HWND, w.HWND, c.c_int, c.c_int, c.c_int, c.c_int, w.UINT], w.BOOL),
        ('ShowWindow', [w.HWND, c.c_int], w.BOOL),
        ('IsZoomed', [w.HWND], w.BOOL),
        ('IsIconic', [w.HWND], w.BOOL),
        ('GetThreadDesktop', [w.DWORD], w.HANDLE),
        ('SetThreadDesktop', [w.HANDLE], w.BOOL),
        ('SetThreadDpiAwarenessContext', [w.HANDLE], w.HANDLE),
    ]:
        function = getattr(api.user, name)
        function.argtypes, function.restype = arguments, result
    api.kernel.GetCurrentThreadId.restype = w.DWORD
    station = api.user.GetProcessWindowStation()
    desktop = api.user.GetThreadDesktop(api.kernel.GetCurrentThreadId())
    checked(api.user.SetProcessWindowStation(api.station))
    dpi_context = None
    try:
        checked(api.user.SetThreadDesktop(api.desktop_handle))
        try:
            dpi_context = checked(api.user.SetThreadDpiAwarenessContext(c.c_void_p(-4)))
            def wait_for(predicate):
                deadline = time.monotonic() + 15
                while time.monotonic() < deadline:
                    value = predicate()
                    if value:
                        return value
                    time.sleep(.05)
                raise AssertionError('Timed out waiting for real window geometry: ' + scenario)

            def find_window():
                matches = []
                @callback_type
                def visit(window, _):
                    title = c.create_unicode_buffer(256)
                    api.user.GetWindowTextW(window, title, len(title))
                    if title.value == 'Clash of Rust · 原生代理客户端':
                        matches.append(window)
                    return True
                checked(api.user.EnumDesktopWindows(api.desktop_handle, visit, 0))
                return matches[0] if len(matches) == 1 else None

            window = wait_for(find_window)
            dpi = checked(api.user.GetDpiForWindow(window))
            scale = dpi / 96
            expected = {'width': 900, 'height': 500}
            state_path = directory/'window-state.json'

            def stored():
                try:
                    return json.loads(state_path.read_text('utf-8'))
                except (OSError, ValueError):
                    return None

            def dimensions():
                bounds = w.RECT()
                checked(api.user.GetClientRect(window, c.byref(bounds)))
                return (bounds.right - bounds.left) / scale, (bounds.bottom - bounds.top) / scale

            def matches_expected():
                width, height = dimensions()
                return abs(width - expected['width']) <= 1 and abs(height - expected['height']) <= 1

            if scenario == 'resize':
                client, outer = w.RECT(), w.RECT()
                checked(api.user.GetClientRect(window, c.byref(client)))
                checked(api.user.GetWindowRect(window, c.byref(outer)))
                frame_width = outer.right - outer.left - client.right + client.left
                frame_height = outer.bottom - outer.top - client.bottom + client.top
                checked(api.user.SetWindowPos(window, None, 0, 0,
                    round(expected['width'] * scale) + frame_width,
                    round(expected['height'] * scale) + frame_height, 0x16))
                wait_for(matches_expected)
                wait_for(lambda: stored() == expected)
                api.user.ShowWindow(window, 3)  # SW_MAXIMIZE
                wait_for(lambda: api.user.IsZoomed(window))
                time.sleep(1.2)
                assert stored() == expected, 'Maximization overwrote ordinary dimensions'
                api.user.ShowWindow(window, 6)  # SW_MINIMIZE
                wait_for(lambda: api.user.IsIconic(window))
                time.sleep(1.2)
                assert stored() == expected, 'Minimization overwrote ordinary dimensions'
            else:
                assert scenario == 'relaunch'
                wait_for(matches_expected)
                assert not api.user.IsZoomed(window) and not api.user.IsIconic(window)
                assert stored() == expected
            print(f'PASS: real Windows {scenario}; ordinary client=900x500 logical pixels; DPI={dpi}', flush=True)
        finally:
            if dpi_context is not None:
                checked(api.user.SetThreadDpiAwarenessContext(dpi_context))
    finally:
        try:
            checked(api.user.SetProcessWindowStation(station))
        finally:
            checked(api.user.SetThreadDesktop(desktop))


def check_launcher(api, token, sid):
    script = Path(__file__).resolve()
    for arguments, expected in (([str(script), '--probe', sid], 0),
                                ([str(script), '--probe-exit', '37'], 37)):
        process = api.launch(Path(sys.executable), token, dict(os.environ), arguments)
        try:
            api.wait(process, 30, expected)
            print(api.output(process), flush=True)
        finally:
            api.cleanup(process)
    print('PASS: non-admin token, session namespace and child failure propagation', flush=True)


def probe(sid):
    import uuid
    api = Windows()
    assert not api.shell.IsUserAnAdmin(), 'Child must really have ordinary permissions'
    length = w.DWORD()
    token = w.HANDLE()
    checked(api.security.OpenProcessToken(api.kernel.GetCurrentProcess(), 8, c.byref(token)))
    try:
        api.security.GetTokenInformation(token, 25, None, 0, c.byref(length))
        buffer = c.create_string_buffer(length.value)
        checked(api.security.GetTokenInformation(token, 25, buffer, length, c.byref(length)))
        integrity = c.cast(SidAndAttributes.from_buffer(buffer).sid, c.POINTER(c.c_ubyte))
        level = c.cast(c.addressof(integrity.contents) + 8 + 4 * (integrity[1] - 1),
                       c.POINTER(w.DWORD)).contents.value
        assert level == 8192, f'Expected medium-integrity token, got {level}'
    finally:
        api.kernel.CloseHandle(token)
    descriptor = c.c_void_p()
    checked(api.security.ConvertStringSecurityDescriptorToSecurityDescriptorW(
        f'D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})S:(ML;;NW;;;ME)',
        1, c.byref(descriptor), None))
    try:
        attributes = SecurityAttributes(c.sizeof(SecurityAttributes), descriptor.value, False)
        for security in (None, c.byref(attributes)):
            event = checked(api.kernel.CreateEventW(
                security, False, False, 'Local\\ClashOfRust.CI.Probe.' + uuid.uuid4().hex))
            api.kernel.CloseHandle(event)
    finally:
        api.kernel.LocalFree(descriptor)
    print('PASS: genuine ordinary medium-integrity process creates local named events')


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
        api.allow_test_namespace(sid)
        api.grant(executable.parent, sid, '(OI)(CI)RX')
        api.grant(test_binary.parent, sid, 'RX')
        api.grant(test_binary, sid, 'RX')
        with tempfile.TemporaryDirectory(prefix='clash startup 中文 ') as work:
            root = Path(work)
            api.grant(root, sid, '(OI)(CI)M')
            api.user_temp = str(root)
            check_launcher(api, limited, sid)
            # Use a real local Users account, rather than a filtered service
            # token that Windows may reject before application initialization.
            for arguments in (['config::tests::', '--test-threads=1'],
                              ['profile_transaction::tests::', '--test-threads=1'],
                              ['platform::windows::show_event_tests::', '--test-threads=1'],
                              ['platform::application_guard_tests::', '--test-threads=1'],
                              ['native_ordinary_user_can_manage_a_new_logon_task',
                               '--ignored', '--test-threads=1']):
                process = api.launch(test_binary, limited, dict(os.environ), arguments)
                try:
                    api.wait(process, 60)
                    print(api.output(process), flush=True)
                finally:
                    api.cleanup(process)
                print('PASS: ordinary user ' + arguments[0], flush=True)
            fresh = root/'ordinary first run'
            fresh.mkdir()
            check_startup(api, executable, limited, fresh)
            check_startup(api, executable, limited, fresh, background=True)
            check_startup(api, executable, limited, fresh, geometry='resize')
            check_startup(api, executable, limited, fresh, geometry='relaunch')
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
            api.restore_test_namespace()
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
    if sys.argv[1:2] == ['--probe']:
        probe(sys.argv[2])
    elif sys.argv[1:2] == ['--probe-exit']:
        sys.exit(int(sys.argv[2]))
    else:
        main()
