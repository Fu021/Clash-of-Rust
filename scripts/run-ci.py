"""Compile/test one native platform; preserve release binaries for packaging."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

from build_support import ROOT, bundle_directory, host_arch, run, write_json
from workflow_artifacts import export_build, test_binaries


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--system', choices=('windows', 'linux'), required=True)
    parser.add_argument('--arch', choices=('x64', 'arm64'), required=True)
    args = parser.parse_args()
    if (args.system, args.arch) != ('windows' if os.name == 'nt' else 'linux', host_arch()):
        raise ValueError('CI must run on the selected native architecture')
    run(['cargo', 'fmt', '--all', '--', '--check'])
    if (args.system, args.arch) == ('linux', 'x64'):
        run([sys.executable, ROOT/'scripts/native-detector-codegen.py', '--check'])
        run([sys.executable, ROOT/'scripts/test-build-tools.py'])
        run([sys.executable, ROOT/'scripts/test-workflow-artifacts.py'])
    # One profile across Clippy, tests and production builds avoids a second
    # dependency compilation and produces the exact binaries released later.
    run(['cargo', 'clippy', '--release', '--all-targets', '--locked', '--', '-D', 'warnings'])
    run(['cargo', 'build', '--release', '--locked', '--bins'])
    messages = subprocess.check_output(['cargo', 'test', '--release', '--locked', '--all-targets',
                                        '--no-run', '--message-format=json'], cwd=ROOT, text=True)
    tests = test_binaries(messages)
    run(['cargo', 'test', '--release', '--locked', '--all-targets', '--', '--test-threads=1'])
    spec = importlib.util.spec_from_file_location('prepare', ROOT/'scripts/prepare-resources.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.prepare(args.system, args.arch)
    bundle = bundle_directory(args.system, args.arch)
    extension = '.exe' if args.system == 'windows' else ''
    for name in ('clash-of-rust', 'clash-tun-launcher'):
        shutil.copy2(ROOT/'target/release'/(name+extension), bundle/(name+extension))
    # Invoke already-built test programs directly instead of repeating Cargo.
    run([tests['core_integration'], '--ignored', '--test-threads=1'])
    if args.system == 'linux':
        run([tests['library'], 'native_gsettings', '--ignored'],
            env=dict(os.environ, GSETTINGS_BACKEND='memory'))
        for name in ('native_kde_signal', 'no_tray_host'):
            run(['dbus-run-session', '--', tests['library'], name, '--ignored'])
        run(['dbus-run-session', '--', tests['tray_integration'], 'login_tray', '--ignored'])
    else:
        run([tests['tray_integration'], '--ignored'])
        run([tests['library'], 'native_autostart', '--ignored'])
        run([sys.executable, ROOT/'scripts/verify-icon-resources.py', bundle/'clash-of-rust.exe'])
        run([sys.executable, ROOT/'scripts/test-update-helper.py', bundle/'clash-of-rust.exe'])
        run([sys.executable, ROOT/'scripts/test-windows-startup.py', bundle/'clash-of-rust.exe',
             '--test-binary', tests['library']])
        run([sys.executable, ROOT/'scripts/test-build-tools.py',
             'BuildTests.test_windows_pe_version_is_read_without_executing_binary'])
    write_json(ROOT/'target/ci-tests.json', tests)
    export_build(args.system, args.arch, tests, ROOT/'dist/build')


if __name__ == '__main__':
    main()
