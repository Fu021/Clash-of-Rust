"""Temporary native Mac online update test, served exclusively on loopback."""
import argparse
import contextlib
import http.server
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time
import tomllib
import urllib.parse
import sys

from build_support import ROOT, cargo_messages, host_arch, run, sha256, write_json
from macos_package import APP_NAME, build_app, build_dmg

ORIGIN = 'http://127.0.0.1:18080'
REPORT = ROOT/'target/macos-online-update-report'


@contextlib.contextmanager
def server(package, version):
    requests = []
    sidecar = (sha256(package)+'\n').encode('ascii')
    metadata = json.dumps([{'tag_name': 'v'+version, 'draft': False, 'prerelease': False,
                           'assets': [{'name': package.name, 'size': package.stat().st_size,
                                       'digest': 'sha256:'+sha256(package)},
                                      {'name': package.name+'.sha256', 'size': len(sidecar)}]}]).encode()
    prefix = '/releases/download/v'+version+'/'

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            path = urllib.parse.urlsplit(self.path).path
            requests.append({'path': path, 'time': time.time()})
            write_json(REPORT/'http-requests.json', requests)
            if path == '/api/releases':
                body = metadata
            elif path == prefix+package.name+'.sha256':
                body = sidecar
            elif path == prefix+package.name:
                self.send_response(200)
                self.send_header('Content-Length', str(package.stat().st_size))
                self.end_headers()
                with package.open('rb') as source:
                    shutil.copyfileobj(source, self.wfile)
                return
            else:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, message, *args):
            print('loopback: '+message % args, flush=True)

    service = http.server.ThreadingHTTPServer(('127.0.0.1', 18080), Handler)
    worker = threading.Thread(target=service.serve_forever, daemon=True)
    worker.start()
    try:
        yield requests
    finally:
        service.shutdown()
        service.server_close()
        worker.join(timeout=5)


def fixture_build(version):
    # The stable target must compare greater than the compiled test client.
    # Change only the package version in this disposable runner's source tree.
    manifest, lock = ROOT/'Cargo.toml', ROOT/'Cargo.lock'
    original_manifest, original_lock = manifest.read_text(), lock.read_text()
    current = tomllib.loads(original_manifest)['package']['version']
    manifest.write_text(original_manifest.replace('version = "'+current+'"', 'version = "'+version+'"', 1))
    lock.write_text(original_lock.replace('name = "clash-of-rust"\nversion = "'+current+'"',
                                        'name = "clash-of-rust"\nversion = "'+version+'"', 1))
    previous_test_mode = os.environ.get('CLASH_MAC_UPDATE_TEST')
    try:
        os.environ['CLASH_MAC_UPDATE_TEST'] = '1'
        messages = cargo_messages(['cargo', 'test', '--release', '--locked', '--features', 'ui-preview',
                                  '--test', 'ui_preview', '--no-run', '--message-format=json'],
                                 log=REPORT/'build.jsonl', timeout=1800)
        entries = [json.loads(line) for line in messages.splitlines()]
        ui = [entry['executable'] for entry in entries if entry.get('reason') == 'compiler-artifact'
              and entry.get('executable') and entry['target']['name'] == 'ui_preview']
        if len(ui) != 1:
            raise RuntimeError('Online update GUI fixture was not built')
        run(['cargo', 'build', '--release', '--locked', '--features', 'ui-preview', '--bin', 'clash-tun-launcher'])
        return Path(ui[0])
    finally:
        if previous_test_mode is None:
            os.environ.pop('CLASH_MAC_UPDATE_TEST', None)
        else:
            os.environ['CLASH_MAC_UPDATE_TEST'] = previous_test_mode
        manifest.write_text(original_manifest)
        lock.write_text(original_lock)


def stock_probe(package, version, arch):
    """Build the published source with only its version lowered, no test feature."""
    from macos_ci import mounted
    probe_version = version+'-update-probe.1'
    reference = 'refs/tags/v'+version
    run(['git', 'fetch', '--no-tags', 'origin', reference])
    commit = subprocess.check_output(['git', 'rev-parse', 'FETCH_HEAD^{commit}'], cwd=ROOT, text=True).strip()
    with tempfile.TemporaryDirectory(prefix='clash-stock-update-') as temporary:
        directory = Path(temporary)
        source = directory/'source'
        run(['git', 'worktree', 'add', '--detach', source, commit])
        try:
            for name in ('Cargo.toml', 'Cargo.lock'):
                path = source/name
                text = path.read_text()
                old = ('version = "'+version+'"') if name == 'Cargo.toml' else ('name = "clash-of-rust"\nversion = "'+version+'"')
                new = old.replace('"'+version+'"', '"'+probe_version+'"')
                if old not in text:
                    raise ValueError('Published source version does not match target')
                path.write_text(text.replace(old, new, 1))
            environment = dict(os.environ, CARGO_TARGET_DIR=str(ROOT/'target/stock-probe'))
            environment.pop('CLASH_MAC_UPDATE_TEST', None)
            subprocess.run(['cargo', 'build', '--release', '--locked', '--bins'], cwd=source,
                           env=environment, check=True)
            changed = subprocess.check_output(['git', 'diff', '--name-only'], cwd=source, text=True).splitlines()
            if set(changed) != {'Cargo.toml', 'Cargo.lock'}:
                raise ValueError('Stock probe changed more than package versions')
            payload = directory/'payload'
            payload.mkdir()
            with mounted(package) as volume:
                released = volume/APP_NAME/'Contents/Resources'
                shutil.copytree(released/'resources', payload/'resources')
                shutil.copy2(released/'LICENSE', payload/'LICENSE')
            for name in ('clash-of-rust', 'clash-tun-launcher'):
                shutil.copy2(ROOT/'target/stock-probe/release'/name, payload/name)
            (ROOT/'dist').mkdir(exist_ok=True)
            result = build_dmg(payload, probe_version, arch)
            write_json(REPORT/'report.json', {'schema': 1, 'status': 'manual_test_package',
                       'source_commit': commit, 'source_tag': 'v'+version,
                       'version': probe_version, 'target_version': version,
                       'changes': changed, 'features': [], 'update_source': 'GitHub production release API',
                       'resources': 'Unmodified resources from the published target DMG',
                       'package': result.name, 'sha256': sha256(result)})
            result.with_name(result.name+'.sha256').write_text(sha256(result)+'\n')
            summary = os.environ.get('GITHUB_STEP_SUMMARY')
            if summary:
                with open(summary, 'a') as stream:
                    stream.write(f'### Stock Mac update probe ({arch})\n\nPublished `{version}` source, only version lowered to `{probe_version}`. No test features or local update source. Update normally through GitHub to `{version}`.\n')
        finally:
            run(['git', 'worktree', 'remove', '--force', source])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--arch', choices=('x64', 'arm64'), default=host_arch())
    parser.add_argument('--target-tag', default='v0.5.8')
    parser.add_argument('--serve-only', type=Path, metavar='DMG', help='Serve a stable target DMG for manual local testing')
    parser.add_argument('--stock-probe', action='store_true', help='Package published source with only its version lowered for manual reproduction')
    args = parser.parse_args()
    version = args.target_tag.removeprefix('v')
    if not version or any(c not in '0123456789.' for c in version):
        raise ValueError('A stable numeric target tag is required')
    REPORT.mkdir(parents=True, exist_ok=True)
    if args.serve_only:
        package = args.serve_only.resolve(strict=True)
        if package.name != f'Clash-of-Rust-{version}-macos-{args.arch}.dmg':
            raise ValueError('DMG name must match target version and native architecture')
        print('Serving '+ORIGIN+'; launch the diagnostic installer app and check updates. Ctrl+C stops.', flush=True)
        with server(package, version):
            try:
                threading.Event().wait()
            except KeyboardInterrupt:
                pass
        return
    if sys.platform != 'darwin' or host_arch() != args.arch or os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('Matching disposable native Mac Actions runner required')
    package_name = f'Clash-of-Rust-{version}-macos-{args.arch}.dmg'
    run(['gh', 'release', 'download', args.target_tag, '--repo', 'Fu021/Clash-of-Rust',
         '--pattern', package_name, '--pattern', package_name+'.sha256', '--dir', REPORT/'target-package'])
    package = REPORT/'target-package'/package_name
    if sha256(package) != package.with_name(package_name+'.sha256').read_text().strip():
        raise ValueError('Published target DMG failed SHA256 verification')
    if args.stock_probe:
        stock_probe(package, version, args.arch)
        return
    test_version = version+'-update-test.1'
    ui = fixture_build(test_version)
    run(['python3', ROOT/'scripts/prepare-resources.py', '--system', 'macos', '--arch', args.arch])
    from build_support import bundle_directory
    bundle = bundle_directory('macos', args.arch)
    shutil.copy2(ui, bundle/'clash-of-rust')
    shutil.copy2(ROOT/'target/release/clash-tun-launcher', bundle/'clash-tun-launcher')
    # Keep an installer artifact with the diagnostic source override. Official
    # installers are never built with this feature or loopback environment.
    (ROOT/'dist').mkdir(exist_ok=True)
    diagnostic_package = build_dmg(bundle, test_version, args.arch)
    spec = importlib.util.spec_from_file_location('flows', ROOT/'scripts/test-macos-flows.py')
    flows = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(flows)
    report = {'schema': 1, 'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
              'test_version': test_version, 'target_version': version, 'origin': ORIGIN,
              'target_sha256': sha256(package), 'diagnostic_package': diagnostic_package.name,
              'authorization': 'Runner sudo substitutes human confirmation; real dialogs remain manual',
              'status': 'running', 'cases': []}
    write_json(REPORT/'report.json', report)
    try:
        with tempfile.TemporaryDirectory(prefix='clash-online-update-') as directory:
            fixture = build_app(bundle, Path(directory)/APP_NAME, test_version, args.arch)
            with server(package, version) as requests:
                for scenario in ('update-online-user', 'update-online-system',
                                 'update-tun-online-user', 'update-tun-online-system'):
                    started, previous = time.monotonic(), len(requests)
                    with flows.network_service_for_ci(REPORT, scenario):
                        details = flows.flow(scenario, fixture, package, version, REPORT, 'online')
                    paths = {entry['path'] for entry in requests[previous:]}
                    required = {'/api/releases', f'/releases/download/v{version}/{package.name}',
                                f'/releases/download/v{version}/{package.name}.sha256'}
                    if not required <= paths:
                        raise RuntimeError('Update bypassed online discovery/download: '+str(paths))
                    report['cases'].append({'name': scenario, 'status': 'passed',
                                            'duration_seconds': round(time.monotonic()-started, 3),
                                            'http_paths': sorted(paths), 'details': details})
                    write_json(REPORT/'report.json', report)
        report['status'] = 'passed'
    except BaseException as error:
        report.update(status='failed', error=str(error))
        raise
    finally:
        write_json(REPORT/'report.json', report)
        summary = os.environ.get('GITHUB_STEP_SUMMARY')
        if summary:
            with open(summary, 'a') as stream:
                stream.write(f'### Mac {args.arch} online update\n\n{test_version} → {version}: **{report["status"]}**\n\n')
                stream.write('| Scenario | Result | Seconds |\n|---|---|---|\n')
                for case in report['cases']:
                    stream.write(f'| {case["name"]} | {case["status"]} | {case["duration_seconds"]} |\n')
                stream.write('\nLoopback release discovery, checksum, DMG transfer, installation, normal-user relaunch and healthy core are required. Actual authorization dialogs and downloaded-app Gatekeeper prompts remain manual.\n')


if __name__ == '__main__':
    main()
