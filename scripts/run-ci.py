"""Compile/test one native platform; preserve release binaries for packaging."""
import argparse
import importlib.util
import shutil
import sys

from build_support import host_system, ROOT, bundle_directory, host_arch, run, cargo_messages, write_json, desktop_cargo_scope
from workflow_artifacts import export_build, test_binaries
from ci_test_policy import desktop_ui_tests, extended_tests, linux_user_tests, proxy_benchmark, update_tests


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--system', choices=('windows', 'linux', 'macos'), required=True)
    parser.add_argument('--arch', choices=('x64', 'arm64'), required=True)
    args = parser.parse_args()
    if (args.system, args.arch) != (host_system(), host_arch()):
        raise ValueError('CI must run on the selected native architecture')
    from action_cache import compiler_cache_checkpoint, workspace_sources
    workspace_sources(ROOT)
    run(['cargo', 'fmt', '--all', '--', '--check'])
    # Shared Python checks run in the workflow's prerequisite job, before the
    # platform matrix restores any large compiler caches.
    # One profile across Clippy, tests and production builds avoids a second
    # dependency compilation and produces the exact binaries released later.
    packages = desktop_cargo_scope(ROOT)
    extended = extended_tests()
    source_regressions = (args.system, args.arch) == ('linux', 'x64') and (
        desktop_ui_tests() or linux_user_tests() or update_tests())
    clippy_targets = ['--all-targets'] if extended or source_regressions else ['--lib', '--bins']
    run(['cargo', 'clippy', *packages, '--release', *clippy_targets, '--locked', '--', '-D', 'warnings'])
    run(['cargo', 'build', *packages, '--release', '--locked', '--bins'])
    workspace_sources(ROOT, save=True)
    compiler_cache_checkpoint(ROOT)
    if not extended and args.system == 'macos' and desktop_ui_tests():
        run(['cargo', 'test', '-p', 'clash-of-rust', '--bin', 'clash-of-rust',
             '--release', '--locked',
             'application::async_state_tests::core_initialization_preserves_a_tray_failure_as_a_recoverable_warning',
             '--', '--exact', '--test-threads=1'])
    if not extended and args.system == 'macos' and proxy_benchmark():
        for case in ('platform::macos::service::tests::', 'platform::macos::cf::tests::',
                     'platform::macos::tests::'):
            run(['cargo', 'test', '-p', 'clash-client-runtime', '--lib', '--release', '--locked',
                 case, '--', '--test-threads=1'])
    if not extended and linux_user_tests() and (args.system, args.arch) == ('linux', 'x64'):
        for package, target, case in (
                ('clash-client-runtime', ['--lib'], 'platform::linux_tun::'),
                ('clash-client-runtime', ['--lib'], 'platform::linux_startup::'),
                ('clash-client-runtime', ['--lib'], 'platform::cli_service::'),
                ('clash-client-runtime', ['--lib'], 'platform::host_network::'),
                ('clash-client-runtime', ['--lib'], 'control::tests::'),
                ('clash-cli', ['--bin', 'cor'], '')):
            run(['cargo', 'test', '-p', package, *target, '--release', '--locked',
                 *([case] if case else []), '--', '--test-threads=1'])
    if not extended and update_tests():
        run(['cargo', 'test', '-p', 'clash-client-runtime', '--lib', '--release', '--locked',
             'update::', '--', '--test-threads=1'])
        if (args.system, args.arch) == ('linux', 'x64'):
            for package, target, case in (
                    ('clash-client-core', ['--lib'], 'update'),
                    ('clash-client-core', ['--lib'], 'download::tests::'),
                    ('clash-client-core', ['--lib'], 'geodata::tests::'),
                    ('clash-client-runtime', ['--lib'], 'update_check_interval'),
                    ('clash-cli', ['--bin', 'cor'], 'update_'),
                    ('clash-of-rust', ['--bin', 'clash-of-rust'], 'update_')):
                run(['cargo', 'test', '-p', package, *target, '--release', '--locked',
                     case, '--', '--test-threads=1'])
        if args.system in ('windows', 'macos'):
            run(['cargo', 'test', '-p', 'clash-client-runtime', '--lib', '--release', '--locked',
                 'update_preparation_releases_owner', '--', '--test-threads=1'])
        if args.system == 'windows':
            for case in ('platform::windows_service::', 'platform::windows_process::'):
                run(['cargo', 'test', '-p', 'clash-client-runtime', '--lib', '--release', '--locked',
                     case, '--', '--test-threads=1', '--show-output'])
    if not extended and desktop_ui_tests() and (args.system, args.arch) == ('linux', 'x64'):
        for case in ('application::ui_tests::network_report_is_replaced_and_late_results_cannot_reappear_after_navigation',
                     'application::ui_tests::english_network_report_displays_translated_status_and_preserves_raw_evidence',
                     'application::ui_tests::network_tools_remain_reachable_after_scrolling_other_pages',
                     'application::ui_tests::remounted_subscription_list_clamps_a_stale_scroll_position',
                     'application::ui_tests::remounted_long_pages_preserve_valid_scroll_positions',
                     'application::ui_tests::window_controls_remain_available_during_subscription_and_proxy_updates',
                     'application::ui_tests::ip_report_shows_each_target_exit_and_its_measurement_state',
                     'application::ui_tests::ip_category_status_and_region_popups_drive_report_filters',
                     'application::ui_tests::desktop_language_menu_dispatches_stable_tags_in_both_catalogs',
                     'application::ui_tests::english_sample_actions_fit_and_dispatch_in_a_scaled_dialog',
                     'application::ui_tests::english_desktop_validation_and_errors_follow_language',
                     'application::ui_tests::profile_edit_closes_on_success_independently_of_presentation_language',
                     'application::ui_tests::english_desktop_dialogs_show_edit_validation_and_protection_effects',
                     'application::interaction::tests::english_delete_confirmation_keeps_the_active_subscription_and_network_consequences',
                     'application::interaction::tests::english_diagnostics_feedback_preserves_paths_and_failure_details',
                     'application::ui_tests::claude_report_rows_keep_titles_and_labeled_explanations',
                     'application::ui_tests::english_claude_report_keeps_original_keys_and_evidence',
                     'application::ui_tests::claude_route_section_keeps_each_domain_and_failure',
                     'application::ui_tests::claude_guard_block_keeps_status_and_interceptions_together',
                     'application::ui_tests::claude_summary_keeps_quality_disagreements_visible_with_many_risks',
                     'application::ui_tests::recursive_dns_flags_follow_the_country_without_changing_risk_or_other_signals',
                     'application::ui_tests::startup_install_failure_reaches_the_visible_recoverable_notice_and_close_action',
                     'application::async_state_tests::initialization_presents_update_failure_after_clearing_the_placeholder_error',
                     'application::async_state_tests::successful_update_initialization_keeps_the_tray_warning_actionable',
                     'application::ui_tests::resizing_a_node_grid_preserves_odd_tail_nodes_and_their_own_actions',
                     'application::ui_tests::shortcut_dialog_edits_the_draft_and_keeps_disabled_choices_after_document_release',
                     'application::ui_tests::appearance_remains_interactive_while_geo_owns_the_engine_and_late_reply_cannot_undo_it',
                     'application::ui_tests::protection_locks_rule_controls_and_proxy_mode_without_a_warning_or_draft_change',
                     'application::ui_tests::protection_rejects_confirmations_that_would_save_or_delete',
                     'application::ui_tests::a_stopped_protected_core_exposes_a_retry_without_disabling_protection',
                     'application::ui_tests::a_locked_rules_page_still_accepts_the_global_exit_confirmation',
                     'application::ui_tests::entering_proxies_pins_and_expands_the_actual_group_without_reopening_a_manual_collapse',
                     'application::ui_tests::delayed_active_group_respects_manual_navigation_and_direct_mode',
                     'application::ui_tests::proxy_groups_default_to_all_and_keep_independent_node_pagination',
                     'application::ui_tests::group_pagination_button_dispatches_while_protected_and_tracks_saved_state',
                     'application::ui_tests::all_proxy_groups_are_scrollable_without_mounting_every_group',
                     'application::ui_tests::subscription_pagination_preserves_access_to_the_last_profile_in_both_views',
                     'application::dev2_tests::old_settings_receive_default_sort_and_new_preferences_round_trip',
                     'application::dev2_tests::retesting_a_target_exit_does_not_show_its_previous_measurement',
                     'application::commands::synchronization_tests::protection_allows_observation_and_disable_but_rejects_mutations'):
            run(['cargo', 'test', '-p', 'clash-of-rust', '--bin', 'clash-of-rust', '--release',
                 '--locked', case, '--', '--exact', '--test-threads=1'])
        for case in ('application::notice::tests::', 'application::claude::tests::',
                     'application::memory_tests::', 'application::window_state::tests::'):
            run(['cargo', 'test', '-p', 'clash-of-rust', '--bin', 'clash-of-rust', '--release',
                 '--locked', case, '--', '--test-threads=1'])
        run(['cargo', 'test', '-p', 'clash-of-rust', '--bin', 'clash-of-rust', '--release',
             '--locked', 'application::network_watch::tests::', '--', '--test-threads=1'])
        for package, target, cases in (
                ('clash-client-ui', ['--test', 'ui_models'], ['']),
                ('clash-client-ui', ['--test', 'emoji'], ['']),
                ('clash-client-ui', ['--test', 'desktop_lists'], ['']),
                ('clash-desktop-protocol', ['--test', 'schema'], ['']),
                ('clash-desktop-ui', ['--lib'], ['']),
                ('clash-client-core', ['--lib'], ['']),
                ('clash-cli', ['--bin', 'cor'], ['args::tests::', 'tui::tests::',
                                               'output::tests::english_network_report_translates_known_evidence_without_rewriting_external_values']),
                ('clash-client-runtime', ['--lib'],
                 ['control::dispatch::tests::', 'control::transport::tests::']
                 + ([] if linux_user_tests() else ['control::tests::']))):
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
    if args.system == 'linux' and linux_user_tests():
        from cli_tui import verify
        verify(bundle/'cor', bundle/'resources', ROOT/'target/linux-report/tui')
    write_json(ROOT/'target/ci-tests.json', tests)
    # Export the exact production binaries tested above.
    export_build(args.system, args.arch, tests, ROOT/'dist/build', binaries=bundle)
    workspace_sources(ROOT, save=True)


if __name__ == '__main__':
    main()
