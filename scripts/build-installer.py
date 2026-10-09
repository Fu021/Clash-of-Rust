"""Build native Windows EXE or Debian DEB packages for x64 and ARM64."""
import argparse
import gzip
import hashlib
import tempfile
from pathlib import Path
import os
import shutil
import sys
import json
import subprocess
from build_support import ROOT, bundle_directory, copy_static, deb_version, file_version, host_arch, nsis, numeric_version, package_version, run, sha256, stage_resources, staging_directory, validate_binary_arch, validate_version
import importlib.util

_spec = importlib.util.spec_from_file_location('prepare_resources', ROOT/'scripts/prepare-resources.py')
_module = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_module)
prepare = _module.prepare


# GUI libraries loaded dynamically do not appear in ELF DT_NEEDED.
GUI_DEPENDENCIES = (
    'libxkbcommon0', 'libxkbcommon-x11-0', 'libx11-6', 'libx11-xcb1',
    'libxcb1', 'libxcb-render0', 'libxcb-shm0', 'libxrandr2', 'libxi6',
    'libxcursor1', 'libxinerama1', 'libwayland-client0', 'libwayland-cursor0',
    'libfontconfig1', 'libfreetype6', 'gsettings-desktop-schemas',
)


def require_deb_tools():
    if os.name == 'nt' or any(shutil.which(tool) is None for tool in ('dpkg-deb', 'dpkg-shlibdeps')):
        raise RuntimeError('Debian packaging requires native Linux and dpkg-dev')


def build_deb(payload, version, arch):
    require_deb_tools()
    # A WSL checkout may reside on NTFS, where chmod is not honored. Debian
    # control metadata and file modes must be staged on a native filesystem.
    with tempfile.TemporaryDirectory(prefix='clash-of-rust-deb-',dir='/tmp') as work:
        return _build_deb(payload, version, arch, Path(work))


def _build_deb(payload, version, arch, work):
    deb_arch = {'x64':'amd64', 'arm64':'arm64'}[arch]
    root = work/'package'
    application = root/'opt/clash-of-rust'
    shutil.copytree(payload, application)
    # Normalize permissions even when source files reside on a Windows mount.
    for path in root.rglob('*'):
        path.chmod(0o755 if path.is_dir() or path in (application/'clash-of-rust', application/'clash-tun-launcher', application/'resources/mihomo') else 0o644)
    binary_directory = root/'usr/bin'
    binary_directory.mkdir(parents=True)
    (binary_directory/'clash-of-rust').symlink_to('/opt/clash-of-rust/clash-of-rust')
    menu = root/'usr/share/applications/clash-of-rust.desktop'
    menu.parent.mkdir(parents=True)
    menu.write_text('[Desktop Entry]\nType=Application\nName=Clash of Rust\nComment=Native desktop client for mihomo\nExec=/usr/bin/clash-of-rust\nIcon=clash-of-rust\nStartupWMClass=clash-of-rust\nTerminal=false\nCategories=Network;\n', encoding='utf-8')
    policy = root/'usr/share/polkit-1/actions/org.clashofrust.tun.policy'
    policy.parent.mkdir(parents=True)
    shutil.copyfile(ROOT/'resources/linux/org.clashofrust.tun.policy', policy)
    icon = root/'usr/share/icons/hicolor/128x128/apps/clash-of-rust.png'
    icon.parent.mkdir(parents=True)
    shutil.copyfile(ROOT/'resources/icons/app.png', icon)
    documentation = root/'usr/share/doc/clash-of-rust'
    documentation.mkdir(parents=True)
    shutil.copyfile(payload/'resources/THIRD-PARTY-NOTICES.txt', documentation/'copyright')
    with (documentation/'changelog.gz').open('wb') as stream:
        with gzip.GzipFile(filename='', fileobj=stream, mode='wb', mtime=0) as compressed:
            compressed.write(f'Clash of Rust {version}\n\nNative Rust desktop client and native platform checks.\n'.encode())
    # dpkg's symbol database computes minimum versions for this build's ELF ABI.
    source_control = work/'debian/control'
    source_control.parent.mkdir()
    source_control.write_text(f'Source: clash-of-rust\n\nPackage: clash-of-rust\nArchitecture: {deb_arch}\nDescription: Native desktop client for mihomo\n', encoding='utf-8')
    output = subprocess.check_output(['dpkg-shlibdeps','-O',f'-e{application / "clash-of-rust"}'], cwd=work, text=True)
    abi = next((line.split('=',1)[1] for line in output.splitlines() if line.startswith('shlibs:Depends=')), None)
    if not abi:
        raise RuntimeError('Could not determine native library dependencies')
    dependencies = ', '.join([abi, *GUI_DEPENDENCIES])
    control = root/'DEBIAN'
    control.mkdir()
    control.chmod(0o755)
    installed_size = sum(p.stat().st_size for p in root.rglob('*') if p.is_file() and not p.is_symlink())
    (control/'control').write_text(
        f'Package: clash-of-rust\nVersion: {deb_version(version)}\nArchitecture: {deb_arch}\n'
        'Section: net\nPriority: optional\nMaintainer: F021 <flmqs@outlook.com>\n'
        f'Installed-Size: {(installed_size+1023)//1024}\nDepends: {dependencies}\n'
        'Recommends: pkexec\n'
        'Homepage: https://github.com/Fu021/Clash-of-Rust\n'
        'Description: Native Rust desktop client for mihomo\n'
        ' Includes the mihomo core, offline Geo databases and native platform checks.\n', encoding='utf-8')
    checksums = []
    for path in sorted(root.rglob('*')):
        if path.is_file() and not path.is_symlink() and 'DEBIAN' not in path.relative_to(root).parts:
            with path.open('rb') as stream:
                digest = hashlib.file_digest(stream,'md5').hexdigest()
            checksums.append(f'{digest}  {path.relative_to(root).as_posix()}')
    (control/'md5sums').write_text('\n'.join(checksums)+'\n', encoding='ascii')
    artifact = ROOT/f'dist/Clash-of-Rust-{version}-linux-{deb_arch}.deb'
    temporary = artifact.with_name(artifact.name+'.partial')
    try:
        run(['dpkg-deb','--build','--root-owner-group','--uniform-compression','-Zxz','-z6',root,temporary])
        temporary.replace(artifact)
    finally:
        temporary.unlink(missing_ok=True)
    return artifact


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--proxy', default='')
    parser.add_argument('--skip-prepare', action='store_true')
    parser.add_argument('--version', default=package_version())
    parser.add_argument('--system', choices=('windows','linux'), default='windows' if os.name == 'nt' else 'linux')
    parser.add_argument('--arch', choices=('x64','arm64'), default=host_arch())
    args = parser.parse_args()
    version = validate_version(args.version)
    host = 'windows' if os.name == 'nt' else 'linux' if sys.platform.startswith('linux') else 'unsupported'
    if (args.system,args.arch) != (host,host_arch()):
        raise ValueError('Packaging requires a native build on the selected system/architecture')
    if args.system == 'linux':
        require_deb_tools()
    resources = bundle_directory(args.system,args.arch)/'resources'
    if not args.skip_prepare:
        resources = prepare(args.system,args.arch,args.proxy)
    copy_static(resources.parent)
    env = dict(os.environ)
    if version == package_version():
        env.pop('CLASH_OF_RUST_BUILD_VERSION',None)
    else:
        env['CLASH_OF_RUST_BUILD_VERSION'] = version
    run(['cargo','build','--release','--locked'],env=env)
    metadata = json.loads(subprocess.check_output(['cargo','metadata','--format-version','1','--no-deps','--offline'],cwd=ROOT))
    binary = Path(metadata['target_directory'])/'release'/('clash-of-rust.exe' if args.system == 'windows' else 'clash-of-rust')
    validate_binary_arch(binary,args.system,args.arch)
    validate_binary_arch(resources/('mihomo.exe' if args.system == 'windows' else 'mihomo'),args.system,args.arch)
    if args.system == 'windows' and file_version(binary) != version:
        raise ValueError('Application PE version differs from package version')
    # Maintain the developer bundle for installer tests; installer inputs always
    # come from a fresh whitelist stage, so old MSYS/Bash files cannot leak in.
    shutil.copy2(binary, resources.parent/binary.name)
    (ROOT/'dist').mkdir(exist_ok=True)
    artifacts = []
    with staging_directory('release') as stage:
        stage_resources(resources,stage,args.system)
        shutil.copy2(binary,stage/binary.name)
        if args.system == 'linux':
            launcher = binary.with_name('clash-tun-launcher')
            validate_binary_arch(launcher,args.system,args.arch)
            shutil.copy2(launcher,stage/launcher.name)
        if args.system == 'windows':
            artifact = ROOT/f'dist/Clash-of-Rust-{version}-windows-{args.arch}-setup.exe'
            run([nsis(args.proxy),'/V2','/INPUTCHARSET','UTF8',f'/DAPP_VERSION={version}',f'/DAPP_NUMERIC_VERSION={numeric_version(version)}',f'/DAPP_ARCH={args.arch}',f'/DPAYLOAD={stage}',f'/DOUTPUT={artifact}',ROOT/'installer/clash-of-rust.nsi'])
            if file_version(artifact) != version:
                raise ValueError('Installer PE version differs from package version')
            artifacts.append(artifact)
        else:
            artifacts.append(build_deb(stage,version,args.arch))
    for artifact in artifacts:
        artifact.with_name(artifact.name+'.sha256').write_text(sha256(artifact)+'\n',encoding='ascii')
        print('Package ready:',artifact)
    if args.system == 'windows':
        run([sys.executable, ROOT/'scripts/verify-icon-resources.py',binary,artifact])


if __name__ == '__main__':
    main()
