"""Read upstream metadata without resolving, downloading or changing dependencies."""
from concurrent.futures import ThreadPoolExecutor
import datetime
from functools import lru_cache
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import time
import tomllib
import urllib.request
import xml.etree.ElementTree as ET

from build_support import ROOT, opener, write_json
from build_inputs import approved, go_version


def metadata(url, raw=False):
    headers = {'User-Agent': 'Clash-of-Rust-dependency-audit'}
    if url.startswith('https://api.github.com/') and os.environ.get('GH_TOKEN'):
        headers['Authorization'] = 'Bearer ' + os.environ['GH_TOKEN']
    with opener().open(urllib.request.Request(url, headers=headers), timeout=8) as reply:
        content = reply.read(8 * 1024**2 + 1)
    if len(content) > 8 * 1024**2:
        raise ValueError('Upstream metadata exceeded its size limit')
    return content if raw else json.loads(content)


def stable(value):
    return bool(re.fullmatch(r'v?\d+\.\d+\.\d+(?:\+[0-9A-Za-z.-]+)?', value))


def cargo_candidate(name, current):
    if len(name) == 1:
        prefix = '1'
    elif len(name) == 2:
        prefix = '2'
    elif len(name) == 3:
        prefix = '3/' + name[0]
    else:
        prefix = name[:2] + '/' + name[2:4]
    url = 'https://index.crates.io/' + prefix + '/' + name.lower()
    entries = [json.loads(line) for line in metadata(url, raw=True).splitlines()]
    candidates = [entry['vers'] for entry in entries if not entry.get('yanked') and stable(entry['vers'])]
    latest = max(candidates, key=lambda version: go_version('v' + version))
    withdrawn = any(entry['vers'] == current and entry.get('yanked') for entry in entries)
    return latest, 'current version withdrawn; assess replacement' if withdrawn else '', url


def go_candidate(name, current):
    escaped = ''.join('!' + character.lower() if character.isupper() else character for character in name)
    url = 'https://proxy.golang.org/' + escaped + '/@latest'
    latest = metadata(url)['Version']
    declaration = metadata('https://proxy.golang.org/' + escaped + '/@v/' + latest + '.mod', raw=True).decode()
    reasons = []
    if re.search(r'(?m)^//\s*Deprecated:', declaration):
        reasons.append('upstream module deprecated; assess replacement')
    if re.search(r'(?m)^retract\b', declaration):
        reasons.append('upstream declares retractions; check whether the current version is affected')
    return latest, '; '.join(reasons), url


def inventory():
    selected = approved()
    entries = []
    lock = tomllib.loads((ROOT / 'Cargo.lock').read_text())
    for package in lock['package']:
        if package.get('source', '').startswith('registry+'):
            entries.append(('cargo', package['name'], package['version'], cargo_candidate))
        elif package.get('source', '').startswith('git+'):
            entries.append(('manual', package['name'], package['source'], None))
    for name, version in re.findall(r'(?m)^(?:require\s+)?\s*([a-zA-Z0-9.-]+\.[a-z]+/\S+)\s+(v\S+)',
                                    (ROOT / 'apps/android/core/go.mod').read_text()):
        entries.append(('go', name, version, go_candidate))
    for path in (ROOT / '.github').rglob('*.yml'):
        for name, version in re.findall(r'(?m)^\s*-?\s*uses:\s*([A-Za-z0-9_.-]+/[A-Za-z0-9_./-]+)@([^\s#]+)', path.read_text()):
            entries.append(('action', '/'.join(name.split('/')[:2]), version, action_candidate))
    for path in ('apps/android/build.gradle.kts', 'apps/android/app/build.gradle.kts'):
        text = (ROOT / path).read_text()
        for name, version in re.findall(r'"(androidx\.[^"]+):([0-9.]+)"', text):
            entries.append(('maven', name, version, maven_candidate))
        if match := re.search(r'id\("com.android.application"\) version "([0-9.]+)"', text):
            entries.append(('maven', 'com.android.tools.build:gradle', match[1], maven_candidate))
    for name, version in selected['tools'].items():
        entries.append(('tool', name, version, tool_candidate))
    for name, version in (('ndk', '28.2.13676358'), ('build-tools', '36.0.0'), ('platforms', '36')):
        entries.append(('sdk', name, version, android_candidate))
    for name in ('emulator', 'platform-tools', 'cmdline-tools'):
        root = Path(os.environ.get('ANDROID_HOME', '/nonexistent')) / name
        if name == 'cmdline-tools':
            root /= 'latest'
        properties = root / 'source.properties'
        match = re.search(r'(?m)^Pkg.Revision\s*=\s*(\S+)', properties.read_text()) if properties.is_file() else None
        entries.append(('sdk', name, match[1] if match else 'runner-managed', android_candidate))
    for name in ('mihomo', 'geo', 'nsis', 'linux'):
        value = selected['upstream'][name]
        current = value.get('published_at') if name == 'geo' else value.get('image', value.get('version', value.get('tag_name')))
        entries.append(('resource', name, current, resource_candidate))
    return list({(kind, name, current): (kind, name, current, query)
                 for kind, name, current, query in entries}.values())


@lru_cache(maxsize=1)
def android_metadata():
    return ET.fromstring(metadata('https://dl.google.com/android/repository/repository2-3.xml', raw=True))


def android_candidate(name, current):
    candidates = []
    for entry in android_metadata().iter():
        if entry.tag.rsplit('}', 1)[-1] != 'remotePackage':
            continue
        path = entry.get('path', '')
        if path.split(';')[0] != name or any(child.tag.rsplit('}', 1)[-1] == 'obsolete' for child in entry):
            continue
        channel = next((child.get('ref') for child in entry if child.tag.rsplit('}', 1)[-1] == 'channelRef'), 'channel-0')
        if channel != 'channel-0':
            continue
        if name in ('ndk', 'build-tools', 'cmdline-tools'):
            version = path.split(';', 1)[1]
        elif name == 'platforms':
            version = path.split(';android-', 1)[1]
        else:
            revision = next(child for child in entry if child.tag.rsplit('}', 1)[-1] == 'revision')
            parts = {child.tag.rsplit('}', 1)[-1]: child.text for child in revision}
            version = '.'.join(parts.get(part, '0') for part in ('major', 'minor', 'micro'))
        if re.fullmatch(r'\d+(?:\.\d+)*', version):
            candidates.append(version)
    latest = max(candidates, key=lambda version: tuple(map(int, version.split('.'))))
    reason = 'installed revision requires the corresponding runner runtime manifest' if current == 'runner-managed' else ''
    return latest, reason, 'https://dl.google.com/android/repository/repository2-3.xml'


def action_candidate(name, current):
    url = 'https://api.github.com/repos/' + name
    repo = metadata(url)
    notes = []
    if repo.get('archived') or repo.get('disabled'):
        notes.append('upstream archived/disabled; assess replacement')
    if not re.fullmatch(r'[0-9a-f]{40}', current):
        notes.append('floating action reference; assess immutable commit pin')
    tags = metadata(url + '/tags?per_page=30')
    versions = [entry['name'] for entry in tags if stable(entry['name'])]
    latest = max(versions, key=lambda version: go_version(version if version.startswith('v') else 'v' + version)) if versions else current
    if versions:
        entry = next(entry for entry in tags if entry['name'] == latest)
        if entry.get('commit', {}).get('sha') == current:
            latest = current
    elif reference := approved().get('actions', {}).get(name):
        latest = metadata(url + '/commits/' + reference)['sha']
    return latest, '; '.join(notes), url


def maven_candidate(name, current):
    group, artifact = name.split(':')
    url = 'https://dl.google.com/dl/android/maven2/' + group.replace('.', '/') + '/' + artifact + '/maven-metadata.xml'
    root = ET.fromstring(metadata(url, raw=True))
    versions = [entry.text for entry in root.findall('./versioning/versions/version')
                if entry.text and re.fullmatch(r'\d+\.\d+(?:\.\d+)?', entry.text)]
    return max(versions, key=lambda version: tuple(map(int, version.split('.')))), '', url


def tool_candidate(name, current):
    if name == 'rust':
        url = 'https://static.rust-lang.org/dist/channel-rust-stable.toml'
        value = tomllib.loads(metadata(url, raw=True).decode())
        return value['pkg']['rust']['version'].split()[0], '', url
    if name == 'go':
        url = 'https://go.dev/dl/?mode=json'
        return metadata(url)[0]['version'].removeprefix('go'), '', url
    if name.startswith('python'):
        url = 'https://raw.githubusercontent.com/actions/python-versions/main/versions-manifest.json'
        system = 'win32' if name.startswith('python_windows') else 'darwin' if name == 'python_macos' else 'linux'
        architectures = {'arm64'} if name.endswith('_arm64') else {'x64'} if system == 'win32' else {'x64', 'arm64'}
        versions = [entry['version'] for entry in metadata(url)
                    if entry.get('stable') and entry['version'].startswith('.'.join(current.split('.')[:2]) + '.')
                    and architectures.issubset({item['arch'] for item in entry['files'] if item['platform'] == system})]
        return max(versions, key=lambda version: go_version('v' + version)), '', url
    if name == 'java':
        url = 'https://api.adoptium.net/v3/assets/latest/17/hotspot?architecture=x64&image_type=jdk&os=linux'
        return metadata(url)[0]['version']['semver'], '', url
    if name.startswith('tailscale'):
        url = 'https://pkgs.tailscale.com/stable/?mode=json'
        return metadata(url)['Version'], '', url
    if name == 'gradle':
        url = 'https://services.gradle.org/versions/current'
        return metadata(url)['version'], '', url
    raise ValueError('Unknown tool')


def resource_candidate(name, current):
    if name in ('mihomo', 'geo'):
        repo = 'MetaCubeX/' + ('mihomo' if name == 'mihomo' else 'meta-rules-dat')
        url = 'https://api.github.com/repos/' + repo + '/releases/latest'
        value = metadata(url)
        return value['published_at'] if name == 'geo' else value['tag_name'], '', url
    if name == 'nsis':
        url = 'https://api.github.com/repos/microsoft/winget-pkgs/contents/manifests/n/NSIS/NSIS'
        versions = [entry['name'] for entry in metadata(url) if re.fullmatch(r'\d+\.\d+(?:\.\d+)?', entry['name'])]
        return max(versions, key=lambda version: tuple(map(int, version.split('.')))), '', url
    from linux_ci import resolve_inputs
    value = resolve_inputs(fetch=metadata, read=lambda url: metadata(url, raw=True).decode())
    changed = value != approved()['upstream']['linux']
    return value['image'], 'Linux environment download metadata changed; review Python/cloud digests' if changed else '', 'https://cloud-images.ubuntu.com/focal/'


def audit(directory):
    directory.mkdir(parents=True, exist_ok=True)
    entries = inventory()
    deadline = time.monotonic() + 180

    def check(entry):
        kind, name, current, query = entry
        record = {'kind': kind, 'name': name, 'current': current}
        if query is None or time.monotonic() >= deadline:
            return {**record, 'status': 'not_checked', 'reason': 'manual source or audit time budget exhausted'}
        try:
            candidate, reason, source = query(name, current)
            newer = candidate != current
            if kind in ('cargo', 'go'):
                newer = go_version(candidate if candidate.startswith('v') else 'v' + candidate) > go_version(
                    current if current.startswith('v') else 'v' + current)
            status = 'not_checked' if current == 'runner-managed' else 'review' if newer or reason else 'current'
            return {**record, 'status': status,
                    'candidate': candidate, 'reason': reason, 'source': source}
        except (OSError, ValueError, KeyError, TypeError, IndexError, ET.ParseError, http.client.HTTPException) as error:
            return {**record, 'status': 'unavailable', 'reason': type(error).__name__}

    with ThreadPoolExecutor(max_workers=4) as workers:
        records = list(workers.map(check, entries))
    result = {'schema': 1, 'checked_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'inventory_sha256': hashlib.sha256(json.dumps([entry[:3] for entry in entries]).encode()).hexdigest(),
              'policy': 'Report only; user approval required before upgrades or replacements.',
              'results': records}
    write_json(directory / 'dependency-audit.json', result)
    counts = {status: sum(record['status'] == status for record in records)
              for status in ('current', 'review', 'unavailable', 'not_checked')}
    rows = ['# Dependency review candidates', '', result['policy'], '',
            ', '.join(f'{status}: {count}' for status, count in counts.items()), '',
            '| Kind | Dependency | Current | Candidate | Reason |',
            '| --- | --- | --- | --- | --- |']
    for record in records:
        if record['status'] != 'current':
            fields = [record['kind'], record['name'], record['current'], record.get('candidate', '—'),
                      record.get('reason') or 'newer upstream version; compatibility requires review']
            rows.append('| ' + ' | '.join(str(field).replace('|', '\\|').replace('\n', ' ') for field in fields) + ' |')
    report = '\n'.join(rows) + '\n'
    (directory / 'dependency-audit.md').write_text(report)
    if summary := os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(summary, 'a') as stream:
            stream.write(report)
    print('Dependency audit:', counts, '; no versions or lockfiles changed.', flush=True)
