"""Windows CI, installed-package checks and Defender verification on Actions."""
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time
from build_support import ROOT, file_version, host_arch, run, sha256, validate_resources, write_json


def allow_ordinary_user_directory(path, permissions='M'):
    """Share only a test-owned fixture with the runner's unelevated token."""
    import csv
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
            try:
                if on_start:
                    on_start(kernel, process.process)
                if kernel.WaitForSingleObject(process.process, int(timeout*1000)) != 0:
                    raise TimeoutError('Non-admin Windows native checks timed out')
            except BaseException:
                kernel.TerminateProcess(process.process, 1)
                kernel.WaitForSingleObject(process.process, 10000)
                raise
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


def _installer_exit(executable,args,code,timeout=60):
    with subprocess.Popen([str(executable),*map(str,args)],
                          creationflags=subprocess.CREATE_NO_WINDOW) as process:
        actual = process.wait(timeout=timeout)
    if actual != code:
        raise RuntimeError(f'{executable.name}: expected exit {code}, got {actual}')


def _verify_failed_update(package, installed):
    """Fail after extraction and require the original owner/renderer pair back."""
    import ctypes as c
    from ctypes import wintypes as w
    kernel = c.WinDLL('kernel32', use_last_error=True)
    kernel.CreateFileW.argtypes = [w.LPCWSTR,w.DWORD,w.DWORD,w.LPVOID,w.DWORD,w.DWORD,w.HANDLE]
    kernel.CreateFileW.restype = w.HANDLE
    kernel.CloseHandle.argtypes = [w.HANDLE]
    renderer = installed/'clash-desktop-ui.exe'
    original = renderer.read_bytes()
    owner_hash = sha256(installed/'clash-of-rust.exe')
    for legacy in (False, True):
        # A harmless PE overlay distinguishes the old renderer from the
        # identical-version installer. Neither probe renderer is executed.
        if legacy: renderer.unlink()
        else: renderer.write_bytes(original+b'installer-rollback-probe')
        expected = None if legacy else sha256(renderer)
        handle = kernel.CreateFileW(str(installed/'uninstall.exe'),0x80000000,1,None,3,0,None)
        if handle == c.c_void_p(-1).value:
            renderer.write_bytes(original)
            raise c.WinError(c.get_last_error())
        try:
            # Read sharing permits verification, but denies WriteUninstaller.
            # Extraction has already replaced both executables at this point.
            _installer_exit(package,['/S','/UPDATE',f'/D={installed}'],6)
            assert sha256(installed/'clash-of-rust.exe') == owner_hash
            if legacy: assert not renderer.exists()
            else: assert sha256(renderer) == expected
            assert not list(installed.glob('*.update-backup.exe'))
        finally:
            kernel.CloseHandle(handle)
            renderer.write_bytes(original)


def installed_gui_smoke(installed, root):
    """Check the installed window and shared CLI with a non-admin token."""
    import ctypes as c
    from ctypes import wintypes as w
    user32 = c.WinDLL('user32', use_last_error=True)
    callback = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    user32.EnumWindows.argtypes = [callback, w.LPARAM]
    user32.EnumWindows.restype = w.BOOL
    user32.IsWindowVisible.argtypes = [w.HWND]
    user32.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, c.c_int]
    user32.GetWindowThreadProcessId.argtypes = [w.HWND, c.POINTER(w.DWORD)]
    user32.GetWindowRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
    user32.ShowWindow.argtypes = [w.HWND, c.c_int]
    profile = root/'gui-profile'
    profile.mkdir()
    allow_ordinary_user_directory(profile)
    env = dict(os.environ, CLASH_OF_RUST_DATA_DIR=str(profile), PYTHONUTF8='1')
    command = [installed/'cor.exe', '--data-dir', profile, '--json']

    def cli(*arguments):
        result = subprocess.run([*map(str, command), *arguments], env=env,
                                capture_output=True, text=True, encoding='utf-8', timeout=30, check=True)
        return json.loads(result.stdout)['data']

    def inspect(kernel, process):
        kernel.GetProcessId.argtypes = [w.HANDLE]
        kernel.GetProcessId.restype = w.DWORD
        owner = kernel.GetProcessId(process)
        try:
            deadline = time.monotonic()+60
            selected = None
            while time.monotonic() < deadline:
                if kernel.WaitForSingleObject(process, 0) == 0:
                    raise RuntimeError('Installed Windows GUI exited before showing a window')
                found = []
                @callback
                def visit(hwnd, _):
                    title = c.create_unicode_buffer(512)
                    if user32.IsWindowVisible(hwnd) and user32.GetWindowTextW(hwnd, title, len(title)):
                        if title.value.startswith('Clash of Rust'):
                            pid, rect = w.DWORD(), w.RECT()
                            user32.GetWindowThreadProcessId(hwnd, c.byref(pid))
                            if user32.GetWindowRect(hwnd, c.byref(rect)):
                                found.append((hwnd, pid.value, rect.right-rect.left, rect.bottom-rect.top))
                    return True
                if not user32.EnumWindows(visit, 0):
                    raise c.WinError(c.get_last_error())
                if found:
                    selected = found[0]
                    break
                time.sleep(.25)
            if not selected:
                raise TimeoutError('Installed Windows GUI did not show a window')
            hwnd, renderer, width, height = selected
            parent = subprocess.check_output(
                ['powershell', '-NoProfile', '-NonInteractive', '-Command',
                 f"(Get-CimInstance Win32_Process -Filter 'ProcessId = {renderer}').ParentProcessId"],
                text=True, timeout=30).strip()
            if parent != str(owner) or width < 800 or height < 450:
                raise RuntimeError('Visible window is not the installed owner\'s usable renderer')
            deadline = time.monotonic()+45
            while True:
                before = cli('status')
                if before.get('owner') == 'gui' and before.get('running'):
                    break
                if time.monotonic() >= deadline:
                    raise TimeoutError('Windows GUI did not start its shared core')
                time.sleep(.5)
            cli('mode', 'global')
            user32.ShowWindow(hwnd, 6)  # Minimize, then restore the real window.
            user32.ShowWindow(hwnd, 9)
            deadline = time.monotonic()+15
            while True:
                after = cli('status')
                if (user32.IsWindowVisible(hwnd) and after.get('running')
                        and after.get('mode') == 'global' and after.get('owner') == 'gui'):
                    break
                if time.monotonic() >= deadline:
                    raise RuntimeError('Window restore lost the GUI-owned CLI/core state')
                time.sleep(.25)
            write_json(ROOT/'target/windows-gui-report.json',
                       {'status': 'passed', 'ordinary_user': True, 'visible_window': True,
                        'window_restore': True, 'shared_cli_mode': True})
        finally:
            try:
                cli('stop')
            finally:
                # Use the same GUI exit lease as the installer/update path.
                session = profile/'gui-session.json'
                if session.is_file():
                    (profile/'gui-exit').write_text(json.loads(session.read_text())['nonce'])

    run_unelevated([installed/'clash-of-rust.exe'], env, timeout=120, on_start=inspect)


def _verify_installed_package(package, binary, root, smoke):
    import winreg
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('Production package tests require a disposable Actions runner')
    key = r'Software\Microsoft\Windows\CurrentVersion\Uninstall\ClashOfRust'
    for hive in (winreg.HKEY_LOCAL_MACHINE, winreg.HKEY_CURRENT_USER):
        try:
            with winreg.OpenKey(hive, key, 0, winreg.KEY_READ | winreg.KEY_WOW64_64KEY):
                raise RuntimeError('Existing production installation; refusing to replace it')
        except FileNotFoundError:
            pass
    # Python 3.13's private temporary directory can be owned by the elevated
    # runner's Administrators group. The ordinary test token must traverse it
    # and read/execute the installed application, without granting write access
    # or changing the production service directory's protected ACL.
    allow_ordinary_user_directory(root, permissions='RX')
    installed = root/'production-installed'
    _installer_exit(package, ['/S', f'/D={installed}'], 0)
    try:
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, key, 0, winreg.KEY_READ | winreg.KEY_WOW64_64KEY) as registration:
            assert winreg.QueryValueEx(registration, 'DisplayVersion')[0] == file_version(binary)
        assert sha256(installed/'clash-of-rust.exe') == sha256(binary)
        assert sha256(installed/'clash-desktop-ui.exe') == sha256(binary.with_name('clash-desktop-ui.exe'))
        validate_resources(installed/'resources', 'windows')
        assert (installed/'clash-tun-launcher.exe').is_file()
        run(['sc.exe', 'query', 'ClashOfRust.Network'])
        installed_gui_smoke(installed, root)
        run([sys.executable, ROOT/'scripts/test-proxy-modes.py',
             '--test-binary', smoke, '--bundle', installed])
        user_file = installed/'unrelated-user-file.txt'
        user_file.write_text('Preserve this unrelated file', encoding='utf-8')
        _verify_failed_update(package, installed)
        # Exercise the same explicit update mode used by the client. Service
        # replacement must finish before installation reports success.
        _installer_exit(package, ['/S', '/UPDATE', f'/D={installed}'], 0)
        assert sha256(installed/'clash-desktop-ui.exe') == sha256(binary.with_name('clash-desktop-ui.exe'))
        assert not list(installed.glob('*.update-backup.exe'))
        assert user_file.read_text(encoding='utf-8') == 'Preserve this unrelated file'
        run([sys.executable, ROOT/'scripts/test-proxy-modes.py',
             '--test-binary', smoke, '--bundle', installed])
    finally:
        uninstaller = installed/'uninstall.exe'
        if uninstaller.is_file():
            _installer_exit(uninstaller, ['/S', f'_?={installed}'], 0)
    assert not (installed/'clash-of-rust.exe').exists()
    assert not (installed/'clash-desktop-ui.exe').exists()
    assert not (installed/'clash-tun-launcher.exe').exists()
    assert subprocess.run(['sc.exe', 'query', 'ClashOfRust.Network'], capture_output=True).returncode == 1060
    assert user_file.is_file()
    try:
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, key, 0, winreg.KEY_READ | winreg.KEY_WOW64_64KEY):
            raise AssertionError('Production registration survived uninstall')
    except FileNotFoundError:
        pass
    print('PASS: final production package installation, proxy/TUN and uninstall')


def _cleanup_installer(temporary):
    # NSIS/Defender can briefly retain the executable after the process exits,
    # especially under Windows ARM emulation. Persistent failures still fail.
    deadline = time.monotonic() + 10
    while True:
        try:
            temporary.cleanup()
            return
        except PermissionError:
            if time.monotonic() >= deadline:
                raise
            time.sleep(0.1)


def defender_command():
    if os.name != 'nt':
        raise RuntimeError('Windows Defender scans require Windows')
    platform = Path(os.environ['ProgramData'])/'Microsoft/Windows Defender/Platform'
    candidates = sorted(
        platform.glob('*/MpCmdRun.exe'),
        key=lambda path: tuple(int(n) for n in re.findall(r'\d+', path.parent.name)),
        reverse=True,
    )
    candidates.append(Path(os.environ['ProgramFiles'])/'Windows Defender/MpCmdRun.exe')
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    raise RuntimeError('Windows Defender MpCmdRun.exe is unavailable; scan cannot be skipped')


def update_signatures(timeout=300):
    deadline = time.monotonic() + timeout
    # MMPC is Microsoft's direct security-intelligence source. Avoid the
    # runner's Windows Update queue, and wait only for its explicit MSI-busy error.
    for delay in (15, 30, 45, 60, None):
        command = [str(defender_command()), '-SignatureUpdate', '-MMPC']
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise subprocess.TimeoutExpired(command, timeout)
        print('Updating Defender security intelligence:', command[0], flush=True)
        result = subprocess.run(
            command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, encoding='utf-8', errors='replace', timeout=remaining,
        )
        output = result.stdout or ''
        if output:
            print(output, end='' if output.endswith('\n') else '\n', flush=True)
        if result.returncode == 0:
            # The platform itself may have upgraded while we waited.
            return defender_command()
        busy = re.search(r'\b0x80070652\b', output.replace('\0', ''), re.IGNORECASE)
        if not busy or delay is None:
            raise subprocess.CalledProcessError(result.returncode, command, output=output)
        remaining = deadline - time.monotonic()
        if remaining <= delay:
            raise subprocess.TimeoutExpired(command, timeout, output=output)
        print(f'Defender installation is busy; retrying in {delay}s', flush=True)
        time.sleep(delay)


def defender_versions():
    command = [
        'powershell.exe', '-NoProfile', '-NonInteractive', '-Command',
        "$ErrorActionPreference = 'Stop'; Get-MpComputerStatus | "
        'Select-Object AMEngineVersion, AMProductVersion, AntivirusSignatureVersion | ConvertTo-Json -Compress',
    ]
    for attempt in range(2):
        try:
            status = subprocess.run(command, check=True, capture_output=True,
                                    text=True, encoding='utf-8', timeout=120)
            break
        except subprocess.TimeoutExpired:
            if attempt:
                raise
            print('Defender status timed out; retrying once', flush=True)
            time.sleep(5)
    versions = json.loads(status.stdout)
    if not isinstance(versions, dict) or any(
        not versions.get(field)
        for field in ('AMEngineVersion', 'AMProductVersion', 'AntivirusSignatureVersion')
    ):
        raise RuntimeError('Windows Defender version information is unavailable')
    return versions


def scan_windows_files(files):
    files = [path.resolve(strict=True) for path in files]
    if not files or any(not path.is_file() or path.suffix.lower() not in ('.exe', '.dll') for path in files):
        raise ValueError('Expected Windows .exe or .dll files')
    command = update_signatures()
    print('Windows Defender:', command, flush=True)
    versions = defender_versions()
    print('Windows Defender versions:', json.dumps(versions, sort_keys=True), flush=True)
    records = []
    for path in files:
        digest = sha256(path)
        print('Scanning:', path, flush=True)
        # A custom scan ignores exclusions and scans archives. Keeping detected
        # files unmodified makes a threat return nonzero and blocks CI upload.
        subprocess.run([
            str(command), '-Scan', '-ScanType', '3', '-File', str(path),
            '-DisableRemediation',
        ], check=True, timeout=300)
        if sha256(path) != digest:
            raise RuntimeError('File changed during Defender scan: '+path.name)
        records.append({
            'file': path.relative_to(ROOT).as_posix() if path.is_relative_to(ROOT) else path.name,
            'sha256': digest,
        })
        print('PASS: Defender scan completed:', path.name, digest, flush=True)
    return {
        'scanned_at': datetime.now(timezone.utc).isoformat(),
        'versions': versions, 'files': records,
    }


def package_tests(package, manifest, build):
    """Scan unpacked programs and the installer, then verify installed behavior."""
    if os.name != 'nt':
        raise RuntimeError('Installer tests require Windows')
    payload_files = sorted(
        path for path in (build/'payload').rglob('*')
        if path.is_file() and path.suffix.lower() in ('.exe', '.dll')
    )
    if not payload_files:
        raise RuntimeError('Windows package payload contains no programs to scan')
    defender = scan_windows_files([package, *payload_files])
    (ROOT/'dist').mkdir(exist_ok=True)
    temporary = tempfile.TemporaryDirectory(prefix='installer-test-', dir=ROOT/'dist')
    try:
        _verify_installed_package(package.resolve(strict=True), build/'payload/clash-of-rust.exe',
                           Path(temporary.name),
                           (build/manifest['tests']['core_integration']).resolve(strict=True))
    finally:
        _cleanup_installer(temporary)
    return defender
