"""Verify a local Debian artifact in an isolated dpkg root (Linux, root)."""
import argparse
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile

from build_support import sha256, validate_resources


def command(args, **kwargs):
    return subprocess.run([str(a) for a in args],check=True,**kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('artifact',type=Path)
    parser.add_argument('--upgrade-to',type=Path,help='Optional package to install over the initial isolated version')
    args = parser.parse_args()
    artifact = args.artifact.resolve()
    if os.name == 'nt' or os.geteuid() != 0:
        raise RuntimeError('Run this isolated dpkg test as root on Linux')
    assert artifact.with_name(artifact.name+'.sha256').read_text().strip() == sha256(artifact)
    fields = subprocess.check_output(['dpkg-deb','--field',str(artifact),'Package','Version','Architecture','Depends'],text=True)
    assert 'Package: clash-of-rust\n' in fields
    control_tar = subprocess.check_output(['dpkg-deb','--ctrl-tarfile',str(artifact)])
    with tarfile.open(fileobj=io.BytesIO(control_tar)) as control:
        assert {m.name.removeprefix('./') for m in control if m.isfile()} == {'control','md5sums'}, 'Unexpected maintainer script'
    with tempfile.TemporaryDirectory(prefix='clash-of-rust-deb-test-') as folder:
        root = Path(folder)
        # No host packages or desktop settings are changed by this dpkg root.
        command(['dpkg',f'--root={root}','--unpack',artifact])
        app = root/'opt/clash-of-rust'
        assert (app/'clash-of-rust').read_bytes()[:4] == b'\x7fELF'
        assert (app/'clash-of-rust').stat().st_mode & 0o111
        assert (root/'usr/bin/clash-of-rust').readlink() == Path('/opt/clash-of-rust/clash-of-rust')
        assert (root/'usr/share/applications/clash-of-rust.desktop').is_file()
        assert (root/'usr/share/icons/hicolor/128x128/apps/clash-of-rust.png').is_file()
        assert {p.name for p in (app/'resources/ip-check').iterdir()} == {'LICENSE','SOURCE.md'}
        assert not any(p.suffix in {'.sh','.ps1','.py','.dll','.exe'} for p in app.rglob('*') if p.is_file())
        validate_resources(app/'resources','linux')
        data = json.loads((app/'resources/core.json').read_text())
        version = subprocess.check_output([str(app/'resources/mihomo'),'-v'],text=True)
        assert data['version'] in version
        command(['md5sum','--check',root/'var/lib/dpkg/info/clash-of-rust.md5sums'],cwd=root)
        unrelated = app/'user-file.txt'
        unrelated.write_text('Keep unrelated user files')
        if args.upgrade_to:
            upgrade = args.upgrade_to.resolve(strict=True)
            assert upgrade.with_name(upgrade.name+'.sha256').read_text().strip() == sha256(upgrade)
            expected = subprocess.check_output(['dpkg-deb','--field',str(upgrade),'Version'],text=True).strip()
            command(['dpkg',f'--root={root}','--unpack',upgrade])
            actual = subprocess.check_output(['dpkg-query',f'--admindir={root / "var/lib/dpkg"}','--show','--showformat=${Version}','clash-of-rust'],text=True)
            assert actual == expected and unrelated.read_text() == 'Keep unrelated user files'
            command(['md5sum','--check',root/'var/lib/dpkg/info/clash-of-rust.md5sums'],cwd=root)
            print('PASS: isolated Debian upgrade to',expected)
        command(['dpkg',f'--root={root}','--purge','clash-of-rust'])
        assert not (app/'clash-of-rust').exists()
        assert not (root/'usr/bin/clash-of-rust').is_symlink()
        assert not (root/'usr/share/applications/clash-of-rust.desktop').exists()
        assert unrelated.read_text() == 'Keep unrelated user files'
    print('PASS: Debian metadata, no maintainer scripts, isolated unpack/purge, native command link, menu/icon, core/Geo hashes, package checksums and unrelated file preservation')
    print(fields.strip())


if __name__ == '__main__':
    main()
