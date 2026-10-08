"""Native Debian packaging; no shell launchers or maintainer scripts."""
import gzip
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

from build_support import ROOT, run

# GUI libraries loaded dynamically do not appear in ELF DT_NEEDED.
GUI_DEPENDENCIES = (
    'libxkbcommon0', 'libxkbcommon-x11-0', 'libx11-6', 'libx11-xcb1',
    'libxcb1', 'libxcb-render0', 'libxcb-shm0', 'libxrandr2', 'libxi6',
    'libxcursor1', 'libxinerama1', 'libwayland-client0', 'libwayland-cursor0',
    'libfontconfig1', 'libfreetype6', 'gsettings-desktop-schemas',
)


def require_tools():
    if os.name == 'nt' or any(shutil.which(tool) is None for tool in ('dpkg-deb', 'dpkg-shlibdeps')):
        raise RuntimeError('Debian packaging requires native Linux and dpkg-dev')


def build_deb(payload, version, arch):
    require_tools()
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
        path.chmod(0o755 if path.is_dir() or path == application/'clash-of-rust' or path == application/'resources/mihomo' else 0o644)
    binary_directory = root/'usr/bin'
    binary_directory.mkdir(parents=True)
    (binary_directory/'clash-of-rust').symlink_to('/opt/clash-of-rust/clash-of-rust')
    menu = root/'usr/share/applications/clash-of-rust.desktop'
    menu.parent.mkdir(parents=True)
    menu.write_text('[Desktop Entry]\nType=Application\nName=Clash of Rust\nComment=Native desktop client for mihomo\nExec=/usr/bin/clash-of-rust\nIcon=clash-of-rust\nTerminal=false\nCategories=Network;\n', encoding='utf-8')
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
        f'Package: clash-of-rust\nVersion: {version}\nArchitecture: {deb_arch}\n'
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
