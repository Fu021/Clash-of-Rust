"""Incremental Rust mirrors over the existing authenticated cache protocol.

An immutable, checksummed index names 32 independently verified archives. Only
changed shards travel back to the private host; the index is published last.
Completed shards can be reused on the next run. Missing/evicted shards fall back
to Cargo after invalidating build-script success markers, so generated sources
are recreated too. No server upgrade or broader filesystem access is needed.
"""
from concurrent.futures import ThreadPoolExecutor
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import tarfile
import tempfile
import time
import urllib.parse

import action_cache as cache

COUNT = 32


def lookup(config, group, key, prefixes=()):
    query = urllib.parse.urlencode({'group': group, 'key': key, 'prefix': '\n'.join(prefixes)})
    with cache.response(config, '/v1/lookup?' + query) as reply:
        if reply.status == 404:
            return None
        if reply.status != 200:
            raise RuntimeError(f'Cache lookup HTTP {reply.status}')
        value = json.load(reply)
    validate(value, group)
    if value['key'] != key and not any(value['key'].startswith(p) for p in prefixes):
        raise ValueError('Rust shard generation mismatch')
    return value


def validate(record, group):
    if record.get('schema') != cache.SCHEMA or record.get('group') != group:
        raise ValueError('Invalid Rust cache metadata')
    for field in ('id', 'sha256'):
        if not isinstance(record.get(field), str) or not cache.re.fullmatch('[0-9a-f]{64}', record[field]):
            raise ValueError('Invalid Rust cache checksum')
    if not isinstance(record.get('size'), int) or not 0 < record['size'] <= 8 * 1024**3:
        raise ValueError('Invalid Rust cache size')


def download(config, record, destination):
    query = urllib.parse.urlencode({'group': record['group'], 'id': record['id']})
    with cache.response(config, '/v1/blob?' + query) as reply:
        if reply.status != 200:
            raise RuntimeError(f'Cache download HTTP {reply.status}')
        with destination.open('wb') as output:
            remaining = record['size']
            while remaining:
                data = reply.read(min(cache.CHUNK, remaining))
                if not data:
                    raise ValueError('Incomplete Rust cache shard')
                output.write(data)
                remaining -= len(data)
            if reply.read(1):
                raise ValueError('Oversized Rust cache shard')
    if cache.digest(destination) != record['sha256']:
        raise ValueError('Rust cache shard checksum mismatch')


def invalidate_build_scripts(paths):
    """Cargo cannot detect missing arbitrary files in a cached OUT_DIR.

    Independently evicted shards may remove generated sources while retaining
    the build-script success fingerprint. Re-run those scripts before trusting
    partial mirrors; keep compiled libraries and build-script executables.
    """
    stale = set()
    for root in paths:
        for profile in (root/'release', *root.glob('*/release')):
            fingerprints = profile/'.fingerprint'
            if fingerprints.is_symlink() or not fingerprints.is_dir():
                continue
            for pattern in ('*/run-build-script-*', '*/*/run-build-script-*'):
                for marker in fingerprints.glob(pattern):
                    if marker.is_file() and not marker.is_symlink() and not marker.parent.is_symlink():
                        stale.add(marker.parent)
    for directory in sorted(stale):
        shutil.rmtree(directory)
    print(f'Partial Rust cache: invalidated {len(stale)} build-script fingerprints; '
          'compiled dependency objects retained.', flush=True)


def restore(config, github):
    with cache.phase(config, 'lookup'):
        record = lookup(config, config['group'] + '/index', config['key'], config['prefix'])
    if record is None:
        return False  # Use the existing full archive during migration.
    if record['size'] > 1024 * 1024:
        raise ValueError('Rust cache index exceeds limit')
    paths = [Path(p) for p in config['resolved_paths']]
    with tempfile.TemporaryDirectory(dir=os.environ.get('RUNNER_TEMP')) as temporary:
        index = Path(temporary) / 'index.json'
        download(config, record, index)
        value = json.loads(index.read_text())
        if value.get('format') != 'rust-shards-v1' or len(value.get('shards', [])) != COUNT:
            raise ValueError('Invalid Rust cache index')
        for i, shard in enumerate(value['shards']):
            if shard is not None:
                validate(shard, f'{config["group"]}/p{i:02d}')
        config.update(available=True, shard_index=value, mirror_required=not cache.private_current(config, record),
                      private_exact=record['key'] == config['key'])
        if github:
            cache.cache_report(config, 'restore', 'GitHub restored; incremental mirror checked')
            return True
        missing = []
        def extract(item):
            i, shard = item
            if shard is None:
                return 0
            archive = Path(temporary) / f'{i}.tar.gz'
            try:
                download(config, shard, archive)
            except RuntimeError as error:
                if str(error) != 'Cache download HTTP 404':
                    raise
                # A concurrent successful build or LRU eviction can replace
                # one shard. Keep the remaining verified dependencies useful.
                missing.append(i)
                return 0
            config['restore_started'] = True
            cache.unpack(archive, paths)
            archive.unlink()
            return shard['size']
        # Limit load on the 1-core cache host; only byte serving is done there.
        with cache.phase(config, 'download'), ThreadPoolExecutor(max_workers=2) as workers:
            size = sum(workers.map(extract, enumerate(value['shards'])))
    config.update(restored=bool(size), hit=not missing and record['key'] == config['key'])
    if missing:
        config['mirror_required'] = True
        invalidate_build_scripts(paths)
    cache.cache_report(config, 'restore', 'partial mirror; Cargo fills missing shards' if missing
                       else 'verified incremental mirror', size)
    return True


def partition(paths):
    """Stable path buckets; timestamps alone never force a dependency upload."""
    groups = [[] for _ in range(COUNT)]
    include = cache.archive_filter(paths, rust=True)
    with tarfile.open(fileobj=io.BytesIO(), mode='w') as tar:
        def walk(path, name):
            if path.is_symlink():
                return
            info = include(tar.gettarinfo(str(path), arcname=name))
            if info is None:
                return
            if path.is_dir():
                for child in sorted(path.iterdir()):
                    walk(child, name + '/' + child.name)
            elif path.is_file():
                index = int(hashlib.sha256(name.encode()).hexdigest()[:8], 16) % COUNT
                groups[index].append((path, info, cache.digest(path)))
        for i, root in enumerate(paths):
            if root.exists():
                walk(root, str(i))
    return groups


def upload(config, group, key, path):
    size, checksum = path.stat().st_size, cache.digest(path)
    query = urllib.parse.urlencode({'group': group, 'key': key})
    sequence = int(os.environ.get('GITHUB_RUN_ID', '0')) * 1000 + min(int(os.environ.get('GITHUB_RUN_ATTEMPT', '0')), 999)
    headers = {'Content-Length': str(size), 'X-Content-SHA256': checksum,
               'X-Cache-Sequence': str(sequence), 'Content-Type': 'application/octet-stream'}
    with path.open('rb') as stream, cache.response(config, '/v1/blob?' + query, method='PUT', file=stream, headers=headers) as reply:
        if reply.status == 409:
            return lookup(config, group, key)
        if reply.status not in (200, 201):
            raise RuntimeError(f'Cache upload HTTP {reply.status}')
    return {'schema': cache.SCHEMA, 'group': group, 'key': key,
            'id': hashlib.sha256(key.encode()).hexdigest(), 'size': size, 'sha256': checksum}


def save(config, paths):
    with cache.phase(config, 'fingerprint'):
        groups = partition(paths)
    deadline = time.monotonic() + 240
    records = [None] * COUNT
    previous = config.get('shard_index', {}).get('shards', [None] * COUNT)
    def save_one(item):
        i, files = item
        if not files:
            return i, None, 0, True
        if time.monotonic() > deadline - 30:
            return i, None, 0, False
        group = f'{config["group"]}/p{i:02d}'
        fingerprint = hashlib.sha256()
        for _, info, checksum in files:
            fingerprint.update(json.dumps([info.name, info.mode, info.size, checksum]).encode())
        key = config['prefix'][-1] + fingerprint.hexdigest()
        # Look up each exact shard, including successful pieces of an interrupted
        # previous migration. Never discard a usable archive before replacement.
        record = previous[i] if previous[i] and previous[i]['key'] == key else lookup(config, group, key)
        if record:
            return i, record, 0, True
        with tempfile.TemporaryDirectory(dir=os.environ.get('RUNNER_TEMP')) as temporary:
            archive = Path(temporary) / 'part.tar.gz'
            with tarfile.open(archive, 'w:gz', compresslevel=1) as tar:
                for path, info, _ in files:
                    with path.open('rb') as source:
                        tar.addfile(info, source)
            record = upload(config, group, key, archive)
            return i, record, archive.stat().st_size, record is not None
    with cache.phase(config, 'upload'), ThreadPoolExecutor(max_workers=2) as workers:
        results = list(workers.map(save_one, enumerate(groups)))
    complete = all(result[3] for result in results)
    sent = sum(result[2] for result in results)
    if not complete:
        cache.cache_report(config, 'save', 'partial shards saved; previous index retained', sent)
        return
    for i, record, _, _ in results:
        records[i] = record
    with tempfile.TemporaryDirectory(dir=os.environ.get('RUNNER_TEMP')) as temporary:
        index = Path(temporary) / 'index.json'
        index.write_text(json.dumps({'format': 'rust-shards-v1', 'shards': records}))
        saved = upload(config, config['group'] + '/index', config['key'], index)
    cache.cache_report(config, 'save', 'incremental mirror committed' if saved else 'newer mirror retained', sent)
