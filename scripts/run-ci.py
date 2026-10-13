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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--system', choices=('windows', 'linux', 'macos'), required=True)
    parser.add_argument('--arch', choices=('x64', 'arm64'), required=True)
    args = parser.parse_args()
    if (args.system, args.arch) != (host_system(), host_arch()):
        raise ValueError('CI must run on the selected native architecture')
    # Rust's target cache can also restore old diagnostic files. Keep compiled
    # dependencies, but generate reports/screenshots from this commit afresh.
    for name in ('claude-ui', 'claude-report'):
        path = ROOT/'target'/name
        if path.is_symlink():
            path.unlink()
        elif path.exists():
            shutil.rmtree(path)
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
    run([sys.executable, ROOT/'scripts/test-claude-diagnostics.py',
         '--executable', bundle/('clash-of-rust'+extension), '--output', ROOT/'target/claude-report/terminal'])
    if (args.system, args.arch) == ('linux', 'x64'):
        run([sys.executable, '-m', 'pip', 'install', '--disable-pip-version-check', 'playwright==1.55.0'])
        run([sys.executable, ROOT/'scripts/test-claude-diagnostics.py', '--browser', '--output', ROOT/'target/claude-report/browser'])
        scenes = [scene+suffix for scene in ('home', 'proxies-ascending', 'profiles', 'connections', 'rules', 'logs', 'diagnostics', 'ip-summary', 'settings',
                  'claude', 'claude-web', 'claude-desktop', 'claude-code', 'claude-details', 'claude-empty',
                  'profiles-edit', 'profiles-delete-active', 'profiles-delete-inactive', 'profiles-discard', 'connections-details',
                  'rules-discard', 'rules-discard-editor', 'rules-switch-confirm', 'rules-exit-confirm', 'settings-large-text')
                  for suffix in ('', '-light')]
        scenes += [scene+suffix for scene in ('claude', 'claude-web', 'claude-desktop', 'claude-code', 'claude-details', 'claude-empty',
                   'settings-large-text', 'profiles-edit', 'rules-discard-editor', 'connections-details')
                   for suffix in ('-compact', '-compact-light')]
        run(['dbus-run-session', '--', 'xvfb-run', '-a', '--server-args=-screen 0 1800x1200x24',
             sys.executable, ROOT/'scripts/run-ui-preview.py', '--source', ROOT,
             '--output', ROOT/'target/claude-ui', '--scenes', *scenes])
    # Invoke already-built test programs directly instead of repeating Cargo.
    if args.system == 'linux':
        run([tests['core_integration'], '--ignored', '--test-threads=1'])
        run([tests['library'], 'native_gsettings', '--ignored'],
            env=dict(os.environ, GSETTINGS_BACKEND='memory'))
        run(['dbus-run-session', '--', tests['tray_integration'], 'native_portal', '--ignored', '--test-threads=1'])
        for name in ('native_kde_signal', 'no_tray_host'):
            run(['dbus-run-session', '--', tests['library'], name, '--ignored'])
        run(['dbus-run-session', '--', tests['tray_integration'], 'login_tray', '--ignored'])
        if args.arch == 'x64':
            run(['dbus-run-session', '--', 'xvfb-run', '-a',
                 '--server-args=-screen 0 3200x2400x24', sys.executable,
                 ROOT/'scripts/test-linux-gui.py', '--executable', bundle/'clash-of-rust',
                 '--scales', '1', '1.25', '1.5', '2', '--output', ROOT/'target/linux-window-report.json'])
            run(['dbus-run-session', '--', sys.executable, ROOT/'scripts/test-linux-wayland.py',
                 '--executable', bundle/'clash-of-rust', '--output', ROOT/'target/linux-wayland-report'])
    elif args.system == 'macos':
        from macos_ci import native_tests
        native_tests(tests, bundle)
    else:
        from windows_ci import windows_tests
        windows_tests(tests, bundle)
    write_json(ROOT/'target/ci-tests.json', tests)
    # Optional UI fixture compilation may rebuild Cargo's bin targets with its
    # feature. Export the default-feature binaries frozen before native tests.
    export_build(args.system, args.arch, tests, ROOT/'dist/build', binaries=bundle)


if __name__ == '__main__':
    main()
