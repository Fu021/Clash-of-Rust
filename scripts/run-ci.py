"""Compile/test one native platform; preserve release binaries for packaging."""
import argparse
import importlib.util
import shutil
import sys

from build_support import host_system, ROOT, bundle_directory, host_arch, run, cargo_messages, write_json, desktop_cargo_scope
from workflow_artifacts import export_build, test_binaries
from ci_test_policy import desktop_ui_tests, extended_tests


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
    packages = desktop_cargo_scope(ROOT)
    extended = extended_tests()
    clippy_targets = ['--all-targets'] if extended else ['--lib', '--bins']
    run(['cargo', 'clippy', *packages, '--release', *clippy_targets, '--locked', '--', '-D', 'warnings'])
    run(['cargo', 'build', *packages, '--release', '--locked', '--bins'])
    if not extended and desktop_ui_tests() and (args.system, args.arch) == ('linux', 'x64'):
        for case in ('resizing_a_node_grid_preserves_odd_tail_nodes_and_their_own_actions',
                     'shortcut_dialog_edits_the_draft_and_keeps_disabled_choices_after_document_release',
                     'appearance_remains_interactive_while_geo_owns_the_engine_and_late_reply_cannot_undo_it'):
            run(['cargo', 'test', '-p', 'clash-of-rust', '--bin', 'clash-of-rust', '--release',
                 '--locked', 'application::ui_tests::' + case, '--', '--exact', '--test-threads=1'])
        for case in ('notice_countdown_restarts_and_expires_after_five_seconds',
                     'applications_scroll_across_phone_and_tablet_viewports'):
            run(['cargo', 'test', '-p', 'clash-client-ui', '--test', 'mobile_lists', '--release',
                 '--locked', case, '--', '--exact', '--test-threads=1'])
    test_scope = [*packages, '--lib', '--bins', '--tests'] if extended else [
        '-p', 'clash-of-rust', '--test', 'core_integration']
    messages = cargo_messages(['cargo', 'test', *test_scope, '--release', '--locked',
                              '--no-run', '--message-format=json'])
    tests = test_binaries(messages)
    if extended:
        run(['cargo', 'test', *test_scope, '--release', '--locked',
             '--no-fail-fast', '--', '--test-threads=1'])
    spec = importlib.util.spec_from_file_location('prepare', ROOT/'scripts/prepare-resources.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.prepare(args.system, args.arch)
    bundle = bundle_directory(args.system, args.arch)
    extension = '.exe' if args.system == 'windows' else ''
    for name in ('clash-of-rust', 'clash-desktop-ui', 'clash-tun-launcher', 'cor'):
        shutil.copy2(ROOT/'target/release'/(name+extension), bundle/(name+extension))
    # Headless console smoke checks need neither a display nor a running kernel.
    run([bundle/('cor'+extension), '--json', 'version'])
    run([bundle/('cor'+extension), 'help'])
    write_json(ROOT/'target/ci-tests.json', tests)
    # Export the exact production binaries tested above.
    export_build(args.system, args.arch, tests, ROOT/'dist/build', binaries=bundle)
    workspace_sources(ROOT, save=True)


if __name__ == '__main__':
    main()
