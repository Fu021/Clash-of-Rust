"""Named Windows regression suites, with a failure-preserving CI receipt."""
import sys
import time
from build_support import ROOT, host_arch, run, write_json


def windows_tests(tests, bundle):
    report_path = ROOT/'target/windows-ci-report.json'
    report = {'schema': 1, 'system': 'windows', 'arch': host_arch(),
              'status': 'running', 'suites': []}
    suites = [
        ('python-build-tools', [sys.executable, ROOT/'scripts/test-build-tools.py']),
        ('artifact-protocol', [sys.executable, ROOT/'scripts/test-workflow-artifacts.py']),
        ('offline-core-integration', [tests['core_integration'], '--ignored', '--test-threads=1']),
        ('native-tray', [tests['tray_integration'], '--ignored', '--test-threads=1']),
        ('elevated-task-permissions', [tests['library'], 'native_autostart', '--ignored']),
        ('embedded-icons', [sys.executable, ROOT/'scripts/verify-icon-resources.py',
                            bundle/'clash-of-rust.exe']),
        ('update-helper-rejections', [sys.executable, ROOT/'scripts/test-update-helper.py',
                                     bundle/'clash-of-rust.exe']),
        ('ordinary-user-startup', [sys.executable, ROOT/'scripts/test-windows-startup.py',
                                   bundle/'clash-of-rust.exe', '--test-binary', tests['library']]),
    ]
    report['suites'] = [{'name': name, 'status': 'not_run'} for name, _ in suites]
    write_json(report_path, report)
    for entry, (_, command) in zip(report['suites'], suites):
        started = time.monotonic()
        entry['status'] = 'running'
        write_json(report_path, report)
        print('::group::Windows regression: ' + entry['name'], flush=True)
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
    print('PASS: all Windows regression suites; report: ' + str(report_path), flush=True)
