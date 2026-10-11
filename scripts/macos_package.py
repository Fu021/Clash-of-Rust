"""Native .app/DMG assembly from verified CI binaries; no Rust recompilation."""
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
from build_support import ROOT, run, sha256, validate_binary_arch, write_json

APP_NAME = 'Clash of Rust.app'
BUNDLE_ID = 'org.clashofrust.desktop'


def build_app(payload, destination, version, arch):
    contents = destination/'Contents'
    binaries, resources = contents/'MacOS', contents/'Resources'
    binaries.mkdir(parents=True)
    resources.mkdir()
    for name in ('clash-of-rust', 'clash-tun-launcher'):
        validate_binary_arch(payload/name, 'macos', arch)
        shutil.copy2(payload/name, binaries/name)
        (binaries/name).chmod(0o755)
    shutil.copytree(payload/'resources', resources/'resources')
    shutil.copy2(payload/'LICENSE', resources/'LICENSE')
    info = {'CFBundleIdentifier': BUNDLE_ID, 'CFBundleName': 'Clash of Rust',
            'CFBundleDisplayName': 'Clash of Rust', 'CFBundleExecutable': 'clash-of-rust',
            'CFBundlePackageType': 'APPL', 'CFBundleShortVersionString': version.split('-')[0],
            'CFBundleVersion': version.split('-')[0], 'ClashOfRustVersion': version,
            'CFBundleIconFile': 'AppIcon', 'LSMinimumSystemVersion': '14.0',
            'NSHighResolutionCapable': True, 'NSSupportsAutomaticGraphicsSwitching': True}
    (contents/'Info.plist').write_bytes(plistlib.dumps(info))
    iconset = destination.parent/'AppIcon.iconset'
    iconset.mkdir()
    for size in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            name = f'icon_{size}x{size}'+('@2x' if scale == 2 else '')+'.png'
            run(['/usr/bin/sips', '-z', size*scale, size*scale, ROOT/'resources/icons/app.png', '--out', iconset/name], stdout=subprocess.DEVNULL)
    run(['/usr/bin/iconutil', '-c', 'icns', iconset, '-o', resources/'AppIcon.icns'])
    shutil.rmtree(iconset)
    identity = os.environ.get('MACOS_SIGNING_IDENTITY', '-')
    for binary in (resources/'resources/mihomo', binaries/'clash-tun-launcher', binaries/'clash-of-rust'):
        command = ['/usr/bin/codesign', '--force', '--sign', identity]
        if identity != '-': command += ['--options', 'runtime', '--timestamp']
        run([*command, binary])
    # Signing changes the Go Mach-O executable. Refresh the installed-byte hash
    # while keeping its verified official archive hash/source in the manifest.
    core_manifest = resources/'resources/core.json'
    import json
    core = json.loads(core_manifest.read_text())
    core['exe_sha256'] = sha256(resources/'resources/mihomo')
    write_json(core_manifest, core)
    command = ['/usr/bin/codesign', '--force', '--sign', identity]
    if identity != '-': command += ['--options', 'runtime', '--timestamp']
    run([*command, destination])
    run(['/usr/bin/codesign', '--verify', '--deep', '--strict', destination])
    return destination


def build_dmg(payload, version, arch):
    artifact = ROOT/f'dist/Clash-of-Rust-{version}-macos-{arch}.dmg'
    with tempfile.TemporaryDirectory(prefix='clash-macos-') as temporary:
        root = Path(temporary)
        build_app(payload, root/APP_NAME, version, arch)
        (root/'Applications').symlink_to('/Applications')
        artifact.unlink(missing_ok=True)
        run(['/usr/bin/hdiutil', 'create', '-volname', 'Clash of Rust', '-srcfolder', root,
             '-format', 'UDZO', '-ov', artifact])
    profile = os.environ.get('MACOS_NOTARY_PROFILE')
    if profile:
        if os.environ.get('MACOS_SIGNING_IDENTITY', '-') == '-':
            raise ValueError('Notarization requires a Developer ID signing identity')
        run(['/usr/bin/xcrun', 'notarytool', 'submit', artifact, '--keychain-profile', profile, '--wait'])
        run(['/usr/bin/xcrun', 'stapler', 'staple', artifact])
        run(['/usr/bin/xcrun', 'stapler', 'validate', artifact])
    return artifact
