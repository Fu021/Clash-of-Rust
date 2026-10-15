"""Temporary native launchd/update lifetime and system-resolver verification."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile

from build_support import ROOT, host_arch, write_json
from macos_ci import REPORT, logged, suite
import workflow_artifacts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--build', type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != 'darwin' or os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('This changes network settings on a disposable macOS Actions VM only')
    build = args.build.resolve(strict=True)
    manifest = json.loads((build/'manifest.json').read_text())
    workflow_artifacts.verify_build(build, 'macos', host_arch(), workflow_artifacts.commit())
    REPORT.mkdir(parents=True, exist_ok=True)
    library = build/manifest['tests']['library']
    env = dict(os.environ, CLASH_MAC_FIX_REPORT=str(REPORT/'update-launchd.json'))
    suite('Updater survives real launchd job exit (including failing control)', lambda: logged(
        'update-launchd', [library, 'update::install::tests::native_updates_survive_login_job_shutdown',
                           '--exact', '--ignored', '--nocapture'], env=env, timeout=90))
    spec = importlib.util.spec_from_file_location('claude_native_tests', ROOT/'scripts/test-claude-protection.py')
    fixture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixture)
    spec = importlib.util.spec_from_file_location('proxy_native_tests', ROOT/'scripts/test-proxy-modes.py')
    proxy = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(proxy)
    with tempfile.TemporaryDirectory(prefix='clash-mac-dns-ca-') as temporary:
        with fixture.trusted_fixture(Path(temporary)) as (cert, key, ca):
            settings = {
                'CLASH_CLAUDE_TEST_CERT': str(cert), 'CLASH_CLAUDE_TEST_CA': str(ca),
                'CLASH_CLAUDE_TEST_KEY': str(key),
                'CLASH_CLAUDE_TEST_SCRIPT': str(ROOT/'scripts/test-claude-protection.py'),
                'CLASH_CLAUDE_TEST_PYTHON': sys.executable,
                'CLASH_MAC_DNS_REPORT': str(REPORT/'system-dns.json'),
            }
            original = {name: os.environ.get(name) for name in settings}
            os.environ.update(settings)
            try:
                suite('System resolver through pinned DoH; restart, disable, exit and crash restore', lambda:
                      proxy.check(build/manifest['tests']['core_integration'], build/'payload',
                                  test_case='macos_claude_system_dns'))
            finally:
                for name, value in original.items():
                    if value is None:
                        os.environ.pop(name, None)
                    else:
                        os.environ[name] = value
    report = json.loads((REPORT/'report.json').read_text())
    report['status'] = 'passed'
    write_json(REPORT/'report.json', report)


if __name__ == '__main__':
    main()
