"""Named Windows basic suites, with a failure-preserving CI receipt."""
import sys
import time
from build_support import ROOT, host_arch, run, write_json


def run_unelevated(command, env, timeout=240, on_start=None, expected_code=0):
    """Run native checks with an actual medium-integrity, non-admin token."""
    import ctypes as c
    from ctypes import wintypes as w
    import subprocess
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
    kernel.LocalFree.argtypes = [w.LPVOID]
    advapi.OpenProcessToken.argtypes = [w.HANDLE, w.DWORD, c.POINTER(w.HANDLE)]
    advapi.GetTokenInformation.argtypes = [w.HANDLE, c.c_int, w.LPVOID, w.DWORD, c.POINTER(w.DWORD)]
    advapi.DuplicateTokenEx.argtypes = [w.HANDLE, w.DWORD, w.LPVOID, c.c_int, c.c_int, c.POINTER(w.HANDLE)]
    advapi.CreateRestrictedToken.argtypes = [w.HANDLE, w.DWORD, w.DWORD, c.POINTER(SID_ATTRIBUTES), w.DWORD, w.LPVOID, w.DWORD, w.LPVOID, c.POINTER(w.HANDLE)]
    advapi.ConvertStringSidToSidW.argtypes = [w.LPCWSTR, c.POINTER(w.LPVOID)]
    advapi.SetTokenInformation.argtypes = [w.HANDLE, c.c_int, w.LPVOID, w.DWORD]
    advapi.CreateProcessAsUserW.argtypes = [w.HANDLE, w.LPCWSTR, w.LPWSTR, w.LPVOID, w.LPVOID, w.BOOL, w.DWORD, w.LPVOID, w.LPCWSTR, c.POINTER(STARTUP), c.POINTER(PROCESS)]
    def require(ok):
        if not ok:
            raise c.WinError(c.get_last_error())
    source, linked, token = w.HANDLE(), w.HANDLE(), w.HANDLE()
    process = PROCESS()
    admin_sid, medium_sid = w.LPVOID(), w.LPVOID()
    try:
        require(advapi.OpenProcessToken(kernel.GetCurrentProcess(), 0xF01FF, c.byref(source)))
        needed = w.DWORD()
        if advapi.GetTokenInformation(source, 19, c.byref(linked), c.sizeof(linked), c.byref(needed)):
            require(advapi.DuplicateTokenEx(linked, 0xF01FF, None, 2, 1, c.byref(token)))
        else:
            require(advapi.ConvertStringSidToSidW('S-1-5-32-544', c.byref(admin_sid)))
            disabled = SID_ATTRIBUTES(admin_sid, 0)
            require(advapi.CreateRestrictedToken(source, 1, 1, c.byref(disabled), 0, None, 0, None, c.byref(token)))
        require(advapi.ConvertStringSidToSidW('S-1-16-8192', c.byref(medium_sid)))
        label = SID_ATTRIBUTES(medium_sid, 0x20)
        require(advapi.SetTokenInformation(token, 25, c.byref(label), c.sizeof(label)))
        startup = STARTUP()
        startup.cb = c.sizeof(startup)
        startup.flags = 0x100
        startup.stdin = kernel.GetStdHandle(-10 & 0xFFFFFFFF)
        startup.stdout = kernel.GetStdHandle(-11 & 0xFFFFFFFF)
        startup.stderr = kernel.GetStdHandle(-12 & 0xFFFFFFFF)
        environment = c.create_unicode_buffer('\0'.join(f'{k}={v}' for k, v in sorted(env.items(), key=lambda item: item[0].upper()))+'\0\0')
        arguments = c.create_unicode_buffer(subprocess.list2cmdline(list(map(str, command))))
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
        ('python-build-tools', [sys.executable, ROOT/'scripts/test-build-tools.py']),
        ('artifact-protocol', [sys.executable, ROOT/'scripts/test-workflow-artifacts.py']),
        ('system-proxy-and-tun', [sys.executable, ROOT/'scripts/test-proxy-modes.py',
                                  '--test-binary', tests['core_integration'], '--bundle', bundle]),
        ('embedded-icons', [sys.executable, ROOT/'scripts/prepare-assets.py', 'verify-icons',
                            bundle/'clash-of-rust.exe']),
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
