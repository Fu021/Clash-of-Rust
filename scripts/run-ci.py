"""Compile/test one native platform; preserve release binaries for packaging."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

from build_support import host_system, ROOT, bundle_directory, host_arch, run, cargo_messages, write_json
from workflow_artifacts import export_build, test_binaries


def clear_test_reports():
    # The Rust target cache may contain previous runs' diagnostics, too.
    for name in ('claude-ui', 'claude-report', 'linux-wayland-report', 'linux-x11-logs', 'linux-ui'):
        path = ROOT/'target'/name
        if path.is_symlink():
            path.unlink()
        elif path.exists():
            shutil.rmtree(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--system', choices=('windows', 'linux', 'macos'), required=True)
    parser.add_argument('--arch', choices=('x64', 'arm64'), required=True)
    args = parser.parse_args()
    if (args.system, args.arch) != (host_system(), host_arch()):
        raise ValueError('CI must run on the selected native architecture')
    # Rust's target cache can also restore old diagnostic files. Keep compiled
    # dependencies, but generate reports from this commit afresh.
    clear_test_reports()
    run(['cargo', 'fmt', '--all', '--', '--check'])
    if (args.system, args.arch) == ('linux', 'x64'):
        run([sys.executable, ROOT/'scripts/native-detector-codegen.py', '--check'])
        run([sys.executable, ROOT/'scripts/test-build-tools.py'])
        run([sys.executable, ROOT/'scripts/test-workflow-artifacts.py'])
    # One profile across Clippy, tests and production builds avoids a second
    # dependency compilation and produces the exact binaries released later.
    run(['cargo', 'clippy', '--release', '--all-targets', '--locked', '--', '-D', 'warnings'])
    run(['cargo', 'build', '--release', '--locked', '--bins'])
    messages = cargo_messages(['cargo', 'test', '--release', '--locked', '--all-targets',
                              '--no-run', '--message-format=json'])
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
    # Focus native checks on the core lifecycle and real proxy/TUN traffic.
    if args.system == 'macos':
        from macos_ci import native_tests
        native_tests(tests, bundle)
    elif args.system == 'windows':
        from windows_ci import windows_tests
        windows_tests(tests, bundle)
    else:
        run([sys.executable, ROOT/'scripts/test-proxy-modes.py',
             '--test-binary', tests['core_integration'], '--bundle', bundle])
    write_json(ROOT/'target/ci-tests.json', tests)
    # Export the exact production binaries tested above.
    export_build(args.system, args.arch, tests, ROOT/'dist/build', binaries=bundle)


if __name__ == '__main__':
    main()
