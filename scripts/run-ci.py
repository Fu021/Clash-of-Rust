"""Compile/test one native platform; preserve release binaries for packaging."""
import argparse
import importlib.util
import shutil
import sys

from build_support import host_system, ROOT, bundle_directory, host_arch, run, cargo_messages, write_json, desktop_cargo_scope
from workflow_artifacts import export_build, test_binaries
from ci_test_policy import desktop_ui_tests, extended_tests, linux_user_tests, update_tests


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
    if not extended and linux_user_tests() and (args.system, args.arch) == ('linux', 'x64'):
        for package, target, case in (
                ('clash-client-runtime', ['--lib'], 'platform::linux_tun::'),
                ('clash-client-runtime', ['--lib'], 'platform::host_network::'),
                ('clash-client-runtime', ['--lib'], 'control::tests::'),
                ('clash-cli', ['--bin', 'cor'], 'backend::tests::')):
            run(['cargo', 'test', '-p', package, *target, '--release', '--locked',
                 case, '--', '--test-threads=1'])
    if not extended and update_tests():
        run(['cargo', 'test', '-p', 'clash-client-runtime', '--lib', '--release', '--locked',
             'update::', '--', '--test-threads=1'])
        if (args.system, args.arch) == ('linux', 'x64'):
            for package, target, case in (
                    ('clash-client-core', ['--lib'], 'update'),
                    ('clash-client-runtime', ['--lib'], 'update_check_interval'),
                    ('clash-cli', ['--bin', 'cor'], 'update_'),
                    ('clash-of-rust', ['--bin', 'clash-of-rust'], 'update_')):
                run(['cargo', 'test', '-p', package, *target, '--release', '--locked',
                     case, '--', '--test-threads=1'])
        if args.system in ('windows', 'macos'):
            run(['cargo', 'test', '-p', 'clash-client-runtime', '--lib', '--release', '--locked',
                 'update_preparation_releases_owner', '--', '--test-threads=1'])
    if not extended and desktop_ui_tests() and (args.system, args.arch) == ('linux', 'x64'):
        for case in ('application::ui_tests::resizing_a_node_grid_preserves_odd_tail_nodes_and_their_own_actions',
                     'application::ui_tests::shortcut_dialog_edits_the_draft_and_keeps_disabled_choices_after_document_release',
                     'application::ui_tests::appearance_remains_interactive_while_geo_owns_the_engine_and_late_reply_cannot_undo_it',
                     'application::ui_tests::protection_locks_rule_controls_and_proxy_mode_without_a_warning_or_draft_change',
                     'application::ui_tests::protection_rejects_confirmations_that_would_save_or_delete',
                     'application::ui_tests::a_locked_rules_page_still_accepts_the_global_exit_confirmation',
                     'application::commands::synchronization_tests::protection_allows_observation_and_disable_but_rejects_mutations'):
            run(['cargo', 'test', '-p', 'clash-of-rust', '--bin', 'clash-of-rust', '--release',
                 '--locked', case, '--', '--exact', '--test-threads=1'])
        for case in ('mobile_rule_rows_keep_actions_aligned_for_all_sources_and_sizes',
                     'notice_countdown_restarts_and_expires_after_five_seconds',
                     'applications_scroll_across_phone_and_tablet_viewports',
                     'application_policy_form_respects_platform_protection_and_scrolls',
                     'mobile_accent_picker_remains_usable_while_protected',
                     'mobile_connections_keep_actions_reachable_and_scroll_with_long_names',
                     'report_categories_share_one_panel_without_gaps_from_hidden_rows',
                     'mobile_rules_drag_real_rows_at_phone_pad_and_large_text_sizes'):
            run(['cargo', 'test', '-p', 'clash-client-ui', '--test', 'mobile_lists', '--release',
                 '--locked', case, '--', '--exact', '--test-threads=1'])
        for package, target, cases in (
                ('clash-client-ui', ['--test', 'ui_models'], ['']),
                ('clash-desktop-ui', ['--lib'], ['']),
                ('clash-client-core', ['--lib'], ['']),
                ('clash-client-runtime', ['--lib'],
                 [] if linux_user_tests() else ['control::tests::'])):
            for case in cases:
                run(['cargo', 'test', '-p', package, *target, '--release', '--locked',
                     *([case] if case else []), '--', '--test-threads=1'])
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
