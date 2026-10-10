"""Run the real Windows GUI with a medium-integrity, non-admin token.

Uses an isolated data directory and no system proxy/TUN. No local accounts or
passwords are created; the restricted token retains the runner user's SID.
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


def checked(result):
    if not result:
        raise c.WinError(c.get_last_error())
    return result


class Windows:
    def __init__(self):
        self.kernel = c.WinDLL('kernel32', use_last_error=True)
        self.security = c.WinDLL('advapi32', use_last_error=True)
        self.shell = c.WinDLL('shell32', use_last_error=True)
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
            (self.security, 'GetLengthSid', [c.c_void_p], w.DWORD),
            (self.security, 'CreateRestrictedToken',
             [w.HANDLE, w.DWORD, w.DWORD, c.POINTER(SidAndAttributes),
              w.DWORD, c.c_void_p, w.DWORD, c.c_void_p, c.POINTER(w.HANDLE)], w.BOOL),
            (self.security, 'SetTokenInformation', [w.HANDLE, c.c_int, c.c_void_p, w.DWORD], w.BOOL),
            (self.security, 'ImpersonateLoggedOnUser', [w.HANDLE], w.BOOL),
            (self.security, 'RevertToSelf', [], w.BOOL),
            (self.security, 'CreateProcessAsUserW',
             [w.HANDLE, w.LPCWSTR, w.LPWSTR, c.c_void_p, c.c_void_p, w.BOOL,
              w.DWORD, c.c_void_p, w.LPCWSTR, c.POINTER(StartupInfo), c.POINTER(ProcessInfo)], w.BOOL),
            (self.shell, 'IsUserAnAdmin', [], w.BOOL),
        ]
        for library, name, args, result in signatures:
            function = getattr(library, name)
            function.argtypes, function.restype = args, result

    def limited_token(self):
        original, limited = w.HANDLE(), w.HANDLE()
        admin_sid, medium_sid = c.c_void_p(), c.c_void_p()
        try:
            checked(self.security.OpenProcessToken(self.kernel.GetCurrentProcess(), 0x8B, c.byref(original)))
            checked(self.security.ConvertStringSidToSidW('S-1-5-32-544', c.byref(admin_sid)))
            disabled = SidAndAttributes(admin_sid.value, 0)
            checked(self.security.CreateRestrictedToken(original, 5, 1, c.byref(disabled),
                                                       0, None, 0, None, c.byref(limited)))
            checked(self.security.ConvertStringSidToSidW('S-1-16-8192', c.byref(medium_sid)))
            label = SidAndAttributes(medium_sid.value, 0x20)
            checked(self.security.SetTokenInformation(limited, 25, c.byref(label),
                                                     c.sizeof(label) + self.security.GetLengthSid(medium_sid)))
            checked(self.security.ImpersonateLoggedOnUser(limited))
            try:
                assert not self.shell.IsUserAnAdmin(), 'Startup test must really use non-admin permissions'
            finally:
                checked(self.security.RevertToSelf())
            token, limited = limited, w.HANDLE()
            return token
        finally:
            for sid in (admin_sid, medium_sid):
                if sid:
                    self.kernel.LocalFree(sid)
            for token in (original, limited):
                if token:
                    self.kernel.CloseHandle(token)

    def launch(self, executable, token, environment, arguments=()):
        import subprocess
        command = c.create_unicode_buffer(subprocess.list2cmdline([str(executable), *arguments]))
        env = c.create_unicode_buffer('\0'.join(f'{k}={v}' for k, v in sorted(environment.items())) + '\0\0')
        startup, process = StartupInfo(), ProcessInfo()
        startup.cb = c.sizeof(startup)
        original = w.HANDLE()
        try:
            if token is None:
                checked(self.security.OpenProcessToken(self.kernel.GetCurrentProcess(), 0x8B,
                                                       c.byref(original)))
                token = original
            checked(self.security.CreateProcessAsUserW(token, str(executable), command,
                                                      None, None, False, 0x08000400, env,
                                                      str(executable.parent), c.byref(startup), c.byref(process)))
        finally:
            if original:
                self.kernel.CloseHandle(original)
        self.kernel.CloseHandle(process.thread)
        return process

    def wait(self, process, seconds):
        result = self.kernel.WaitForSingleObject(process.process, seconds * 1000)
        if result != 0:
            raise TimeoutError(f'Process {process.pid} did not finish: wait={result}')
        code = w.DWORD()
        checked(self.kernel.GetExitCodeProcess(process.process, c.byref(code)))
        assert code.value == 0, f'Process {process.pid} failed: 0x{code.value:08x}'

    def stop(self, process):
        event = self.kernel.OpenEventW(2, False, 'Local\\ClashOfRust.Exit')
        if event:
            try:
                checked(self.kernel.SetEvent(event))
            finally:
                self.kernel.CloseHandle(event)
        self.wait(process, 20)

    def cleanup(self, process):
        if self.kernel.WaitForSingleObject(process.process, 0) == 258:
            self.kernel.TerminateProcess(process.process, 1)
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
                raise AssertionError('GUI exited before the core became ready')
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
        api.stop(process)
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
    limited = api.limited_token()
    try:
        # These tests now run with real ordinary permissions, not just the
        # elevated runner token that masked the original startup failures.
        process = api.launch(test_binary, limited, dict(os.environ),
                             ['config::tests::', '--test-threads=1'])
        try:
            api.wait(process, 60)
        finally:
            api.cleanup(process)
        with tempfile.TemporaryDirectory(prefix='clash startup 中文 ') as work:
            root = Path(work)
            fresh = root/'ordinary first run'
            fresh.mkdir()
            check_startup(api, executable, limited, fresh)
            check_startup(api, executable, limited, fresh, background=True)
            migrated = root/'elevated then ordinary'
            migrated.mkdir()
            check_startup(api, executable, None, migrated)
            check_startup(api, executable, limited, migrated)
    finally:
        api.kernel.CloseHandle(limited)
    print('PASS: Windows startup checks use a verified non-admin medium-integrity token', flush=True)


if __name__ == '__main__':
    main()
