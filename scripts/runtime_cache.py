"""Cache explicit runtime/tool directories; credentials and machine state are excluded."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import tarfile
import tempfile

from build_inputs import approved, tool_versions
from build_support import download, host_arch, host_system, sha256, write_json

KINDS = ('bootstrap', 'rust', 'go', 'java', 'android', 'android-package', 'emulator', 'apt')


def java_archive():
    value = approved()['upstream']['java']
    if value['version'] != tool_versions()['java']:
        raise ValueError('Java archive differs from the approved tool version')
    return value['architectures'][host_system() + '-' + host_arch()]


def roots(kind):
    versions = tool_versions()
    system, arch = host_system(), host_arch()
    toolcache = Path(os.environ['RUNNER_TOOL_CACHE'])
    if kind == 'bootstrap':
        python = versions['python']
        python_root = toolcache / 'Python' / python
        if system == 'macos':
            python_root = Path('/Library/Frameworks/Python.framework/Versions') / '.'.join(python.split('.')[:2])
        return [python_root, toolcache / 'tailscale' / versions['tailscale']]
    if kind == 'go':
        return [toolcache / 'go' / versions['go']]
    if kind == 'java':
        return [toolcache / 'Java_Temurin-Hotspot_jdk' / versions['java'].replace('+', '-')]
    if kind == 'rust':
        triple = ('x86_64' if arch == 'x64' else 'aarch64') + {
            'linux': '-unknown-linux-gnu', 'windows': '-pc-windows-msvc', 'macos': '-apple-darwin'}[system]
        return [Path(os.environ.get('RUSTUP_HOME', Path.home() / '.rustup')) / 'toolchains' / (versions['rust'] + '-' + triple)]
    if kind in ('android', 'android-package', 'emulator'):
        sdk = Path(os.environ['ANDROID_HOME'])
        names = ('cmdline-tools', 'platform-tools', 'platforms/android-36', 'build-tools/36.0.0', 'ndk/28.2.13676358')
        if kind == 'emulator':
            names = ('emulator', 'system-images/android-35/google_apis/x86_64')
        elif kind == 'android-package':
            names = ('cmdline-tools', 'platform-tools', 'build-tools/36.0.0')
        return [sdk / name for name in names]
    return []


def plan(kind):
    system, arch = host_system(), host_arch()
    selected = roots(kind)
    versions = tool_versions()
    requested = ('python', 'tailscale') if kind == 'bootstrap' else (kind,) if kind in ('rust', 'go', 'java') else ()
    identity = {'schema': 1, 'kind': kind, 'system': system, 'arch': arch,
                'tools': {name: versions[name] for name in requested},
                'image': os.environ.get('ImageOS', platform.release()),
                'roots': [str(root) for root in selected]}
    if kind in ('android', 'android-package', 'emulator', 'bootstrap'):
        identity['image_version'] = os.environ.get('ImageVersion', '')
    if kind == 'rust':
        identity['target'] = os.environ.get('RUNTIME_RUST_TARGET', '')
    if kind == 'java':
        identity['archive'] = java_archive()
    checksum = hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()
    prefix = f'runtime-v1-{kind}-{system}-{arch}-{checksum}-'
    key = prefix + (os.environ['GITHUB_RUN_ID'] + '-' + os.environ.get('GITHUB_RUN_ATTEMPT', '1') if kind == 'apt' else 'approved')
    directory = Path(os.environ['RUNNER_TEMP']) / ('cor-host-apt' if kind == 'apt' else 'cor-runtime-' + kind)
    value = {'key': key, 'prefix': prefix if kind == 'apt' else '', 'directory': str(directory),
             'group': ('dependencies/host-apt-' if kind == 'apt' else 'tools/runtime-') + system + '-' + arch,
             'immutable': 'false'}
    if output := os.environ.get('GITHUB_OUTPUT'):
        with open(output, 'a') as stream:
            for name, item in value.items():
                stream.write(name + '=' + item + '\n')
    if kind == 'apt' and (output := os.environ.get('GITHUB_ENV')):
        with open(output, 'a') as stream:
            stream.write('COR_HOST_APT_CACHE=' + str(directory) + '\n')
            if not os.environ.get('COR_LINUX_APT_CACHE'):
                stream.write('COR_LINUX_APT_CACHE=' + str(directory) + '\n')
    print('Runtime cache:', kind, key)
    return directory, selected


def capture(kind, directory, selected):
    if kind == 'apt':
        return
    if kind == 'java':
        record = java_archive()
        download(record['url'], directory / 'jdk.tar.gz', digest=record['sha256'])
        print('Captured approved official Java archive', flush=True)
        return
    directory.mkdir(parents=True, exist_ok=True)
    receipt = directory / 'manifest.json'
    previous = json.loads(receipt.read_text()) if receipt.is_file() else {}
    records = []
    for index, root in enumerate(selected):
        if not root.is_dir() or root.is_symlink():
            print('Runtime mirror missing:', kind, root.name, flush=True)
            continue
        archive = directory / (str(index) + '.tar')
        old = next((entry for entry in previous.get('roots', []) if entry['index'] == index), None)
        tree = hashlib.sha256()
        for parent, directories, files in os.walk(root):
            directories[:] = sorted(name for name in directories if name != '__pycache__')
            for name in sorted([*directories, *files]):
                path = Path(parent) / name
                metadata = path.lstat()
                content = os.readlink(path) if path.is_symlink() else sha256(path) if path.is_file() else ''
                tree.update(json.dumps([path.relative_to(root).as_posix(), metadata.st_mode & 0o777,
                                        content]).encode())
        tree_checksum = tree.hexdigest()
        if (old and old.get('root') == str(root) and old.get('tree') == tree_checksum
                and archive.is_file() and sha256(archive) == old['sha256']):
            records.append(old)
            continue
        temporary = archive.with_suffix('.partial')

        def include(info):
            relative = Path(info.name).relative_to('payload')
            if '__pycache__' in relative.parts:
                return None
            path = root / relative
            if info.issym():
                target = path.resolve()
                if not target.is_relative_to(root.resolve()):
                    raise ValueError('Tool link escapes its allowed directory: ' + kind + '/' + relative.as_posix())
                info.linkname = os.path.relpath(target, path.parent)
            if not (info.isfile() or info.isdir() or info.issym() or info.islnk()):
                return None
            info.uid = info.gid = 0
            info.uname = info.gname = ''
            return info

        try:
            with tarfile.open(temporary, 'w') as stream:
                stream.add(root, arcname='payload', filter=include)
            temporary.replace(archive)
        finally:
            temporary.unlink(missing_ok=True)
        records.append({'index': index, 'root': str(root), 'size': archive.stat().st_size,
                        'sha256': sha256(archive), 'tree': tree_checksum})
        print('Captured runtime:', kind, root.name, flush=True)
    write_json(receipt, {'schema': 1, 'kind': kind, 'roots': records})


def hydrate(kind, directory, selected):
    if kind == 'java':
        archive = directory / 'jdk.tar.gz'
        if not archive.is_file():
            return
        record = java_archive()
        if archive.is_symlink() or sha256(archive) != record['sha256']:
            raise ValueError('Java archive checksum mismatch')
        root = selected[0] / host_arch()
        if root.exists() or root.is_symlink():
            return
        root.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='.cor-java-', dir=root.parent) as temporary:
            stage = Path(temporary)
            with tarfile.open(archive, 'r:*') as stream:
                members = stream.getmembers()
                names = {Path(member.name).parts[0] for member in members if Path(member.name).parts}
                if (len(names) != 1 or len(members) > 500_000
                        or sum(member.size for member in members) > 3 * 1024**3
                        or any(Path(member.name).is_absolute() or '..' in Path(member.name).parts
                               or '\\' in member.name for member in members)):
                    raise ValueError('Invalid official Java archive layout')
                stream.extractall(stage, members=members, filter='data')
            payload = stage / names.pop()
            if not payload.is_dir() or payload.is_symlink() or not (payload / 'bin/java').is_file():
                raise ValueError('Java archive lacks its approved runtime')
            payload.replace(root)
        (root.parent / (host_arch() + '.complete')).touch()
        print('Restored approved official Java runtime', flush=True)
        return
    if kind == 'apt' or not (directory / 'manifest.json').is_file():
        return
    receipt = json.loads((directory / 'manifest.json').read_text())
    if receipt.get('schema') != 1 or receipt.get('kind') != kind:
        raise ValueError('Runtime snapshot kind mismatch')
    seen = set()
    for entry in receipt['roots']:
        index = entry['index']
        if not isinstance(index, int) or index < 0 or index >= len(selected) or index in seen:
            raise ValueError('Invalid runtime directory index')
        seen.add(index)
        root = selected[index]
        if entry['root'] != str(root):
            raise ValueError('Runtime directory differs from its approved destination')
        if root.exists() or root.is_symlink():
            continue
        archive = directory / (str(index) + '.tar')
        if archive.is_symlink() or archive.stat().st_size != entry['size'] or sha256(archive) != entry['sha256']:
            raise ValueError('Runtime snapshot checksum mismatch')
        root.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='.cor-runtime-', dir=root.parent) as temporary:
            stage = Path(temporary)
            with tarfile.open(archive, 'r:') as stream:
                members = stream.getmembers()
                if len(members) > 500_000 or sum(member.size for member in members) > 30 * 1024**3:
                    raise ValueError('Runtime snapshot exceeds extraction limits')
                for member in members:
                    if (not member.name.startswith('payload/') and member.name != 'payload'
                            or '..' in Path(member.name).parts or '\\' in member.name):
                        raise ValueError('Invalid runtime snapshot path')
                    if member.issym():
                        target = Path(os.path.normpath(str(Path(member.name).parent / member.linkname)))
                        if target.is_absolute() or not target.is_relative_to('payload'):
                            raise ValueError('Runtime link escapes its payload')
                    if member.islnk() and (Path(member.linkname).is_absolute()
                                           or '..' in Path(member.linkname).parts
                                           or not Path(member.linkname).is_relative_to('payload')):
                        raise ValueError('Runtime hard link escapes its payload')
                stream.extractall(stage, members=members, filter='data')
            payload = stage / 'payload'
            if not payload.is_dir() or payload.is_symlink():
                raise ValueError('Runtime snapshot lacks its payload')
            payload.replace(root)
        print('Restored runtime:', kind, root.name, flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('plan', 'hydrate', 'capture'))
    parser.add_argument('--kind', choices=KINDS, required=True)
    parser.add_argument('--directory', type=Path)
    args = parser.parse_args()
    if args.operation == 'plan':
        plan(args.kind)
    else:
        {'hydrate': hydrate, 'capture': capture}[args.operation](args.kind, args.directory, roots(args.kind))


if __name__ == '__main__':
    main()
