"""Mirror verified original downloads by URL; never upload runner archives."""
import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import stat
import tarfile
import tempfile
import time
import urllib.parse

import action_cache
from build_support import ROOT, sha256, write_json

GROUP = 'upstream/files'
HOSTS = {
    'github.com', 'api.github.com', 'cloud-images.ubuntu.com',
    'services.gradle.org', 'downloads.gradle.org', 'sourceforge.net',
    'pkgs.tailscale.com', 'go.dev', 'dl.google.com', 'static.rust-lang.org',
    'proxy.golang.org', 'repo.maven.apache.org', 'archive.ubuntu.com',
    'security.ubuntu.com', 'ports.ubuntu.com',
}
ERRORS = (OSError, ValueError, RuntimeError, KeyError, TypeError, tarfile.TarError,
          http.client.HTTPException)


def valid_source(url, checksum, size=None):
    try:
        parsed = urllib.parse.urlsplit(url)
        return (parsed.scheme == 'https' and parsed.hostname in HOSTS
                and not parsed.username and not parsed.password
                and not parsed.query and not parsed.fragment and parsed.port in (None, 443)
                and parsed.path.startswith('/') and not re.search(r'[\s\\|]', url)
                and re.fullmatch(r'[0-9a-f]{64}', checksum) is not None
                and (size is None or type(size) is int and 0 < size <= 4 * 1024**3))
    except (TypeError, ValueError):
        return False


def directory():
    return Path(os.environ.get('RUNNER_TEMP', tempfile.gettempdir())) / 'cor-upstream-downloads'


def record(url, destination, checksum):
    destination = Path(destination)
    if (os.environ.get('GITHUB_ACTIONS') != 'true' or not os.environ.get('RUNNER_TEMP')
            or destination.is_symlink() or not destination.is_file()):
        return
    size = destination.stat().st_size
    if not valid_source(url, checksum, size):
        return
    identity = hashlib.sha256((url + '\n' + checksum).encode()).hexdigest()
    write_json(directory() / (identity + '.json'),
               {'url': url, 'sha256': checksum, 'size': size})


def configuration():
    path = os.environ.get('COR_UPSTREAM_CACHE_CONFIG')
    if not path:
        return {}
    value = json.loads(Path(path).read_text())
    if not isinstance(value, dict):
        raise ValueError('Invalid original download configuration')
    return value if value.get('url') and value.get('token') else {}


def lookup(config, checksum):
    key = 'sha256-' + checksum
    query = urllib.parse.urlencode({'group': GROUP, 'key': key})
    with action_cache.response(config, '/v1/lookup?' + query) as reply:
        if reply.status == 404:
            return None
        if reply.status != 200:
            raise RuntimeError(f'Upstream lookup HTTP {reply.status}')
        value = json.load(reply)
    if (not isinstance(value, dict) or value.get('schema') != 1 or value.get('group') != GROUP or value.get('key') != key
            or not re.fullmatch(r'[0-9a-f]{64}', value.get('id', ''))
            or not re.fullmatch(r'[0-9a-f]{64}', value.get('sha256', ''))
            or type(value.get('size')) is not int or not 0 < value['size'] <= 5 * 1024**3):
        raise ValueError('Invalid original download metadata')
    return value


def restore(url, destination, checksum, deadline=None):
    if not valid_source(url, checksum):
        return False
    try:
        config = configuration()
        if not config:
            return False
        config['restore_deadline'] = min(time.monotonic() + 120, deadline) if deadline else time.monotonic() + 120
        action_cache.restore_remaining(config)
        value = lookup(config, checksum)
        if value is None:
            return False
        destination = Path(destination)
        destination.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='.cor-upstream-', dir=destination.parent) as temporary:
            archive = Path(temporary) / 'download.tar.gz'
            query = urllib.parse.urlencode({'group': GROUP, 'id': value['id']})
            with action_cache.response(config, '/v1/blob?' + query) as reply:
                if reply.status != 200:
                    raise RuntimeError(f'Upstream blob HTTP {reply.status}')
                with archive.open('wb') as output:
                    action_cache.download_body(config, reply, output, value['size'])
            if archive.stat().st_size != value['size'] or sha256(archive) != value['sha256']:
                raise ValueError('Original download envelope checksum mismatch')
            staged = Path(temporary) / 'original'
            with tarfile.open(archive, 'r:gz') as source:
                entry = source.next()
                if (entry is None or entry.name != '0' or not entry.isfile()
                        or not 0 < entry.size <= 4 * 1024**3):
                    raise ValueError('Expected a single original download')
                with source.extractfile(entry) as incoming, staged.open('wb') as output:
                    action_cache.download_body(config, incoming, output, entry.size)
                if source.next() is not None:
                    raise ValueError('Unexpected files in original download')
            if sha256(staged) != checksum:
                raise ValueError('Original download SHA256 mismatch')
            staged.chmod(stat.S_IRUSR | stat.S_IWUSR | stat.S_IRGRP | stat.S_IROTH)
            staged.replace(destination)
        record(url, destination, checksum)
        print('Original download: private fallback verified (' + destination.name + ')', flush=True)
        return True
    except ERRORS as error:
        print('Original download: private fallback unavailable (' + type(error).__name__ + ')', flush=True)
        return False


def submit(output):
    def persist(report):
        output.parent.mkdir(parents=True, exist_ok=True)
        action_cache.write_json(output, report)

    entries = {}
    rejected = 0
    for path in sorted(directory().glob('*.json')):
        try:
            value = json.loads(path.read_text())
            value = {name: value[name] for name in ('url', 'sha256', 'size')}
            if not valid_source(value['url'], value['sha256'], value['size']):
                raise ValueError('Invalid original download receipt')
            previous = entries.get(value['sha256'])
            if previous and previous['size'] != value['size']:
                raise ValueError('Conflicting original download receipt')
            entries[value['sha256']] = value
        except ERRORS:
            rejected += 1
    sequence = int(os.environ.get('GITHUB_RUN_ID', '0')) * 1000 + min(
        int(os.environ.get('GITHUB_RUN_ATTEMPT', '0')), 999)
    report = {'schema': 1, 'group': GROUP, 'sequence': sequence,
              'rejected': rejected, 'files': [
                  {**value, 'key': 'sha256-' + checksum, 'path': '0', 'status': 'not_submitted'}
                  for checksum, value in sorted(entries.items())]}
    persist(report)
    deadline = time.monotonic() + 60
    try:
        config = configuration()
        if config:
            config['restore_deadline'] = deadline
        capabilities = action_cache.server_capabilities(config) if config and entries else set()
    except ERRORS:
        config, capabilities = {}, set()
    enabled = {'validated-cache-commit-v1', 'complete-cache-retention-v1',
               'upstream-fetch-v1'}.issubset(capabilities)
    report['import_enabled'] = enabled
    report['capabilities'] = sorted(capabilities)
    for result in report['files']:
        checksum = result['sha256']
        if not enabled or time.monotonic() >= deadline:
            continue
        config['restore_deadline'] = deadline
        try:
            if lookup(config, checksum) is not None:
                result['status'] = 'present'
                persist(report)
                continue
            payload = {'schema': 1, 'group': GROUP, 'key': result['key'], 'sequence': sequence,
                       'format': 'indexed-tar-gzip-v1',
                       'files': [{'path': '0', 'url': result['url'], 'size': result['size'],
                                  'sha256': checksum, 'mode': 420}]}
            body = json.dumps(payload).encode()
            headers = {'Content-Length': str(len(body)), 'Content-Type': 'application/json'}
            with action_cache.response(config, '/v1/import', method='POST', file=body, headers=headers) as reply:
                if reply.status not in (200, 201, 202):
                    raise RuntimeError(f'Upstream import HTTP {reply.status}')
                receipt = json.load(reply)
            if (not isinstance(receipt, dict)
                    or any(receipt.get(name) != payload[name] for name in ('schema', 'group', 'key', 'sequence'))
                    or receipt.get('status') not in ('queued', 'ready')):
                raise ValueError('Upstream import receipt mismatch')
            result['status'] = receipt['status']
        except ERRORS as error:
            result['status'] = 'unavailable'
            result['error'] = type(error).__name__
            if status := re.search(r'HTTP ([0-9]{3})', str(error)):
                result['http_status'] = int(status[1])
        persist(report)
    persist(report)
    counts = {status: sum(value['status'] == status for value in report['files'])
              for status in sorted({value['status'] for value in report['files']})}
    message = 'Original download manifest: ' + json.dumps(counts, sort_keys=True) + (
        '; queued imports are not verified restores; no runner archives uploaded.')
    print(message, flush=True)
    if summary := os.environ.get('GITHUB_STEP_SUMMARY'):
        with Path(summary).open('a', encoding='utf-8') as stream:
            stream.write('\n' + message + '\n')
            stream.write('\n| Original URL | Bytes | SHA256 | Import |\n| --- | ---: | --- | --- |\n')
            for value in report['files']:
                stream.write(f"| {value['url']} | {value['size']} | {value['sha256']} | {value['status']} |\n")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('submit',))
    parser.add_argument('--output', type=Path, default=ROOT / 'target/upstream-downloads.json')
    arguments = parser.parse_args()
    submit(arguments.output)


if __name__ == '__main__':
    main()
