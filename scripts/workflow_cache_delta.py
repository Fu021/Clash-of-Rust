"""Store branch compiler changes relative to a verified main cache."""

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import tempfile
import time


MAX_FILES = 200_000
MAX_BLOB_BYTES = 256 * 1024**2
MAX_MANIFEST_BYTES = 64 * 1024**2
CHUNK = 1024 * 1024
TARGET = re.compile(r'(?:aarch64|armv7|x86_64)-[a-z0-9_-]+')


def checksum(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(CHUNK), b''):
            digest.update(block)
    return digest.hexdigest()


def safe_relative(value):
    if not isinstance(value, str) or not value or len(value) > 4096:
        raise ValueError('Invalid relative cache path')
    path = PurePosixPath(value)
    if (not path.parts or path.is_absolute() or str(path) != value or
            any(part in ('.', '..') or '\\' in part or ':' in part or '\0' in part
                for part in path.parts)):
        raise ValueError('Invalid relative cache path')
    return path


def target_file(relative):
    parts = safe_relative(relative).parts
    if any(part in ('incremental', '__pycache__') for part in parts):
        return False
    if parts[-1].endswith('.lock') and parts[-1] != 'Cargo.lock':
        return False
    if parts[0] not in ('release', 'debug', '.rustc_info.json',
                        'CACHEDIR.TAG', 'workspace-sources.json'):
        if not TARGET.fullmatch(parts[0]) or len(parts) < 2 or parts[1] not in ('release', 'debug'):
            return False
    build_script = 'build' in parts[:-1]
    return not ((parts[-1].endswith('.exe') and not build_script)
                or (len(parts) > 1 and parts[-2] == 'deps' and '.' not in parts[-1])
                or parts[-1] in ('clash-of-rust', 'clash-desktop-ui', 'clash-tun-launcher',
                                 'cor', 'libclash_mobile.so'))


def root_map(roots):
    result = {}
    for name, path in roots.items():
        if not re.fullmatch(r'[a-z][a-z0-9_-]{0,31}', name):
            raise ValueError('Invalid cache root name')
        root = Path(path).absolute()
        if root.is_symlink():
            raise ValueError('Cache root must not be a symlink')
        result[name] = root
    if not result:
        raise ValueError('At least one cache root is required')
    resolved = [root.resolve() for root in result.values()]
    if any(left == right or left.is_relative_to(right) or right.is_relative_to(left)
           for index, left in enumerate(resolved) for right in resolved[index + 1:]):
        raise ValueError('Cache roots must not overlap')
    return result


def collect(roots):
    records = {}
    for name, root in sorted(root_map(roots).items()):
        if not root.exists():
            continue
        if not root.is_dir():
            raise ValueError('Cache root must be a directory')
        for directory, subdirectories, files in os.walk(root, followlinks=False):
            parent = Path(directory)
            subdirectories[:] = sorted(
                value for value in subdirectories
                if not (parent / value).is_symlink() and value not in ('incremental', '__pycache__'))
            for filename in sorted(files):
                path = parent / filename
                relative = path.relative_to(root).as_posix()
                if name == 'target' and not target_file(relative):
                    continue
                safe_relative(relative)
                before = path.lstat()
                if not stat.S_ISREG(before.st_mode):
                    continue
                digest = checksum(path)
                after = path.lstat()
                if (not stat.S_ISREG(after.st_mode) or
                        (before.st_size, before.st_mtime_ns, before.st_mode) !=
                        (after.st_size, after.st_mtime_ns, after.st_mode)):
                    raise ValueError('Compiler object changed during snapshot')
                records[name + '/' + relative] = {
                    'sha256': digest, 'size': before.st_size,
                    'mode': stat.S_IMODE(before.st_mode), 'mtime_ns': before.st_mtime_ns,
                }
                if len(records) > MAX_FILES:
                    raise ValueError('Compiler cache file limit exceeded')
    return records


def identity(records):
    normalized = {
        key: {field: value[field] for field in ('sha256', 'size', 'mode')}
        for key, value in sorted(records.items())
    }
    return hashlib.sha256(json.dumps(normalized, sort_keys=True,
                                     separators=(',', ':')).encode()).hexdigest()


def snapshot(roots):
    files = collect(roots)
    return {'schema': 1, 'base': identity(files), 'files': files}


def resolve_key(roots, key):
    path = safe_relative(key)
    if len(path.parts) < 2 or path.parts[0] not in roots:
        raise ValueError('Unknown cache root')
    root = roots[path.parts[0]]
    relative = PurePosixPath(*path.parts[1:])
    if path.parts[0] == 'target' and not target_file(str(relative)):
        raise ValueError('Non-compiler target path')
    current = root
    for part in relative.parts:
        current = current / part
        if current.is_symlink():
            raise ValueError('Cache destination contains a symlink')
    if not current.resolve().is_relative_to(root.resolve()):
        raise ValueError('Cache path escaped its root')
    return current


def validate_record(record):
    if (not isinstance(record, dict) or set(record) != {'sha256', 'size', 'mode', 'mtime_ns'} or
            not isinstance(record['sha256'], str) or
            not re.fullmatch(r'[0-9a-f]{64}', record['sha256']) or
            type(record['size']) is not int or record['size'] < 0 or
            type(record['mode']) is not int or not 0 <= record['mode'] <= 0o777 or
            type(record['mtime_ns']) is not int or record['mtime_ns'] < 0):
        raise ValueError('Invalid cache file record')


def verify_snapshot(value, roots):
    if (not isinstance(value, dict) or value.get('schema') != 1 or
            not isinstance(value.get('files'), dict) or len(value['files']) > MAX_FILES):
        raise ValueError('Invalid baseline')
    for key, record in value['files'].items():
        resolve_key(roots, key)
        validate_record(record)
    if value.get('base') != identity(value['files']):
        raise ValueError('Invalid baseline identity')


def pack(roots, base, destination):
    roots = root_map(roots)
    verify_snapshot(base, roots)
    current = collect(roots)
    changed = {}
    metadata_only = {}
    removed = sorted(set(base['files']) - set(current))
    for key, record in current.items():
        old = base['files'].get(key)
        if not old or any(record[field] != old[field] for field in ('sha256', 'size', 'mode')):
            changed[key] = record
        elif record['mtime_ns'] != old['mtime_ns']:
            metadata_only[key] = record
    total = sum({record['sha256']: record['size'] for record in changed.values()}.values())
    if total > MAX_BLOB_BYTES:
        return {'saved': False, 'reason': 'delta_too_large', 'bytes': total}
    if not (changed or metadata_only or removed):
        return {'saved': False, 'reason': 'unchanged', 'bytes': 0}
    destination = Path(destination)
    if destination.exists() or destination.is_symlink():
        raise ValueError('Delta staging directory already exists')
    destination.mkdir(parents=True)
    try:
        blobs = destination / 'blobs'
        blobs.mkdir()
        for key, record in changed.items():
            path = resolve_key(roots, key)
            blob = blobs / record['sha256']
            if not blob.exists():
                shutil.copyfile(path, blob)
                if blob.stat().st_size != record['size'] or checksum(blob) != record['sha256']:
                    raise ValueError('Compiler object changed during snapshot')
        payload = {'schema': 1, 'base': base['base'], 'changed': changed,
                   'metadata': metadata_only, 'removed': removed}
        manifest = json.dumps(payload, sort_keys=True, separators=(',', ':')).encode()
        if len(manifest) > MAX_MANIFEST_BYTES:
            raise ValueError('Delta manifest exceeds limit')
        (destination / 'manifest.json').write_bytes(manifest)
        return {'saved': True, 'bytes': total, 'changed': len(changed), 'removed': len(removed)}
    except BaseException:
        shutil.rmtree(destination)
        raise


def read_json(path):
    path = Path(path)
    if path.is_symlink() or not path.is_file() or path.stat().st_size > MAX_MANIFEST_BYTES:
        raise ValueError('Invalid cache manifest')
    return json.loads(path.read_text(encoding='utf-8'))


def apply(roots, base, directory):
    roots = root_map(roots)
    verify_snapshot(base, roots)
    directory = Path(directory)
    blobs_directory = directory / 'blobs'
    if (directory.is_symlink() or not directory.is_dir() or
            blobs_directory.is_symlink() or not blobs_directory.is_dir()):
        raise ValueError('Invalid delta directory')
    manifest = read_json(directory / 'manifest.json')
    if (not isinstance(manifest, dict) or set(manifest) !=
            {'schema', 'base', 'changed', 'metadata', 'removed'} or
            manifest['schema'] != 1 or manifest['base'] != base['base'] or
            not isinstance(manifest['changed'], dict) or
            not isinstance(manifest['metadata'], dict) or
            not isinstance(manifest['removed'], list)):
        raise ValueError('Delta does not match its baseline')
    changed, metadata, removed = manifest['changed'], manifest['metadata'], manifest['removed']
    if (len(changed) + len(metadata) + len(removed) > MAX_FILES or
            any(not isinstance(key, str) for key in removed) or
            len(set(removed)) != len(removed) or
            set(changed) & set(metadata) or set(changed) & set(removed) or
            set(metadata) & set(removed)):
        raise ValueError('Conflicting delta paths')
    blob_paths, blob_sizes = {}, {}
    total = 0
    for key, record in [*changed.items(), *metadata.items()]:
        validate_record(record)
        resolve_key(roots, key)
        if key in metadata:
            old = base['files'].get(key)
            if not old or any(record[field] != old[field] for field in ('sha256', 'size', 'mode')):
                raise ValueError('Invalid metadata-only change')
        else:
            blob = blobs_directory / record['sha256']
            prior_size = blob_sizes.setdefault(record['sha256'], record['size'])
            if prior_size != record['size']:
                raise ValueError('Conflicting delta blob sizes')
            if record['sha256'] not in blob_paths:
                if (blob.is_symlink() or not blob.is_file() or
                        blob.stat().st_size != record['size'] or checksum(blob) != record['sha256']):
                    raise ValueError('Delta blob failed verification')
                blob_paths[record['sha256']] = blob
                total += record['size']
    if total > MAX_BLOB_BYTES:
        raise ValueError('Delta exceeds limit')
    for key in removed:
        resolve_key(roots, key)
        if key not in base['files']:
            raise ValueError('Delta deletes an unknown baseline object')
    for key in set(changed) | set(metadata) | set(removed):
        path = resolve_key(roots, key)
        if key not in base['files']:
            if path.exists():
                raise ValueError('New delta destination appeared after the baseline snapshot')
            continue
        old = base['files'][key]
        if not path.is_file() or path.stat().st_size != old['size'] or checksum(path) != old['sha256']:
            raise ValueError('Baseline changed before delta application')
    for key, record in changed.items():
        path = resolve_key(roots, key)
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(dir=path.parent, prefix='.cor-delta-', delete=False) as stream:
                temporary = Path(stream.name)
                with blob_paths[record['sha256']].open('rb') as incoming:
                    shutil.copyfileobj(incoming, stream, CHUNK)
            temporary.chmod(record['mode'])
            os.utime(temporary, ns=(record['mtime_ns'], record['mtime_ns']))
            temporary.replace(path)
        finally:
            if temporary:
                temporary.unlink(missing_ok=True)
    for key, record in metadata.items():
        path = resolve_key(roots, key)
        os.utime(path, ns=(record['mtime_ns'], record['mtime_ns']))
    for key in removed:
        resolve_key(roots, key).unlink(missing_ok=True)
    return {'changed': len(changed), 'removed': len(removed), 'bytes': total}


def output(name, value):
    if destination := os.environ.get('GITHUB_OUTPUT'):
        with Path(destination).open('a', encoding='utf-8') as stream:
            stream.write(f'{name}={value}\n')


def compatibility(source, base):
    source = Path(source)
    tracked = subprocess.check_output(
        ['git', 'ls-files', '-z', '--', 'Cargo.lock', 'Cargo.toml', '**/Cargo.toml',
         '.cargo', '.github/build-versions.json', 'scripts/android.py', 'scripts/run-ci.py',
         'scripts/linux_ci.py', 'scripts/run-memory-benchmark.py'], cwd=source, timeout=30,
    ).decode('utf-8').split('\0')
    digest = hashlib.sha256()
    digest.update(base.encode())
    digest.update(subprocess.check_output(['rustc', '-vV'], cwd=source, timeout=30))
    keys = {'RUSTFLAGS', 'RUSTUP_TOOLCHAIN', 'MACOSX_DEPLOYMENT_TARGET', 'CC', 'CXX', 'AR'}
    keys.update(key for key in os.environ if key.startswith((
        'CARGO_BUILD_', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_INCREMENTAL',
        'CARGO_PROFILE_', 'CARGO_TARGET_', 'CC_', 'CXX_', 'AR_', 'CFLAGS_', 'CXXFLAGS_',
    )))
    for key in sorted(keys):
        digest.update(json.dumps([key, os.environ.get(key, '')]).encode())
    for name in sorted(filter(None, tracked)):
        path = source / name
        if path.is_file() and not path.is_symlink():
            digest.update(name.encode())
            digest.update(checksum(path).encode())
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('snapshot', 'apply', 'pack'))
    parser.add_argument('--source', default='.')
    parser.add_argument('--sources', default='')
    parser.add_argument('--state', required=True)
    parser.add_argument('--directory', default='')
    args = parser.parse_args()
    roots = {'target': Path(args.source) / 'target'}
    if args.sources:
        roots['sources'] = Path(args.sources)
    started = time.monotonic()
    if args.operation == 'snapshot':
        value = snapshot(roots)
        if not any('/deps/' in key and key.endswith(('.rlib', '.rmeta', '.a'))
                   for key in value['files']):
            print('No restored compiler baseline; branch overlay skipped.')
            return
        encoded = json.dumps(value, sort_keys=True, separators=(',', ':')).encode()
        if len(encoded) > MAX_MANIFEST_BYTES:
            raise ValueError('Compiler baseline exceeds manifest limit')
        compatible = compatibility(args.source, value['base'])
        state = Path(args.state)
        state.parent.mkdir(parents=True, exist_ok=True)
        state.write_bytes(encoded)
        output('compatibility', compatible)
        output('ready', 'true')
        result = {'files': len(value['files']), 'base': value['base']}
    else:
        if not args.directory:
            raise ValueError('Delta directory is required')
        if args.operation == 'pack':
            value = read_json(args.state)
            result = pack(roots, value, args.directory)
            output('saved', str(result['saved']).lower())
        else:
            try:
                value = read_json(args.state)
                result = apply(roots, value, args.directory)
            except (OSError, ValueError, TypeError, KeyError):
                if os.environ.get('GITHUB_ACTIONS') != 'true':
                    raise
                from action_cache import discard_restore
                discard_restore({'kind': 'rust', 'source': args.source})
                output('applied', 'false')
                print('::warning::Branch compiler changes were rejected; the compiler cache '
                      'was discarded and the build will reconstruct it.', flush=True)
                return
            output('applied', 'true')
    print(json.dumps({'operation': args.operation, 'seconds': round(time.monotonic() - started, 2),
                      **result}), flush=True)


if __name__ == '__main__':
    main()
