"""Verify the final Windows package, native proxy/TUN and uninstall on Actions."""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import winreg
from build_support import ROOT, bundle_directory, file_version, host_arch, run, sha256, validate_resources


def start(executable,args):
    return subprocess.Popen([str(executable),*map(str,args)], creationflags=subprocess.CREATE_NO_WINDOW)


def expect(executable,args,code,timeout=60):
    with start(executable,args) as process:
        actual = process.wait(timeout=timeout)
    if actual != code:
        raise RuntimeError(f'{executable.name}: expected exit {code}, got {actual}')


def production_package(package, binary, root, smoke):
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('Production package tests require a disposable Actions runner')
    key = r'Software\Microsoft\Windows\CurrentVersion\Uninstall\ClashOfRust'
    for hive in (winreg.HKEY_LOCAL_MACHINE, winreg.HKEY_CURRENT_USER):
        try:
            with winreg.OpenKey(hive, key, 0, winreg.KEY_READ | winreg.KEY_WOW64_64KEY):
                raise RuntimeError('Existing production installation; refusing to replace it')
        except FileNotFoundError:
            pass
    installed = root/'production-installed'
    expect(package, ['/S', f'/D={installed}'], 0)
    try:
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, key, 0, winreg.KEY_READ | winreg.KEY_WOW64_64KEY) as registration:
            assert winreg.QueryValueEx(registration, 'DisplayVersion')[0] == file_version(binary)
        assert sha256(installed/'clash-of-rust.exe') == sha256(binary)
        validate_resources(installed/'resources', 'windows')
        run([sys.executable, ROOT/'scripts/test-proxy-modes.py',
             '--test-binary', smoke, '--bundle', installed])
        user_file = installed/'unrelated-user-file.txt'
        user_file.write_text('Preserve this unrelated file', encoding='utf-8')
    finally:
        uninstaller = installed/'uninstall.exe'
        if uninstaller.is_file():
            expect(uninstaller, ['/S', f'_?={installed}'], 0)
    assert not (installed/'clash-of-rust.exe').exists()
    assert user_file.is_file()
    try:
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, key, 0, winreg.KEY_READ | winreg.KEY_WOW64_64KEY):
            raise AssertionError('Production registration survived uninstall')
    except FileNotFoundError:
        pass
    print('PASS: final production package installation, proxy/TUN and uninstall')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--package', type=Path, required=True)
    parser.add_argument('--test-binary', type=Path, required=True)
    args = parser.parse_args()
    if os.name != 'nt':
        raise RuntimeError('Installer smoke tests require Windows')
    binary = bundle_directory('windows', host_arch())/'clash-of-rust.exe'
    (ROOT/'dist').mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='installer-test-', dir=ROOT/'dist') as temporary:
        production_package(args.package.resolve(strict=True), binary, Path(temporary),
                           args.test_binary.resolve(strict=True))


if __name__ == '__main__':
    main()
