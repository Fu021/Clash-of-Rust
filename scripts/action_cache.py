"""Private Actions cache client/server; compilation always stays on the runner."""
import argparse
from contextlib import contextmanager
import hashlib
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import ipaddress
import json
import logging
from logging.handlers import RotatingFileHandler
import os
from pathlib import Path, PurePosixPath
import re
import secrets
import shutil
import socket
import sqlite3
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request

SCHEMA = 1
CHUNK = 1024 * 1024
DEFAULT_ROOT = Path('/home/coder/github_action_cache')
RESTORE_SECONDS = 300


class RestoreBudgetExceeded(TimeoutError):
    pass


def restore_remaining(config):
    deadline = config.get('restore_deadline')
    if deadline is None:
        return None
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise RestoreBudgetExceeded('Private cache restore budget exhausted')
    return remaining


def download_body(config, reply, output, length, label=None):
    """Read available bytes so trickling traffic cannot defeat the wall clock."""
    remaining = length
    started = last_progress = time.monotonic()
    while remaining:
        restore_remaining(config)
        block = reply.read1(min(CHUNK, remaining))
        if not block:
            raise ValueError('Incomplete cache download')
        output.write(block)
        remaining -= len(block)
        now = time.monotonic()
        if label and now - last_progress >= 10:
            print(f'Private cache {label}: {(length-remaining)/1024**2:.1f}/{length/1024**2:.1f} MiB, '
                  f'{(length-remaining)/1024/max(now-started, 0.001):.0f} KiB/s', flush=True)
            last_progress = now
    if reply.read1(1):
        raise ValueError('Oversized cache download')


@contextmanager
def phase(config, name):
    started = time.monotonic()
    try:
        yield
    finally:
        seconds = round(time.monotonic() - started, 3)
        config.setdefault('timings', {})[name] = seconds
        print(f'Private cache {config["group"]}: {name} {seconds:.3f}s', flush=True)


def digest(path):
    value = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(CHUNK), b''):
            value.update(block)
    return value.hexdigest()


def fingerprint(paths):
    value = hashlib.sha256()
    for index, root in enumerate(paths):
        root = Path(root)
        for path in sorted(root.rglob('*')) if root.is_dir() else [root]:
            parts = path.relative_to(root).parts if path != root else (path.name,)
            if (path.is_file() and not path.is_symlink()
                    and not any(p in ('incremental', '__pycache__') for p in parts)
                    and (not path.name.endswith('.lock') or path.name == 'Cargo.lock')):
                value.update(str(index).encode())
                value.update((path.relative_to(root).as_posix() if path != root else '.').encode())
                value.update(digest(path).encode())
    return value.hexdigest()


def cache_report(config, operation, result, size=0):
    # Allowlist public fields. The action state also contains a credential.
    record = {'group': config['group'], 'key': config['key'], 'operation': operation,
              'result': result, 'bytes': size, 'run_id': os.environ.get('GITHUB_RUN_ID', ''),
              'commit': os.environ.get('GITHUB_SHA', '')}
    phases = ('lookup', 'download', 'verify', 'extract', 'fingerprint', 'compress', 'upload')
    record['timings_seconds'] = {name: config['timings'][name] for name in phases if name in config.get('timings', {})}
    if config.get('started') is not None:
        record['timings_seconds']['total'] = round(time.monotonic() - config['started'], 3)
    directory = Path(os.environ.get('GITHUB_WORKSPACE', '.'))/'target/cache-report'
    directory.mkdir(parents=True, exist_ok=True)
    name = hashlib.sha256((config['group']+operation).encode()).hexdigest()[:16]+'.json'
    write_json(directory/name, record)
    if summary := os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(summary, 'a') as output:
            output.write('\n### Private cache '+operation+'\n\n'
                         '| Group | Result | Size | Key |\n| --- | --- | --- | --- |\n'
                         f'| {record["group"]} | {result} | {size/1024**2:.1f} MiB | {record["key"]} |\n')
            if record['timings_seconds']:
                output.write('\n| Phase | Seconds |\n| --- | ---: |\n')
                for name,seconds in record['timings_seconds'].items():
                    output.write(f'| {name} | {seconds:.3f} |\n')


def write_json(path, value):
    path = Path(path)
    temporary = path.with_name(path.name + '.new')
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding='utf-8')
    temporary.chmod(0o600)
    temporary.replace(path)


def valid_group(group):
    parts = group.split('/')
    if not 1 <= len(parts) <= 3 or any(not re.fullmatch(r'[a-z0-9][a-z0-9_-]{0,63}', p) for p in parts):
        raise ValueError('Invalid cache group')
    return group


def valid_key(key):
    if not key or len(key) > 512 or any(ord(c) < 33 or ord(c) > 126 for c in key):
        raise ValueError('Invalid cache key')
    return key


def rust_environment_key(compiler, environment):
    value = hashlib.sha256(compiler)
    for name, item in sorted(environment.items()):
        if name.startswith(('RUST', 'CARGO_PROFILE_', 'CARGO_TARGET_', 'CC_', 'CXX_')):
            value.update((name+'='+item).encode())
    return f'v{SCHEMA}-' + value.hexdigest() + '-'


def workspace_sources(source, save=False):
    """Reuse unchanged workspace objects without trusting checkout timestamps.

    Cargo normally rebuilds workspace crates after every checkout because their
    source mtimes are newer than restored objects. Restore a previous mtime only
    after hashing the current contents; invalidate changed crates explicitly,
    including a branch whose commit/source timestamp predates the cache.
    """
    source = Path(source)
    snapshot = source/'target/workspace-sources.json'
    external = Path(os.environ['WORKSPACE_SOURCE_CACHE']) if os.environ.get('WORKSPACE_SOURCE_CACHE') else None
    tracked = subprocess.check_output(['git', 'ls-files', '-z'], cwd=source).decode().split('\0')
    names = [name for name in tracked if name and (name.startswith(('apps/', 'crates/', 'resources/', '.cargo/'))
             or name in ('Cargo.toml', 'Cargo.lock', 'LICENSE'))]
    files, directories, packages = {}, {}, {}
    for name in names:
        path = source/name
        if not path.is_file() or path.is_symlink():
            continue
        checksum = digest(path)
        files[name] = {'sha256': checksum, 'mtime_ns': path.stat().st_mtime_ns}
        if path.name == 'Cargo.toml':
            package = tomllib.loads(path.read_text(encoding='utf-8')).get('package', {}).get('name')
            if package:
                packages[path.parent.relative_to(source).as_posix()] = package
        for parent in path.relative_to(source).parents:
            if parent == Path('.'):
                continue
            relative = parent.as_posix()
            directories.setdefault(relative, []).append((name, checksum))
    folders = {name: {'sha256': hashlib.sha256(json.dumps(children).encode()).hexdigest(),
                      'mtime_ns': (source/name).stat().st_mtime_ns}
               for name, children in directories.items()}
    if save:
        snapshot.parent.mkdir(parents=True, exist_ok=True)
        value = {'schema': 1, 'files': files, 'directories': folders}
        write_json(snapshot, value)
        if external:
            # rust-cache deletes non-compiler files from target before saving.
            # Keep its source receipt outside target; the private mirror uses
            # the copy inside target without duplicating compiler objects.
            external.parent.mkdir(parents=True, exist_ok=True)
            write_json(external, value)
        return
    if external and external.is_file():
        snapshot = external
    if not snapshot.is_file() or snapshot.is_symlink() or snapshot.stat().st_size > 8*1024**2:
        return
    try:
        previous = json.loads(snapshot.read_text())
        if (previous.get('schema') != 1 or not isinstance(previous.get('files'), dict)
                or not isinstance(previous.get('directories'), dict)
                or any(not isinstance(record, dict) for record in previous['files'].values())
                or any(not isinstance(record, dict) for record in previous['directories'].values())):
            return
    except (ValueError, OSError):
        return
    now = time.time_ns()
    changed = {name for name in set(files) | set(previous['files'])
               if files.get(name, {}).get('sha256') != previous['files'].get(name, {}).get('sha256')}
    invalid = set()
    for name in changed:
        owners = [directory for directory in packages if name.startswith(directory + '/')]
        if owners:
            invalid.add(packages[max(owners, key=len)])
        elif name.startswith(('resources/', '.cargo/')) or name in ('Cargo.toml', 'LICENSE'):
            invalid.update(packages.values())  # Shared resources/manifest changed.
        # Cargo.lock changes are handled by Cargo's dependency fingerprints.
        # A reqwest update alone must not discard unrelated Slint/UI objects.
    # Mobile builds compile shared .slint files outside their own crate. Honor
    # the public rerun-if-changed directives emitted by those build scripts,
    # instead of assuming every input lives under the owning Cargo.toml.
    changed_paths = [(source/name).resolve() for name in changed]
    if changed_paths:
        patterns = ('release/build/*/output', 'release/build/*/*/output',
                    '*/release/build/*/output', '*/release/build/*/*/output')
        for pattern in patterns:
            for output in (source/'target').glob(pattern):
                owners = [(directory, package) for directory, package in packages.items()
                          if any(name == package or name.startswith(package + '-')
                                 for name in (output.parent.name, output.parent.parent.name))]
                for directory, package in owners:
                    if package in invalid:
                        continue
                    for line in output.read_text(encoding='utf-8', errors='replace').splitlines():
                        match = re.match(r'cargo::?rerun-if-changed=(.+)', line)
                        if not match:
                            continue
                        path = Path(match[1])
                        watched = (path if path.is_absolute() else source/directory/path).resolve()
                        if any(path == watched or path.is_relative_to(watched) for path in changed_paths):
                            invalid.add(package)
                            break
    restored = 0
    for values, old in ((files, previous['files']), (folders, previous.get('directories', {}))):
        for name, record in values.items():
            saved = old.get(name, {})
            stamp = saved.get('mtime_ns')
            if (record['sha256'] == saved.get('sha256') and isinstance(stamp, int) and 0 < stamp <= now):
                os.utime(source/name, ns=(now, stamp))
                restored += 1
    # Content changes always invalidate their owning crate, independently of
    # clock skew or how old that branch/commit is. Cargo rebuilds dependants.
    for pattern in ('release/.fingerprint', '*/release/.fingerprint'):
        for directory in (source/'target').glob(pattern):
            for package in invalid:
                # Cargo supports both <package>-<hash> and <package>/<hash>.
                for artifact in [directory/package, *directory.glob(package + '-*')]:
                    if artifact.is_dir() and not artifact.is_symlink():
                        shutil.rmtree(artifact)
    print(f'Workspace cache: verified {restored} source/directory timestamps; '
          f'invalidated {len(invalid)} changed crates.', flush=True)


def rust_keys(source, compiler, environment):
    """Changed dependencies invalidate exact hits; Cargo validates compatible objects."""
    source = Path(source)
    dependencies = hashlib.sha256(compiler)
    files = {source/'Cargo.lock', source/'Cargo.toml'}
    for directory in ('apps', 'crates', '.cargo'):
        base = source/directory
        files.update(base.rglob('Cargo.toml'))
        if directory == '.cargo':
            files.update(p for p in base.rglob('*') if p.is_file())
    for path in sorted(files):
        if path.is_file():
            dependencies.update(path.relative_to(source).as_posix().encode())
            content = path.read_bytes()
            if path.name in ('Cargo.toml', 'Cargo.lock'):
                # A client version bump is not a new third-party dependency.
                # Keep every dependency constraint, feature and build profile.
                try:
                    parsed = tomllib.loads(content.decode())
                    if path.name == 'Cargo.lock':
                        for package in parsed.get('package', []):
                            if 'source' not in package:
                                package.pop('version', None)
                    else:
                        parsed.get('workspace', {}).get('package', {}).pop('version', None)
                        parsed.get('package', {}).pop('version', None)
                    content = json.dumps(parsed, sort_keys=True).encode()
                except (ValueError, UnicodeDecodeError):
                    pass  # Invalid manifests must still invalidate the cache.
            dependencies.update(content)
    for name, value in sorted(environment.items()):
        if name.startswith(('RUST', 'CARGO_PROFILE_', 'CARGO_TARGET_', 'CC_', 'CXX_')):
            dependencies.update((name+'='+value).encode())
    generation = rust_environment_key(compiler, environment) + dependencies.hexdigest()
    sources = hashlib.sha256()
    if (source/'Cargo.toml').is_file():
        sources.update((source/'Cargo.toml').read_bytes())
    for directory in ('apps', 'crates', 'resources', 'scripts', '.github'):
        for path in sorted((source/directory).rglob('*')):
            relative = path.relative_to(source/directory).parts
            if not path.is_file() or any(part in ('target', '.gradle', '__pycache__', 'generated-res', 'native-libs', 'offline-assets') for part in relative):
                continue
            if directory == 'apps' and relative[:3] == ('android', 'app', 'build'):
                continue
            if directory != 'resources' and path.suffix not in ('.rs', '.slint', '.toml', '.py', '.yml', '.yaml', '.cjs'):
                continue
            sources.update(path.relative_to(source).as_posix().encode())
            sources.update(path.read_bytes())
    return generation + '-' + sources.hexdigest(), generation + '-'


def cache_paths(config):
    if config['kind'] == 'rust':
        cargo = Path(os.environ.get('CARGO_HOME', str(Path.home()/'.cargo')))
        return [cargo/'registry/index', cargo/'registry/cache', cargo/'git/db', Path(config['source'])/'target']
    result = []
    import glob
    for pattern in config['paths'].splitlines():
        if pattern := pattern.strip():
            expanded = os.path.expandvars(os.path.expanduser(pattern))
            matches = sorted(glob.glob(expanded)) if glob.has_magic(expanded) else [expanded]
            result.extend(Path(p).absolute() for p in matches)
    return result


def github_restored(config):
    hit = config.get('github_hit')
    if hit not in ('true', 'false'):
        return False
    if config['kind'] != 'rust' or hit == 'true':
        return True
    # rust-cache reports false for both a prefix hit and a miss. It is the only
    # step that can populate target before this action; require real dependency
    # objects before treating that ambiguous result as a restored cache.
    target = Path(config['source'])/'target'
    return any(path.stat().st_size for pattern in ('**/deps/*.rlib', '**/deps/*.rmeta', '**/deps/*.a')
               for path in target.glob(pattern) if path.is_file() and not path.is_symlink())


def archive_filter(paths, rust=False):
    targets = {str(index) for index, path in enumerate(paths) if path.name == 'target'} if rust else set()
    def include(info):
        if not (info.isfile() or info.isdir()):
            return None
        parts = PurePosixPath(info.name).parts
        if parts[0] in targets and len(parts) > 1:
            # Never let an old target/dependency-snapshot overwrite this run's
            # resolved inputs. Diagnostics, test staging and reports are not
            # compiler caches either.
            native = parts[1] in ('release', '.rustc_info.json', 'CACHEDIR.TAG', 'workspace-sources.json')
            cross = bool(re.fullmatch(r'(?:aarch64|armv7|x86_64)-[a-z0-9_-]+', parts[1]))
            if not native and not (cross and (len(parts) == 2 or parts[2] == 'release')):
                return None
        if any(p in ('incremental', '__pycache__') for p in parts) or (info.name.endswith('.lock') and parts[-1] != 'Cargo.lock'):
            return None
        # Executable test/application outputs are exported as verified artifacts,
        # not dependency caches. Keep build-script executables and library data.
        if rust:
            build_script = 'build' in parts[1:-1]
            if ((info.name.endswith('.exe') and not build_script)
                    or (len(parts) > 2 and parts[-2] == 'deps' and '.' not in parts[-1])
                    or parts[-1] in ('clash-of-rust', 'clash-tun-launcher', 'libclash_mobile.so')):
                return None
        info.uid = info.gid = 0
        info.uname = info.gname = ''
        return info
    return include


def pack(paths, destination, rust=False):
    include = archive_filter(paths, rust)
    with tarfile.open(destination, 'w:gz', compresslevel=1) as archive:
        for index, path in enumerate(paths):
            if path.exists() and not path.is_symlink():
                archive.add(path, arcname=str(index), filter=include)


def unpack(archive_path, paths):
    with tarfile.open(archive_path, 'r:gz') as archive:
        members = archive.getmembers()
        if len(members) > 500_000 or sum(m.size for m in members) > 60 * 1024**3:
            raise ValueError('Cache archive exceeds extraction limits')
        destinations = []
        for member in members:
            parts = PurePosixPath(member.name).parts
            if not parts or not parts[0].isdigit() or int(parts[0]) >= len(paths) or any(p in ('.', '..') or '\\' in p or ':' in p for p in parts):
                raise ValueError('Invalid cache archive path')
            if not (member.isfile() or member.isdir()) or member.name.startswith('/'):
                raise ValueError('Cache links and special files are forbidden')
            root = paths[int(parts[0])]
            destination = root.joinpath(*parts[1:])
            if root.is_symlink() or (destination != root and not destination.resolve().is_relative_to(root.resolve())) or destination.is_symlink():
                raise ValueError('Cache destination escapes its root')
            destinations.append((member, destination))
        for member, destination in destinations:
            if member.isdir():
                destination.mkdir(parents=True, exist_ok=True)
            else:
                destination.parent.mkdir(parents=True, exist_ok=True)
                temporary = None
                try:
                    with tempfile.NamedTemporaryFile(dir=destination.parent, prefix='.cache-extract-', delete=False) as output:
                        temporary = Path(output.name)
                        with archive.extractfile(member) as incoming:
                            shutil.copyfileobj(incoming, output, CHUNK)
                    temporary.chmod(member.mode & 0o777)
                    os.utime(temporary, (member.mtime, member.mtime))
                    temporary.replace(destination)
                finally:
                    if temporary:
                        temporary.unlink(missing_ok=True)


def discard_restore(config):
    """A failed restore must not leave fresh fingerprints beside partial objects."""
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        raise ValueError('Interrupted cache cleanup is restricted to Actions runners')
    def remove(path):
        if path.is_symlink() or path.is_file():
            path.unlink(missing_ok=True)
        elif path.is_dir():
            def writable_retry(operation, name, _error):
                os.chmod(name, 0o700)
                operation(name)
            shutil.rmtree(path, onerror=writable_retry)
    paths = cache_paths(config)
    if config['kind'] == 'rust':
        for path in paths[:-1]:
            remove(path)
        # rust-cache also restores unpacked -sys crates and Git checkouts.
        # Cargo must reconstruct those from verified downloads after a failure.
        remove(paths[0].parent/'src')
        remove(paths[2].parent/'checkouts')
        target = paths[-1]
        if target.is_dir() and not target.is_symlink():
            remove(target/'release')
            for child in target.iterdir():
                if re.fullmatch(r'(?:aarch64|armv7|x86_64)-[a-z0-9_-]+', child.name) and not child.is_symlink():
                    remove(child/'release')
            for name in ('workspace-sources.json', '.rustc_info.json'):
                remove(target/name)
        if external := os.environ.get('WORKSPACE_SOURCE_CACHE'):
            remove(Path(external))
    else:
        for path in paths:
            remove(path)
    config.update(hit=False, restored=False, private_exact=False, mirror_required=True)
    print('Discarded interrupted compiler/download cache; source snapshots and reports preserved.', flush=True)


def endpoint(config):
    url = urllib.parse.urlsplit(config['url'])
    if url.username or url.password or url.query or url.fragment or url.path not in ('', '/'):
        raise ValueError('Invalid cache service URL')
    if url.scheme == 'http':
        address = ipaddress.ip_address(url.hostname)
        if not address.is_loopback and address not in ipaddress.ip_network('100.64.0.0/10'):
            raise ValueError('Plain HTTP is restricted to loopback/Tailscale addresses')
    elif url.scheme != 'https':
        raise ValueError('Cache service requires HTTPS or Tailscale transport')
    return url


def request(config, path, method='GET', file=None, headers=None):
    url = endpoint(config)
    proxy = urllib.parse.urlsplit(config.get('proxy', ''))
    # Large archives share the small cache host's uplink. Allow temporary
    # backpressure without slowing metadata lookups or upstream version checks.
    timeout = 120 if urllib.parse.urlsplit(path).path == '/v1/blob' else 30
    remaining = restore_remaining(config)
    if remaining is not None:
        timeout = min(timeout, remaining, 30)
    if proxy.hostname:
        if proxy.scheme != 'http' or proxy.hostname not in ('127.0.0.1', 'localhost') or url.scheme != 'http':
            raise ValueError('Cache proxy must be the local Tailscale userspace proxy')
        connection = http.client.HTTPConnection(proxy.hostname, proxy.port, timeout=timeout)
        path = config['url'].rstrip('/') + path
    else:
        connection = (http.client.HTTPSConnection if url.scheme == 'https' else http.client.HTTPConnection)(url.hostname, url.port, timeout=timeout)
    auth = {'Authorization': 'Bearer '+config['token'], **(headers or {})}
    connection.request(method, path, body=file, headers=auth)
    return connection, connection.getresponse()


@contextmanager
def response(config, path, **kwargs):
    connection, reply = request(config, path, **kwargs)
    try:
        yield reply
    finally:
        reply.close()
        connection.close()


def restore(config):
    seconds = int(config.get('restore_seconds', RESTORE_SECONDS))
    if not 30 <= seconds <= 900:
        raise ValueError('Invalid cache restore budget')
    config['restore_deadline'] = time.monotonic() + seconds
    try:
        return restore_snapshot(config)
    finally:
        # A later post-job upload has its own budget, after compilation/tests.
        config.pop('restore_deadline', None)


def restore_snapshot(config):
    config.update(hit=False, restored=False, available=False)
    config.update(timings={}, started=time.monotonic())
    if config['kind'] == 'rust':
        compiler = subprocess.check_output(['rustc', '-vV'])
        key, prefix = rust_keys(config['source'], compiler, dict(os.environ))
        config['key'] += '-' + key
        config['prefix'] = [config['base_key'] + '-' + prefix,
                            config['base_key'] + '-' + rust_environment_key(compiler, dict(os.environ))]
        if output := os.environ.get('GITHUB_ENV'):
            with open(output, 'a') as stream:
                stream.write('CARGO_INCREMENTAL=0\n')
    else:
        generation = f'v{SCHEMA}-' + hashlib.sha256(config['paths'].encode()).hexdigest()[:16] + '-'
        config['key'] = generation + config['key']
        config['prefix'] = [generation + p for p in config['prefix']]
    config['resolved_paths'] = [str(p) for p in cache_paths(config)]
    github = github_restored(config) and not config.get('github_failed')
    config['github_supplied'] = github
    if config.get('github_failed') and not github:
        discard_restore(config)
    if github:
        config.update(restored=True, hit=config['github_hit'] == 'true')
    if not config['url'] or not config['token']:
        cache_report(config, 'restore', 'GitHub cache restored; private mirror unavailable' if github
                     else 'not configured; normal build')
        return
    if config['kind'] == 'rust':
        import cache_shards
        if cache_shards.restore(config, github):
            return
    query = urllib.parse.urlencode({'group': config['group'], 'key': config['key'], 'prefix': '\n'.join(config['prefix'])})
    with phase(config, 'lookup'), response(config, '/v1/lookup?'+query) as reply:
        status = reply.status
        record = json.load(reply) if status == 200 else None
    if status == 404 and not github:
        config['available'] = True
        print('Local cache miss:', config['group'])
        cache_report(config, 'restore', 'miss; upstream/repository inputs required')
        return
    if status not in (200, 404):
        raise RuntimeError(f'Cache lookup HTTP {status}')
    if record is not None:
        if record.get('schema') != SCHEMA or record['group'] != config['group'] or not re.fullmatch('[0-9a-f]{64}', record['id']):
            raise ValueError('Invalid cache metadata')
        if record['key'] != config['key'] and not any(record['key'].startswith(p) for p in config['prefix']):
            raise ValueError('Cache generation mismatch')
        config['private_exact'] = record['key'] == config['key']
    if github:
        # GitHub supplied the data already. Only inspect private metadata; never
        # download the same archive twice. Source-only edits do not require a
        # new full Rust dependency mirror when compiler and lockfiles match.
        config['available'] = True
        config['mirror_required'] = not private_current(config, record)
        if config['kind'] != 'rust' and not config.get('immutable'):
            with phase(config, 'fingerprint'):
                config['fingerprint'] = fingerprint([Path(p) for p in config['resolved_paths']])
        cache_report(config, 'restore', 'GitHub cache restored; private metadata checked')
        return
    with tempfile.TemporaryDirectory(prefix='actions-cache-', dir=os.environ.get('RUNNER_TEMP')) as temporary:
        archive = Path(temporary)/'cache.tar.gz'
        with phase(config, 'download'), response(config, '/v1/blob?'+urllib.parse.urlencode({'group': config['group'], 'id': record['id']})) as reply:
            if reply.status != 200:
                raise RuntimeError(f'Cache download HTTP {reply.status}')
            with archive.open('wb') as output:
                download_body(config, reply, output, record['size'], config['group'])
        with phase(config, 'verify'):
            if archive.stat().st_size != record['size'] or digest(archive) != record['sha256']:
                raise ValueError('Cache archive checksum mismatch')
        paths = [Path(p) for p in config['resolved_paths']]
        with phase(config, 'extract'):
            config['restore_started'] = True
            unpack(archive, paths)
    config.update(available=True, restored=True, hit=record['key'] == config['key'])
    if config['kind'] == 'rust':
        config['mirror_required'] = not private_current(config, record)
    if config['kind'] != 'rust' and not config.get('immutable'):
        with phase(config, 'fingerprint'):
            config['fingerprint'] = fingerprint(paths)
    cache_report(config, 'restore', 'exact' if config['hit'] else 'compatible snapshot', record['size'])
    print('Restored local cache:', config['group'], f'{record["size"]/1024**2:.1f} MiB', 'exact' if config['hit'] else 'compatible generation')


def save(config):
    if not config.get('available') or not config['save']:
        return
    config.update(timings={}, started=time.monotonic())
    # Globs such as Cargo's Skia source directory may not exist until compilation.
    paths = cache_paths(config)
    if not any(p.exists() for p in paths):
        return
    if config.get('immutable') and config.get('hit') and not config.get('mirror_required'):
        cache_report(config, 'save', 'verified immutable version reused')
        return
    # The sharded mirror can retain fresh workspace objects cheaply too. Only
    # skip an exact source/toolchain snapshot; a dependency-only GitHub hit must
    # not prevent updated workspace/NDK outputs reaching the private tier.
    unchanged = config['kind'] == 'rust' and config.get('private_exact', False)
    if config['kind'] != 'rust':
        with phase(config, 'fingerprint'):
            unchanged = config.get('fingerprint') == fingerprint(paths)
    unchanged = unchanged and not config.get('mirror_required', False)
    if unchanged:
        cache_report(config, 'save', 'unchanged; reused existing cache')
        return
    if config['kind'] == 'rust':
        import cache_shards
        cache_shards.save(config, paths)
        return
    with tempfile.TemporaryDirectory(prefix='actions-cache-', dir=os.environ.get('RUNNER_TEMP')) as temporary:
        archive = Path(temporary)/'cache.tar.gz'
        with phase(config, 'compress'):
            pack(paths, archive, rust=config['kind'] == 'rust')
        query = urllib.parse.urlencode({'group': config['group'], 'key': config['key']})
        sequence = int(os.environ.get('GITHUB_RUN_ID', '0'))*1000 + min(int(os.environ.get('GITHUB_RUN_ATTEMPT', '0')), 999)
        with phase(config, 'verify'):
            checksum = digest(archive)
        headers = {'Content-Length': str(archive.stat().st_size), 'X-Content-SHA256': checksum,
                   'X-Cache-Sequence': str(sequence), 'Content-Type': 'application/octet-stream'}
        with phase(config, 'upload'), archive.open('rb') as incoming, response(config, '/v1/blob?'+query, method='PUT', file=incoming, headers=headers) as reply:
            if reply.status not in (200, 201, 409):
                raise RuntimeError(f'Cache upload HTTP {reply.status}')
            superseded = reply.status == 409
        cache_report(config, 'save', 'superseded; newer cache retained' if superseded
                     else 'saved; older snapshot replaced', archive.stat().st_size)
        print('Saved local cache:', config['group'], f'{archive.stat().st_size/1024**2:.1f} MiB')


def private_current(config, record):
    if record is None:
        return False
    if config['kind'] == 'rust':
        return record['key'].startswith(config['prefix'][0])
    return record['key'] == config['key']


class Store:
    def __init__(self, root, budget=40*1024**3, min_free=15*1024**3, versions=1):
        self.root = Path(root)
        self.budget, self.min_free, self.versions = budget, min_free, versions
        self.lock = threading.RLock()
        self.reservations = 0
        for name in ('data', 'state', 'logs'):
            (self.root/name).mkdir(mode=0o700, parents=True, exist_ok=True)
        # Keep upload staging on the same mount as the final archive so an
        # isolated service can publish completed uploads with an atomic rename.
        self.incoming = self.root/'data'/'.incoming'
        self.incoming.mkdir(mode=0o700, parents=True, exist_ok=True)
        with self.database() as database:
            database.execute('CREATE TABLE IF NOT EXISTS entries (id TEXT, category TEXT, key TEXT, size INTEGER, sha256 TEXT, accessed REAL, created REAL, PRIMARY KEY(category,id))')
            if 'sequence' not in {row[1] for row in database.execute('PRAGMA table_info(entries)')}:
                database.execute('ALTER TABLE entries ADD COLUMN sequence INTEGER NOT NULL DEFAULT 0')

    @contextmanager
    def database(self):
        database = sqlite3.connect(self.root/'state/index.sqlite', timeout=30)
        database.row_factory = sqlite3.Row
        try:
            with database:
                yield database
        finally:
            database.close()

    def path(self, group, identifier):
        valid_group(group)
        if not re.fullmatch('[0-9a-f]{64}', identifier):
            raise ValueError('Invalid cache identifier')
        return self.root/'data'/group/(identifier+'.tar.gz')

    def lookup(self, group, key, prefixes):
        valid_group(group); valid_key(key)
        with self.lock, self.database() as database:
            rows = database.execute('SELECT * FROM entries WHERE category=? ORDER BY accessed DESC,created DESC,rowid DESC', (group,)).fetchall()
            for wanted in (key, *prefixes):
                for row in rows:
                    if (row['key'] == wanted if wanted == key else row['key'].startswith(wanted)) and self.path(group, row['id']).is_file():
                        database.execute('UPDATE entries SET accessed=? WHERE category=? AND id=?', (time.time(), group, row['id']))
                        return {'schema': SCHEMA, 'group': group, **{k: row[k] for k in ('id', 'key', 'size', 'sha256')}}
        return None

    def fetch(self, group, identifier):
        path = self.path(group, identifier)
        with self.lock, self.database() as database:
            if not path.is_file() or not database.execute(
                    'SELECT 1 FROM entries WHERE category=? AND id=?', (group, identifier)).fetchone():
                raise FileNotFoundError('Cache miss')
            # Index-based restores fetch shards directly, without a per-shard
            # metadata lookup. Those reads must refresh LRU priority too.
            database.execute('UPDATE entries SET accessed=? WHERE category=? AND id=?',
                             (time.time(), group, identifier))
        return path

    def prune(self, incoming=0, protected_group=None):
        with self.lock, self.database() as database:
            for path in self.incoming.glob('*.part'):
                try:
                    if path.stat().st_mtime < time.time() - 86400:
                        path.unlink(missing_ok=True)
                except FileNotFoundError:
                    pass  # A concurrent failed upload may already have removed it.
            rows = database.execute('SELECT * FROM entries ORDER BY accessed DESC,created DESC,rowid DESC').fetchall()
            total = sum(row['size'] for row in rows)
            free = shutil.disk_usage(self.root/'data').free
            counts = {}
            remove = []
            for row in sorted(rows, key=lambda row: (row['sequence'], row['created']), reverse=True):
                group = row['category']
                counts[group] = counts.get(group, 0) + 1
                if counts[group] > self.versions and group != protected_group:
                    remove.append(row)
            total -= sum(row['size'] for row in remove)
            free += sum(row['size'] for row in remove)
            for row in reversed(rows):
                if row not in remove and row['category'] != protected_group and (total + self.reservations + incoming > self.budget or free - self.reservations - incoming < self.min_free):
                    remove.append(row)
                    total -= row['size']
                    free += row['size']
            for row in remove:
                self.path(row['category'], row['id']).unlink(missing_ok=True)
                database.execute('DELETE FROM entries WHERE category=? AND id=?', (row['category'], row['id']))

    def put(self, group, key, length, checksum, stream, sequence=0):
        valid_group(group); valid_key(key)
        if not 0 < length <= 8*1024**3 or not re.fullmatch('[0-9a-f]{64}', checksum) or not 0 <= sequence < 2**63:
            raise ValueError('Invalid cache upload')
        identifier = hashlib.sha256(key.encode()).hexdigest()
        with self.lock:
            if self.reservations + length > self.budget:
                raise OSError('Upload exceeds cache budget')
            self.prune(length, protected_group=group)
            with self.database() as database:
                used = database.execute('SELECT COALESCE(SUM(size),0) FROM entries').fetchone()[0]
            if used + self.reservations + length > self.budget or shutil.disk_usage(self.root/'data').free - self.reservations - length < self.min_free:
                raise OSError('Insufficient cache disk space')
            self.reservations += length
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(dir=self.incoming, suffix='.part', delete=False) as output:
                temporary = Path(output.name)
                remaining = length
                actual = hashlib.sha256()
                while remaining:
                    block = stream.read(min(CHUNK, remaining))
                    if not block:
                        raise ValueError('Incomplete cache upload')
                    output.write(block); actual.update(block); remaining -= len(block)
                output.flush(); os.fsync(output.fileno())
            if actual.hexdigest() != checksum:
                raise ValueError('Cache upload checksum mismatch')
            with self.lock, self.database() as database:
                destination = self.path(group, identifier)
                newest = database.execute('SELECT MAX(sequence) FROM entries WHERE category=?', (group,)).fetchone()[0] or 0
                previous = database.execute('SELECT sequence FROM entries WHERE category=? AND id=?', (group, identifier)).fetchone()
                if newest > sequence or (previous and previous[0] >= sequence and destination.is_file()):
                    return False
                destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
                temporary.replace(destination)
                stamp = time.time()
                database.execute('INSERT OR REPLACE INTO entries VALUES (?,?,?,?,?,?,?,?)', (identifier, group, key, length, checksum, stamp, stamp, sequence))
                return True
        finally:
            if temporary:
                temporary.unlink(missing_ok=True)
            with self.lock:
                self.reservations -= length
                self.prune()

    def status(self):
        with self.database() as database:
            groups = database.execute('SELECT category,COUNT(*) AS count,SUM(size) AS bytes FROM entries GROUP BY category ORDER BY category').fetchall()
            entries = [dict(row) for row in database.execute('SELECT category,key,size,sha256,created,accessed,sequence FROM entries ORDER BY category')]
        return {'root': str(self.root), 'budget_gib': self.budget/1024**3, 'free_gib': shutil.disk_usage(self.root/'data').free/1024**3,
                'groups': [dict(row) for row in groups], 'entries': entries}


class Handler(BaseHTTPRequestHandler):
    def setup(self):
        super().setup()
        self.connection.settimeout(60)

    def log_message(self, _format, *args):
        code = args[1] if len(args) > 1 else ''
        self.server.logger.info('%s %s %s %s', self.client_address[0], self.command, urllib.parse.urlsplit(self.path).path, code)

    def json_reply(self, status, value):
        body = json.dumps(value).encode()
        self.send_response(status); self.send_header('Content-Type', 'application/json'); self.send_header('Content-Length', str(len(body))); self.end_headers()
        self.wfile.write(body)

    def authenticated(self):
        expected = self.server.config['token_sha256']
        actual = hashlib.sha256(self.headers.get('Authorization', '').removeprefix('Bearer ').encode()).hexdigest()
        if not secrets.compare_digest(actual, expected):
            self.json_reply(401, {'error': 'Authentication required'})
            return False
        return True

    def do_GET(self):
        url = urllib.parse.urlsplit(self.path)
        if url.path == '/health':
            self.json_reply(200, {'schema': SCHEMA, 'status': 'ready'})
            return
        if not self.authenticated():
            return
        query = urllib.parse.parse_qs(url.query)
        try:
            if url.path == '/v1/inventory':
                self.json_reply(200, self.server.store.status())
                return
            group = query['group'][0]
            if url.path == '/v1/lookup':
                result = self.server.store.lookup(group, query['key'][0], query.get('prefix', [''])[0].splitlines())
                self.json_reply(200 if result else 404, result or {'error': 'Cache miss'})
            elif url.path == '/v1/blob':
                path = self.server.store.fetch(group, query['id'][0])
                with path.open('rb') as incoming:
                    self.send_response(200); self.send_header('Content-Length', str(os.fstat(incoming.fileno()).st_size)); self.end_headers()
                    shutil.copyfileobj(incoming, self.wfile, CHUNK)
            else:
                self.json_reply(404, {'error': 'Unknown endpoint'})
        except FileNotFoundError:
            self.json_reply(404, {'error': 'Cache miss'})
        except (KeyError, ValueError):
            self.json_reply(400, {'error': 'Invalid cache request'})

    def do_PUT(self):
        if not self.authenticated():
            return
        url = urllib.parse.urlsplit(self.path)
        if url.path != '/v1/blob':
            self.json_reply(404, {'error': 'Unknown endpoint'}); return
        query = urllib.parse.parse_qs(url.query)
        try:
            saved = self.server.store.put(query['group'][0], query['key'][0], int(self.headers['Content-Length']), self.headers['X-Content-SHA256'], self.rfile, int(self.headers.get('X-Cache-Sequence', '0')))
            self.json_reply(201 if saved else 409, {'saved': saved})
        except (KeyError, ValueError, TypeError):
            self.json_reply(400, {'error': 'Invalid cache upload'})
        except (TimeoutError, ConnectionError):
            # A slow/disconnected uploader is not a disk-capacity failure.
            try:
                self.json_reply(408, {'error': 'Cache upload interrupted'})
            except (TimeoutError, ConnectionError):
                pass
        except OSError:
            self.json_reply(507, {'error': 'Cache storage unavailable'})


def serve(root):
    os.umask(0o077)
    config = json.loads((root/'config/server.json').read_text())
    address = ipaddress.ip_address(config['bind'])
    if not address.is_loopback and address not in ipaddress.ip_network('100.64.0.0/10'):
        raise ValueError('The cache server may only bind to loopback or Tailscale')
    server = ThreadingHTTPServer((str(address), config['port']), Handler)
    server.config = config
    server.store = Store(root, config['budget_gib']*1024**3, config['min_free_gib']*1024**3)
    server.logger = logging.getLogger('actions-cache')
    server.logger.setLevel(logging.INFO)
    server.logger.addHandler(RotatingFileHandler(root/'logs/server.log', maxBytes=1024**2, backupCount=2))
    server.serve_forever()


def inventory():
    config = {'url': os.environ.get('CACHE_URL', ''), 'token': os.environ.get('CACHE_TOKEN', ''),
              'proxy': os.environ.get('CACHE_PROXY', '')}
    if not config['url'] or not config['token']:
        print('Private cache inventory skipped: connection is not configured.')
        return
    with response(config, '/v1/inventory') as reply:
        if reply.status != 200:
            raise RuntimeError('Private cache inventory HTTP ' + str(reply.status))
        value = json.load(reply)
    output = Path('target/cache-inventory.json')
    output.parent.mkdir(parents=True, exist_ok=True)
    write_json(output, value)
    if summary := os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(summary, 'a') as stream:
            stream.write('\n## Private cache inventory\n\n| Group | Size | Run | Key |\n| --- | --- | --- | --- |\n')
            for entry in value.get('entries', []):
                stream.write(f'| {entry["category"]} | {entry["size"]/1024**2:.1f} MiB | {entry["sequence"]//1000} | {entry["key"]} |\n')
            stream.write('\nEach group keeps its latest validated snapshot. Older concurrent jobs cannot overwrite a newer snapshot.\n')
    print('Saved authenticated private cache inventory:', output)


def preserve_network(restore=False):
    """The upstream macOS login action overwrites Ethernet DNS even with accept-dns=false."""
    if sys.platform != 'darwin':
        return
    snapshot = Path('target/cache-network.json')
    if restore:
        value = json.loads(snapshot.read_text())
        for kind in ('dnsservers', 'searchdomains'):
            subprocess.run(['sudo', '-n', 'networksetup', '-set'+kind, 'Ethernet',
                            *(value[kind] or ['Empty'])], check=True)
        print('Restored original runner DNS and search domains after cache login.')
        return
    value = {}
    for kind in ('dnsservers', 'searchdomains'):
        text = subprocess.check_output(['networksetup', '-get'+kind, 'Ethernet'], text=True).strip()
        value[kind] = [] if text.startswith(("There aren't any", "You haven't set")) else text.splitlines()
        if kind == 'dnsservers':
            for address in value[kind]:
                ipaddress.ip_address(address)
    snapshot.parent.mkdir(parents=True, exist_ok=True)
    write_json(snapshot, value)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('restore', 'discard-restore', 'save', 'serve', 'status', 'inventory', 'network-save', 'network-restore'))
    parser.add_argument('--state', type=Path)
    parser.add_argument('--root', type=Path, default=DEFAULT_ROOT)
    args = parser.parse_args()
    if args.operation in ('network-save', 'network-restore'):
        preserve_network(args.operation == 'network-restore')
    elif args.operation == 'inventory':
        inventory()
    elif args.operation == 'serve':
        serve(args.root)
    elif args.operation == 'status':
        settings = json.loads((args.root/'config/server.json').read_text())
        print(json.dumps(Store(args.root, settings['budget_gib']*1024**3, settings['min_free_gib']*1024**3).status(), ensure_ascii=False, indent=2))
    else:
        config = json.loads(args.state.read_text())
        try:
            if args.operation == 'discard-restore':
                discard_restore(config)
                config['available'] = False
            else:
                (restore if args.operation == 'restore' else save)(config)
        except (OSError, ValueError, RuntimeError, tarfile.TarError, http.client.HTTPException) as error:
            if args.operation == 'discard-restore':
                raise  # The action must stop if safe cleanup cannot be completed.
            # A cache outage must never prevent normal builds/tests. Do not log
            # exceptions containing request headers, URLs or credential values.
            status = re.search(r'HTTP [0-9]{3}', str(error))
            detail = type(error).__name__ + (' '+status[0] if status else '')
            print('::warning::Local cache unavailable ('+detail+'); normal compilation remains enabled.')
            if args.operation == 'restore' and config.get('restore_started') and not config.get('github_supplied'):
                discard_restore(config)
        finally:
            write_json(args.state, config)


if __name__ == '__main__':
    main()
