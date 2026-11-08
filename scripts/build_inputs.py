"""Resolve one dependency snapshot per CI, then reuse it unchanged for packaging."""
import argparse
import datetime
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import urllib.error
import urllib.parse

from build_support import ROOT, download_asset, host_arch, host_system, read_json, sha256, write_json

FILES = ('Cargo.lock', 'apps/android/core/go.mod', 'apps/android/core/go.sum',
         'apps/android/build.gradle.kts', 'apps/android/app/build.gradle.kts')
SNAPSHOT = ROOT / 'target/dependency-snapshot'
GEO = {'geoip.dat': 'GeoIP.dat', 'geosite.dat': 'GeoSite.dat',
       'country.mmdb': 'Country.mmdb', 'GeoLite2-ASN.mmdb': 'ASN.mmdb'}
# AGP 9.4 requires Gradle 9.6 or newer; stay within its supported major.
GRADLE_MIN = '9.6.0'


def approved():
    value = json.loads((ROOT / '.github/build-versions.json').read_text())
    if value.get('schema') != 1 or not value.get('tools') or not value.get('upstream'):
        raise ValueError('Approved build versions are incomplete')
    return value


def tool_versions():
    selected = approved()['tools'].copy()
    system, arch = host_system(), host_arch()
    if system == 'macos':
        selected['python'] = selected['python_macos']
    elif system == 'windows':
        selected['python'] = selected['python_windows_arm64' if arch == 'arm64' else 'python_windows']
    if os.environ.get('COR_USE_BOOTSTRAP_TAILNET') == 'true':
        selected['tailscale'] = selected['tailscale_bootstrap']
    return selected


def android_versions():
    return approved()['upstream']['android']


def check_tools(tools=('go', 'java')):
    selected = tool_versions()
    if sys.version.split()[0] != selected['python']:
        raise ValueError('Active Python differs from the approved version')
    for name, command, expected in (
        ('Go', ['go', 'env', 'GOVERSION'], 'go' + selected['go']),
        ('Java', ['java', '-XshowSettings:properties', '-version'], selected['java'].split('+')[0]),
    ):
        if name.lower() not in tools:
            continue
        result = subprocess.run(command, check=True, text=True, capture_output=True, timeout=30)
        output = result.stdout + result.stderr
        version = re.search(r'(?m)^\s*java\.version\s*=\s*(\S+)', output) if name == 'Java' else None
        actual = version[1] if version else output.strip()
        if actual != expected:
            raise ValueError(f'Active {name} differs from the approved version: {actual}')
        print(f'Approved {name}: {actual}', flush=True)
    print('Approved Python:', sys.version.split()[0], flush=True)


def versions():
    selected = tool_versions()
    for target in ('GITHUB_ENV', 'GITHUB_OUTPUT'):
        if output := os.environ.get(target):
            with open(output, 'a') as stream:
                for name, version in selected.items():
                    if not re.fullmatch(r'[0-9][0-9.+-]*', version):
                        raise ValueError('Invalid approved tool version')
                    stream.write(('COR_' + name.upper() if target == 'GITHUB_ENV' else name)
                                 + '=' + version + '\n')


def original(path):
    return subprocess.check_output(['git', 'show', 'HEAD:' + path], cwd=ROOT)


def current_commit():
    return subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()


def constraints():
    paths = subprocess.check_output(['git', 'ls-files', 'Cargo.toml', 'apps/**/Cargo.toml',
                                     'crates/**/Cargo.toml', 'vendor/**/Cargo.toml', '.cargo/*', 'apps/android/gradle.properties',
                                     'resources/linux/ubuntu20.Dockerfile', 'scripts/linux_ci.py',
                                     'scripts/prepared_vm.py', 'scripts/prepared_vm_environment.py',
                                     '.github/build-versions.json'],
                                    cwd=ROOT, text=True).splitlines()
    return {path: hashlib.sha256(original(path)).hexdigest() for path in paths}


def manifest(directory=None):
    path = Path(directory or os.environ.get('BUILD_INPUTS_DIR', SNAPSHOT)) / 'inputs.json'
    return json.loads(path.read_text()) if path.is_file() else None


def verified_manifest():
    value = manifest()
    if not value or value.get('schema') != 1 or value.get('commit') != current_commit():
        raise ValueError('Build export requires the current verified dependency snapshot')
    if set(value.get('files', {})) != set(FILES):
        raise ValueError('Build dependency snapshot is incomplete')
    for relative, checksum in value['files'].items():
        path = ROOT / relative
        if not path.is_file() or sha256(path) != checksum:
            raise ValueError('Build dependencies changed after resolution: ' + relative)
    verify_geo_snapshot(value, Path(os.environ.get('BUILD_INPUTS_DIR', SNAPSHOT)))
    from prepared_vm_environment import frozen_environments, verify_frozen
    for frozen in frozen_environments(value).values():
        verify_frozen(Path(os.environ.get('BUILD_INPUTS_DIR', SNAPSHOT)),
                      frozen, value['upstream']['linux'])
    return value


def geo_assets(upstream):
    release = upstream.get('geo')
    if release is None:
        return {}
    assets = {asset['name']: asset for asset in release['assets'] if asset['name'] in GEO}
    if set(assets) != set(GEO):
        raise ValueError('Dependency snapshot Geo resources are incomplete')
    for asset in assets.values():
        if (not re.fullmatch(r'sha256:[0-9a-f]{64}', asset.get('digest', ''))
                or type(asset.get('size')) is not int or asset['size'] <= 0):
            raise ValueError('Dependency snapshot Geo identity is invalid')
    return assets


def freeze_geo(directory, upstream):
    files = {}
    for name, asset in geo_assets(upstream).items():
        relative = 'resources/' + GEO[name]
        path = directory / relative
        download_asset(asset, path)
        if path.is_symlink() or path.stat().st_size != asset['size'] or sha256(path) != asset['digest'][7:]:
            raise ValueError('Dependency snapshot Geo checksum or size mismatch: ' + name)
        files[relative] = asset['digest'][7:]
    return files


def verify_geo_snapshot(value, directory):
    assets = geo_assets(value.get('upstream', {}))
    expected = {'resources/' + GEO[name]: asset['digest'][7:] for name, asset in assets.items()}
    if value.get('resources', {}) != expected:
        raise ValueError('Dependency snapshot Geo resource inventory mismatch')
    for name, asset in assets.items():
        path = directory / 'resources' / GEO[name]
        if (path.is_symlink() or not path.is_file() or path.stat().st_size != asset['size']
                or sha256(path) != asset['digest'][7:]):
            raise ValueError('Dependency snapshot Geo checksum or size mismatch: ' + name)
    return assets


def component(name):
    value = manifest()
    return value.get('upstream', {}).get(name) if value else None


def resource_release(name, cached, proxy='', version=None):
    frozen = component(name) or approved()['upstream'].get(name)
    if frozen and (version is None or frozen['tag_name'] == version):
        return frozen
    repo = 'MetaCubeX/' + ('mihomo' if name == 'mihomo' else 'meta-rules-dat')
    endpoint = 'tags/' + version if version else 'latest'
    try:
        return read_json('https://api.github.com/repos/' + repo + '/releases/' + endpoint, proxy)
    except (OSError, ValueError, http.client.HTTPException) as error:
        value = json.loads(cached.read_text()) if cached.is_file() else {}
        release = value.get('release')
        # Migrate verified resource receipts created before full upstream
        # metadata was retained. Their URLs and digests suffice to reuse the
        # existing files; prepare-resources still checks every actual file.
        checksum = lambda text: isinstance(text, str) and re.fullmatch(r'[0-9a-fA-F]{64}', text)
        if not release and name == 'mihomo' and value.get('source') and value.get('version') and checksum(value.get('archive_sha256')):
            release = {'tag_name':value['version'], 'published_at':'cached', 'assets':[
                {'name':urllib.parse.unquote(urllib.parse.urlsplit(value['source']).path.rsplit('/',1)[-1]),
                 'browser_download_url':value['source'], 'digest':'sha256:'+value['archive_sha256'].lower()}]}
        if not release and name == 'geo' and value.get('version') and value.get('files'):
            tag, _, published = value['version'].partition(' | ')
            if {entry.get('name') for entry in value['files']} == set(GEO.values()) and all(
                    checksum(entry.get('sha256')) and entry.get('source') and isinstance(entry.get('size'), int)
                    for entry in value['files']):
                release = {'tag_name':tag, 'published_at':published or 'cached', 'assets':[
                    {'name':urllib.parse.unquote(urllib.parse.urlsplit(entry['source']).path.rsplit('/',1)[-1]),
                     'browser_download_url':entry['source'], 'digest':'sha256:'+entry['sha256'].lower(), 'size':entry['size']}
                    for entry in value['files']]}
        if release and (version is None or release.get('tag_name') == version):
            print('::warning::Upstream unavailable; using cached verified ' + name + ' release metadata.')
            return release
        raise RuntimeError(name + ' upstream is unavailable and no verified release metadata is cached') from error


def go_version(value):
    match = re.fullmatch(r'v(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?', value)
    if not match:
        raise ValueError('Invalid Go module version')
    pre = match[4]
    identifiers = tuple((0, int(p)) if p.isdigit() else (1, p) for p in pre.split('.')) if pre else ()
    return (*map(int, match.group(1, 2, 3)), pre is None, identifiers)


def resolve(directory):
    import tomllib

    directory.mkdir(parents=True, exist_ok=True)
    baseline = {path: hashlib.sha256(original(path)).hexdigest() for path in FILES}
    upstream = {name: {**value, 'resolution': 'approved'}
                for name, value in approved()['upstream'].items()}
    files = {}
    for relative in FILES:
        if sha256(ROOT / relative) != baseline[relative]:
            raise ValueError('Source dependency differs from approved repository version: ' + relative)
        destination = directory / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / relative, destination)
        files[relative] = sha256(destination)
    packages = tomllib.loads((ROOT / 'Cargo.lock').read_text())['package']
    go_modules = [list(match) for match in re.findall(
        r'(?m)^(?:require\s+)?\s*([a-zA-Z0-9.-]+\.[a-z]+/\S+)\s+(v\S+)',
        (ROOT / 'apps/android/core/go.mod').read_text())]
    value = {'schema': 1, 'commit': current_commit(), 'baseline': baseline, 'constraints': constraints(), 'files': files,
             'resolved_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
             'upstream': upstream, 'resources': freeze_geo(directory, upstream), 'warnings': [],
             'rust_packages': [{key: package[key] for key in ('name', 'version', 'source', 'checksum') if key in package}
                               for package in packages],
             'go_modules': go_modules,
             'policy': 'Repository lockfiles and approved build versions only; apply the maintainer loop update policy and validate committed changes.'}
    if 'linux' in upstream:
        from prepared_vm_environment import freeze
        from prepared_vm import ARCHITECTURES
        value['prepared_vms'] = {arch: freeze(directory, upstream['linux'], arch)
                                for arch in ARCHITECTURES}
    write_json(directory / 'inputs.json', value)
    report(value)
    if output := os.environ.get('GITHUB_OUTPUT'):
        with open(output, 'a') as stream:
            stream.write('resources=' + resource_key(upstream) + '\n')
            for name, fallback in (('tailscale', '1.104.5'), ('gradle', GRADLE_MIN)):
                version = upstream.get(name, {}).get('version', fallback)
                if not re.fullmatch(r'v?[0-9.]+', version):
                    raise ValueError('Invalid resolved tool version')
                stream.write(name + '=' + version + '\n')
            stream.write('nsis=' + upstream.get('nsis', {}).get('sha256', 'repository-default') + '\n')


def refresh(directory):
    if os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('GITHUB_EVENT_NAME') != 'workflow_dispatch':
        raise ValueError('Dependency refresh requires an explicitly dispatched CI run')
    subprocess.run(['cargo', 'update'], cwd=ROOT, check=True, timeout=600)
    module = ROOT / 'apps/android/core'
    subprocess.run(['go', 'mod', 'edit', '-go=' + tool_versions()['go']], cwd=module, check=True, timeout=30)
    subprocess.run(['go', 'get', '-u', './...'], cwd=module, check=True, timeout=600)
    subprocess.run(['go', 'mod', 'tidy'], cwd=module, check=True, timeout=300)
    directory.mkdir(parents=True, exist_ok=True)
    files = {}
    for relative in ('Cargo.lock', 'apps/android/core/go.mod', 'apps/android/core/go.sum'):
        destination = directory / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / relative, destination)
        files[relative] = sha256(destination)
    write_json(directory / 'updates.json', {
        'schema': 1, 'commit': current_commit(), 'constraints': constraints(), 'files': files,
    })
    print('Updated lockfiles exported for review and a subsequent committed CI run.')


def resource_key(upstream):
    selected = {name: {key: value for key, value in upstream.get(name, {}).items() if key != 'resolution'}
                for name in ('mihomo', 'geo')}
    return hashlib.sha256(json.dumps(selected, sort_keys=True).encode()).hexdigest()


def packaging(directory):
    value = manifest(directory)
    if not value or value.get('commit') != current_commit():
        raise ValueError('Packaging requires the verified CI dependency snapshot')
    report(value)
    if output := os.environ.get('GITHUB_OUTPUT'):
        with open(output, 'a') as stream:
            stream.write('nsis=' + value['upstream'].get('nsis', {}).get('sha256', 'verified-cache') + '\n')
            stream.write('tailscale=' + value['upstream'].get('tailscale', {}).get('version', '1.104.5') + '\n')


def apply(directory, require_commit=True):
    value = manifest(directory)
    if not value or value.get('schema') != 1 or set(value.get('files', {})) != set(FILES):
        raise ValueError('Dependency snapshot is incomplete')
    if require_commit and value.get('commit') != current_commit():
        raise ValueError('Dependency snapshot source commit mismatch')
    if value.get('constraints') != constraints():
        raise ValueError('Dependency snapshot source constraints mismatch')
    for relative, checksum in value['files'].items():
        if hashlib.sha256(original(relative)).hexdigest() != value['baseline'].get(relative):
            raise ValueError('Source constraints changed: ' + relative)
        path = directory / relative
        if path.is_symlink() or sha256(path) != checksum:
            raise ValueError('Dependency snapshot checksum mismatch: ' + relative)
    assets = verify_geo_snapshot(value, directory)
    from prepared_vm_environment import frozen_environments, verify_frozen
    for frozen in frozen_environments(value).values():
        verify_frozen(directory, frozen, value['upstream']['linux'])
    for relative in FILES:
        shutil.copy2(directory / relative, ROOT / relative)
    for name in assets:
        destination = ROOT / 'bundle/resources' / GEO[name]
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(directory / 'resources' / GEO[name], destination)
    print('Applied verified dependency snapshot:', value.get('resolved_at', 'unknown'))


def report(value):
    rows = ['## Build dependency snapshot', '', '| Component | Selected version | Resolution |', '| --- | --- | --- |']
    for name, entry in value['upstream'].items():
        rows.append('| ' + name + ' | ' + entry.get('version', entry.get('tag_name', '—')) + ' | ' + entry['resolution'] + ' |')
    rows += ['', 'Source dependencies: Cargo.lock, Go module/sums and Android Gradle declarations are attached with SHA-256 hashes.',
             'Packaging reuses this tested snapshot without resolving application dependencies again.', '']
    rows += ['- ' + warning for warning in value['warnings']]
    if output := os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(output, 'a') as stream:
            stream.write('\n'.join(rows) + '\n')


def package_contents(build):
    value = json.loads((build / 'manifest.json').read_text())
    tool = ROOT / 'tools/nsis/tool.json'
    return {'ci_dependencies': value.get('build_inputs'), 'ci_toolchain': value.get('toolchain'),
            'payload_files': {path: record for path, record in value['files'].items() if path.startswith('payload/')},
            'installer_compiler': json.loads(tool.read_text()) if tool.is_file() else None}


def toolchain_report():
    """Record the actual runner tools, in addition to resolved dependency versions."""
    tools = {'python': sys.version.split()[0]}
    for name, args in {'rustc': ['-vV'], 'cargo': ['--version'], 'go': ['version'],
                       'java': ['-version'], 'gradle': ['--version']}.items():
        if executable := shutil.which(name):
            result = subprocess.run([executable, *args], capture_output=True, text=True, timeout=60)
            tools[name] = (result.stdout + result.stderr).strip()[:8192]
            if result.returncode:
                tools[name] = 'version query failed: ' + str(result.returncode)
    ndk = os.environ.get('ANDROID_NDK_ROOT')
    if ndk and (path := Path(ndk)/'source.properties').is_file():
        tools['android_ndk'] = path.read_text().strip()
    return tools


def shared_go(directory, restore=False):
    """Restore shared downloads once, then fan them out through this run's artifact."""
    import action_cache
    directory.mkdir(parents=True, exist_ok=True)
    archive = directory / 'go-source.tar.gz'
    receipt = directory / 'receipt.json'
    # Keep canonical downloads rather than both the ZIP and an expanded copy.
    # Go reconstructs/validates module sources locally from these cached ZIPs.
    paths = [Path.home() / 'go/pkg/mod/cache/download', Path.home() / 'go/bin/gomobile', Path.home() / 'go/bin/gobind']
    hashes = {path: sha256(ROOT / path) for path in FILES if path.endswith(('go.mod', 'go.sum'))}
    if restore:
        value = json.loads(receipt.read_text())
        if (value.get('schema') != 1 or value.get('commit') != current_commit()
                or value.get('go_files') != hashes or archive.is_symlink()
                or archive.stat().st_size != value.get('size') or sha256(archive) != value.get('sha256')):
            raise ValueError('Shared Android dependencies differ from this CI snapshot')
        action_cache.unpack(archive, paths)
        print('Restored verified Android source downloads from this CI run.')
        return
    from android import ensure_binding_tool
    subprocess.run(['go', '-C', str(ROOT / 'apps/android/core'), 'mod', 'tidy', '-diff'], check=True, timeout=180)
    subprocess.run(['go', '-C', str(ROOT / 'apps/android/core'), 'mod', 'download'], check=True, timeout=180)
    ensure_binding_tool('gomobile')
    ensure_binding_tool('gobind')
    subprocess.run(['go', '-C', str(ROOT / 'apps/android/core'), 'mod', 'verify'], check=True)
    from build_support import record_download
    downloads = paths[0]
    for pattern in ('**/*.zip', '**/*.mod'):
        for path in downloads.glob(pattern):
            if path.is_file() and not path.is_symlink():
                record_download('https://proxy.golang.org/' + path.relative_to(downloads).as_posix(),
                                path, sha256(path))
    action_cache.pack(paths, archive)
    write_json(receipt, {'schema': 1, 'commit': current_commit(), 'go_files': hashes,
                         'size': archive.stat().st_size, 'sha256': sha256(archive)})
    print(f'Shared Android source archive: {archive.stat().st_size / 1024**2:.1f} MiB')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('resolve', 'refresh', 'apply', 'packaging', 'share-go', 'restore-go', 'audit', 'versions', 'check-tools'))
    parser.add_argument('--directory', type=Path, default=Path(os.environ.get('BUILD_INPUTS_DIR', SNAPSHOT)))
    parser.add_argument('--tools', nargs='+', choices=('go', 'java'), default=('go', 'java'))
    args = parser.parse_args()
    if args.operation == 'audit':
        from dependency_audit import audit
        audit(args.directory)
    elif args.operation == 'versions':
        versions()
    elif args.operation == 'check-tools':
        check_tools(args.tools)
    elif args.operation in ('share-go', 'restore-go'):
        shared_go(args.directory, args.operation == 'restore-go')
    else:
        {'resolve': resolve, 'refresh': refresh, 'apply': apply, 'packaging': packaging}[args.operation](args.directory)


if __name__ == '__main__':
    main()
