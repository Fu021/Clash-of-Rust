"""Compiler cache receipts and safe shared Go dependency archives for GitHub Actions."""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tarfile
import tempfile
import time
import tomllib

SCHEMA = 1
CHUNK = 1024 * 1024
COMPILER_ENV_PREFIXES = (
    'RUST', 'CARGO_PROFILE_', 'CARGO_TARGET_', 'CARGO_BUILD_TARGET',
    'CARGO_ENCODED_RUSTFLAGS', 'CC', 'CXX', 'CFLAGS', 'CMAKE',
)


def digest(path):
    value = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(CHUNK), b''):
            value.update(block)
    return value.hexdigest()


def write_json(path, value):
    path = Path(path)
    temporary = path.with_name(path.name + '.new')
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding='utf-8')
    temporary.chmod(0o600)
    temporary.replace(path)


def rust_environment_key(compiler, environment):
    value = hashlib.sha256(compiler)
    for name, item in sorted(environment.items()):
        if name.startswith(COMPILER_ENV_PREFIXES):
            value.update((name+'='+item).encode())
    return f'v{SCHEMA}-' + value.hexdigest() + '-'


def workspace_vendor_objects(source, cache, packages, save=False):
    target = source/'target'
    directory = cache/'vendor-objects'
    receipt = cache/'vendor-objects.json'
    names = {name for path, name in packages.items() if path.startswith('vendor/')}
    prefixes = {prefix for name in names for prefix in (name, name.replace('-', '_'),
                                                       'lib'+name.replace('-', '_'))}

    def allowed(relative):
        path = PurePosixPath(relative)
        parts = path.parts
        if path.is_absolute() or '..' in parts or '\\' in relative or str(path) != relative:
            return False
        position = 0 if parts and parts[0] == 'release' else 1
        if len(parts) < position+3 or parts[position:position+2] not in (
                ('release', 'deps'), ('release', 'build'), ('release', '.fingerprint')):
            return False
        name = parts[position+2]
        return any(name == prefix or name.startswith(prefix+'-') for prefix in prefixes)

    if save:
        if directory.is_symlink():
            directory.unlink()
        elif directory.exists():
            shutil.rmtree(directory)
        records = {}
        for profile in [target/'release', *target.glob('*/release')]:
            for group in ('deps', 'build', '.fingerprint'):
                root = profile/group
                if not root.is_dir():
                    continue
                for entry in root.iterdir():
                    if not allowed(entry.relative_to(target).as_posix()) or entry.is_symlink():
                        continue
                    for path in ([entry] if entry.is_file() else entry.rglob('*')):
                        if not path.is_file() or path.is_symlink() or not path.resolve().is_relative_to(target.resolve()):
                            continue
                        relative = path.relative_to(target).as_posix()
                        destination = directory/relative
                        destination.parent.mkdir(parents=True, exist_ok=True)
                        try:
                            os.link(path, destination)
                        except OSError:
                            shutil.copy2(path, destination)
                        records[relative] = {'size': path.stat().st_size, 'sha256': digest(path)}
                        if len(records) > 10000:
                            raise ValueError('Vendor artifact limit exceeded')
        write_json(receipt, {'schema': 1, 'files': records})
        if records:
            print(f'Workspace cache: preserved {len(records)} local vendor artifacts', flush=True)
        return
    if not receipt.is_file():
        return
    try:
        if receipt.is_symlink() or receipt.stat().st_size > 8*1024**2:
            raise ValueError('Invalid vendor artifact receipt')
        value = json.loads(receipt.read_text())
        if not isinstance(value, dict) or value.get('schema') != 1 or not isinstance(value.get('files'), dict):
            raise ValueError('Invalid vendor artifact receipt')
        records = value['files']
        if len(records) > 10000:
            raise ValueError('Vendor artifact limit exceeded')
        for relative, record in records.items():
            path = directory/relative
            if (not allowed(relative) or not isinstance(record, dict) or path.is_symlink()
                    or not path.resolve().is_relative_to(directory.resolve())
                    or not (target/relative).resolve().is_relative_to(target.resolve())
                    or path.stat().st_size != record.get('size') or digest(path) != record.get('sha256')):
                raise ValueError('Vendor artifact identity mismatch')
        for relative in records:
            destination = target/relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(directory/relative, destination)
        if records:
            print(f'Workspace cache: restored {len(records)} verified local vendor artifacts', flush=True)
    except (OSError, ValueError):
        print('Workspace cache: incomplete local vendor artifacts ignored; Cargo will rebuild them', flush=True)


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
    names = [name for name in tracked if name and (name.startswith(('apps/', 'crates/', 'vendor/', 'resources/', '.cargo/'))
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
            # Keep its source receipt outside target.
            external.parent.mkdir(parents=True, exist_ok=True)
            try:
                workspace_vendor_objects(source, external.parent, packages, save=True)
            except (OSError, ValueError):
                print('::warning::Local vendor cache save failed; compiled binaries remain usable', flush=True)
            write_json(external, value)
        return
    if external and external.is_file():
        snapshot = external
    if not snapshot.is_file() or snapshot.is_symlink() or snapshot.stat().st_size > 8*1024**2:
        return
    try:
        previous = json.loads(snapshot.read_text())
        if (not isinstance(previous, dict) or previous.get('schema') != 1 or not isinstance(previous.get('files'), dict)
                or not isinstance(previous.get('directories'), dict)
                or any(not isinstance(record, dict) for record in previous['files'].values())
                or any(not isinstance(record, dict) for record in previous['directories'].values())):
            return
    except (ValueError, OSError):
        return
    if external:
        workspace_vendor_objects(source, external.parent, packages)
    now = time.time_ns()
    changed = {name for name in set(files) | set(previous['files'])
               if files.get(name, {}).get('sha256') != previous['files'].get(name, {}).get('sha256')}
    invalid = set()
    for name in changed:
        owners = [directory for directory in packages if name.startswith(directory + '/')]
        if owners:
            invalid.add(packages[max(owners, key=len)])
        elif name.startswith('.cargo/') or name == 'LICENSE':
            invalid.update(packages.values())
        # Cargo.lock changes are handled by Cargo's dependency fingerprints.
        # A reqwest update alone must not discard unrelated Slint/UI objects.
    # Mobile builds compile shared .slint files outside their own crate. Honor
    # the public rerun-if-changed directives emitted by those build scripts,
    # instead of assuming every input lives under the owning Cargo.toml.
    changed_paths = [(source/name).resolve() for name in changed]
    if changed_paths:
        patterns = ('release/build/*/output', 'release/build/*/*/output',
                    '*/release/build/*/output', '*/release/build/*/*/output',
                    'release/build/*/*/run/stdout', '*/release/build/*/*/run/stdout')
        for pattern in patterns:
            for output in (source/'target').glob(pattern):
                owners = [(directory, package) for directory, package in packages.items()
                          if any(name == package or name.startswith(package + '-')
                                 for name in (output.parent.name, output.parent.parent.name,
                                              output.parent.parent.parent.name))]
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
    resource_paths = set()
    for name in changed:
        if not name.startswith('resources/'):
            continue
        path = source/name
        resource_paths.add(path)
        resource_paths.update(parent for parent in path.parents
                              if parent != source and parent.is_relative_to(source))
    for path in resource_paths:
        if path.exists() and not path.is_symlink():
            os.utime(path, ns=(now, now))
    # Content changes always invalidate their owning crate, independently of
    # clock skew or how old that branch/commit is. Cargo rebuilds dependants.
    for pattern in ('release/.fingerprint', '*/release/.fingerprint'):
        for directory in (source/'target').glob(pattern):
            for package in invalid:
                # Cargo supports both <package>-<hash> and <package>/<hash>.
                for artifact in [directory/package, *directory.glob(package + '-*')]:
                    if artifact.is_dir() and not artifact.is_symlink():
                        shutil.rmtree(artifact)
    for profile in (source/'target/release', *(source/'target').glob('*/release')):
        for package in invalid:
            directory = profile/'build'/package
            if directory.is_dir() and not directory.is_symlink():
                for fingerprint in directory.glob('*/fingerprint'):
                    if fingerprint.is_dir() and not fingerprint.is_symlink():
                        shutil.rmtree(fingerprint)
    print(f'Workspace cache: verified {restored} source/directory timestamps; '
          f'invalidated {len(invalid)} changed crates.', flush=True)
    if invalid:
        print('Workspace cache: invalidated crates: ' + ', '.join(sorted(invalid)), flush=True)


def workspace_restore_identity(source, family, receipt_directory, compiler=None, environment=None):
    if not re.fullmatch(r'(?:native-main-v4-(?:linux|macos|windows)|android-main-v4)-(?:x64|arm64|armv7)', family):
        raise ValueError('Invalid compiler cache family')
    source = Path(source).resolve()
    environment = dict(os.environ if environment is None else environment)
    compiler = subprocess.check_output(['rustc', '-vV']) if compiler is None else compiler
    legacy = rust_keys(source, compiler, environment)[1].rstrip('-')
    versions = source/'.github/build-versions.json'
    toolchain = versions.read_bytes() if versions.is_file() else b''
    legacy = rust_environment_key(compiler, environment) + hashlib.sha256(
        (legacy+hashlib.sha256(toolchain).hexdigest()).encode()).hexdigest()
    baseline = json.loads(toolchain) if toolchain else {}
    tools = baseline.get('tools', {})
    system = family.split('-')[-2] if family.startswith('native-') else 'linux'
    python = 'python_windows_arm64' if family.endswith('windows-arm64') else 'python_'+system
    inputs = {
        'profile': tomllib.loads((source/'Cargo.toml').read_text()).get('profile', {}),
        'tools': {'rust': tools.get('rust'), 'python': tools.get(python, tools.get('python'))},
        'cargo': {path.relative_to(source).as_posix(): digest(path)
                  for path in sorted((source/'.cargo').rglob('*')) if path.is_file()},
    }
    if family.startswith('android-'):
        android = baseline.get('upstream', {}).get('android', {})
        inputs['tools'].update(go=tools.get('go'), ndk=android.get('ndk'),
                               platform=android.get('platform'), build_tools=android.get('build_tools'))
    elif system == 'linux':
        inputs['linux'] = baseline.get('upstream', {}).get('linux', {})
        dockerfile = source/'resources/linux/ubuntu20.Dockerfile'
        inputs['dockerfile'] = digest(dockerfile) if dockerfile.is_file() else None
    compatible = rust_environment_key(compiler, environment) + hashlib.sha256(
        json.dumps(inputs, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    cargo = Path(environment.get('CARGO_HOME', str(Path.home()/'.cargo'))).resolve()
    return {
        'prefix': family+'--'+compatible,
        'legacy_prefix': family+'--'+legacy,
        'paths': [str(cargo/'registry'), str(cargo/'git'), str(source/'target'), str(receipt_directory)],
    }


def compiler_cache_checkpoint(source):
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        return
    source = Path(source)
    path = source/'target/compiler-cache-ready.json'
    path.parent.mkdir(parents=True, exist_ok=True)
    write_json(path, {
        'schema': 1, 'run_id': os.environ.get('GITHUB_RUN_ID'),
        'attempt': os.environ.get('GITHUB_RUN_ATTEMPT'),
        'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=source, text=True).strip(),
    })


def allow_completed_compiler_cache(source):
    if os.environ.get('GITHUB_REF') != 'refs/heads/main' or not os.environ.get('GITHUB_ENV'):
        return False
    source = Path(source)
    path = source/'target/compiler-cache-ready.json'
    try:
        if path.is_symlink() or path.stat().st_size > 4096:
            return False
        record = json.loads(path.read_text())
        if (not isinstance(record, dict) or record.get('schema') != 1
                or not os.environ.get('GITHUB_RUN_ID') or not os.environ.get('GITHUB_RUN_ATTEMPT')
                or record.get('run_id') != os.environ['GITHUB_RUN_ID']
                or record.get('attempt') != os.environ['GITHUB_RUN_ATTEMPT']
                or record.get('commit') != subprocess.check_output(
                    ['git', 'rev-parse', 'HEAD'], cwd=source, text=True).strip()):
            return False
    except (OSError, ValueError, subprocess.SubprocessError):
        return False
    with open(os.environ['GITHUB_ENV'], 'a') as output:
        output.write('CACHE_ON_FAILURE=true\n')
    print('Compiler cache: this attempt completed compilation; preserve objects after later failures.', flush=True)
    return True


def rust_keys(source, compiler, environment):
    """Changed dependencies invalidate exact hits; Cargo validates compatible objects."""
    source = Path(source)
    dependencies = hashlib.sha256(compiler)
    files = {source/'Cargo.lock', source/'Cargo.toml'}
    for directory in ('apps', 'crates', 'vendor', '.cargo'):
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
        if name.startswith(COMPILER_ENV_PREFIXES):
            dependencies.update((name+'='+value).encode())
    generation = rust_environment_key(compiler, environment) + dependencies.hexdigest()
    sources = hashlib.sha256()
    if (source/'Cargo.toml').is_file():
        sources.update((source/'Cargo.toml').read_bytes())
    for directory in ('apps', 'crates', 'vendor', 'resources', 'scripts', '.github'):
        for path in sorted((source/directory).rglob('*')):
            relative = path.relative_to(source/directory).parts
            if not path.is_file() or any(part in ('target', '.gradle', '__pycache__', 'generated-res', 'native-libs', 'offline-assets') for part in relative):
                continue
            if directory == 'apps' and relative[:3] == ('android', 'app', 'build'):
                continue
            if (directory != 'resources'
                    and path.suffix not in ('.rs', '.slint', '.svg', '.toml', '.py', '.yml', '.yaml', '.cjs')
                    and not (directory == 'vendor' and path.suffix == '.java')):
                continue
            sources.update(path.relative_to(source).as_posix().encode())
            sources.update(path.read_bytes())
    return generation + '-' + sources.hexdigest(), generation + '-'


def archive_filter(info):
    if not (info.isfile() or info.isdir()):
        return None
    parts = PurePosixPath(info.name).parts
    if any(part in ('incremental', '__pycache__') for part in parts):
        return None
    if info.name.endswith('.lock') and parts[-1] != 'Cargo.lock':
        return None
    info.uid = info.gid = 0
    info.uname = info.gname = ''
    return info


def pack(paths, destination):
    with tarfile.open(destination, 'w:gz', compresslevel=1) as archive:
        for index, path in enumerate(paths):
            if path.exists() and not path.is_symlink():
                archive.add(path, arcname=str(index), filter=archive_filter)


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


def discard_restore(source):
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

    cargo = Path(os.environ.get('CARGO_HOME', str(Path.home()/'.cargo')))
    for relative in ('registry/index', 'registry/cache', 'registry/src', 'git/db', 'git/checkouts'):
        remove(cargo/relative)
    target = Path(source)/'target'
    if target.is_dir() and not target.is_symlink():
        for profile in ('release', 'debug'):
            remove(target/profile)
        for child in target.iterdir():
            if re.fullmatch(r'(?:aarch64|armv7|x86_64)-[a-z0-9_-]+', child.name) and not child.is_symlink():
                for profile in ('release', 'debug'):
                    remove(child/profile)
        for name in ('workspace-sources.json', '.rustc_info.json'):
            remove(target/name)
    if external := os.environ.get('WORKSPACE_SOURCE_CACHE'):
        remove(Path(external))
    print('Discarded interrupted compiler cache; source snapshots and reports preserved.', flush=True)
