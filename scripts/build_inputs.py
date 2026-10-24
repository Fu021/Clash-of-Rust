"""Resolve one dependency snapshot per CI, then reuse it unchanged for packaging."""
import argparse
import base64
from concurrent.futures import ThreadPoolExecutor
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
import tomllib
import urllib.error
import urllib.parse
import xml.etree.ElementTree as ET

from build_support import ROOT, read_json, opener, sha256, write_json

FILES = ('Cargo.lock', 'apps/android/core/go.mod', 'apps/android/core/go.sum',
         'apps/android/build.gradle.kts', 'apps/android/app/build.gradle.kts')
SNAPSHOT = ROOT / 'target/dependency-snapshot'
GEO = {'geoip.dat': 'GeoIP.dat', 'geosite.dat': 'GeoSite.dat',
       'country.mmdb': 'Country.mmdb', 'GeoLite2-ASN.mmdb': 'ASN.mmdb'}
# AGP 9.4 requires Gradle 9.6 or newer; stay within its supported major.
GRADLE_MIN = '9.6.0'


def original(path):
    return subprocess.check_output(['git', 'show', 'HEAD:' + path], cwd=ROOT)


def current_commit():
    return subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()


def constraints():
    paths = subprocess.check_output(['git', 'ls-files', 'Cargo.toml', 'apps/**/Cargo.toml',
                                     'crates/**/Cargo.toml', '.cargo/*', 'apps/android/gradle.properties'],
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
    return value


def component(name):
    value = manifest()
    return value.get('upstream', {}).get(name) if value else None


def resource_release(name, cached, proxy='', version=None):
    frozen = component(name)
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


def version_tuple(value):
    if not re.fullmatch(r'\d+(?:\.\d+){1,3}', value):
        raise ValueError('Expected a stable numeric version')
    return tuple(map(int, value.split('.')))


def stable_release(repo):
    release = read_json('https://api.github.com/repos/' + repo + '/releases/latest')
    if release.get('draft') or release.get('prerelease') or not release.get('tag_name'):
        raise ValueError('No stable release: ' + repo)
    # Retain immutable asset IDs/digests and URLs; never cache access credentials.
    return {'tag_name': release['tag_name'], 'published_at': release['published_at'],
            'assets': [{key: asset[key] for key in ('id', 'name', 'size', 'digest', 'browser_download_url')}
                       for asset in release['assets'] if asset.get('digest')]}


def nsis_release():
    base = 'https://api.github.com/repos/microsoft/winget-pkgs/contents/manifests/n/NSIS/NSIS'
    entries = read_json(base)
    version = max((entry['name'] for entry in entries if entry['type'] == 'dir'
                   and re.fullmatch(r'3\.\d+(?:\.\d+)?', entry['name'])), key=version_tuple)
    entry = read_json(base + '/' + version + '/NSIS.NSIS.installer.yaml')
    text = base64.b64decode(entry['content'], validate=False).decode()
    url = re.search(r'(?m)^\s*InstallerUrl:\s*(https://\S+)\s*$', text)
    checksum = re.search(r'(?m)^\s*InstallerSha256:\s*([A-Fa-f0-9]{64})\s*$', text)
    if not url or not checksum or not url[1].startswith('https://sourceforge.net/projects/nsis/files/'):
        raise ValueError('NSIS official installer/verification metadata is missing')
    return {'version': version, 'url': url[1], 'sha256': checksum[1].lower(),
            'verification_source': entry['html_url']}


def gradle_release():
    versions = read_json('https://services.gradle.org/versions/all')
    major = GRADLE_MIN.split('.')[0]
    release = max((entry for entry in versions
                   if re.fullmatch(re.escape(major) + r'\.\d+(?:\.\d+)?', entry['version'])
                   and version_tuple(entry['version']) >= version_tuple(GRADLE_MIN)
                   and not entry.get('snapshot') and not entry.get('broken')),
                  key=lambda e: version_tuple(e['version']))
    return {'version': release['version'], 'url': release['downloadUrl'],
            'checksum_url': release['checksumUrl']}


def maven_latest(group, artifact, current):
    path = group.replace('.', '/') + '/' + artifact + '/maven-metadata.xml'
    with opener().open('https://dl.google.com/dl/android/maven2/' + path, timeout=20) as response:
        root = ET.fromstring(response.read(1024 * 1024))
    major = current.split('.')[0]
    versions = [entry.text for entry in root.findall('./versioning/versions/version')
                if entry.text and re.fullmatch(re.escape(major) + r'\.\d+(?:\.\d+)?', entry.text)]
    return max(versions, key=version_tuple)


def refresh_command(command, paths, warnings):
    before = {path: (ROOT / path).read_bytes() if (ROOT / path).exists() else None for path in paths}
    print('Checking source dependencies:', ' '.join(command), flush=True)
    try:
        environment = dict(os.environ)
        if command[0] == 'go':
            # Avoid falling through from the public module proxy to unbounded
            # git/vanity-host probes for every transitive dependency.
            environment.setdefault('GOPROXY', 'https://proxy.golang.org')
        result = subprocess.run(command, cwd=ROOT, text=True, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, timeout=90, env=environment)
    except subprocess.TimeoutExpired as error:
        partial = error.stdout or ''
        if isinstance(partial, bytes):
            partial = partial.decode(errors='replace')
        result = subprocess.CompletedProcess(command, 1, partial + '\nDependency metadata refresh timed out after 90s')
    if not result.returncode:
        print('Refreshed source dependencies:', ' '.join(command), flush=True)
        return
    for path, value in before.items():
        if value is None:
            (ROOT / path).unlink(missing_ok=True)
        else:
            (ROOT / path).write_bytes(value)
    network = re.compile(r'timed? ?out|timeout|network|connection|resolve host|no such host|'
                         r'name resolution|no route to host|download|'
                         r'HTTP.*(?:40[3489]|429|50[0-9])|status code.*(?:40[3489]|429|50[0-9])|'
                         r'failed to fetch|unable to access|TLS|SSL|unreachable|'
                         r'\b429\b.*Too Many Requests|'
                         r'\b50[0-9]\b.*(?:Unavailable|Gateway|Error)', re.I)
    if not network.search(result.stdout):
        print(result.stdout, flush=True)
        raise RuntimeError('Dependency resolution failed; this was not a network fallback')
    warning = 'Upstream unavailable; retained compatible cached/repository dependencies: ' + ' '.join(command)
    warnings.append(warning)
    print('::warning::' + warning, flush=True)
    # Keep the upstream host/reason when falling back; a generic warning hid
    # repeated Go timeouts in earlier runs. Never publish URL credentials.
    detail = re.sub(r'(https?://)[^\s/@]+:[^\s/@]+@', r'\1[redacted]@', result.stdout)
    print('\n'.join(detail.splitlines()[-12:])[-4096:], flush=True)


def refresh_go():
    warnings = []
    go = ['go', '-C', str(ROOT / 'apps/android/core')]
    files = ('apps/android/core/go.mod', 'apps/android/core/go.sum')
    refresh_command([*go, 'get', '-tool', 'golang.org/x/mobile/cmd/gomobile@latest',
                     'golang.org/x/mobile/cmd/gobind@latest'], files, warnings)
    # Query the declared module paths directly. `go get -u ./...` also probes
    # alternative parent modules for packages (e.g. go4.org/netipx -> go4.org),
    # repeatedly hanging on unrelated archives despite a complete source cache.
    modules = json.loads(subprocess.check_output([*go, 'mod', 'edit', '-json'], text=True, timeout=15))['Require']
    def latest(module):
        escaped = ''.join('!' + c.lower() if c.isupper() else c for c in module['Path'])
        try:
            with opener().open('https://proxy.golang.org/' + escaped + '/@latest', timeout=8) as reply:
                version = json.load(reply)['Version']
            if go_version(version) > go_version(module['Version']):
                return module['Path'] + '@' + version, None
            return None, None
        except (OSError, ValueError, KeyError, http.client.HTTPException) as error:
            return None, module['Path'] + ': latest metadata unavailable (' + type(error).__name__ + '); retained cached version'
    with ThreadPoolExecutor(max_workers=8) as workers:
        selected = list(workers.map(latest, modules))
    updates = []
    for update, warning in selected:
        if update:
            updates.append(update)
        if warning:
            warnings.append(warning)
            print('::warning::' + warning, flush=True)
    if updates:
        refresh_command([*go, 'get', *updates], files, warnings)
    else:
        print('Checked all declared Go modules; no newer available versions.', flush=True)
    refresh_command([*go, 'mod', 'tidy'], files, warnings)
    return warnings


def go_version(value):
    match = re.fullmatch(r'v(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?', value)
    if not match:
        raise ValueError('Invalid Go module version')
    pre = match[4]
    identifiers = tuple((0, int(p)) if p.isdigit() else (1, p) for p in pre.split('.')) if pre else ()
    return (*map(int, match.group(1, 2, 3)), pre is None, identifiers)


def resolve(directory):
    directory.mkdir(parents=True, exist_ok=True)
    previous = manifest(directory) or {}
    baseline = {path: hashlib.sha256(original(path)).hexdigest() for path in FILES}
    warnings = []
    reusable = previous.get('baseline') == baseline and previous.get('constraints') == constraints()
    if reusable:
        apply(directory, require_commit=False)
        print('Restored source dependency snapshot for the same source constraints.')
    upstream = {}
    queries = {'mihomo': lambda: stable_release('MetaCubeX/mihomo'),
               'geo': lambda: stable_release('MetaCubeX/meta-rules-dat'),
               'nsis': nsis_release, 'gradle': gradle_release,
               'tailscale': lambda: {'version': read_json('https://pkgs.tailscale.com/stable/?mode=json')['Version']}}
    with ThreadPoolExecutor(max_workers=len(queries) + 2) as workers:
        rust_warnings = []
        rust = workers.submit(refresh_command, ['cargo', 'update'], ('Cargo.lock',), rust_warnings)
        go = workers.submit(refresh_go)
        pending = {name: workers.submit(query) for name, query in queries.items()}
        print('Checking upstream metadata:', ', '.join(queries), flush=True)
        for name, future in pending.items():
            try:
                value = future.result()
                upstream[name] = {**value, 'resolution': 'upstream'}
                print(name + ': ' + value.get('version', value.get('tag_name', 'resolved')), flush=True)
            except (OSError, ValueError, KeyError, TypeError, http.client.HTTPException) as error:
                cached = previous.get('upstream', {}).get(name)
                if cached:
                    upstream[name] = {**cached, 'resolution': 'cache'}
                    warning = name + ': upstream unavailable (' + type(error).__name__ + '); using cached version'
                else:
                    warning = name + ': no upstream metadata or previous snapshot; repository/tool default is required'
                warnings.append(warning)
                print('::warning::' + warning, flush=True)
        rust.result()
        warnings.extend(rust_warnings)
        warnings.extend(go.result())
    for path, group, artifact, pattern in [
        ('apps/android/build.gradle.kts', 'com.android.tools.build', 'gradle',
         r'(id\("com.android.application"\) version ")([0-9.]+)(")'),
        ('apps/android/app/build.gradle.kts', 'androidx.test', 'runner',
         r'("androidx.test:runner:)([0-9.]+)(")'),
        ('apps/android/app/build.gradle.kts', 'androidx.test.ext', 'junit',
         r'("androidx.test.ext:junit:)([0-9.]+)(")'),
    ]:
        target = ROOT / path
        text = target.read_text()
        match = re.search(pattern, text)
        if not match:
            raise ValueError('Android dependency declaration missing: ' + artifact)
        try:
            updated = maven_latest(group, artifact, match[2])
            target.write_text(re.sub(pattern, lambda m: m[1] + updated + m[3], text))
        except (OSError, ValueError, ET.ParseError, http.client.HTTPException) as error:
            warnings.append(artifact + ': upstream unavailable (' + type(error).__name__ + '); retained source snapshot')
    files = {}
    for relative in FILES:
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
             'upstream': upstream, 'warnings': warnings,
             'rust_packages': [{key: package[key] for key in ('name', 'version', 'source', 'checksum') if key in package}
                               for package in packages],
             'go_modules': go_modules,
             'policy': 'Latest versions within declared source constraints; network errors use a compatible previous snapshot.'}
    write_json(directory / 'inputs.json', value)
    report(value)
    if output := os.environ.get('GITHUB_OUTPUT'):
        with open(output, 'a') as stream:
            stream.write('resources=' + resource_key(upstream) + '\n')
            for name, fallback in (('tailscale', '1.94.2'), ('gradle', GRADLE_MIN)):
                version = upstream.get(name, {}).get('version', fallback)
                if not re.fullmatch(r'v?[0-9.]+', version):
                    raise ValueError('Invalid resolved tool version')
                stream.write(name + '=' + version + '\n')
            stream.write('nsis=' + upstream.get('nsis', {}).get('sha256', 'repository-default') + '\n')


def resource_key(upstream):
    selected = {name: {key: value for key, value in upstream.get(name, {}).items() if key != 'resolution'}
                for name in ('mihomo', 'geo')}
    return hashlib.sha256(json.dumps(selected, sort_keys=True).encode()).hexdigest()


def packaging(directory):
    value = manifest(directory)
    if not value or value.get('commit') != current_commit():
        raise ValueError('Packaging requires the verified CI dependency snapshot')
    try:
        value['upstream']['nsis'] = {**nsis_release(), 'resolution': 'upstream'}
    except (OSError, ValueError, KeyError, http.client.HTTPException) as error:
        if value['upstream'].get('nsis'):
            value['upstream']['nsis']['resolution'] = 'cache'
        value['warnings'].append('Installer upstream unavailable; using verified CI tool metadata.')
    write_json(directory / 'inputs.json', value)
    report(value)
    if output := os.environ.get('GITHUB_OUTPUT'):
        with open(output, 'a') as stream:
            stream.write('nsis=' + value['upstream'].get('nsis', {}).get('sha256', 'verified-cache') + '\n')
            stream.write('tailscale=' + value['upstream'].get('tailscale', {}).get('version', '1.94.2') + '\n')


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
    for relative in FILES:
        shutil.copy2(directory / relative, ROOT / relative)
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
    subprocess.run(['go', '-C', str(ROOT / 'apps/android/core'), 'mod', 'download'], check=True, timeout=180)
    ensure_binding_tool('gomobile')
    ensure_binding_tool('gobind')
    subprocess.run(['go', '-C', str(ROOT / 'apps/android/core'), 'mod', 'verify'], check=True)
    action_cache.pack(paths, archive)
    write_json(receipt, {'schema': 1, 'commit': current_commit(), 'go_files': hashes,
                         'size': archive.stat().st_size, 'sha256': sha256(archive)})
    print(f'Shared Android source archive: {archive.stat().st_size / 1024**2:.1f} MiB')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('resolve', 'apply', 'packaging', 'share-go', 'restore-go'))
    parser.add_argument('--directory', type=Path, default=Path(os.environ.get('BUILD_INPUTS_DIR', SNAPSHOT)))
    args = parser.parse_args()
    if args.operation in ('share-go', 'restore-go'):
        shared_go(args.directory, args.operation == 'restore-go')
    else:
        {'resolve': resolve, 'apply': apply, 'packaging': packaging}[args.operation](args.directory)


if __name__ == '__main__':
    main()
