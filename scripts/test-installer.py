"""Isolated Windows installer smoke tests using Win32/registry APIs directly."""
import ctypes
import argparse
from ctypes import wintypes
import os
from pathlib import Path
import subprocess
import uuid
import winreg
from build_support import ROOT, bundle_directory, file_version, host_arch, nsis, run, sha256, stage_resources, staging_directory, validate_resources

TEST_KEY = r'Software\Microsoft\Windows\CurrentVersion\Uninstall\ClashOfRustInstallerSmokeTest'
LEGACY_KEY = r'Software\Microsoft\Windows\CurrentVersion\Uninstall\ClashOfRustLegacySmokeTest'
MUTEX = r'Local\ClashOfRust.InstallerSmoke'
EVENT = r'Local\ClashOfRust.InstallerSmoke.Exit'
kernel = ctypes.WinDLL('kernel32',use_last_error=True)
kernel.CreateMutexW.argtypes = [ctypes.c_void_p,wintypes.BOOL,wintypes.LPCWSTR]
kernel.CreateMutexW.restype = wintypes.HANDLE
kernel.CreateEventW.argtypes = [ctypes.c_void_p,wintypes.BOOL,wintypes.BOOL,wintypes.LPCWSTR]
kernel.CreateEventW.restype = wintypes.HANDLE
kernel.CloseHandle.argtypes = [wintypes.HANDLE]
kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE,wintypes.DWORD]
kernel.WaitForSingleObject.restype = wintypes.DWORD


def key_exists(path):
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER,path,0,winreg.KEY_READ|winreg.KEY_WOW64_64KEY):
            return True
    except FileNotFoundError:
        return False


def registry_value(path,name):
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER,path,0,winreg.KEY_READ|winreg.KEY_WOW64_64KEY) as key:
        return winreg.QueryValueEx(key,name)[0]


class Handle:
    def __init__(self,kind,name):
        self.handle = kernel.CreateMutexW(None,False,name) if kind == 'mutex' else kernel.CreateEventW(None,False,False,name)
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        if ctypes.get_last_error() == 183:
            self.close()
            raise RuntimeError('Existing test synchronization object; refusing to interfere')
    def close(self):
        if self.handle:
            kernel.CloseHandle(self.handle)
            self.handle = None
    def __enter__(self):
        return self
    def __exit__(self,*_):
        self.close()
    def wait(self,timeout=10000):
        if kernel.WaitForSingleObject(self.handle,timeout) != 0:
            raise RuntimeError('Installer did not send cooperative exit event')


def start(executable,args):
    return subprocess.Popen([str(executable),*map(str,args)], creationflags=subprocess.CREATE_NO_WINDOW)


def expect(executable,args,code,timeout=60):
    with start(executable,args) as process:
        actual = process.wait(timeout=timeout)
    if actual != code:
        raise RuntimeError(f'{executable.name}: expected exit {code}, got {actual}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--old-executable',type=Path,help='Optional real lower-version GUI executable for an upgrade fixture')
    args = parser.parse_args()
    if os.name != 'nt':
        raise RuntimeError('Installer smoke tests require Windows')
    if key_exists(TEST_KEY) or key_exists(LEGACY_KEY):
        raise RuntimeError('Existing smoke-test registration; refusing to overwrite it')
    arch = host_arch()
    bundle = bundle_directory('windows',arch)
    binary = bundle/'clash-of-rust.exe'
    version = file_version(binary)
    root = ROOT/'dist'/('installer-test-'+uuid.uuid4().hex)
    root.mkdir(parents=True)
    installed, setup = root/'installed', root/'smoke-setup.exe'
    compiler = nsis()
    with staging_directory('installer-test') as stage:
        stage_resources(bundle/'resources',stage,'windows')
        import shutil
        shutil.copy2(binary,stage/binary.name)
        run([compiler,'/V2','/INPUTCHARSET','UTF8','/DINSTALLER_TESTING',f'/DAPP_VERSION={version}',f'/DAPP_ARCH={arch}',f'/DPAYLOAD={stage}',f'/DOUTPUT={setup}',ROOT/'installer/clash-of-rust.nsi'])
        normal = ['/S',f'/D={installed}']
        reinstall = ['/S','/TESTREINSTALL',f'/D={installed}']
        update = ['/S','/UPDATE',f'/D={installed}']
        expect(setup,update,2)
        with Handle('mutex',MUTEX):
            expect(setup,normal,3)
        expect(setup,normal,0)
        assert registry_value(TEST_KEY,'DisplayVersion') == version
        assert file_version(installed/'clash-of-rust.exe') == version
        assert sha256(installed/'clash-of-rust.exe') == sha256(binary)
        for name in ('GeoIP.dat','GeoSite.dat','Country.mmdb','ASN.mmdb','mihomo.exe','default.yaml','settings-defaults.json','geodata.json'):
            assert (installed/'resources'/name).is_file(), name
        validate_resources(installed/'resources','windows')
        ip = installed/'resources/ip-check'
        assert {p.name for p in ip.iterdir()} == {'LICENSE','SOURCE.md'}, 'Unexpected script/runtime in payload'
        expect(setup,normal,2)
        user_file = installed/'user-file.txt'
        user_file.write_text('Must survive uninstall',encoding='utf-8')
        with Handle('mutex',MUTEX) as mutex, Handle('event',EVENT) as event:
            with start(setup,update) as process:
                event.wait()
                mutex.close()
                assert process.wait(timeout=30) == 0
        assert user_file.is_file() and registry_value(TEST_KEY,'DisplayVersion') == version
        assert sha256(installed/'clash-of-rust.exe') == sha256(binary)
        assert not (installed/'clash-of-rust.update-backup.exe').exists()
        with Handle('mutex',MUTEX), Handle('event',EVENT) as event:
            with start(setup,update) as process:
                event.wait()
                assert process.wait(timeout=25) == 5
        assert (installed/'clash-of-rust.exe').is_file() and user_file.is_file()
        # Force a payload write failure after the running app has been backed up.
        license_file = installed/'LICENSE'
        saved_license = installed/'user-license-backup.txt'
        license_file.rename(saved_license)
        license_file.mkdir()
        expect(setup,update,6)
        assert sha256(installed/'clash-of-rust.exe') == sha256(binary), 'Failed update did not restore original executable'
        assert not (installed/'clash-of-rust.update-backup.exe').exists()
        license_file.rmdir()
        saved_license.rename(license_file)
        with Handle('mutex',MUTEX):
            expect(setup,reinstall,5)
            assert (installed/'clash-of-rust.exe').is_file()
        with Handle('mutex',MUTEX) as mutex, Handle('event',EVENT) as event:
            with start(setup,reinstall) as process:
                event.wait()
                mutex.close()
                assert process.wait(timeout=30) == 0
        with Handle('mutex',MUTEX), Handle('event',EVENT) as event:
            with start(setup,reinstall) as process:
                event.wait()
                assert process.wait(timeout=25) == 5
                assert (installed/'clash-of-rust.exe').is_file()
        expect(setup,reinstall,0)
        assert user_file.is_file()
        uninstaller = installed/'uninstall.exe'
        expect(uninstaller,['/S',f'_?={installed}'],0)
        assert not (installed/'clash-of-rust.exe').exists() and not key_exists(TEST_KEY)
        assert user_file.is_file()
        legacy_setup, legacy_install = root/'legacy-setup.exe', root/'old-user-install'
        run([compiler,'/V2','/INPUTCHARSET','UTF8',f'/DPAYLOAD={stage}',f'/DOUTPUT={legacy_setup}',ROOT/'installer/legacy-test.nsi'])
        expect(legacy_setup,['/S',f'/D={legacy_install}'],0)
        legacy_user = legacy_install/'user-file.txt'
        legacy_user.write_text('Must survive migration',encoding='utf-8')
        expect(setup,reinstall,0)
        assert not key_exists(LEGACY_KEY) and not (legacy_install/'clash-of-rust.exe').exists()
        assert legacy_user.is_file() and (installed/'clash-of-rust.exe').is_file()
        expect(uninstaller,['/S',f'_?={installed}'],0)
        assert not key_exists(TEST_KEY)
        if args.old_executable:
            old_binary = args.old_executable.resolve(strict=True)
            old_version = file_version(old_binary)
            old_setup = root/'old-version-setup.exe'
            shutil.copy2(old_binary,stage/binary.name)
            run([compiler,'/V2','/INPUTCHARSET','UTF8','/DINSTALLER_TESTING',f'/DAPP_VERSION={old_version}',f'/DAPP_ARCH={arch}',f'/DPAYLOAD={stage}',f'/DOUTPUT={old_setup}',ROOT/'installer/clash-of-rust.nsi'])
            expect(old_setup,normal,0)
            assert registry_value(TEST_KEY,'DisplayVersion') == old_version
            assert file_version(installed/'clash-of-rust.exe') == old_version
            with Handle('mutex',MUTEX) as mutex, Handle('event',EVENT) as event:
                with start(setup,update) as process:
                    event.wait()
                    mutex.close()
                    assert process.wait(timeout=30) == 0
            assert file_version(installed/'clash-of-rust.exe') == version
            assert registry_value(TEST_KEY,'DisplayVersion') == version
            assert sha256(installed/'clash-of-rust.exe') == sha256(binary)
            assert user_file.is_file()
            expect(uninstaller,['/S',f'_?={installed}'],0)
            assert not key_exists(TEST_KEY)
            print(f'PASS: native automatic upgrade {old_version} -> {version}')
    print('PASS: fresh install, native-only payload, duplicate refusal, automatic update, cooperative exit, refusal and timeout, reinstall, legacy migration, uninstall, unrelated user file preservation')
    print('Test artifacts:',root)


if __name__ == '__main__':
    main()
