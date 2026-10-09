"""Export and verify native CI binaries without rebuilding them during release."""
import argparse
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tarfile
import tempfile
import zipfile

from build_support import (ROOT, bundle_directory, package_version, sha256,
                           stage_resources, validate_binary_arch, validate_resources, file_version,
                           write_json)

BUILD_NAMES = {f'build-{system}-{arch}' for system in ('windows', 'linux') for arch in ('x64', 'arm64')}
ARCHIVE_NAME = 'build.tar.gz'
TEST_NAMES = {'library', 'clash-of-rust', 'clash-tun-launcher', 'core_integration',
              'geo_download', 'linux_tun', 'tray_integration', 'memory_benchmark'}


def commit():
    return subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()


def release_module():
    spec = importlib.util.spec_from_file_location('publish_release', ROOT/'scripts/publish-release.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def safe_name(name):
    path = PurePosixPath(name)
    return bool(name and not path.is_absolute() and '\\' not in name and ':' not in name
                and '..' not in path.parts and str(path) == name)


def test_binaries(messages):
    result = {}
    for line in messages.splitlines():
        entry = json.loads(line)
        if entry.get('reason') != 'compiler-artifact' or not entry.get('profile', {}).get('test') or not entry.get('executable'):
            continue
        target = entry['target']
        if not Path(target['src_path']).is_relative_to(ROOT):
            continue
        role = 'library' if 'lib' in target['kind'] else target['name']
        if role not in TEST_NAMES or role in result:
            raise ValueError('Unexpected or duplicate test executable: '+role)
        result[role] = entry['executable']
    if not {'library', 'clash-of-rust', 'core_integration', 'tray_integration', 'memory_benchmark'} <= result.keys():
        raise ValueError('Required test executables are missing')
    return result


def export_build(system, arch, tests, output):
    output.mkdir(parents=True, exist_ok=True)
    archive = output/ARCHIVE_NAME
    if archive.exists():
        raise ValueError('Build archive already exists')
    with tempfile.TemporaryDirectory(prefix='ci-payload-') as directory:
        root = Path(directory)
        payload = root/'payload'
        stage_resources(bundle_directory(system, arch)/'resources', payload, system)
        extension = '.exe' if system == 'windows' else ''
        for name in ('clash-of-rust', 'clash-tun-launcher'):
            source = ROOT/'target/release'/(name+extension)
            validate_binary_arch(source, system, arch)
            shutil.copy2(source, payload/source.name)
        paths = {}
        for name, source in tests.items():
            if name not in TEST_NAMES:
                raise ValueError('Unexpected test target')
            relative = 'tests/'+name+extension
            destination = root/relative
            destination.parent.mkdir(exist_ok=True)
            shutil.copy2(source, destination)
            paths[name] = relative
        records = {}
        for path in sorted(root.rglob('*')):
            if path.is_file():
                relative = path.relative_to(root).as_posix()
                executable = relative.startswith('tests/') or path.name in {
                    'clash-of-rust'+extension, 'clash-tun-launcher'+extension, 'mihomo'+extension}
                records[relative] = {'sha256': sha256(path), 'size': path.stat().st_size,
                                     'executable': executable}
        manifest = {'schema': 1, 'commit': commit(), 'version': package_version(),
                    'system': system, 'arch': arch, 'profile': 'release',
                    'cargo_lock_sha256': sha256(ROOT/'Cargo.lock'),
                    'rustc': subprocess.check_output(['rustc', '-Vv'], text=True).strip(),
                    'tests': paths, 'files': records}
        write_json(root/'manifest.json', manifest)
        with tarfile.open(archive, 'w:gz', compresslevel=1) as tar:
            for path in sorted(root.rglob('*')):
                if path.is_file():
                    name = path.relative_to(root).as_posix()
                    info = tar.gettarinfo(str(path), arcname=name)
                    info.mode = 0o755 if records.get(name, {}).get('executable') else 0o644
                    info.uid = info.gid = 0
                    info.uname = info.gname = ''
                    with path.open('rb') as stream:
                        tar.addfile(info, stream)
    archive.with_name(ARCHIVE_NAME+'.sha256').write_text(sha256(archive)+'\n', encoding='ascii')
    return archive


def verify_build(root, system, arch, expected_commit):
    manifest = json.loads((root/'manifest.json').read_text(encoding='utf-8'))
    if (manifest.get('schema'), manifest.get('commit'), manifest.get('version'),
        manifest.get('system'), manifest.get('arch'), manifest.get('profile')) != (
            1, expected_commit, package_version(), system, arch, 'release'):
        raise ValueError('Build metadata does not match the selected source and platform')
    if manifest.get('cargo_lock_sha256') != sha256(ROOT/'Cargo.lock'):
        raise ValueError('Build dependency lockfile differs from the selected source')
    records = manifest['files']
    extension = '.exe' if system == 'windows' else ''
    required = {'payload/LICENSE', 'payload/clash-of-rust'+extension,
                'payload/clash-tun-launcher'+extension,
                'payload/resources/mihomo'+extension,
                'payload/resources/core.json', 'payload/resources/geodata.json',
                *(f'payload/resources/{name}' for name in ('GeoIP.dat', 'GeoSite.dat', 'Country.mmdb', 'ASN.mmdb'))}
    if not required <= records.keys():
        raise ValueError('Build manifest omits a required native payload file')
    actual = {path.relative_to(root).as_posix() for path in root.rglob('*') if path.is_file()}
    if actual != set(records) | {'manifest.json', 'source-artifact.json'} and actual != set(records) | {'manifest.json'}:
        raise ValueError('Build payload contains missing or unexpected files')
    if not isinstance(manifest['tests'], dict) or not set(manifest['tests']) <= TEST_NAMES:
        raise ValueError('Unexpected build test targets')
    if not {'library', 'clash-of-rust', 'core_integration', 'tray_integration', 'memory_benchmark'} <= manifest['tests'].keys():
        raise ValueError('Required build test targets are missing')
    if system == 'linux' and 'linux_tun' not in manifest['tests']:
        raise ValueError('Linux routing test executable is missing')
    for name, relative in manifest['tests'].items():
        if not safe_name(relative) or relative != 'tests/'+name+('.exe' if system == 'windows' else '') or relative not in records:
            raise ValueError('Invalid test executable path')
    for relative, record in records.items():
        if not safe_name(relative) or not relative.startswith(('payload/', 'tests/')):
            raise ValueError('Invalid build payload path')
        path = root/relative
        if path.is_symlink() or not path.is_file() or path.stat().st_size != record['size'] or sha256(path) != record['sha256']:
            raise ValueError('Build payload checksum mismatch: '+relative)
        if os.name != 'nt':
            path.chmod(0o755 if record['executable'] else 0o644)
    payload = root/'payload'
    for name in ('clash-of-rust', 'clash-tun-launcher'):
        validate_binary_arch(payload/(name+extension), system, arch)
    validate_binary_arch(payload/'resources'/('mihomo'+extension), system, arch)
    validate_resources(payload/'resources', system)
    if system == 'windows' and file_version(payload/'clash-of-rust.exe') != manifest['version']:
        raise ValueError('Native PE version differs from build metadata')
    return manifest


def extract_build(archive, root):
    # Only ordinary files are exported; no links, devices, directory modes or ownership.
    with tarfile.open(archive, 'r:gz') as tar:
        seen = set()
        total = 0
        for member in tar:
            if not member.isfile() or not safe_name(member.name) or member.name in seen:
                raise ValueError('Invalid build archive member')
            seen.add(member.name)
            total += member.size
            if member.size > 512*1024*1024 or total > 1024*1024*1024 or len(seen) > 1000:
                raise ValueError('Build archive exceeds payload limits')
            path = root/member.name
            path.parent.mkdir(parents=True, exist_ok=True)
            with tar.extractfile(member) as source, path.open('xb') as destination:
                shutil.copyfileobj(source, destination)


def fetch_build(client, run_id, system, arch, root, expected_commit):
    name = f'build-{system}-{arch}'
    matches = [a for a in client(f'/actions/runs/{run_id}/artifacts?per_page=100')['artifacts'] if a['name'] == name]
    if len(matches) != 1 or matches[0]['expired']:
        raise ValueError('CI build artifact missing, duplicated or expired: '+name)
    artifact = matches[0]
    blob = client(f"/actions/artifacts/{artifact['id']}/zip", raw=True)
    if artifact.get('digest') != 'sha256:'+hashlib.sha256(blob).hexdigest():
        raise ValueError('CI artifact archive checksum mismatch')
    with zipfile.ZipFile(io.BytesIO(blob)) as zipped:
        if len(zipped.infolist()) != 2 or set(zipped.namelist()) != {ARCHIVE_NAME, ARCHIVE_NAME+'.sha256'}:
            raise ValueError('Unexpected CI artifact contents')
        data = zipped.read(ARCHIVE_NAME)
        digest = zipped.read(ARCHIVE_NAME+'.sha256').decode('ascii').strip()
        if not re.fullmatch('[a-f0-9]{64}', digest) or digest != hashlib.sha256(data).hexdigest():
            raise ValueError('CI build tar checksum mismatch')
    root.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(prefix='ci-download-') as tmp:
        archive = Path(tmp)/ARCHIVE_NAME
        archive.write_bytes(data)
        extract_build(archive, root)
    verify_build(root, system, arch, expected_commit)
    write_json(root/'source-artifact.json', {key: artifact[key] for key in ('id', 'name', 'digest')})
    return root


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('export', 'fetch'))
    parser.add_argument('--system', choices=('windows', 'linux'), required=True)
    parser.add_argument('--arch', choices=('x64', 'arm64'), required=True)
    parser.add_argument('--tests', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--run-id', type=int)
    args = parser.parse_args()
    if args.operation == 'export':
        export_build(args.system, args.arch, json.loads(args.tests.read_text()), args.output)
    else:
        module = release_module()
        repo = module.repository()
        client = module.GitHub(repo, module.credential(repo))
        fetch_build(client, args.run_id, args.system, args.arch, args.output, commit())


if __name__ == '__main__':
    main()
