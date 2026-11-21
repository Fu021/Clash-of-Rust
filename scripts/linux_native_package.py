"""Build native compatibility packages from the verified Linux candidate payload."""
import argparse
import json
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

from build_support import ROOT, run, sha256, validate_version, write_json

RPM_ARCHITECTURES = {'x64': 'x86_64', 'arm64': 'aarch64'}
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
    base, _, prerelease = validate_version(version).partition('-')
    return base + (separator + prerelease.replace('-', '+') if prerelease else '')


def rpm_filename(version, arch):
    return f'Clash-of-Rust-{version}-linux-{RPM_ARCHITECTURES[arch]}.rpm'


def arch_filename(version):
    return f'Clash-of-Rust-{validate_version(version)}-linux-x86_64.pkg.tar.zst'


def build_rpm(deb, version, output, arch):
    rpm_arch = RPM_ARCHITECTURES[arch]
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
            f'BuildArch: {rpm_arch}\nAutoReqProv: no\n'
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
        packages = list((work/'RPMS'/rpm_arch).glob('*.rpm'))
        if len(packages) != 1:
            raise ValueError(f'Expected a single native {rpm_arch} RPM')
        output.mkdir(parents=True, exist_ok=True)
        result = output/rpm_filename(version, arch)
        shutil.copyfile(packages[0], result)
        return result


def build_arch(deb, version, output, expected_digest):
    if sha256(deb) != expected_digest:
        raise ValueError('Arch recipe source digest mismatch')
    pkgver = native_version(version, 'pre.')
    with tempfile.TemporaryDirectory(prefix='cor-arch-') as temporary:
        directory = Path(temporary)
        shutil.copyfile(deb, directory/'candidate.deb')
        recipe = (ROOT/'installer/arch/PKGBUILD').read_text()
        recipe = recipe.replace('@VERSION@', pkgver)
        recipe = recipe.replace('@SHA256@', expected_digest)
        (directory/'PKGBUILD').write_text(recipe)
        info = directory/'.SRCINFO'
        with info.open('w') as stream:
            subprocess.run(['makepkg', '--printsrcinfo'], cwd=directory, stdout=stream, check=True)
        values = {line.strip() for line in info.read_text().splitlines()}
        if not {'pkgname = clash-of-rust', 'arch = x86_64', 'source = candidate.deb',
                f'pkgver = {pkgver}', f'sha256sums = {expected_digest}'} <= values:
            raise ValueError('Arch recipe metadata differs from the verified input')
        subprocess.run(['makepkg', '--cleanbuild', '--noconfirm', '--nodeps'],
                       cwd=directory, check=True)
        packages = list(directory.glob('*.pkg.tar.zst'))
        if len(packages) != 1:
            raise ValueError('Expected a single native Arch package')
        output.mkdir(parents=True, exist_ok=True)
        result = output/arch_filename(version)
        shutil.copyfile(packages[0], result)
        return result


def rpm_candidate():
    source = ROOT/'dist/package-receipt.json'
    receipt = json.loads(source.read_text())
    deb = source.parent/receipt['package']
    arch = receipt['arch']
    if receipt['system'] != 'linux' or arch not in RPM_ARCHITECTURES or receipt['tests_passed'] is not True:
        raise ValueError('RPM assembly requires a verified Linux candidate')
    if deb.parent != source.parent or deb.suffix != '.deb' or sha256(deb) != receipt['sha256']:
        raise ValueError('RPM assembly input differs from the verified DEB')
    output = ROOT/'dist/linux-rpm'
    package = build_rpm(deb, receipt['version'], output, arch)
    write_json(output/'rpm-receipt.json', {
        'schema': 1, 'commit': receipt['commit'], 'ci_run_id': receipt['ci_run_id'],
        'source_sha256': receipt['sha256'], 'package': package.name, 'sha256': sha256(package),
    })


def verify_rpm(directory, deb_receipt):
    receipt = json.loads((directory/'rpm-receipt.json').read_text())
    name = rpm_filename(deb_receipt['version'], deb_receipt['arch'])
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
