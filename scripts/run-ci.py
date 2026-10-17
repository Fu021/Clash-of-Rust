"""Compile/test one native platform; preserve release binaries for packaging."""
import argparse
import importlib.util
import shutil
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
    from action_cache import workspace_sources
    workspace_sources(ROOT)
    run(['cargo', 'fmt', '--all', '--', '--check'])
    # Shared Python checks run in the workflow's prerequisite job, before the
    # platform matrix restores any large compiler caches.
    # One profile across Clippy, tests and production builds avoids a second
    # dependency compilation and produces the exact binaries released later.
    run(['cargo', 'clippy', '--workspace', '--release', '--all-targets', '--locked', '--', '-D', 'warnings'])
    run(['cargo', 'build', '--workspace', '--release', '--locked', '--bins'])
    messages = cargo_messages(['cargo', 'test', '--workspace', '--release', '--locked', '--lib', '--bins', '--tests',
                              '--no-run', '--message-format=json'])
    tests = test_binaries(messages)
    run(['cargo', 'test', '--workspace', '--release', '--locked', '--lib', '--bins', '--tests', '--', '--test-threads=1'])
    spec = importlib.util.spec_from_file_location('prepare', ROOT/'scripts/prepare-resources.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.prepare(args.system, args.arch)
    bundle = bundle_directory(args.system, args.arch)
    extension = '.exe' if args.system == 'windows' else ''
    for name in ('clash-of-rust', 'clash-tun-launcher'):
        shutil.copy2(ROOT/'target/release'/(name+extension), bundle/(name+extension))
    if args.system == 'windows':
        shutil.copy2(ROOT/'target/release/clash-wsl-guardian.exe', bundle/'clash-wsl-guardian.exe')
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
    workspace_sources(ROOT, save=True)


if __name__ == '__main__':
    main()
