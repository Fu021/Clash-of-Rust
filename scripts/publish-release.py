"""Publish packages tested by the manual release workflow using verified CI binaries.

The workflow and local CLI share the same CI, checksum and release checks.
Credentials stay in memory; Python 3.11+ and the standard library only.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib
import urllib.error
import urllib.parse
import urllib.request
import zipfile

from build_support import ROOT, opener, sha256, validate_version

CI_JOBS = {
    'windows-x64', 'windows-arm64', 'linux-x64', 'linux-arm64',
}
PACKAGES = {
    'package-windows-x64': 'windows-x64-setup.exe',
    'package-windows-arm64': 'windows-arm64-setup.exe',
    'package-linux-x64': 'linux-amd64.deb',
    'package-linux-arm64': 'linux-arm64.deb',
}
PACKAGE_JOBS = set(PACKAGES) | {'Ubuntu latest-x64 TUN and GUI'}


def git(*args):
    return subprocess.check_output(['git', *args], cwd=ROOT, text=True).strip()


def repository():
    remote = urllib.parse.urlparse(git('remote', 'get-url', 'origin'))
    if remote.scheme != 'https' or remote.netloc != 'github.com':
        raise ValueError('Expected an HTTPS github.com origin')
    repo = remote.path.strip('/').removesuffix('.git')
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repo):
        raise ValueError('Invalid repository path')
    if os.environ.get('GITHUB_REPOSITORY', repo).lower() != repo.lower():
        raise ValueError('Workflow repository does not match origin')
    return repo


def credential(repo):
    token = os.environ.get('GH_TOKEN') or os.environ.get('GITHUB_TOKEN')
    if token:
        return token
    result = subprocess.run(
        ['git', 'credential', 'fill'], cwd=ROOT,
        env=dict(os.environ, GIT_TERMINAL_PROMPT='0', GCM_INTERACTIVE='never'),
        input=f'protocol=https\nhost=github.com\npath={repo}.git\n\n',
        text=True, capture_output=True, timeout=45,
    )
    values = dict(line.split('=', 1) for line in result.stdout.splitlines() if '=' in line)
    if result.returncode or not values.get('password'):
        raise RuntimeError('Set GH_TOKEN/GITHUB_TOKEN or authenticate Git locally')
    return values['password']


class GitHub:
    def __init__(self, repo, token, proxy=''):
        self.repo = repo
        self.base = '/repos/' + repo
        self.token = token
        self.opener = opener(proxy)

    def __call__(self, path, method='GET', data=None, raw=False):
        url = 'https://api.github.com' + self.base + path if path.startswith('/') else path
        parsed = urllib.parse.urlparse(url)
        if parsed.scheme != 'https' or parsed.netloc not in ('api.github.com', 'uploads.github.com'):
            raise ValueError('Refusing authenticated request outside GitHub')
        binary = isinstance(data, bytes)
        if isinstance(data, dict):
            data = json.dumps(data, ensure_ascii=False).encode('utf-8')
        request = urllib.request.Request(url, data=data, method=method, headers={
            'Authorization': 'Bearer ' + self.token,
            'Accept': 'application/vnd.github+json',
            'X-GitHub-Api-Version': '2026-03-10',
            'User-Agent': 'Clash-of-Rust-release',
            'Content-Type': 'application/octet-stream' if binary else 'application/json',
        })
        with self.opener.open(request, timeout=180) as response:
            body = response.read()
            return body if raw else json.loads(body) if body else None


def require(condition, message):
    if not condition:
        raise ValueError(message)


def verified_run(client, commit, run_id=None, check_only=False):
    if run_id is None:
        query = urllib.parse.urlencode({'head_sha': commit, 'event': 'push', 'branch': 'main', 'per_page': 20})
        runs = client('/actions/workflows/ci.yml/runs?' + query)['workflow_runs']
        require(bool(runs), 'No main-branch CI found for the checked-out commit')
        run_id = runs[0]['id']
    run = client(f'/actions/runs/{run_id}')
    require(run['path'].split('@', 1)[0] == '.github/workflows/ci.yml', 'Selected run is not the build CI')
    main_ci = run['event'] == 'push' and run['head_branch'] == 'main'
    require(main_ci or (check_only and run['event'] == 'pull_request'),
            'Release requires main-branch push CI; test-only packaging also accepts same-repository PR CI')
    require(run['head_repository']['full_name'].lower() == client.repo.lower(), 'CI repository mismatch')
    require(run['head_sha'] == commit, 'CI commit differs from the checked-out commit')
    require(run['status'] == 'completed' and run['conclusion'] == 'success', 'CI has not completed successfully')
    branch = 'main' if main_ci else run['head_branch']
    require(client('/branches/' + urllib.parse.quote(branch, safe=''))['commit']['sha'] == commit,
            'Source branch has changed; select its current successful CI')
    jobs = client(f'/actions/runs/{run_id}/jobs?filter=latest&per_page=100')['jobs']
    require(len(jobs) == len(CI_JOBS) and {j['name'] for j in jobs} == CI_JOBS,
            'CI does not cover all four native platforms')
    require(all(j['status'] == 'completed' and j['conclusion'] == 'success' for j in jobs), 'A required CI job did not pass')
    return run


def release_metadata(commit, tag=None, notes_path=None, check_only=False):
    require(bool(re.fullmatch(r'[0-9a-f]{40}', commit)), 'Invalid release commit')
    version = validate_version(tomllib.loads(git('show', commit + ':Cargo.toml'))['package']['version'])
    expected_tag = 'v' + version
    require(tag is None or tag == expected_tag, 'Release tag differs from the tested Cargo version')
    if check_only:
        require(notes_path is None, 'Release notes are not used by test-only packaging')
        return version, expected_tag, ''
    # Read release notes from the tested Git tree, not uncommitted local files.
    notes = subprocess.check_output(['git', 'show', f'{commit}:docs/releases/{version}.md'], cwd=ROOT, text=True)
    if notes_path:
        require(notes_path.read_text(encoding='utf-8') == notes, 'Release notes differ from the tested commit')
    return version, expected_tag, notes


def filenames(version):
    names = {'Clash-of-Rust-' + version + '-' + suffix for suffix in PACKAGES.values()}
    return names | {name + '.sha256' for name in names}


def validate_files(paths, version):
    files = {p.name: p for p in paths}
    require(len(files) == len(paths) and set(files) == filenames(version), 'Expected exactly four installers and four checksum files')
    for name, path in files.items():
        require(path.is_file() and not path.is_symlink(), 'Release asset must be a regular file: ' + name)
        if name.endswith('.sha256'):
            fields = path.read_text(encoding='ascii').strip().split(maxsplit=1)
            require(bool(fields) and bool(re.fullmatch(r'[0-9a-fA-F]{64}', fields[0])), 'Invalid checksum file: ' + name)
            target = name.removesuffix('.sha256')
            require(len(fields) == 1 or fields[1].lstrip('*') == target, 'Checksum filename mismatch: ' + name)
            require(fields[0].lower() == sha256(files[target]), 'Installer checksum mismatch: ' + target)
    return files


def build_artifacts(client, run_id):
    from workflow_artifacts import BUILD_NAMES
    artifacts = client(f'/actions/runs/{run_id}/artifacts?per_page=100')['artifacts']
    builds = [a for a in artifacts if a['name'].startswith('build-')]
    require(len(builds) == len(BUILD_NAMES) and {a['name'] for a in builds} == BUILD_NAMES,
            'CI native build artifacts are incomplete or duplicated')
    require(all(not a['expired'] for a in builds), 'CI native build artifact expired')
    return {a['name']: {key: a[key] for key in ('id', 'name', 'digest')} for a in builds}


def verified_package_run(client, run_id, commit, check_only=False):
    run = client(f'/actions/runs/{run_id}')
    require(run['path'].split('@', 1)[0] == '.github/workflows/release.yml'
            and run['event'] == 'workflow_dispatch' and (run['head_branch'] == 'main' or check_only),
            'Packages must come from the manual release workflow; branch packages require test-only mode')
    require(run['head_sha'] == commit and run['head_repository']['full_name'].lower() == client.repo.lower(),
            'Package workflow source differs from the selected CI')
    jobs = client(f'/actions/runs/{run_id}/jobs?filter=latest&per_page=100')['jobs']
    required = [job for job in jobs if job['name'] in PACKAGE_JOBS]
    require(len(required) == len(PACKAGE_JOBS) and {j['name'] for j in required} == PACKAGE_JOBS,
            'Release workflow does not cover every package and Ubuntu compatibility test')
    require(all(j['status'] == 'completed' and j['conclusion'] == 'success' for j in required),
            'A package or installed-program test did not pass')
    # When called by the final job this workflow is still in progress. Outside
    # that job only a completely successful package workflow is accepted.
    active = os.environ.get('GITHUB_RUN_ID') == str(run_id)
    require(active or (run['status'] == 'completed' and run['conclusion'] == 'success'),
            'Package workflow has not completed successfully')
    return run


def download_package_files(client, run_id, ci_run_id, commit, version, directory):
    builds = build_artifacts(client, ci_run_id)
    artifacts = client(f'/actions/runs/{run_id}/artifacts?per_page=100')['artifacts']
    packages = [a for a in artifacts if a['name'].startswith('package-')]
    require(len(packages) == len(PACKAGES) and {a['name'] for a in packages} == set(PACKAGES),
            'Release package artifacts are incomplete or duplicated')
    directory.mkdir(parents=True, exist_ok=True)
    require({p.name for p in directory.iterdir()} <= filenames(version), 'Output directory contains unexpected files')
    for artifact in packages:
        require(not artifact['expired'], 'Release artifact expired: ' + artifact['name'])
        archive = client(f"/actions/artifacts/{artifact['id']}/zip", raw=True)
        require(artifact.get('digest') == 'sha256:' + hashlib.sha256(archive).hexdigest(), 'Package archive checksum mismatch')
        name = f"Clash-of-Rust-{version}-{PACKAGES[artifact['name']]}"
        with zipfile.ZipFile(io.BytesIO(archive)) as zipped:
            entries = zipped.infolist()
            require(len(entries) == 3 and {e.filename for e in entries} == {name, name + '.sha256', 'package-receipt.json'},
                    'Unexpected files inside package artifact')
            receipt = json.loads(zipped.read('package-receipt.json'))
            system, arch = artifact['name'].removeprefix('package-').split('-')
            require((receipt.get('schema'), receipt.get('commit'), receipt.get('version'),
                     receipt.get('ci_run_id'), receipt.get('system'), receipt.get('arch'),
                     receipt.get('package'), receipt.get('tests_passed')) ==
                    (1, commit, version, ci_run_id, system, arch, name, True), 'Package receipt mismatch')
            require(receipt.get('build_artifact') == builds[f'build-{system}-{arch}'],
                    'Package was not produced from the selected CI build artifact')
            require(receipt.get('sha256') == hashlib.sha256(zipped.read(name)).hexdigest(),
                    'Package differs from the tested package receipt')
            for filename in (name, name + '.sha256'):
                destination = directory / filename
                require(not destination.is_symlink(), 'Output path is a symbolic link')
                data = zipped.read(filename)
                if destination.exists():
                    require(destination.is_file() and destination.read_bytes() == data, 'Existing output differs from release tests: ' + filename)
                else:
                    destination.write_bytes(data)
    return validate_files(list(directory.iterdir()), version)


def find_release(client, tag):
    for page in range(1, 101):
        releases = client(f'/releases?per_page=100&page={page}')
        found = next((r for r in releases if r['tag_name'] == tag), None)
        if found is not None or len(releases) < 100:
            return found
    raise ValueError('Too many release pages')


def verify_tag(client, tag, commit, allow_missing=False):
    try:
        obj = client('/git/ref/tags/' + tag)['object']
    except urllib.error.HTTPError as error:
        if error.code == 404 and allow_missing:
            return
        raise
    for _ in range(8):
        if obj['type'] != 'tag':
            break
        obj = client('/git/tags/' + obj['sha'])['object']
    require(obj['type'] == 'commit' and obj['sha'] == commit, 'Existing version tag points to a different commit')


def verify_release(release, tag, notes, files):
    preview = '-' in tag.removeprefix('v')
    require(release['tag_name'] == tag and release['body'] == notes and release['prerelease'] == preview, 'Release metadata mismatch')
    assets = release['assets']
    require(len(assets) == len(files) and {a['name'] for a in assets} == set(files), 'Release attachment list differs from tested packages')
    for asset in assets:
        path = files[asset['name']]
        require(asset['state'] == 'uploaded' and asset['size'] == path.stat().st_size
                and asset.get('digest') == 'sha256:' + sha256(path), 'Uploaded asset mismatch: ' + asset['name'])


def publish(client, commit, tag, notes, files):
    preview = '-' in tag.removeprefix('v')
    release = find_release(client, tag)
    require(release is None or release['draft'], 'Release is already published; refusing to replace it')
    verify_tag(client, tag, commit, allow_missing=True)
    if release:
        require(release['target_commitish'] == commit, 'Existing draft targets a different commit')
        require({a['name'] for a in release['assets']} <= set(files), 'Draft contains unexpected attachments')
    payload = {'tag_name': tag, 'target_commitish': commit, 'name': 'Clash of Rust ' + tag.removeprefix('v'),
               'body': notes, 'draft': True, 'prerelease': preview, 'make_latest': 'false'}
    # GitHub enforces contents:write here for both personal and Actions tokens.
    # Installation tokens need not expose the personal-token permissions.push field.
    release = client('/releases' + (f"/{release['id']}" if release else ''), 'PATCH' if release else 'POST', payload)
    upload = release['upload_url'].split('{', 1)[0]
    existing = {a['name']: a for a in release['assets']}
    for name, path in sorted(files.items()):
        asset = existing.get(name)
        if asset is not None and asset['state'] == 'starter' and asset['size'] == 0:
            client(f"/releases/assets/{asset['id']}", 'DELETE')
            asset = None
        if asset is None:
            print('Uploading:', name, flush=True)
            asset = client(upload + '?' + urllib.parse.urlencode({'name': name}), 'POST', path.read_bytes())
        require(asset['state'] == 'uploaded' and asset['size'] == path.stat().st_size
                and asset.get('digest') == 'sha256:' + sha256(path), 'Uploaded asset mismatch: ' + name)
    draft = client(f"/releases/{release['id']}")
    require(draft['draft'], 'Release changed while uploading')
    verify_release(draft, tag, notes, files)
    require(client('/branches/main')['commit']['sha'] == commit, 'Main changed while uploading; leaving draft unpublished')
    verify_tag(client, tag, commit, allow_missing=True)
    client(f"/releases/{release['id']}", 'PATCH', {'draft': False, 'prerelease': preview, 'make_latest': 'false' if preview else 'true'})
    published = client('/releases/tags/' + tag)
    require(not published['draft'], 'Release is still a draft')
    verify_release(published, tag, notes, files)
    verify_tag(client, tag, commit)
    print('Published and verified:', published['html_url'])
    return published['html_url']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run-id', type=int, help='Successful CI run; PR CI requires --check and an explicit run ID')
    parser.add_argument('--check', action='store_true', help='Verify the complete packaging/tests run without publishing')
    parser.add_argument('--resolve', action='store_true', help='Verify CI and emit immutable source/run metadata for packaging')
    parser.add_argument('--package-run-id', type=int, help='Successful packaging run; defaults to the current Actions run')
    parser.add_argument('--proxy', default='')
    parser.add_argument('--tag', help='Optional confirmation of the tested Cargo version tag')
    parser.add_argument('--notes', type=Path, help='Optional local copy; must match committed release notes')
    parser.add_argument('--asset', action='append', type=Path, default=[], help='Optional local files; must match CI assets')
    parser.add_argument('--output-dir', type=Path, help='Directory for downloaded CI installers')
    args = parser.parse_args()
    repo = repository()
    client = GitHub(repo, credential(repo), args.proxy)
    commit = git('rev-parse', 'HEAD')
    run = verified_run(client, commit, args.run_id, check_only=args.check)
    version, tag, notes = release_metadata(commit, args.tag, args.notes, check_only=args.check)
    if args.resolve:
        build_artifacts(client, run['id'])
        if os.environ.get('GITHUB_OUTPUT'):
            with Path(os.environ['GITHUB_OUTPUT']).open('a', encoding='utf-8') as output:
                output.write(f"commit={commit}\nci_run_id={run['id']}\nversion={version}\n")
        print('Verified CI source:', commit, 'run:', run['id'], 'version:', version)
        return
    package_run_id = args.package_run_id or int(os.environ.get('GITHUB_RUN_ID', '0'))
    require(package_run_id > 0, 'Specify --package-run-id for the verified package workflow')
    verified_package_run(client, package_run_id, commit, check_only=args.check)
    directory = args.output_dir or ROOT / f"dist/release-{version}-run-{package_run_id}"
    files = download_package_files(client, package_run_id, run['id'], commit, version, directory)
    if args.asset:
        local = validate_files(args.asset, version)
        require(all(sha256(local[name]) == sha256(path) for name, path in files.items()), 'Local assets differ from selected CI')
    print('Verified CI:', run['html_url'])
    print('Verified version and all eight attachments:', tag)
    if args.check:
        print('Check complete; no release changes made')
        return
    url = publish(client, commit, tag, notes, files)
    if os.environ.get('GITHUB_STEP_SUMMARY'):
        with Path(os.environ['GITHUB_STEP_SUMMARY']).open('a', encoding='utf-8') as summary:
            summary.write(f'[Published {tag}]({url}) — four installers and four SHA256 files verified.\n')


if __name__ == '__main__':
    try:
        main()
    except urllib.error.HTTPError as error:
        print(f'GitHub request failed (HTTP {error.code}); credentials were not printed', file=sys.stderr)
        sys.exit(1)
    except Exception as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
