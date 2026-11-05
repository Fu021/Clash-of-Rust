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
IMPORT_CAPABILITIES = {'validated-cache-commit-v1', 'complete-cache-retention-v1',
                       'upstream-fetch-v1'}
QUEUE_CAPABILITY = 'upstream-import-queue-v1'
JOB_STATES = {'queued', 'running', 'retry_wait', 'ready', 'failed', 'superseded'}
MANIFEST_LIMIT = 1024
MANIFEST_BYTES = 1024 * 1024


class ImportFailure(RuntimeError):
    def __init__(self, code, http_status=None):
        super().__init__(code)
        self.code = code
        self.http_status = http_status


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
            descriptor, name = tempfile.mkstemp(
                prefix='.' + destination.name + '-', suffix='.partial', dir=destination.parent)
            staged = Path(name)
            try:
                with os.fdopen(descriptor, 'wb') as output, tarfile.open(archive, 'r:gz') as source:
                    entry = source.next()
                    if (entry is None or entry.name != '0' or not entry.isfile()
                            or not 0 < entry.size <= 4 * 1024**3):
                        raise ValueError('Expected a single original download')
                    with source.extractfile(entry) as incoming:
                        action_cache.download_body(config, incoming, output, entry.size)
                    if source.next() is not None:
                        raise ValueError('Unexpected files in original download')
                if sha256(staged) != checksum:
                    raise ValueError('Original download SHA256 mismatch')
                staged.chmod(stat.S_IRUSR | stat.S_IWUSR | stat.S_IRGRP | stat.S_IROTH)
                staged.replace(destination)
            finally:
                staged.unlink(missing_ok=True)
        record(url, destination, checksum)
        print('Original download: private fallback verified (' + destination.name + ')', flush=True)
        return True
    except ERRORS as error:
        print('Original download: private fallback unavailable (' + type(error).__name__ + ')', flush=True)
        return False


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def failure(error):
    result = {'error': error.code if isinstance(error, ImportFailure) else type(error).__name__}
    if isinstance(error, ImportFailure) and error.http_status is not None:
        result['http_status'] = error.http_status
    return result


def request_json(config, path, deadline, payload=None):
    action_cache.restore_remaining({'restore_deadline': deadline})
    config = {**config, 'restore_deadline': min(deadline, time.monotonic() + 8)}
    options = {}
    if payload is not None:
        body = encoded(payload)
        options = {'method': 'POST', 'file': body, 'headers': {
            'Content-Length': str(len(body)), 'Content-Type': 'application/json',
            'Prefer': 'respond-async'}}
    with action_cache.response(config, path, **options) as reply:
        retry_after = reply.getheader('Retry-After', '5') if hasattr(reply, 'getheader') else '5'
        status = reply.status
        request_id = reply.getheader('X-Request-ID', '') if hasattr(reply, 'getheader') else ''
        data = bytearray()
        while True:
            action_cache.restore_remaining(config)
            block = reply.read1(min(64 * 1024, 2 * MANIFEST_BYTES + 1 - len(data)))
            if not block:
                break
            data.extend(block)
            if len(data) > 2 * MANIFEST_BYTES:
                raise ValueError('Oversized import receipt')
    try:
        value = json.loads(data)
    except (ValueError, UnicodeError):
        if status not in (200, 201, 202):
            raise ImportFailure('http_rejected', status) from None
        raise
    if not isinstance(value, dict):
        if status not in (200, 201, 202):
            raise ImportFailure('http_rejected', status)
        raise ValueError('Invalid import receipt')
    if status not in (200, 201, 202):
        code = value.get('error_code', value.get('error', 'http_rejected'))
        if isinstance(code, dict):
            code = code.get('code', 'http_rejected')
        if not isinstance(code, str) or not re.fullmatch(r'[a-z][a-z0-9_]{0,79}', code):
            code = 'http_rejected'
        raise ImportFailure(code, status)
    if re.fullmatch(r'[A-Za-z0-9_-]{1,100}', request_id):
        value['request_id'] = request_id
    return value, max(5, min(60, int(retry_after))) if str(retry_after).isdigit() else 5


def manifest_batches(files, sequence):
    batch = []
    for entry in files:
        item = {name: entry[name] for name in ('url', 'size', 'sha256')}
        candidate = {'schema': 1, 'group': GROUP, 'sequence': sequence,
                     'retry_failed': False, 'files': [*batch, item]}
        if batch and (len(candidate['files']) > MANIFEST_LIMIT or len(encoded(candidate)) > MANIFEST_BYTES):
            yield {'schema': 1, 'group': GROUP, 'sequence': sequence,
                   'retry_failed': False, 'files': batch}
            batch = []
        batch.append(item)
        if len(encoded({'schema': 1, 'group': GROUP, 'sequence': sequence,
                        'retry_failed': False, 'files': batch})) > MANIFEST_BYTES:
            raise ValueError('Original download metadata exceeds manifest limit')
    if batch:
        yield {'schema': 1, 'group': GROUP, 'sequence': sequence,
               'retry_failed': False, 'files': batch}


def manifest_header(value, batch):
    payload = batch['request']
    identifier = value.get('manifest_id')
    counts = value.get('counts')
    if (any(value.get(name) != payload[name] for name in ('schema', 'group', 'sequence'))
            or not isinstance(identifier, str) or not re.fullmatch(r'[0-9a-f]{64}', identifier)
            or batch.get('manifest_id', identifier) != identifier
            or type(value.get('total')) is not int or value['total'] != len(payload['files'])
            or not isinstance(counts, dict) or not set(counts).issubset(JOB_STATES)
            or any(type(number) is not int or number < 0 for number in counts.values())
            or sum(counts.values()) != value['total']
            or not isinstance(value.get('status'), str)
            or not re.fullmatch(r'[a-z_]{1,40}', value['status'])
            or value['status'] == 'ready' and counts.get('ready', 0) != value['total']):
        raise ValueError('Manifest receipt identity or counts mismatch')
    header = {name: value[name] for name in ('schema', 'group', 'sequence', 'manifest_id',
                                            'status', 'total', 'counts')}
    if isinstance(value.get('request_id'), str) and re.fullmatch(r'[A-Za-z0-9_-]{1,100}', value['request_id']):
        header['request_id'] = value['request_id']
    return header


def manifest_page(value, batch, offset):
    header = manifest_header(value, batch)
    containers = [value[name] for name in ('files', 'items', 'jobs') if name in value]
    if len(containers) != 1 or not isinstance(containers[0], list):
        raise ValueError('Missing manifest item receipts')
    entries = {item['sha256']: item for item in batch['request']['files']}
    items = []
    seen = set()
    for entry in containers[0]:
        if not isinstance(entry, dict):
            raise ValueError('Invalid manifest item')
        checksum = str(entry.get('key', '')).removeprefix('sha256-')
        if (checksum not in entries or entry.get('key') != 'sha256-' + checksum
                or entry.get('sequence') != batch['request']['sequence']
                or entry.get('status') not in JOB_STATES or checksum in seen
                or not isinstance(entry.get('job_id'), str)
                or not re.fullmatch(r'[0-9a-f]{64}', entry['job_id'])):
            raise ValueError('Manifest item identity mismatch')
        seen.add(checksum)
        item = {name: entry[name] for name in ('key', 'sequence', 'job_id', 'status')}
        for name in ('error_code', 'error'):
            if isinstance(entry.get(name), str) and re.fullmatch(r'[a-z][a-z0-9_]{0,79}', entry[name]):
                item['error_code'] = entry[name]
        for name in ('attempts', 'next_attempt_at'):
            if type(entry.get(name)) in (int, float) and entry[name] >= 0:
                item[name] = entry[name]
        items.append(item)
    next_offset = value.get('next_offset')
    if (not items and offset < header['total'] or len(items) > 200
            or offset + len(items) > header['total']
            or next_offset is not None and (
                type(next_offset) is not int or next_offset != offset + len(items)
                or next_offset >= header['total'])
            or next_offset is None and offset + len(items) != header['total']):
        raise ValueError('Invalid manifest pagination')
    return {**header, 'offset': offset, 'next_offset': next_offset, 'files': items}


def apply_manifest_page(report, batch, page):
    results = {value['key']: value for value in report['files']}
    for item in page['files']:
        result = results[item['key']]
        result.update(accepted=True, manifest_id=batch['manifest_id'],
                      job_id=item['job_id'], job_status=item['status'],
                      status=item['status'] if item['status'] in ('ready', 'failed', 'superseded') else 'pending')
        result.pop('error', None)
        result.pop('http_status', None)
        if 'error_code' in item:
            result['error_code'] = item['error_code']


def submit_manifests(config, report, persist, deadline, poll_seconds):
    assigned = {entry['sha256'] for batch in report['manifests'] for entry in batch['request']['files']}
    unassigned = [entry for entry in report['files'] if entry['sha256'] not in assigned]
    for payload in manifest_batches(unassigned, report['sequence']):
        report['manifests'].append({'request': payload, 'status': 'not_submitted'})
    persist(report)
    for batch in report['manifests']:
        if batch.get('manifest_id') or time.monotonic() >= deadline:
            continue
        for attempt in range(2):
            try:
                value, retry_after = request_json(config, '/v1/import/manifest', deadline, batch['request'])
                header = manifest_header(value, batch)
                batch.update(manifest_id=header['manifest_id'], status='pending',
                             receipt=header, retry_after=retry_after, receipt_verified=False)
                batch.pop('error', None)
                batch.pop('http_status', None)
                keys = {'sha256-' + item['sha256'] for item in batch['request']['files']}
                for result in report['files']:
                    if result['key'] in keys:
                        result.update(status='pending', accepted=True, manifest_id=batch['manifest_id'])
                        result.pop('error', None)
                        result.pop('http_status', None)
                persist(report)
                page = manifest_page(value, batch, 0)
                batch['pages'] = [page]
                apply_manifest_page(report, batch, page)
                if page['next_offset'] is None:
                    batch['receipt_verified'] = True
                    batch['status'] = ('ready' if header['status'] == 'ready'
                                       and all(item['status'] == 'ready' for item in page['files'])
                                       else 'failed' if any(item['status'] in ('failed', 'superseded')
                                                            for item in page['files'])
                                       else 'pending')
                persist(report)
                break
            except ERRORS as error:
                batch.update(failure(error))
                if not batch.get('manifest_id'):
                    for result in report['files']:
                        if result['sha256'] in {entry['sha256'] for entry in batch['request']['files']}:
                            result.update(status='not_submitted', accepted=False, **failure(error))
                persist(report)
                if (batch.get('manifest_id') or isinstance(error, (ValueError, ImportFailure))
                        or time.monotonic() >= deadline or attempt):
                    break
    poll_deadline = min(deadline, time.monotonic() + poll_seconds)
    for batch in report['manifests']:
        if not batch.get('manifest_id') or time.monotonic() >= poll_deadline:
            continue
        batch.update(status='pending', receipt_verified=False)
        keys = {'sha256-' + item['sha256'] for item in batch['request']['files']}
        for result in report['files']:
            if result['key'] in keys and result['status'] == 'ready':
                result['status'] = 'pending'
        persist(report)
        offset = 0
        pages = []
        seen = set()
        try:
            while True:
                query = urllib.parse.urlencode({'id': batch['manifest_id'], 'offset': offset, 'limit': 100})
                value, retry_after = request_json(
                    config, '/v1/import/manifest?' + query, poll_deadline)
                page = manifest_page(value, batch, offset)
                keys = {entry['key'] for entry in page['files']}
                if keys & seen:
                    raise ValueError('Repeated manifest items across pages')
                seen.update(keys)
                pages.append(page)
                batch.update(pages=pages, receipt=manifest_header(value, batch),
                             retry_after=retry_after, receipt_verified=False, status='pending')
                apply_manifest_page(report, batch, page)
                persist(report)
                if page['next_offset'] is None:
                    break
                offset = page['next_offset']
            batch['receipt_verified'] = len(seen) == len(batch['request']['files'])
            states = [entry['status'] for page in pages for entry in page['files']]
            batch['status'] = ('ready' if batch['receipt_verified']
                               and value['status'] == 'ready'
                               and value['counts'].get('ready', 0) == value['total']
                               and all(state == 'ready' for state in states)
                               else 'failed' if any(state in ('failed', 'superseded') for state in states)
                               else 'pending')
            batch.pop('error', None)
            batch.pop('http_status', None)
        except ERRORS as error:
            batch.update(failure(error))
        persist(report)


def submit_legacy(config, report, persist, deadline):
    for result in report['files']:
        if time.monotonic() >= deadline:
            break
        try:
            config['restore_deadline'] = min(deadline, time.monotonic() + 8)
            if lookup(config, result['sha256']) is not None:
                result['status'] = 'present'
                persist(report)
                continue
            payload = {'schema': 1, 'group': GROUP, 'key': result['key'], 'sequence': report['sequence'],
                       'format': 'indexed-tar-gzip-v1',
                       'files': [{'path': '0', 'url': result['url'], 'size': result['size'],
                                  'sha256': result['sha256'], 'mode': 420}]}
            receipt, _retry_after = request_json(config, '/v1/import', deadline, payload)
            if (any(receipt.get(name) != payload[name] for name in ('schema', 'group', 'key', 'sequence'))
                    or receipt.get('status') not in ('queued', 'ready')):
                raise ValueError('Upstream import receipt mismatch')
            result.update(status=receipt['status'], accepted=True)
        except ERRORS as error:
            result.update(status='unavailable', **failure(error))
        persist(report)


def submit(output, budget_seconds=60, poll_seconds=15, summary=True):
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
              'rejected': rejected, 'manifests': [], 'files': [
                  {**value, 'key': 'sha256-' + checksum, 'path': '0',
                   'status': 'not_submitted', 'accepted': False}
                  for checksum, value in sorted(entries.items())]}
    if output.is_file():
        try:
            previous = json.loads(output.read_text())
            if all(previous.get(name) == report[name] for name in ('schema', 'group', 'sequence')):
                old = {item['key']: item for item in previous['files']}
                for entry in report['files']:
                    candidate = old.get(entry['key'], {})
                    if all(candidate.get(name) == entry[name] for name in ('url', 'size', 'sha256')):
                        entry.update(candidate)
                report['manifests'] = previous.get('manifests', [])
        except ERRORS:
            pass
    persist(report)
    deadline = time.monotonic() + budget_seconds
    try:
        config = configuration()
        if config:
            config['restore_deadline'] = min(deadline, time.monotonic() + 8)
        capabilities = action_cache.server_capabilities(config) if config and entries else set()
    except ERRORS as error:
        report['error'] = failure(error)
        config, capabilities = {}, set()
    enabled = IMPORT_CAPABILITIES.issubset(capabilities)
    report['import_enabled'] = enabled
    report['capabilities'] = sorted(capabilities)
    report['protocol'] = ('manifest-queue-v1' if QUEUE_CAPABILITY in capabilities and enabled
                          else 'legacy-import' if enabled else 'disabled')
    if enabled:
        if QUEUE_CAPABILITY in capabilities:
            submit_manifests(config, report, persist, deadline, poll_seconds)
        else:
            submit_legacy(config, report, persist, deadline)
    persist(report)
    counts = {status: sum(value['status'] == status for value in report['files'])
              for status in sorted({value['status'] for value in report['files']})}
    message = ('Original download manifest (' + report['protocol'] + '): '
               + json.dumps(counts, sort_keys=True)
               + '; pending/queued is not ready or a verified restore; no runner archives uploaded.')
    print(message, flush=True)
    if summary and (summary_path := os.environ.get('GITHUB_STEP_SUMMARY')):
        with Path(summary_path).open('a', encoding='utf-8') as stream:
            stream.write('\n' + message + '\n')
            for batch in report['manifests']:
                stream.write(f"\nManifest `{batch.get('manifest_id', 'unconfirmed')}`: "
                             f"{batch['status']}, {len(batch['request']['files'])} files, "
                             f"identity/pagination verified: {batch.get('receipt_verified', False)}.\n")
            stream.write('\n| Original URL | Bytes | SHA256 | Import |\n| --- | ---: | --- | --- |\n')
            for value in report['files']:
                stream.write(f"| {value['url']} | {value['size']} | {value['sha256']} | {value['status']} |\n")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('submit',))
    parser.add_argument('--output', type=Path, default=ROOT / 'target/upstream-downloads.json')
    parser.add_argument('--budget-seconds', type=int, choices=range(1, 61), default=60, metavar='1..60')
    parser.add_argument('--poll-seconds', type=int, choices=range(0, 31), default=15, metavar='0..30')
    parser.add_argument('--no-summary', action='store_true')
    arguments = parser.parse_args()
    submit(arguments.output, arguments.budget_seconds, arguments.poll_seconds, not arguments.no_summary)


if __name__ == '__main__':
    main()
