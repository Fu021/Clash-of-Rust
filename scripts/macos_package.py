"""Native .app/DMG assembly from verified CI binaries; no Rust recompilation."""
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
import time
from build_support import ROOT, run, sha256, validate_binary_arch, write_json

APP_NAME = 'Clash of Rust.app'
BUNDLE_ID = 'org.clashofrust.desktop'
UI_APP = Path('Contents/Helpers/Clash of Rust UI.app')
UI_BUNDLE_ID = BUNDLE_ID+'.renderer'


def disk_image(arguments, timeout=120):
    """Retry only DiskImages' transient busy error, within one total deadline."""
    command = ['/usr/bin/hdiutil', *(str(value) for value in arguments)]
    deadline = time.monotonic()+timeout
    for attempt in range(5):
        remaining = deadline-time.monotonic()
        if remaining <= 0:
            raise subprocess.TimeoutExpired(command, timeout)
        result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=remaining)
        sys.stdout.write(result.stdout)
        sys.stderr.write(result.stderr)
        sys.stdout.flush()
        sys.stderr.flush()
        if result.returncode == 0:
            return result
        if 'Resource busy' not in result.stdout+result.stderr or attempt == 4:
            raise subprocess.CalledProcessError(result.returncode, command,
                                                result.stdout, result.stderr)
        print(f'DiskImages resource busy; retry {attempt+1}/4', flush=True)
        time.sleep(min(attempt+1, max(0, deadline-time.monotonic())))


def build_app(payload, destination, version, arch):
    contents = destination/'Contents'
    binaries, resources = contents/'MacOS', contents/'Resources'
    binaries.mkdir(parents=True)
    resources.mkdir()
    for name in ('clash-of-rust', 'clash-tun-launcher', 'cor'):
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
    # Launch Services must know this is an agent before the renderer executes.
    # Sharing the owner's bundle identity and changing activation policy later
    # can leave an additional Dock/recent-app entry after each relaunch.
    helper = destination/UI_APP
    helper_binary = helper/'Contents/MacOS/clash-desktop-ui'
    helper_binary.parent.mkdir(parents=True)
    validate_binary_arch(payload/'clash-desktop-ui','macos',arch)
    shutil.copy2(payload/'clash-desktop-ui',helper_binary)
    helper_binary.chmod(0o755)
    helper_info = {key:value for key,value in info.items() if key != 'CFBundleIconFile'}
    helper_info.update(CFBundleIdentifier=UI_BUNDLE_ID,CFBundleExecutable='clash-desktop-ui',LSUIElement=True)
    (helper/'Contents/Info.plist').write_bytes(plistlib.dumps(helper_info))
    iconset = destination.parent/'AppIcon.iconset'
    iconset.mkdir()
    for size in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            name = f'icon_{size}x{size}'+('@2x' if scale == 2 else '')+'.png'
            run(['/usr/bin/sips', '-z', size*scale, size*scale, ROOT/'resources/icons/app.png', '--out', iconset/name], stdout=subprocess.DEVNULL)
    run(['/usr/bin/iconutil', '-c', 'icns', iconset, '-o', resources/'AppIcon.icns'])
    shutil.rmtree(iconset)
    identity = os.environ.get('MACOS_SIGNING_IDENTITY', '-')
    for binary in (resources/'resources/mihomo', helper_binary, helper, binaries/'clash-tun-launcher', binaries/'cor', binaries/'clash-of-rust'):
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
        disk_image(['create', '-volname', 'Clash of Rust', '-srcfolder', root,
                    '-fs', 'HFS+', '-format', 'UDZO', '-ov', artifact])
    profile = os.environ.get('MACOS_NOTARY_PROFILE')
    if profile:
        if os.environ.get('MACOS_SIGNING_IDENTITY', '-') == '-':
            raise ValueError('Notarization requires a Developer ID signing identity')
        run(['/usr/bin/xcrun', 'notarytool', 'submit', artifact, '--keychain-profile', profile, '--wait'])
        run(['/usr/bin/xcrun', 'stapler', 'staple', artifact])
        run(['/usr/bin/xcrun', 'stapler', 'validate', artifact])
    return artifact
