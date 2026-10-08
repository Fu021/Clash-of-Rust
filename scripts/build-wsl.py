"""Mirror source and verified Linux resources to a native WSL filesystem.

Run from Windows. Cargo work and dependencies remain in WSL; only the .deb and
SHA256 are copied back to the Windows project's dist directory.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys

from build_support import ROOT, sha256, validate_version

SOURCE_DIRECTORIES = ('src','scripts','resources','installer','vendor','examples','tests','docs','.github')
SOURCE_FILES = ('Cargo.toml','Cargo.lock','build.rs','LICENSE','README.md','.gitattributes','.gitignore')


def native_build(source, workspace, cargo_bin, cargo_home, proxy, version=None):
    source, workspace = source.resolve(), workspace.resolve()
    if workspace == source or workspace.is_relative_to(source) or str(workspace).startswith('/mnt/'):
        raise ValueError('WSL build workspace must be separate and on the native Linux filesystem')
    marker = workspace/'.clash-of-rust-mirror.json'
    previous = {'source':str(source),'files':[]}
    if workspace.exists():
        if not marker.is_file():
            raise ValueError('Existing directory is not an owned build mirror')
        previous = json.loads(marker.read_text())
        if previous.get('source') != str(source):
            raise ValueError('Build mirror belongs to a different source directory')
    workspace.mkdir(parents=True,exist_ok=True)
    files = [Path(name) for name in SOURCE_FILES if (source/name).is_file()]
    for name in SOURCE_DIRECTORIES:
        for path in (source/name).rglob('*'):
            if path.is_file() and not path.is_symlink() and '__pycache__' not in path.parts and path.suffix != '.pyc':
                files.append(path.relative_to(source))
    current = {p.as_posix() for p in files}
    for name in previous.get('files',[]):
        obsolete = (workspace/name).resolve()
        if not obsolete.is_relative_to(workspace):
            raise ValueError('Invalid mirror ownership record')
        if name not in current:
            obsolete.unlink(missing_ok=True)
    for relative in files:
        destination = workspace/relative
        if not destination.resolve().is_relative_to(workspace):
            raise ValueError('Build mirror contains a path outside its workspace')
        destination.parent.mkdir(parents=True,exist_ok=True)
        original = source/relative
        if not destination.is_file() or sha256(destination) != sha256(original):
            shutil.copy2(original,destination)
    marker.write_text(json.dumps({'source':str(source),'files':sorted(current)},indent=2)+'\n')
    arch = 'arm64' if platform.machine().lower() in ('aarch64','arm64') else 'x64'
    resources = workspace/f'bundle/linux-{arch}/resources'
    if not resources.resolve().is_relative_to(workspace):
        raise ValueError('Build mirror resource directory points outside its workspace')
    resources.mkdir(parents=True,exist_ok=True)
    prepared = source/f'bundle/linux-{arch}/resources'
    required = ('mihomo','core.json','geodata.json','GeoIP.dat','GeoSite.dat','Country.mmdb','ASN.mmdb')
    ready = all((prepared/name).is_file() for name in required)
    if ready:
        for name in required:
            original, destination = prepared/name, resources/name
            if not destination.is_file() or sha256(destination) != sha256(original):
                shutil.copy2(original,destination)
        (resources/'mihomo').chmod(0o755)
    environment = dict(os.environ)
    executable_directory = Path(cargo_bin) if cargo_bin else Path.home()/'.cargo/bin'
    if executable_directory.is_dir():
        environment['PATH'] = str(executable_directory)+os.pathsep+environment.get('PATH','')
    environment['CARGO_TARGET_DIR'] = str(workspace/'target')
    if cargo_home:
        environment['CARGO_HOME'] = str(Path(cargo_home).resolve())
    cache_directory = environment.get('CARGO_HOME')
    if cache_directory and str(Path(cache_directory).resolve()).startswith('/mnt/'):
        raise ValueError('CARGO_HOME must also be on the native Linux filesystem')
    options = ['--skip-prepare'] if ready else []
    if version:
        options += ['--version',validate_version(version)]
    if proxy:
        options += ['--proxy',proxy]
    print('Native WSL source and Cargo target:',workspace,flush=True)
    subprocess.run([sys.executable,workspace/'scripts/build-installer.py',*options],cwd=workspace,env=environment,check=True)
    deb_arch = 'arm64' if arch == 'arm64' else 'amd64'
    version = version or json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1','--offline'],cwd=workspace,env=environment))['packages'][0]['version']
    artifact = workspace/f'dist/Clash-of-Rust-{version}-linux-{deb_arch}.deb'
    assert artifact.with_name(artifact.name+'.sha256').read_text().strip() == sha256(artifact)
    (source/'dist').mkdir(exist_ok=True)
    for path in (artifact,artifact.with_name(artifact.name+'.sha256')):
        shutil.copy2(path,source/'dist'/path.name)
    print('Copied verified Debian package back:',source/'dist'/artifact.name,flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--distro',help='WSL distribution; omitted uses the default')
    parser.add_argument('--user',help='Optional WSL user; omitted uses the distribution default')
    parser.add_argument('--workspace',help='Native Linux build directory; defaults to a repo-specific /var/tmp directory')
    parser.add_argument('--cargo-bin',help='Optional native Linux directory containing Cargo')
    parser.add_argument('--cargo-home',help='Optional native Linux Cargo cache directory')
    parser.add_argument('--proxy',default='',help='Resource download proxy reachable from Linux')
    parser.add_argument('--version',help='Optional package/application version override for update testing')
    parser.add_argument('--native-build',action='store_true',help=argparse.SUPPRESS)
    parser.add_argument('--source',type=Path,help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.native_build:
        if not sys.platform.startswith('linux') or args.source is None:
            raise ValueError('Internal mirror operation requires Linux and a source directory')
        native_build(args.source,Path(args.workspace),args.cargo_bin,args.cargo_home,args.proxy,args.version)
        return
    if os.name != 'nt':
        raise ValueError('Use build-installer.py directly on native Linux')
    wsl = ['wsl.exe'] + (['--distribution',args.distro] if args.distro else [])
    if args.user:
        wsl += ['--user',args.user]
    source = subprocess.check_output([*wsl,'--exec','wslpath','-u',ROOT.as_posix()],text=True,encoding='utf-8').strip()
    workspace = args.workspace or '/var/tmp/clash-of-rust-build-'+hashlib.sha256(str(ROOT).encode()).hexdigest()[:12]
    command = [*wsl,'--exec','python3',source+'/scripts/build-wsl.py','--native-build','--source',source,'--workspace',workspace]
    if args.cargo_bin:
        command += ['--cargo-bin',args.cargo_bin]
    if args.cargo_home:
        command += ['--cargo-home',args.cargo_home]
    if args.proxy:
        command += ['--proxy',args.proxy]
    if args.version:
        command += ['--version',validate_version(args.version)]
    subprocess.run(command,check=True)


if __name__ == '__main__':
    main()
