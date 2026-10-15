"""Named Windows basic suites, with a failure-preserving CI receipt."""
import sys
import time
from build_support import ROOT, host_arch, run, write_json


def allow_ordinary_user_directory(path, permissions='M'):
    """Share only a test-owned fixture with the runner's unelevated token."""
    import csv
    import subprocess
    identity = subprocess.check_output(['whoami', '/user', '/fo', 'csv', '/nh'], text=True)
    sid = next(csv.reader(identity.splitlines()))[-1]
    if not sid.startswith('S-1-'):
        raise RuntimeError('Cannot identify the Windows fixture owner')
    subprocess.run(['icacls', str(path), '/grant', f'*{sid}:(OI)(CI){permissions}', '/Q'],
                   check=True, capture_output=True)


def run_unelevated(command, env, timeout=240, on_start=None, expected_code=0):
    """Run native checks with an actual medium-integrity, non-admin token."""
    import ctypes as c
    from ctypes import wintypes as w
    import subprocess
    import contextlib
    kernel = c.WinDLL('kernel32', use_last_error=True)
    advapi = c.WinDLL('advapi32', use_last_error=True)
    class SID_ATTRIBUTES(c.Structure):
        _fields_ = [('sid', w.LPVOID), ('attributes', w.DWORD)]
    class STARTUP(c.Structure):
        _fields_ = [('cb', w.DWORD), ('reserved', w.LPWSTR), ('desktop', w.LPWSTR),
                    ('title', w.LPWSTR), ('x', w.DWORD), ('y', w.DWORD), ('cx', w.DWORD),
                    ('cy', w.DWORD), ('chars_x', w.DWORD), ('chars_y', w.DWORD),
                    ('fill', w.DWORD), ('flags', w.DWORD), ('show', w.WORD),
                    ('reserved_count', w.WORD), ('reserved_bytes', c.POINTER(w.BYTE)),
                    ('stdin', w.HANDLE), ('stdout', w.HANDLE), ('stderr', w.HANDLE)]
    class PROCESS(c.Structure):
        _fields_ = [('process', w.HANDLE), ('thread', w.HANDLE), ('pid', w.DWORD), ('tid', w.DWORD)]
    kernel.GetCurrentProcess.restype = w.HANDLE
    kernel.GetStdHandle.argtypes = [w.DWORD]
    kernel.GetStdHandle.restype = w.HANDLE
    kernel.CloseHandle.argtypes = [w.HANDLE]
    kernel.WaitForSingleObject.argtypes = [w.HANDLE, w.DWORD]
    kernel.GetExitCodeProcess.argtypes = [w.HANDLE, c.POINTER(w.DWORD)]
    kernel.TerminateProcess.argtypes = [w.HANDLE, w.UINT]
    kernel.SetErrorMode.argtypes = [w.UINT]
    kernel.SetErrorMode.restype = w.UINT
    kernel.LocalFree.argtypes = [w.LPVOID]
    advapi.OpenProcessToken.argtypes = [w.HANDLE, w.DWORD, c.POINTER(w.HANDLE)]
    advapi.GetTokenInformation.argtypes = [w.HANDLE, c.c_int, w.LPVOID, w.DWORD, c.POINTER(w.DWORD)]
    advapi.DuplicateTokenEx.argtypes = [w.HANDLE, w.DWORD, w.LPVOID, c.c_int, c.c_int, c.POINTER(w.HANDLE)]
    advapi.CreateRestrictedToken.argtypes = [w.HANDLE, w.DWORD, w.DWORD, c.POINTER(SID_ATTRIBUTES), w.DWORD, w.LPVOID, w.DWORD, w.LPVOID, c.POINTER(w.HANDLE)]
    advapi.ConvertStringSidToSidW.argtypes = [w.LPCWSTR, c.POINTER(w.LPVOID)]
    advapi.SetTokenInformation.argtypes = [w.HANDLE, c.c_int, w.LPVOID, w.DWORD]
    advapi.GetLengthSid.argtypes = [w.LPVOID]
    advapi.GetLengthSid.restype = w.DWORD
    advapi.CheckTokenMembership.argtypes = [w.HANDLE, w.LPVOID, c.POINTER(w.BOOL)]
    advapi.CreateProcessAsUserW.argtypes = [w.HANDLE, w.LPCWSTR, w.LPWSTR, w.LPVOID, w.LPVOID, w.BOOL, w.DWORD, w.LPVOID, w.LPCWSTR, c.POINTER(STARTUP), c.POINTER(PROCESS)]
    def require(ok):
        if not ok:
            raise c.WinError(c.get_last_error())
    def token_defaults(token):
        # Restricted tokens can retain an Administrators-only default DACL.
        # CreateProcessAsUser uses it for the new process/thread objects; once
        # Administrators is deny-only, the child cannot initialize itself.
        # Give the actual user access before creating any child objects.
        advapi.ConvertSidToStringSidW.argtypes = [w.LPVOID, c.POINTER(w.LPWSTR)]
        advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW.argtypes = [w.LPCWSTR, w.DWORD, c.POINTER(w.LPVOID), w.LPVOID]
        advapi.GetSecurityDescriptorDacl.argtypes = [w.LPVOID, c.POINTER(w.BOOL), c.POINTER(w.LPVOID), c.POINTER(w.BOOL)]
        needed = w.DWORD()
        advapi.GetTokenInformation(token, 1, None, 0, c.byref(needed))
        user = c.create_string_buffer(needed.value)
        require(advapi.GetTokenInformation(token, 1, user, len(user), c.byref(needed)))
        sid = c.cast(user, c.POINTER(SID_ATTRIBUTES)).contents.sid
        text, descriptor = w.LPWSTR(), w.LPVOID()
        try:
            require(advapi.ConvertSidToStringSidW(sid, c.byref(text)))
            sddl = 'D:P(A;;GA;;;SY)(A;;GA;;;' + text.value + ')'
            require(advapi.ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl, 1, c.byref(descriptor), None))
            acl = w.LPVOID()
            present, defaulted = w.BOOL(), w.BOOL()
            require(advapi.GetSecurityDescriptorDacl(descriptor, c.byref(present), c.byref(acl), c.byref(defaulted)))
            require(advapi.SetTokenInformation(token, 6, c.byref(acl), c.sizeof(acl)))
        finally:
            if descriptor:
                kernel.LocalFree(descriptor)
            if text:
                kernel.LocalFree(c.cast(text, w.LPVOID))
    @contextlib.contextmanager
    def desktop_access(token):
        # Hosted runners can own a service desktop accessible only through the
        # Administrators group. A filtered token needs its user's explicit ACE
        # before user32 can initialize. Restore the original ACL after testing.
        user32 = c.WinDLL('user32', use_last_error=True)
        user32.GetProcessWindowStation.restype = w.HANDLE
        user32.GetThreadDesktop.argtypes = [w.DWORD]
        user32.GetThreadDesktop.restype = w.HANDLE
        user32.GetUserObjectInformationW.argtypes = [w.HANDLE, c.c_int, w.LPVOID, w.DWORD, c.POINTER(w.DWORD)]
        user32.OpenWindowStationW.argtypes = [w.LPCWSTR, w.BOOL, w.DWORD]
        user32.OpenWindowStationW.restype = w.HANDLE
        user32.OpenDesktopW.argtypes = [w.LPCWSTR, w.DWORD, w.BOOL, w.DWORD]
        user32.OpenDesktopW.restype = w.HANDLE
        user32.CloseWindowStation.argtypes = [w.HANDLE]
        user32.CloseDesktop.argtypes = [w.HANDLE]
        advapi.GetSecurityInfo.argtypes = [w.HANDLE, c.c_int, w.DWORD, w.LPVOID, w.LPVOID, c.POINTER(w.LPVOID), w.LPVOID, c.POINTER(w.LPVOID)]
        advapi.SetSecurityInfo.argtypes = [w.HANDLE, c.c_int, w.DWORD, w.LPVOID, w.LPVOID, w.LPVOID, w.LPVOID]
        class TRUSTEE(c.Structure):
            _fields_ = [('multiple', w.LPVOID), ('operation', c.c_int), ('form', c.c_int), ('type', c.c_int), ('name', w.LPVOID)]
        class ACCESS(c.Structure):
            _fields_ = [('permissions', w.DWORD), ('mode', c.c_int), ('inheritance', w.DWORD), ('trustee', TRUSTEE)]
        advapi.SetEntriesInAclW.argtypes = [w.ULONG, c.POINTER(ACCESS), w.LPVOID, c.POINTER(w.LPVOID)]
        needed = w.DWORD()
        advapi.GetTokenInformation(token, 1, None, 0, c.byref(needed))
        user = c.create_string_buffer(needed.value)
        require(advapi.GetTokenInformation(token, 1, user, len(user), c.byref(needed)))
        sid = c.cast(user, c.POINTER(SID_ATTRIBUTES)).contents.sid
        def name(handle):
            text = c.create_unicode_buffer(512)
            require(user32.GetUserObjectInformationW(handle, 2, text, c.sizeof(text), c.byref(needed)))
            return text.value
        station = name(user32.GetProcessWindowStation())
        desktop = name(user32.GetThreadDesktop(kernel.GetCurrentThreadId()))
        handles = [(user32.OpenWindowStationW(station, False, 0x60000), user32.CloseWindowStation),
                   (user32.OpenDesktopW(desktop, 0, False, 0x60000), user32.CloseDesktop)]
        saved = []
        try:
            for handle, _ in handles:
                require(handle)
                acl, descriptor, updated = w.LPVOID(), w.LPVOID(), w.LPVOID()
                code = advapi.GetSecurityInfo(handle, 7, 4, None, None, c.byref(acl), None, c.byref(descriptor))
                if code:
                    raise c.WinError(code)
                saved.append((handle, acl, descriptor, updated))
                entry = ACCESS(0x10000000, 1, 0, TRUSTEE(None, 0, 0, 1, sid))
                code = advapi.SetEntriesInAclW(1, c.byref(entry), acl, c.byref(updated))
                if code:
                    raise c.WinError(code)
                code = advapi.SetSecurityInfo(handle, 7, 4, None, None, updated, None)
                if code:
                    raise c.WinError(code)
            yield station+'\\'+desktop
        finally:
            for handle, acl, descriptor, updated in reversed(saved):
                advapi.SetSecurityInfo(handle, 7, 4, None, None, acl, None)
                kernel.LocalFree(descriptor)
                if updated:
                    kernel.LocalFree(updated)
            for handle, close in handles:
                if handle:
                    close(handle)
    source, linked, token = w.HANDLE(), w.HANDLE(), w.HANDLE()
    process = PROCESS()
    admin_sid, medium_sid = w.LPVOID(), w.LPVOID()
    # Loader failures must become test failures, never modal error dialogs on
    # a developer's desktop (or an unattended runner).
    previous_error_mode = kernel.SetErrorMode(0x8003)
    try:
        require(advapi.OpenProcessToken(kernel.GetCurrentProcess(), 0xF01FF, c.byref(source)))
        needed = w.DWORD()
        # A local developer may already be running at medium integrity. Its
        # linked token is the elevated token, not a token we should duplicate.
        # In particular, Windows can return ERROR_BAD_IMPERSONATION_LEVEL here.
        elevation = w.DWORD()
        require(advapi.GetTokenInformation(source, 20, c.byref(elevation), c.sizeof(elevation), c.byref(needed)))
        require(advapi.ConvertStringSidToSidW('S-1-5-32-544', c.byref(admin_sid)))
        member = w.BOOL()
        require(advapi.CheckTokenMembership(None, admin_sid, c.byref(member)))
        if not elevation.value and not member.value:
            with subprocess.Popen(list(map(str, command)), env=env, cwd=ROOT,
                                  creationflags=subprocess.CREATE_NO_WINDOW) as child:
                try:
                    if on_start:
                        on_start(kernel, w.HANDLE(int(child._handle)))
                    code = child.wait(timeout=timeout)
                except BaseException:
                    child.kill()
                    child.wait()
                    raise
            if code != expected_code:
                raise subprocess.CalledProcessError(code, command)
            return
        if advapi.GetTokenInformation(source, 19, c.byref(linked), c.sizeof(linked), c.byref(needed)):
            if not advapi.DuplicateTokenEx(linked, 0xF01FF, None, 2, 1, c.byref(token)):
                # A host can expose an identification-only linked token.
                if c.get_last_error() not in (5, 1314, 1346):
                    raise c.WinError(c.get_last_error())
        if not token:
            disabled = SID_ATTRIBUTES(admin_sid, 0)
            require(advapi.CreateRestrictedToken(source, 1, 1, c.byref(disabled), 0, None, 0, None, c.byref(token)))
        require(advapi.ConvertStringSidToSidW('S-1-16-8192', c.byref(medium_sid)))
        label = SID_ATTRIBUTES(medium_sid, 0x20)
        require(advapi.SetTokenInformation(token, 25, c.byref(label), c.sizeof(label)+advapi.GetLengthSid(medium_sid)))
        token_defaults(token)
        startup = STARTUP()
        startup.cb = c.sizeof(startup)
        startup.flags = 0x100
        startup.stdin = kernel.GetStdHandle(-10 & 0xFFFFFFFF)
        startup.stdout = kernel.GetStdHandle(-11 & 0xFFFFFFFF)
        startup.stderr = kernel.GetStdHandle(-12 & 0xFFFFFFFF)
        environment = c.create_unicode_buffer('\0'.join(f'{k}={v}' for k, v in sorted(env.items(), key=lambda item: item[0].upper()))+'\0\0')
        arguments = c.create_unicode_buffer(subprocess.list2cmdline(list(map(str, command))))
        with desktop_access(token) as desktop:
            startup.desktop = desktop
            require(advapi.CreateProcessAsUserW(token, str(command[0]), arguments, None, None, True, 0x08000400, environment, str(ROOT), c.byref(startup), c.byref(process)))
            if on_start:
                on_start(kernel, process.process)
            if kernel.WaitForSingleObject(process.process, int(timeout*1000)) != 0:
                kernel.TerminateProcess(process.process, 1)
                raise TimeoutError('Non-admin Windows native checks timed out')
            code = w.DWORD()
            require(kernel.GetExitCodeProcess(process.process, c.byref(code)))
            if code.value != expected_code:
                raise subprocess.CalledProcessError(code.value, command)
    finally:
        kernel.SetErrorMode(previous_error_mode)
        for handle in (process.thread, process.process, token, linked, source):
            if handle:
                kernel.CloseHandle(handle)
        for sid in (admin_sid, medium_sid):
            if sid:
                kernel.LocalFree(sid)


def windows_tests(tests, bundle):
    report_path = ROOT/'target/windows-ci-report.json'
    report = {'schema': 1, 'system': 'windows', 'arch': host_arch(),
              'status': 'running', 'suites': []}
    suites = [
        ('system-proxy-and-tun', [sys.executable, ROOT/'scripts/test-proxy-modes.py',
                                  '--test-binary', tests['core_integration'], '--bundle', bundle]),
    ]
    report['suites'] = [{'name': name, 'status': 'not_run'} for name, _ in suites]
    write_json(report_path, report)
    for entry, (_, command) in zip(report['suites'], suites):
        started = time.monotonic()
        entry['status'] = 'running'
        write_json(report_path, report)
        print('::group::Windows basic: ' + entry['name'], flush=True)
        try:
            run(command)
            entry['status'] = 'passed'
        except BaseException as error:
            entry['status'] = 'failed'
            entry['error'] = str(error)
            report['status'] = 'failed'
            raise
        finally:
            entry['duration_seconds'] = round(time.monotonic() - started, 3)
            write_json(report_path, report)
            print('::endgroup::', flush=True)
    report['status'] = 'passed'
    write_json(report_path, report)
    print('PASS: all Windows basic suites; report: ' + str(report_path), flush=True)
