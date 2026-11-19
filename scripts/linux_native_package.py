"""Build native compatibility packages from the verified Linux candidate payload."""
import argparse
import json
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

from build_support import ROOT, run, sha256, write_json

RPM_REQUIREMENTS = (
    'glibc', 'libgcc_s.so.1()(64bit)', 'libstdc++.so.6()(64bit)',
    'libxkbcommon.so.0()(64bit)', 'libxkbcommon-x11.so.0()(64bit)',
    'libX11.so.6()(64bit)', 'libX11-xcb.so.1()(64bit)', 'libxcb.so.1()(64bit)',
    'libxcb-render.so.0()(64bit)', 'libxcb-shm.so.0()(64bit)',
    'libXrandr.so.2()(64bit)', 'libXi.so.6()(64bit)', 'libXcursor.so.1()(64bit)',
    'libXinerama.so.1()(64bit)', 'libwayland-client.so.0()(64bit)',
    'libwayland-cursor.so.0()(64bit)', 'libfontconfig.so.1()(64bit)',
    'libfreetype.so.6()(64bit)', 'gsettings-desktop-schemas', 'util-linux',
    '/usr/bin/pkexec', '/usr/bin/pkttyagent', '/usr/sbin/ip',
)


def native_version(version, separator):
    match = re.fullmatch(r'(\d+\.\d+\.\d+)(?:-([0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*))?', version)
    if not match:
        raise ValueError(f'Unsupported native package version: {version!r}')
    base, prerelease = match.groups()
    return base + (separator + prerelease if prerelease else '')


def build_rpm(deb, version, output):
    with tempfile.TemporaryDirectory(prefix='cor-rpm-') as temporary:
        work = Path(temporary)
        payload = work/'payload'
        payload.mkdir()
        archives = subprocess.check_output(['ar', 't', deb], text=True).splitlines()
        data, = (name for name in archives if re.fullmatch(r'data\.tar(?:\.(?:xz|zst|gz))?', name))
        archive = work/data
        with archive.open('wb') as stream:
            run(['ar', 'p', deb, data], stdout=stream)
        run(['tar', 'xf', archive, '-C', payload])
        marker = payload/'opt/clash-of-rust/package-manager'
        marker.write_text('rpm\n', encoding='ascii')
        marker.chmod(0o644)
        spec = work/'clash-of-rust.spec'
        spec.write_text(
            '%global __os_install_post %{nil}\n'
            'Name: clash-of-rust\n'
            f'Version: {native_version(version, "~")}\n'
            'Release: 1\nSummary: Native desktop client for mihomo\nLicense: GPL-3.0-only\n'
            'BuildArch: x86_64\nAutoReqProv: no\n'
            + ''.join(f'Requires: {dependency}\n' for dependency in RPM_REQUIREMENTS)
            + '\n%description\nNative Rust client for mihomo.\n'
            '%install\nmkdir -p "%{buildroot}"\n'
            f"cp -a '{payload}/.' \"%{{buildroot}}/\"\n"
            '\n%files\n%defattr(-,root,root,-)\n'
            '/opt/clash-of-rust\n/usr/bin/cor\n/usr/bin/clash-of-rust\n'
            '/usr/share/applications/clash-of-rust.desktop\n'
            '/usr/share/polkit-1/actions/org.clashofrust.tun.policy\n'
            '/usr/share/icons/hicolor/128x128/apps/clash-of-rust.png\n'
            '/usr/share/doc/clash-of-rust\n', encoding='utf-8')
        run(['rpmbuild', '-bb', '--define', f'_topdir {work}', spec])
        packages = list((work/'RPMS/x86_64').glob('*.rpm'))
        if len(packages) != 1:
            raise ValueError('Expected a single native x86_64 RPM')
        output.mkdir(parents=True, exist_ok=True)
        result = output/f'Clash-of-Rust-{version}-linux-x86_64.rpm'
        shutil.copyfile(packages[0], result)
        return result


def build_arch(deb, version, output, expected_digest):
    if sha256(deb) != expected_digest:
        raise ValueError('Arch recipe source digest mismatch')
    with tempfile.TemporaryDirectory(prefix='cor-arch-') as temporary:
        directory = Path(temporary)
        shutil.copyfile(deb, directory/'candidate.deb')
        recipe = (ROOT/'installer/arch/PKGBUILD').read_text()
        recipe = recipe.replace('@VERSION@', native_version(version, ''))
        recipe = recipe.replace('@SHA256@', expected_digest)
        (directory/'PKGBUILD').write_text(recipe)
        info = directory/'.SRCINFO'
        with info.open('w') as stream:
            run(['makepkg', '--printsrcinfo'], cwd=directory, stdout=stream)
        values = {line.strip() for line in info.read_text().splitlines()}
        if not {'pkgname = clash-of-rust', 'arch = x86_64', 'source = candidate.deb',
                f'pkgver = {native_version(version, "")}', f'sha256sums = {expected_digest}'} <= values:
            raise ValueError('Arch recipe metadata differs from the verified input')
        run(['makepkg', '--cleanbuild', '--noconfirm', '--nodeps'], cwd=directory)
        packages = list(directory.glob('*.pkg.tar.zst'))
        if len(packages) != 1:
            raise ValueError('Expected a single native Arch package')
        output.mkdir(parents=True, exist_ok=True)
        result = output/packages[0].name
        shutil.copyfile(packages[0], result)
        return result


def rpm_candidate():
    source = ROOT/'dist/package-receipt.json'
    receipt = json.loads(source.read_text())
    deb = source.parent/receipt['package']
    if (receipt['system'], receipt['arch'], receipt['tests_passed']) != ('linux', 'x64', True):
        raise ValueError('RPM assembly requires a verified x64 Linux candidate')
    if deb.parent != source.parent or deb.suffix != '.deb' or sha256(deb) != receipt['sha256']:
        raise ValueError('RPM assembly input differs from the verified DEB')
    output = ROOT/'dist/linux-rpm'
    package = build_rpm(deb, receipt['version'], output)
    write_json(output/'rpm-receipt.json', {
        'schema': 1, 'commit': receipt['commit'], 'ci_run_id': receipt['ci_run_id'],
        'source_sha256': receipt['sha256'], 'package': package.name, 'sha256': sha256(package),
    })


def verify_rpm(directory, deb_receipt):
    receipt = json.loads((directory/'rpm-receipt.json').read_text())
    name = f'Clash-of-Rust-{deb_receipt["version"]}-linux-x86_64.rpm'
    path = directory/name
    if (receipt.get('schema'), receipt.get('commit'), receipt.get('ci_run_id'),
            receipt.get('source_sha256'), receipt.get('package'), receipt.get('sha256')) != (
                1, deb_receipt['commit'], deb_receipt['ci_run_id'],
                deb_receipt['sha256'], name, sha256(path)):
        raise ValueError('RPM candidate differs from its verified CI and DEB inputs')
    return path


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    rpm_candidate()
