"""Build Android on Actions and package the exact APK tested by its CI."""
import argparse
from collections import deque
from contextlib import ExitStack, contextmanager, suppress
import errno
import io
import json
import math
import os
import random
import re
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import time
import uuid
import zipfile

from build_support import ROOT, download, host_arch, mihomo_go_version, package_version, sha256, write_json, write_mihomo_notice
from build_inputs import android_versions

ANDROID = ROOT / 'apps/android'
ADB = str(Path(os.environ.get('ANDROID_HOME', '/nonexistent')) / 'platform-tools/adb')
ABIS = {
    'arm64': ('arm64-v8a', 'aarch64-linux-android', 'aarch64-linux-android', 'arm64'),
    'armv7': ('armeabi-v7a', 'armv7-linux-androideabi', 'armv7a-linux-androideabi', 'arm'),
    'x64': ('x86_64', 'x86_64-linux-android', 'x86_64-linux-android', 'amd64'),
}
MOBILE_TOOL = '19777044486779222400e9ca16c4306f9c6dbfa9'
PROTECTION_DEATH_LAUNCH_FILE = 'cor-protection-death-launch'
PROTECTION_DEATH_GUEST_LAUNCH = '/data/local/tmp/' + PROTECTION_DEATH_LAUNCH_FILE
PROTECTION_DEATH_BOOT_COUNT = ['shell', 'settings', 'get', 'global', 'boot_count']
ANDROID_TEST_DIAGNOSTICS = {
    'connectivity': ['dumpsys', 'connectivity'],
    'routes': ['ip', 'route', 'show', 'table', 'all'],
    'vpn': ['dumpsys', 'vpn_management'],
    'windows': ['dumpsys', 'window', 'windows'],
    'activities': ['dumpsys', 'activity', 'activities'],
    'power': ['dumpsys', 'power'],
}
GRADLE_BUILD_TASKS = ('assembleRelease', 'assembleReleaseAndroidTest', 'lintRelease')
LINUX_X64_EMULATOR_ARCHIVES = {
    '37.2.13': ('https://dl.google.com/android/repository/emulator-linux_x64-16740729.zip',
                'cbc7c3cfb83220b8a0f9c513fdab557712e74d339ec19ddffa43f23350ae9bdd'),
}


def run(command, **kwargs):
    print('+ ' + ' '.join(map(str, command)), flush=True)
    return subprocess.run(list(map(str, command)), check=True, **kwargs)


def streamed_command(command, **kwargs):
    print('+ ' + ' '.join(map(str, command)), flush=True)
    tail = deque(maxlen=160)
    with subprocess.Popen(list(map(str, command)), stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, text=True, **kwargs) as process:
        for line in process.stdout:
            print(line, end='', flush=True)
            tail.append(line)
        status = process.wait()
    return status, ''.join(tail)


def cargo_build(command, *, restored_profiles=(), **kwargs):
    """Retry dependency downloads and recover once from a missing cached Slint crate."""
    transient = re.compile(r'(?:HTTP(?:/[0-9.]+)?\s+|returned error:\s*)(?:429|50[0234])\b'
                           r'|Could not resolve host|Failed to connect to|Connection timed out', re.I)
    for attempt in range(3):
        status, tail = streamed_command(command, **kwargs)
        if (status and restored_profiles
                and re.search(r"\[E0463\].*can't find crate for `slint(?:_macros)?`", tail)):
            print('::warning::Restored Android compiler objects could not supply Slint; '
                  'rebuilding the host and current ABI release objects once', flush=True)
            for profile in restored_profiles:
                if profile.is_dir() and profile.resolve() == profile:
                    shutil.rmtree(profile)
            restored_profiles = ()
            status, tail = streamed_command(command, **kwargs)
        if status == 0:
            return
        if attempt == 2 or not transient.search(tail):
            raise subprocess.CalledProcessError(status, command)
        delay = 15 * (attempt + 1)
        print(f'Transient dependency download failure; retrying in {delay}s', flush=True)
        time.sleep(delay)


def install_sdk_packages(command, device_root):
    """Retry damaged or unavailable SDK downloads with a fresh user cache."""
    transient = re.compile(r'Error reading Zip content from a SeekableByteChannel'
                           r'|Error on ZipFile unknown archive|Connection timed out|Read timed out'
                           r'|Could not resolve host|Failed to connect'
                           r'|Failed to download package archive[^\r\n]*:\s*'
                           r'(?:closed|unexpected EOF|connection reset by peer)\b'
                           r'|An error occurred while preparing SDK package[^\r\n]*:\s*Premature EOF\b'
                           r'|gzip decompression failed:\s*'
                           r'(?:Peer disconnected|unexpected EOF|connection reset by peer)\b'
                           r'|HTTP(?:/[0-9.]+)?\s+(?:429|50[0234])\b', re.I)
    for attempt in range(3):
        user_root = device_root/('sdk-download-'+str(attempt))
        user_home = user_root/'.android'
        user_home.mkdir(parents=True, exist_ok=True)
        env = dict(os.environ, ANDROID_SDK_HOME=str(user_root), ANDROID_USER_HOME=str(user_home))
        status, tail = streamed_command(command, env=env)
        if status == 0:
            return
        if attempt == 2 or not transient.search(tail):
            raise subprocess.CalledProcessError(status, command)
        delay = 15 * (attempt + 1)
        print(f'Transient Android SDK download failure; retrying in {delay}s', flush=True)
        time.sleep(delay)


def commit():
    return subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()


def device_property(name, adb=None):
    command = (adb or [ADB]) + ['shell', 'getprop', name]
    for attempt in range(3):
        try:
            return subprocess.check_output(command, text=True, stderr=subprocess.PIPE, timeout=15).strip()
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
            if isinstance(error, subprocess.CalledProcessError):
                detail = error.stderr or ''
                if error.returncode != 255 and not re.search(
                        r'device offline|device not found|no devices|device .* not found|connection.*closed',
                        detail, re.I):
                    raise
            if attempt == 2:
                raise
            print(f'Transient ADB property read failure ({name}); retrying', flush=True)
            time.sleep(attempt + 1)


def sdk_tools():
    return Path(os.environ['ANDROID_HOME']) / 'build-tools' / android_versions()['build_tools']


def sdk_bin():
    return Path(os.environ['ANDROID_HOME']) / 'cmdline-tools' / android_versions()['cmdline_tools'] / 'bin'


def sdk_install_command(directory, packages):
    sdk = Path(os.environ['ANDROID_HOME'])
    return [directory/'android', '--sdk='+str(sdk), 'sdk', 'install', '--force',
            *(package.replace(';', '/') for package in packages)]


def prepare_sdk_manager():
    sdk = Path(os.environ['ANDROID_HOME'])
    versions = android_versions()
    device_root = Path(os.environ['RUNNER_TEMP']) / 'cor-sdk-install'
    manager = sdk_bin()
    if not (manager/'android').is_file():
        bootstrap = sdk/'cmdline-tools/latest/bin'
        package = 'cmdline-tools;' + versions['cmdline_tools']
        command = sdk_install_command(bootstrap, [package]) if (bootstrap/'android').is_file() else [
            bootstrap/'sdkmanager', package]
        install_sdk_packages(command, device_root)
    return manager


def prepare_sdk(package_only=False):
    sdk = Path(os.environ['ANDROID_HOME'])
    versions = android_versions()
    device_root = Path(os.environ['RUNNER_TEMP']) / 'cor-sdk-install'
    manager = prepare_sdk_manager()
    packages = ['platform-tools@' + versions['platform_tools'], 'build-tools;' + versions['build_tools']]
    if not package_only:
        packages += ['platforms;android-' + versions['platform'], 'ndk;' + versions['ndk']]
    install_sdk_packages(sdk_install_command(manager, packages), device_root)
    if not package_only:
        with open(os.environ['GITHUB_ENV'], 'a') as stream:
            stream.write('ANDROID_NDK_ROOT=' + str(sdk / 'ndk' / versions['ndk']) + '\n')


def emulator_image(legacy=False, api=None):
    versions = android_versions()
    image_key = 'legacy_system_image' if legacy else 'system_image'
    if api is not None:
        if versions['system_image'].split(';')[1] == 'android-'+str(api):
            image_key = 'system_image'
        elif versions['legacy_system_image'].split(';')[1] == 'android-'+str(api):
            image_key = 'legacy_system_image'
        else:
            selected = versions['ime_system_images'].get(str(api))
            if selected is None:
                raise ValueError('No approved Android test image for API '+str(api))
            tag = selected.get('tag', 'google_apis')
            if tag not in ('default', 'google_apis'):
                raise ValueError('Unsupported Android test image tag: '+str(tag))
            return 'system-images;android-'+str(api)+';'+tag+';x86_64', selected['revision'], selected['source_revision']
    return (versions[image_key], versions[image_key+'_revision'],
            versions.get(image_key+'_source_revision', versions[image_key+'_revision']))


def emulator_sdk_packages(image, image_revision, preview=False, *, include_emulator=True):
    versions = android_versions()
    packages = ['platform-tools@'+versions['platform_tools'], image+'@'+image_revision]
    if include_emulator:
        packages.insert(1, 'emulator@'+versions['emulator'])
    if preview:
        packages.append('build-tools;'+versions['build_tools'])
    return packages


def verify_emulator_versions(sdk, legacy=False, api=None, *, emulator=None):
    versions = android_versions()
    image, _, source_revision = emulator_image(legacy, api)
    def revision(value):
        parts = value.split('.')
        return tuple(int(part) for part in parts + ['0'] * (3 - len(parts)))
    relative = image.replace(';', '/')
    for name, directory, expected in (('emulator', emulator or sdk/'emulator', versions['emulator']),
                                      (relative, sdk/relative, source_revision)):
        properties = (directory/'source.properties').read_text()
        match = re.search(r'(?m)^\s*Pkg\.Revision\s*=\s*([0-9.]+)\s*$', properties)
        if not match or revision(match[1]) != revision(expected):
            actual = match[1] if match else 'missing'
            raise ValueError(f'Android runtime revision differs from the approved version: {name}: '
                             f'expected {expected}, got {actual}')
        print('Verified Android runtime:', name, match[1], flush=True)


def prepare_emulator(sdk, manager, device_root, *, legacy=False, api=None):
    version = android_versions()['emulator']
    archived = LINUX_X64_EMULATOR_ARCHIVES.get(version) if sys.platform == 'linux' and host_arch() == 'x64' else None
    image, image_revision, _ = emulator_image(legacy, api)
    packages = emulator_sdk_packages(image, image_revision, preview=api is not None,
                                     include_emulator=archived is None)
    install_sdk_packages(sdk_install_command(manager, packages), device_root)
    emulator = sdk/'emulator'
    if archived:
        url, digest = archived
        print(f'Preparing Android emulator {version} from the verified Google archive', flush=True)
        runtime = Path(tempfile.mkdtemp(prefix='runtime-', dir=device_root))
        with tempfile.TemporaryDirectory(prefix='download-', dir=device_root) as temporary:
            archive = Path(temporary)/'emulator.zip'
            download(url, archive, digest=digest)
            run(['unzip', '-q', archive, '-d', runtime], timeout=120)
        emulator = runtime/'emulator'
    verify_emulator_versions(sdk, legacy, api, emulator=emulator)
    if archived:
        major, minor, micro = version.split('.')
        (emulator/'package.xml').write_text(
            '<sdk:repository xmlns:sdk="http://schemas.android.com/repository/android/common/02" '
            'xmlns:generic="http://schemas.android.com/repository/android/generic/02" '
            'xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">\n'
            '  <localPackage path="emulator" obsolete="false">\n'
            '    <type-details xsi:type="generic:genericDetailsType"/>\n'
            f'    <revision><major>{major}</major><minor>{minor}</minor><micro>{micro}</micro></revision>\n'
            '    <display-name>Android Emulator</display-name>\n'
            '  </localPackage>\n'
            '</sdk:repository>\n', encoding='utf-8')
    return emulator/'emulator', image


@contextmanager
def sdk_emulator_for_avd(sdk, emulator):
    sdk_emulator = sdk/'emulator'
    if sdk_emulator == emulator.parent:
        yield
        return
    with ExitStack() as cleanup:
        if sdk_emulator.exists() or sdk_emulator.is_symlink():
            backup = emulator.parent.with_name('original-sdk-emulator')
            saved = shutil.move(sdk_emulator, backup)
            cleanup.callback(shutil.move, saved, sdk_emulator)
        sdk_emulator.symlink_to(emulator.parent.resolve(), target_is_directory=True)
        cleanup.callback(sdk_emulator.unlink)
        yield


def configure_emulator_storage(avd, report):
    userdata_mib = 6144
    required = ((userdata_mib * 12 + 9) // 10 + 1024) * 1024**2
    free = shutil.disk_usage(avd).free
    storage = {
        'userdata_mib': userdata_mib, 'required_free_bytes': required,
        'free_bytes_before': free, 'free_bytes': free, 'removed_host_tools': [],
    }
    write_json(report/'storage.json', storage)
    hosted_linux = (os.environ.get('GITHUB_ACTIONS') == 'true'
                    and os.environ.get('RUNNER_ENVIRONMENT') == 'github-hosted'
                    and os.environ.get('RUNNER_OS') == 'Linux')
    if free < required and hosted_linux:
        run(['sudo', '-n', 'rm', '-rf', '--', '/usr/share/dotnet'], timeout=120)
        free = shutil.disk_usage(avd).free
        storage.update(free_bytes=free, removed_host_tools=['/usr/share/dotnet'])
        write_json(report/'storage.json', storage)
    if free < required:
        raise RuntimeError(f'Android emulator needs at least {required / 1024**3:.1f} GiB free '
                           f'for its {userdata_mib} MiB userdata partition; '
                           f'available: {free / 1024**3:.2f} GiB at {avd}')
    config = avd/'config.ini'
    lines = [line for line in config.read_text().splitlines()
             if line.partition('=')[0].strip() != 'disk.dataPartition.size']
    lines.append(f'disk.dataPartition.size={userdata_mib}M')
    config.write_text('\n'.join(lines)+'\n')
    print(f'Disposable Android emulator: {userdata_mib} MiB userdata; '
          f'{free / 1024**3:.2f} GiB disk free', flush=True)


def ensure_binding_tool(name):
    package = 'golang.org/x/mobile/cmd/' + name
    selected = re.search(r'(?m)^(?:require\s+)?\s*golang.org/x/mobile\s+(v\S+)',
                         (ANDROID/'core/go.mod').read_text())
    revision = selected[1] if selected else MOBILE_TOOL
    executable = shutil.which(name)
    if executable:
        compiler = subprocess.check_output(['go', 'env', 'GOVERSION'], text=True).strip()
        metadata = subprocess.run(['go', 'version', '-m', executable], text=True, capture_output=True)
        records = [line.split() for line in metadata.stdout.splitlines()]
        if metadata.returncode == 0 and records and records[0][-1] == compiler and ['path', package] in records and any(
                len(record) >= 3 and record[:2] == ['mod', 'golang.org/x/mobile']
                and (record[2] == revision or record[2].endswith('-' + revision[:12])) for record in records):
            print('Reusing verified binding tool:', name, flush=True)
            return
    run(['go', 'install', package + '@' + revision])


def gradle(*args, arch=None, offline=False):
    code = 1_000_000 + int(os.environ.get('GITHUB_RUN_NUMBER', '1'))
    command = ['gradle', '--no-daemon', '--warning-mode', 'all', '-p', ANDROID,
               '-PappVersion=' + package_version(), '-PappVersionCode=' + str(code),
               '-PappNdkVersion=' + android_versions()['ndk']]
    if os.environ.get('RUNNER_DEBUG') == '1':
        command.extend(['--info', '-Dorg.gradle.deprecation.trace=true'])
    if offline:
        command.append('--offline')
    if arch:
        command += ['-PappAbi=' + ABIS[arch][0]]
    return [*command, *args]


def seed_resources():
    generated = ANDROID/'app/generated-res'
    if generated.exists():
        shutil.rmtree(generated)
    shutil.copytree(ROOT/'translations/android/system', generated)
    # Android adaptive launchers do not need the 1254-pixel source artwork.
    logo = generated/'drawable-nodpi/app_logo.png'
    logo.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ROOT/'resources/icons/app-android.png', logo)
    shutil.copy2(ROOT/'resources/icons/app.png', logo.with_name('notification_logo.png'))


def seed_assets():
    from build_support import read_json, download_asset
    from build_inputs import resource_release
    target = ANDROID/'app/offline-assets'
    target.mkdir(parents=True, exist_ok=True)
    seed_resources()
    release = resource_release('geo', target/'geodata.json')
    records = []
    for remote, local in {'geoip.dat':'GeoIP.dat', 'geosite.dat':'GeoSite.dat', 'country.mmdb':'Country.mmdb', 'GeoLite2-ASN.mmdb':'ASN.mmdb'}.items():
        item = next(a for a in release['assets'] if a['name'] == remote)
        download_asset(item, target/local)
        records.append({'name': local, 'source': item['browser_download_url'],
                        'sha256': item['digest'].removeprefix('sha256:'), 'size': item['size']})
    write_json(target/'geodata.json', {'version':release['tag_name']+' | '+release['published_at'],
                                     'updated':int(time.time()), 'files':records, 'release': release})
    shutil.copy2(ROOT/'LICENSE', target/'LICENSE')
    write_mihomo_notice(target/'THIRD-PARTY-NOTICES.txt', mihomo_go_version(ANDROID/'core/go.mod'))
    for directory in ('fonts', 'countries', 'ip-check'):
        for path in (ROOT/'resources'/directory).iterdir():
            if path.name.startswith(('LICENSE', 'SOURCE')):
                destination = target/'licenses'/directory/path.name
                destination.parent.mkdir(parents=True,exist_ok=True)
                shutil.copy2(path,destination)


def validate_apk(path, arch):
    """Reject mismatched libraries, host paths and 4 KiB-only 64-bit ELF files."""
    abi = ABIS[arch][0]
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)) or any(n.startswith('/') or '..' in Path(n).parts for n in names):
            raise ValueError('Invalid APK paths')
        libraries = [n for n in names if n.startswith('lib/') and n.endswith('.so')]
        if {n.split('/')[1] for n in libraries} != {abi}:
            raise ValueError('APK ABI differs from selected architecture')
        if f'lib/{abi}/libclash_mobile.so' not in libraries or f'lib/{abi}/libgojni.so' not in libraries:
            raise ValueError('APK is missing the UI or mihomo native library')
        expected_machine = {'arm64':183, 'armv7':40, 'x64':62}[arch]
        for name in libraries:
            data = archive.read(name)
            if data[:4] != b'\x7fELF' or data[5] != 1 or struct.unpack_from('<H', data, 18)[0] != expected_machine:
                raise ValueError('Invalid Android ELF: '+name)
            if arch != 'armv7':
                offset = struct.unpack_from('<Q', data, 32)[0]
                size, count = struct.unpack_from('<HH', data, 54)
                for index in range(count):
                    start = offset + size * index
                    if struct.unpack_from('<I', data, start)[0] == 1 and struct.unpack_from('<Q', data, start+48)[0] < 16384:
                        raise ValueError('Native library is not 16 KiB aligned: '+name)
        sizes={'apk_bytes':path.stat().st_size, 'native_compressed_bytes':sum(archive.getinfo(n).compress_size for n in libraries),
               'native_bytes':sum(archive.getinfo(n).file_size for n in libraries)}
    tool=sdk_tools()/'aapt2'
    metadata=subprocess.check_output([str(tool),'dump','badging',str(path)],text=True)
    report=ROOT/'target/android-report'/arch
    report.mkdir(parents=True,exist_ok=True)
    (report/'apk-badging.txt').write_text(metadata)
    return {**sizes,'version_code':apk_identity(metadata,package_version())}


def apk_identity(metadata, version):
    package=re.search(r"package: name='([^']+)' versionCode='([0-9]+)' versionName='([^']+)'",metadata)
    if not package or package[1]!='org.clashofrust.android' or package[3]!=version:
        raise ValueError('APK package/version differs from source')
    # Current aapt2 calls this minSdkVersion; older versions print sdkVersion.
    minimum=re.search(r"(?m)^(?:minSdkVersion|sdkVersion):'([0-9]+)'$",metadata)
    target=re.search(r"(?m)^targetSdkVersion:'([0-9]+)'$",metadata)
    if not minimum or minimum[1]!='26' or not target or target[1]!='36':
        raise ValueError('APK SDK range differs from selected Android host')
    return int(package[2])



def bind_core(arch, env):
    ensure_binding_tool('gomobile')
    ensure_binding_tool('gobind')
    aar = ANDROID/'app/libs/mihomo.aar'
    aar.parent.mkdir(parents=True, exist_ok=True)
    # VpnService owns routing and package exclusions. The default Android
    # listener tries to read root-only /data/system/packages.xml; cmfa
    # selects the upstream app-hosted path. Our TUN explicitly uses gVisor.
    run(['gomobile', 'bind', '-target', 'android/'+ABIS[arch][3], '-androidapi', '26',
         '-tags', 'cmfa,with_gvisor',
         '-javapkg', 'org.clashofrust.nativecore', '-ldflags=-s -w -extldflags=-Wl,-z,max-page-size=16384',
         '-o', aar, '.'], cwd=ANDROID/'core', env=env)


def prepare_maven_cache():
    home = Path(os.environ['GRADLE_USER_HOME'])
    modules = Path('caches/modules-2')
    cache = home / modules
    arch = 'x64'
    bind_core(arch, dict(os.environ, ANDROID_NDK_HOME=os.environ['ANDROID_NDK_ROOT']))
    seed_resources()
    with tempfile.TemporaryDirectory(prefix='cor-gradle-online-', dir=home.parent) as temporary:
        online = dict(os.environ, GRADLE_USER_HOME=temporary)
        command = gradle(*GRADLE_BUILD_TASKS, arch=arch)
        for attempt in range(5):
            status, tail = streamed_command(command, cwd=ROOT, env=online)
            if status == 0:
                break
            if 'Received status code 429' not in tail or attempt == 4:
                raise subprocess.CalledProcessError(status, command, output=tail)
            delay = random.uniform(15, 45) * (attempt + 1)
            print(f'Maven rate limited; retrying with the same Gradle cache in {delay:.0f}s',
                  flush=True)
            time.sleep(delay)
        if cache.exists():
            shutil.rmtree(cache)
        shutil.copytree(Path(temporary) / modules, cache,
                        ignore=shutil.ignore_patterns('*.lock', 'gc.properties'))
    run(gradle('--no-build-cache', '--project-cache-dir', home/'project-cache',
               'clean', *GRADLE_BUILD_TASKS, arch=arch, offline=True), cwd=ROOT)
    for path in cache.rglob('*'):
        if path.is_file() and (path.suffix == '.lock' or path.name == 'gc.properties'):
            path.unlink()


def cargo_target_directory():
    ci = os.environ.get('GITHUB_ACTIONS') == 'true'
    configured = os.environ.get('CARGO_TARGET_DIR')
    directory = Path(configured) if configured else ROOT/'target' if ci else ROOT/'target/mobile'
    directory = (ROOT/directory).resolve() if not directory.is_absolute() else directory.resolve()
    default = (ROOT/'target').resolve()
    if ci and directory != default:
        raise ValueError('Android CI requires the runner target directory')
    if not ci and directory == default:
        raise ValueError('Android builds require a target directory separate from desktop')
    return directory


def build(arch, gradle_offline=False):
    from action_cache import compiler_cache_checkpoint, workspace_sources
    from ci_test_policy import enabled, extended_tests, mobile_ui_tests
    abi, target, clang_target, _ = ABIS[arch]
    ci = os.environ.get('GITHUB_ACTIONS') == 'true'
    cargo_output = cargo_target_directory()
    build_env = dict(os.environ, CARGO_TARGET_DIR=str(cargo_output))
    if ci:
        workspace_sources(ROOT)
    profiles = (cargo_output/'release', cargo_output/target/'release')
    restored_profiles = profiles if (
        ci and any(profile.is_dir() for profile in profiles)
    ) else ()
    packages = ['-p', 'clash-mobile-runtime', '-p', 'clash-android']
    ui_tests = arch == 'x64' and mobile_ui_tests()
    targets = ['--all-targets', '--features', 'clash-android/mobile-ui-tests'] if ui_tests else ['--lib']
    cargo_build(['cargo', 'clippy', *packages, '--release', *targets, '--locked', '--', '-D', 'warnings'],
                cwd=ROOT, env=build_env)
    test_packages = packages if extended_tests() or enabled('COR_CI_ANDROID_ROUTING_TESTS') else ['-p', 'clash-android']
    ui_targets = (['--features', 'clash-android/mobile-ui-tests',
                   '--test', 'mobile_lists', '--test', 'scroll_remount', '--test', 'claude_report_blocks']
                  if ui_tests else [])
    run(['cargo', 'test', *test_packages, '--release', '--locked', '--lib', *ui_targets,
         '--no-fail-fast', '--', '--test-threads=1'], cwd=ROOT, env=build_env)
    ndk = Path(os.environ['ANDROID_NDK_ROOT'])
    toolchain = ndk/'toolchains/llvm/prebuilt/linux-x86_64/bin'
    clang = toolchain/(clang_target+'26-clang')
    env = dict(build_env)
    # Fetch Skia's pinned HarfBuzz commit from the official upstream. The
    # Googlesource mirror has returned 503; git-sync-deps still verifies the
    # exact DEPS commit. Scope the rewrite to this build, not global Git config.
    git_config = int(env.get('GIT_CONFIG_COUNT', '0'))
    for key, value in (
        ('url.https://github.com/harfbuzz/harfbuzz.git.insteadOf',
         'https://chromium.googlesource.com/external/github.com/harfbuzz/harfbuzz.git'),
        # Google's ICU mirror has failed during HTTP/2/Git v2 pack negotiation.
        # The same pinned commit is accessible with the conservative protocols.
        ('http.version', 'HTTP/1.1'),
        ('protocol.version', '1'),
    ):
        env[f'GIT_CONFIG_KEY_{git_config}'] = key
        env[f'GIT_CONFIG_VALUE_{git_config}'] = value
        git_config += 1
    env['GIT_CONFIG_COUNT'] = str(git_config)
    env.update({
        'CARGO_TARGET_'+target.upper().replace('-', '_')+'_LINKER':str(clang),
        'CC_'+target.replace('-', '_'):str(clang),
        'CXX_'+target.replace('-', '_'):str(clang)+'++',
        'AR_'+target.replace('-', '_'):str(toolchain/'llvm-ar'),
        'ANDROID_NDK_HOME':str(ndk),
        'ANDROID_PLATFORM':'android-'+android_versions()['platform'],
        'ANDROID_BUILD_TOOLS_VERSION':android_versions()['build_tools'],
        'CXXSTDLIB':'c++_static',
        'RUSTFLAGS':'-C link-arg=-Wl,-z,max-page-size=16384',
    })
    cargo_build(['cargo', 'clippy', '-p', 'clash-android', '--target', target, '--release', '--locked',
                 '--', '-D', 'warnings'], cwd=ROOT, env=env, restored_profiles=restored_profiles)
    cargo_build(['cargo', 'build', '-p', 'clash-android', '--target', target, '--release', '--locked'], cwd=ROOT, env=env)
    if ci:
        workspace_sources(ROOT, save=True)
    compiler_cache_checkpoint(ROOT)
    destination = ANDROID/'app/native-libs'/abi
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copy2(cargo_output/target/'release/libclash_mobile.so', destination)
    # Android's supported backend is Skia, not the desktop software renderer.
    core = ANDROID/'core'
    run(['go', 'mod', 'tidy', '-diff'], cwd=core, env=env)
    if arch == 'x64' and (extended_tests() or enabled('COR_CI_ANDROID_ROUTING_TESTS')):
        run(['go', 'test', '-tags', 'cmfa,with_gvisor', '-count=1', '.'], cwd=core, env=env)
    bind_core(arch, env)
    seed_assets()
    try:
        run(gradle(*GRADLE_BUILD_TASKS, arch=arch, offline=gradle_offline), cwd=ROOT)
    finally:
        report=ROOT/'target/android-report'/arch
        report.mkdir(parents=True,exist_ok=True)
        for path in (ANDROID/'app/build/reports').glob('lint-results-release.*'):
            shutil.copy2(path,report/path.name)
    apk = ANDROID/'app/build/outputs/apk/release/app-release.apk'
    from android_apk import optimize
    compression = optimize(apk, sdk_tools())
    sizes = validate_apk(apk, arch)
    report = {'commit':commit(), 'arch':arch, 'version':package_version(),
              'compression': compression, **sizes}
    write_json(ROOT/'target/android-report'/arch/'build.json', report)
    print(json.dumps(report), flush=True)
    if ci:
        workspace_sources(ROOT, save=True)


def export_build(arch):
    from workflow_artifacts import ARCHIVE_NAME
    source = ANDROID/'app/build/outputs/apk'
    output = ROOT/'dist/android-build'
    output.mkdir(parents=True, exist_ok=True)
    records = {
        'payload/app.apk':source/'release/app-release.apk',
        'tests/androidTest.apk':source/'androidTest/release/app-release-androidTest.apk',
    }
    manifest = {'schema':2, 'commit':commit(), 'version':package_version(), 'system':'android', 'arch':arch,
                'profile':'release', 'cargo_lock_sha256':sha256(ROOT/'Cargo.lock'),
                'files':{name:{'sha256':sha256(path),'size':path.stat().st_size,'executable':False} for name,path in records.items()},
                'tests':{'android-proxy-tun':'tests/androidTest.apk'}}
    from build_inputs import verified_manifest, toolchain_report
    manifest['build_inputs'] = verified_manifest()
    manifest['toolchain'] = toolchain_report('android')
    with tarfile.open(output/ARCHIVE_NAME, 'w:gz', compresslevel=1) as tar:
        for name,path in records.items():
            tar.add(path, arcname=name)
        content=json.dumps(manifest).encode()
        entry=tarfile.TarInfo('manifest.json'); entry.size=len(content)
        tar.addfile(entry, io.BytesIO(content))
    (output/(ARCHIVE_NAME+'.sha256')).write_text(sha256(output/ARCHIVE_NAME)+'\n')


def verify_build(root, arch, expected_commit):
    manifest = json.loads((root/'manifest.json').read_text())
    from build_inputs import manifest as input_manifest
    if snapshot := input_manifest():
        frozen = manifest.get('build_inputs', {})
        if any(frozen.get(key) != snapshot.get(key) for key in ('commit', 'baseline', 'constraints', 'files')):
            raise ValueError('Android dependency snapshot differs from the verified CI')
    expected = {'payload/app.apk', 'tests/androidTest.apk'}
    if (manifest.get('schema'),manifest.get('commit'),manifest.get('version'),manifest.get('system'),
        manifest.get('arch'),manifest.get('profile'),manifest.get('cargo_lock_sha256')) != (
        2,expected_commit,package_version(),'android',arch,'release',sha256(ROOT/'Cargo.lock')):
        raise ValueError('Android CI payload differs from selected source')
    actual = {p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file()}
    if set(manifest['files']) != expected or actual - {'manifest.json','source-artifact.json'} != expected:
        raise ValueError('Unexpected Android build files')
    if manifest.get('tests') != {'android-proxy-tun':'tests/androidTest.apk'}:
        raise ValueError('Android integration test is missing')
    for name,record in manifest['files'].items():
        path=root/name
        if path.is_symlink() or path.stat().st_size != record['size'] or sha256(path) != record['sha256']:
            raise ValueError('Android payload checksum mismatch')
    validate_apk(root/'payload/app.apk', arch)
    return manifest


def adb_evidence_command(command, timeout, binary=False):
    try:
        return subprocess.run(command,capture_output=True,text=not binary,timeout=timeout)
    except subprocess.TimeoutExpired as error:
        stdout = error.stdout or b''
        stderr = error.stderr or b''
        notice = f'ADB command timed out after {timeout}s'
        if binary:
            stdout = stdout.encode() if isinstance(stdout, str) else stdout
            stderr = stderr.encode() if isinstance(stderr, str) else stderr
            stderr += (b'\n' if stderr else b'') + notice.encode()
        else:
            stdout = stdout.decode(errors='replace') if isinstance(stdout, bytes) else stdout
            stderr = stderr.decode(errors='replace') if isinstance(stderr, bytes) else stderr
            stderr += ('\n' if stderr else '') + notice
        return subprocess.CompletedProcess(command, 255, stdout, stderr)


def adb_transport_failed(result):
    return result.returncode != 0 and bool(re.search(
        r'device offline|device not found|no devices|connection.*closed|'
        r'failed to get feature set|ADB command timed out', result.stdout+result.stderr, re.I))


def record_adb_health(report):
    records={}
    for name,command in (
            ('devices',[ADB,'devices','-l']),
            ('processes',['ps','-eo','pid,ppid,etimes,rss,comm'])):
        result=adb_evidence_command(command,10)
        records[name]={'returncode':result.returncode,'output':result.stdout+result.stderr}
    for name in ('meminfo','pressure/memory'):
        try:
            records[name]=Path('/proc',name).read_text()
        except OSError as error:
            records[name]={'unavailable':type(error).__name__}
    write_json(report/'adb-transport-health.json',{'schema':1,'observed_at':time.time(),'records':records})


def wait_adb_identity(output, expected=None, adb=None):
    adb = adb or [ADB]
    for attempt in range(3):
        connected=adb_evidence_command(adb+['wait-for-device'],30)
        output.append(connected.stdout+connected.stderr)
        if connected.returncode==0:
            identity=adb_evidence_command(adb+['shell','id','-u'],15)
            output.append(identity.stdout+identity.stderr)
            uid=identity.stdout.strip()
            if identity.returncode==0 and uid.isdigit() and (expected is None or uid==expected):
                return uid
        if attempt<2:
            time.sleep(attempt+1)
    return None


def pull_device_report(remote, report, required=False, required_pattern='*.png', name='ime-report'):
    destination=report/name
    destination.mkdir(exist_ok=True)
    command=[ADB,'pull',remote.rstrip('/')+'/.',str(destination)]
    result=adb_evidence_command(command,30)
    output=[result.stdout+result.stderr]
    restored=True
    if required and adb_transport_failed(result):
        record_adb_health(report)
        if wait_adb_identity(output) is not None:
            result=adb_evidence_command(command,30)
            output.append(result.stdout+result.stderr)
    if result.returncode and required and re.search(r'permission denied', result.stderr+result.stdout, re.I):
        properties=[adb_evidence_command([ADB,'shell','getprop',name],15)
                    for name in ('ro.kernel.qemu','ro.debuggable')]
        output.extend(value.stdout+value.stderr for value in properties)
        if all(value.returncode==0 and value.stdout.strip()=='1' for value in properties):
            previous=wait_adb_identity(output)
            if previous is not None and previous!='0':
                try:
                    restart=adb_evidence_command([ADB,'root'],30)
                    output.append(restart.stdout+restart.stderr)
                    if wait_adb_identity(output,'0')=='0':
                        result=adb_evidence_command(command,30)
                        output.append(result.stdout+result.stderr)
                finally:
                    restart=adb_evidence_command([ADB,'unroot'],30)
                    output.append(restart.stdout+restart.stderr)
                    restored=wait_adb_identity(output,previous)==previous
    pull_log = name.removesuffix('-report') + '-pull.txt'
    (report/pull_log).write_text('\n'.join(output))
    if not restored:
        raise RuntimeError(f'Android evidence export did not restore the original ADB identity; inspect {pull_log}')
    if required and (result.returncode or not any(destination.rglob(required_pattern))):
        evidence='screenshot' if required_pattern=='*.png' else 'evidence'
        raise RuntimeError(f'{name} {evidence} export failed; inspect {pull_log}')


def scoped_test_log(log, marker, end_marker=None):
    if isinstance(log, bytes):
        log=log.decode('utf-8',errors='replace')
    lines=log.splitlines(keepends=True)
    for index,line in enumerate(lines):
        if marker in line:
            remaining=lines[index+1:]
            if end_marker is None:
                return ''.join(remaining)
            for end_index,entry in enumerate(remaining):
                if end_marker in entry:
                    return ''.join(remaining[:end_index])
            raise RuntimeError('Android test log end marker is missing')
    raise RuntimeError('Android test log start marker is missing')


def start_test_log(report, marker, adb=None, verify_device=None):
    adb = adb or [ADB]
    for attempt in range(3):
        if verify_device is not None:
            verify_device()
        with (report/'logcat.raw').open('wb') as output, (report/'logcat-stderr.txt').open('wb') as errors:
            process=subprocess.Popen(adb+['logcat','-b','main,system,crash','-v','threadtime'],
                                     stdout=output,stderr=errors)
        try:
            if verify_device is not None:
                verify_device()
            run(adb+['shell','log','-t','CorAndroidTest',marker],timeout=15,
                capture_output=True,text=True)
            wait_test_log_marker(process,report,marker)
            return process
        except subprocess.CalledProcessError as error:
            stop_test_log(process)
            detail=(error.stderr or '')+(error.stdout or '')
            if attempt==2 or (error.returncode!=255 and not re.search(
                    r'device offline|device not found|no devices|connection.*closed',detail,re.I)):
                raise
            print('Transient ADB log boundary connection failure; reconnecting before tests',flush=True)
            time.sleep(attempt+1)
        except BaseException:
            stop_test_log(process)
            raise


def stop_app_before_test():
    command=[ADB,'shell','am','force-stop','org.clashofrust.android']
    for attempt in range(3):
        try:
            run(command,timeout=15,capture_output=True,text=True)
            return
        except subprocess.CalledProcessError as error:
            detail=(error.stderr or '')+(error.stdout or '')
            if attempt==2 or (error.returncode!=255 and not re.search(
                    r'device offline|device not found|no devices|connection.*closed',detail,re.I)):
                raise
            print('Transient ADB force-stop connection failure; retrying before instrumentation',flush=True)
            time.sleep(attempt+1)


def stop_test_log(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def wait_test_log_marker(process, report, marker):
    deadline=time.monotonic()+30
    boundary=marker.encode()
    pending=b''
    with (report/'logcat.raw').open('rb') as stream:
        while True:
            chunk=stream.read(64*1024)
            pending+=chunk
            if boundary in pending:
                return
            pending=pending[-len(boundary):]
            if process.poll() is not None:
                raise RuntimeError('Android logcat stream exited before its marker')
            if time.monotonic()>=deadline:
                raise RuntimeError('Android logcat stream did not capture its marker')
            if not chunk:
                time.sleep(0.1)


def finish_test_log(process, report, marker, adb=None, verify_device=None):
    adb = adb or [ADB]
    error=None
    end_marker=marker+'-end'
    try:
        if process.poll() is not None:
            raise RuntimeError('Android logcat stream exited before test completion')
        if verify_device is not None:
            verify_device()
        run(adb+['shell','log','-t','CorAndroidTest',end_marker],timeout=15)
        wait_test_log_marker(process,report,end_marker)
    except Exception as capture_error:
        error=capture_error
    finally:
        try:
            stop_test_log(process)
        except Exception as cleanup_error:
            error=error or cleanup_error
    raw=(report/'logcat.raw').read_bytes()
    if error is not None:
        recovery=[str(error)]
        record_adb_health(report)
        if wait_adb_identity(recovery, adb=adb) is not None:
            try:
                if verify_device is not None:
                    verify_device()
            except Exception as identity_error:
                recovery.append(str(identity_error))
            else:
                boundary=adb_evidence_command(adb+['shell','log','-t','CorAndroidTest',end_marker],15)
                recovery.append(boundary.stdout+boundary.stderr)
                if boundary.returncode == 0:
                    replay=adb_evidence_command(
                        adb+['logcat','-b','main,system,crash','-v','threadtime','-d'],30,binary=True)
                    (report/'logcat-recovery.raw').write_bytes(replay.stdout)
                    recovery.append(replay.stderr.decode(errors='replace'))
                    if replay.returncode == 0:
                        try:
                            raw=merge_test_log(raw,replay.stdout,marker,end_marker)
                            (report/'logcat-recovered.raw').write_bytes(raw)
                            error=None
                        except RuntimeError as recovery_error:
                            recovery.append(str(recovery_error))
        (report/'logcat-recovery.txt').write_text('\n'.join(recovery))
        write_json(report/'logcat-recovery-status.json',{'schema':1,'recovered':error is None})
    try:
        captured_log=scoped_test_log(raw,marker,end_marker)
    except RuntimeError as scope_error:
        error=error or scope_error
        captured_log=raw.decode('utf-8',errors='replace')
    (report/'logcat.txt').write_text(captured_log)
    return captured_log,error


def merge_test_log(raw, replay, marker, end_marker):
    scoped_test_log(raw,marker)
    complete=raw[:raw.rfind(b'\n')+1]
    lines=complete.splitlines(keepends=True)
    overlap=b''.join(lines[-min(8,len(lines)):])
    position=replay.find(overlap) if overlap else -1
    if position < 0:
        raise RuntimeError('Android recovered log has no continuous overlap with the original stream')
    merged=complete+replay[position+len(overlap):]
    scoped_test_log(merged,marker,end_marker)
    return merged


def process_crashes(log, package):
    crashes=[]
    lines=log.splitlines()
    for index,line in enumerate(lines):
        if 'FATAL EXCEPTION:' not in line:
            continue
        context=[]
        for entry in lines[index+1:index+8]:
            if 'FATAL EXCEPTION:' in entry:
                break
            context.append(entry)
        if not any(f'Process: {package},' in entry for entry in context):
            continue
        exception=next((entry.split('AndroidRuntime:',1)[-1].strip()
                        for entry in context if 'AndroidRuntime:' in entry
                        and ('Exception:' in entry or 'Error:' in entry)), 'unknown')
        crashes.append({'process':package,'exception':exception[:500]})
    return crashes


def input_method_crashes(log, package):
    return process_crashes(log, package)


def runtime_crash_status(log, scoped=True):
    native_crashes = [
        {'process': 'org.clashofrust.android', 'exception': 'RustPanic: ' + match.group(1)[:500]}
        for match in re.finditer(r'\bE\s+RustPanic\s*:\s*([^\r\n]+)', log)
    ]
    return {'schema': 1, 'log_scoped': scoped,
            'application_crashes': process_crashes(log, 'org.clashofrust.android') + native_crashes,
            'system_ui_crashes': process_crashes(log, 'com.android.systemui')}


def wait_emulator_boot(process, env, report, timeout=180):
    started=time.monotonic()
    deadline=started+timeout
    status={'schema':1,'complete':False,'probes':[]}
    try:
        while time.monotonic()<deadline:
            if process.poll() is not None:
                raise RuntimeError('Android emulator exited: '+(report/'emulator.txt').read_text()[-4000:])
            remaining=deadline-time.monotonic()
            if remaining<=0:
                break
            try:
                result=subprocess.run([ADB,'shell','getprop','sys.boot_completed'],env=env,
                                      capture_output=True,text=True,timeout=min(10,remaining))
            except subprocess.TimeoutExpired:
                status['probes'].append({'state':'adb_timeout'})
                print('Android boot property probe timed out; waiting within the existing boot deadline',flush=True)
            else:
                ready=result.returncode==0 and result.stdout.strip()=='1'
                status['probes'].append({'state':'ready' if ready else 'not_ready',
                                         'returncode':result.returncode})
                if ready:
                    status['complete']=True
                    return
            remaining=deadline-time.monotonic()
            if remaining>0:
                time.sleep(min(2,remaining))
        raise RuntimeError('Android emulator did not finish booting')
    finally:
        status['elapsed_seconds']=round(time.monotonic()-started,3)
        write_json(report/'boot-status.json',status)


def prepare_emulator_input_method(report):
    properties=[subprocess.check_output([ADB,'shell','getprop',name],text=True,timeout=15).strip()
                for name in ('ro.kernel.qemu','ro.debuggable','ro.build.version.sdk')]
    if properties[2]!='31':
        return
    if properties[:2]!=['1','1']:
        raise RuntimeError('Input method preparation requires a debuggable API 31 emulator')
    service=subprocess.check_output(
        [ADB,'shell','settings','get','secure','default_input_method'],text=True,timeout=15).strip()
    if not re.fullmatch(r'com\.google\.android\.inputmethod\.latin/[A-Za-z0-9_.$]+',service):
        return
    package=service.split('/',1)[0]
    details=subprocess.run([ADB,'shell','dumpsys','package',package],capture_output=True,timeout=15)
    (report/'input-method-package-before-setup.txt').write_bytes(details.stdout+details.stderr)
    if details.returncode:
        raise RuntimeError('Could not record emulator input method before preparation')
    record={'schema':1,'input_method':service,'api':31,'disposable_emulator':True,
            'before_application_tests':True,'complete':False,'commands':[]}
    try:
        commands=[['shell','pm','clear',package],['shell','ime','enable',service],['shell','ime','set',service]]
        for arguments in commands:
            result=subprocess.run([ADB,*arguments],capture_output=True,text=True,timeout=30)
            record['commands'].append({'arguments':arguments,'returncode':result.returncode,
                                       'stdout':result.stdout,'stderr':result.stderr})
            if result.returncode or (arguments[1:3]==['pm','clear'] and result.stdout.strip()!='Success'):
                raise RuntimeError('Could not prepare emulator input method before application tests')
        selected=subprocess.check_output(
            [ADB,'shell','settings','get','secure','default_input_method'],text=True,timeout=15).strip()
        if selected!=service:
            raise RuntimeError('Emulator input method changed during preparation')
        record['complete']=True
    finally:
        write_json(report/'input-method-setup.json',record)
    print('API 31 emulator input method starts from cold data; application crash checks remain enabled.',flush=True)


def platform_ime_control(report, ime_service):
    package=ime_service.split('/',1)[0]
    properties=[subprocess.check_output([ADB,'shell','getprop',name],text=True,timeout=15).strip()
                for name in ('ro.kernel.qemu','ro.debuggable','ro.build.version.sdk')]
    if properties!=['1','1','31']:
        raise RuntimeError('Native IME cold controls require a debuggable API 31 emulator')
    for requests in (1,2):
        directory=report/'platform-control'/('show-'+str(requests))
        directory.mkdir(parents=True,exist_ok=True)
        package_details=subprocess.run([ADB,'shell','dumpsys','package',package],
                                       capture_output=True,timeout=15)
        (directory/'input-method-package.txt').write_bytes(package_details.stdout+package_details.stderr)
        run([ADB,'shell','am','force-stop','org.clashofrust.android'])
        run([ADB,'shell','am','force-stop','org.clashofrust.android.test'])
        cleared=subprocess.run([ADB,'shell','pm','clear',package],capture_output=True,text=True,timeout=30)
        (directory/'reset.txt').write_text(cleared.stdout+cleared.stderr)
        if cleared.returncode or cleared.stdout.strip()!='Success':
            raise RuntimeError('Could not reset emulator input method for native control')
        run([ADB,'shell','ime','enable',ime_service],timeout=15)
        run([ADB,'shell','ime','set',ime_service],timeout=15)
        marker=f'cor-native-ime-{os.getpid()}-{time.time_ns()}'
        capture=start_test_log(directory,marker)
        error=None
        try:
            run([ADB,'shell','am','start','-W','-n',
                 'org.clashofrust.android.test/org.clashofrust.android.PlatformImeControlActivity',
                 '--ei','show_requests',str(requests)],timeout=30)
            deadline=time.monotonic()+15
            while time.monotonic()<deadline:
                time.sleep(0.5)
                if input_method_crashes(scoped_test_log((directory/'logcat.raw').read_bytes(),marker),package):
                    time.sleep(1)
                    break
                result=subprocess.run([ADB,'shell','dumpsys','input_method'],capture_output=True,text=True,timeout=10)
                if 'mInputShown=true' in result.stdout:
                    time.sleep(1)
                    break
        except Exception as control_error:
            error=control_error
        finally:
            captured,log_error=finish_test_log(capture,directory,marker)
            crashes=input_method_crashes(captured,package)
            result={'schema':1,'editor':'android.widget.EditText',
                'requested_shows':requests,'cold_input_method_data':True,'crashes':crashes,
                'log_scoped':log_error is None,'capture_error':type(error or log_error).__name__
                if error or log_error else None,
                'complete':False,'diagnostic_only':True,'replaces_production_result':False}
            write_json(directory/'result.json',result)
            for name,command in {'input-method':['dumpsys','input_method'],
                                 'windows':['dumpsys','window','windows']}.items():
                details=subprocess.run([ADB,'shell',*command],capture_output=True,timeout=15)
                (directory/(name+'.txt')).write_bytes(details.stdout+details.stderr)
            screenshot=subprocess.run([ADB,'exec-out','screencap','-p'],capture_output=True,timeout=15)
            (directory/'screenshot.png').write_bytes(screenshot.stdout)
            try:
                pull_device_report('/sdcard/Android/data/org.clashofrust.android.test/files/platform-ime-control',
                                directory,required=True,required_pattern='state.json')
                state=json.loads((directory/'ime-report/state.json').read_text())
                result['editor_state']=state
                result['complete']=not result['capture_error'] and state.get('show_requests')==requests \
                    and screenshot.returncode==0 and screenshot.stdout.startswith(b'\x89PNG\r\n\x1a\n')
            except Exception as export_error:
                result['capture_error']=type(export_error).__name__
            write_json(directory/'result.json',result)
            print('Native IME control:',requests,'show request(s),',len(crashes),
                  'input method crash(es); original regression remains failed.',flush=True)


def network_origin_handler():
    import http.server

    class Origin(http.server.BaseHTTPRequestHandler):
        protocol_version = 'HTTP/1.1'

        def do_CONNECT(self):
            if self.path not in ('198.51.100.1:18081', 'application-policy.test:18081'):
                self.send_error(403)
                return
            self.send_response(200, 'Connection established')
            self.end_headers()
            self.wfile.flush()
            self.close_connection = False
            self.handle_one_request()

        def do_HEAD(self):
            self.do_GET()

        def do_GET(self):
            if '/diagnostic-probe?' in self.path:
                time.sleep(0.04)
            held = self.path == '/hold'
            if held and self.headers.get('Content-Length') != '1':
                self.send_error(400)
                return
            body = b'cor-android-network-ok'
            self.send_response(200)
            self.send_header('Content-Length', str(len(body) + int(held)))
            self.end_headers()
            if self.command == 'HEAD':
                self.wfile.flush()
                return
            self.wfile.write(body)
            self.wfile.flush()
            if held:
                self.connection.settimeout(10)
                try:
                    if self.rfile.read(1) == b'!':
                        self.wfile.write(b'\n')
                        self.wfile.flush()
                except OSError:
                    pass
                self.close_connection = True

        def log_message(self, *args):
            pass

    return Origin


@contextmanager
def protection_death_origin(port=18081):
    import http.server
    import socket
    import threading

    class OriginServer(http.server.ThreadingHTTPServer):
        daemon_threads = False
        allow_reuse_port = False
        allow_reuse_address = not hasattr(socket, 'SO_EXCLUSIVEADDRUSE')

        def __init__(self, address):
            self.active_requests = set()
            self.requests_lock = threading.Lock()
            super().__init__(address, network_origin_handler())

        def server_bind(self):
            if not self.allow_reuse_address:
                self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
            super().server_bind()

        def process_request(self, request, client_address):
            with self.requests_lock:
                self.active_requests.add(request)
            super().process_request(request, client_address)

        def shutdown_request(self, request):
            with self.requests_lock:
                super().shutdown_request(request)
                self.active_requests.discard(request)

        def server_close(self):
            with self.requests_lock:
                for request in self.active_requests:
                    with suppress(OSError):
                        request.shutdown(socket.SHUT_RDWR)
            super().server_close()

    with OriginServer(('127.0.0.1', port)) as server:
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            yield server
        finally:
            server.shutdown()


def cor_ci_avd_path():
    runner_temp = os.environ.get('RUNNER_TEMP')
    if not runner_temp:
        raise RuntimeError('Set RUNNER_TEMP to the directory used to launch the disposable cor-ci emulator')
    path = Path(runner_temp, 'cor-emulator/.android/avd/cor-ci.avd').absolute()
    if path.resolve() != path:
        raise RuntimeError('The disposable cor-ci emulator path must not traverse a symbolic link')
    return path


def cor_ci_device(expected_path, device=None):
    if device is None:
        serial = subprocess.check_output([ADB, 'get-serialno'], text=True, timeout=15).strip()
        if not re.fullmatch(r'emulator-\d+', serial):
            raise RuntimeError('Protection process death requires a disposable cor-ci emulator')
        device = [ADB, '-s', serial]
    avd = subprocess.check_output(device + ['emu', 'avd', 'path'], text=True, timeout=15).splitlines()
    if len(avd) != 2 or avd[1] != 'OK' or Path(avd[0]) != expected_path:
        raise RuntimeError('Protection process death requires the disposable cor-ci emulator')
    return device


def protection_death_device(device=None, *, launch_identity=None, expected_path=None):
    avd_path = cor_ci_avd_path()
    if expected_path is not None and avd_path != expected_path:
        raise RuntimeError('The disposable cor-ci emulator path changed during protection process death')
    launch_file = avd_path / PROTECTION_DEATH_LAUNCH_FILE
    if launch_file.is_symlink() or not launch_file.is_file():
        raise RuntimeError('Start the disposable cor-ci emulator with scripts/android.py before testing')
    saved_launch = launch_file.read_text().strip()
    token, _, boot_count = saved_launch.partition(':')
    device = cor_ci_device(avd_path, device)
    if (not re.fullmatch(r'[0-9a-f]{32}', token) or not re.fullmatch(r'[0-9]+', boot_count) or
            (launch_identity is not None and saved_launch != launch_identity) or
            device_property('ro.build.version.sdk', adb=device) != '36' or
            subprocess.check_output(device + ['shell', 'cat', PROTECTION_DEATH_GUEST_LAUNCH],
                                    text=True, timeout=15).strip() != saved_launch or
            subprocess.check_output(device + PROTECTION_DEATH_BOOT_COUNT,
                                    text=True, timeout=15).strip() != boot_count):
        raise RuntimeError('Protection process death requires the current API 36 cor-ci emulator launch')
    return device, saved_launch


def record_protection_death_launch(avd_path, device):
    cor_ci_device(avd_path, device)
    boot_count = subprocess.check_output(device + PROTECTION_DEATH_BOOT_COUNT, text=True, timeout=15).strip()
    if not re.fullmatch(r'[0-9]+', boot_count):
        raise RuntimeError('Could not identify the API 36 cor-ci emulator boot')
    launch_identity = f'{uuid.uuid4().hex}:{boot_count}'
    run(device + ['shell', f'printf %s {launch_identity} > {PROTECTION_DEATH_GUEST_LAUNCH}'],
        timeout=15, capture_output=True, text=True)
    (avd_path/PROTECTION_DEATH_LAUNCH_FILE).write_text(launch_identity+'\n')


def test_protection_process_death(device, report, status, verify_device):
    package = 'org.clashofrust.android'
    shell = device + ['shell']
    token = uuid.uuid4().hex

    def command(*args, timeout=15):
        verify_device()
        return run(shell + list(args), capture_output=True, text=True, timeout=timeout).stdout

    def launch():
        output = command('am', 'start', '-W', '-n', package+'/.MainActivity', timeout=45)
        if 'Status: ok' not in output.splitlines():
            raise RuntimeError('Could not launch the Android protection test Activity: '+output.strip())

    def phase(name, *args):
        verify_device()
        result = adb_evidence_command(shell + ['am', 'instrument', '-w', '-r', '--no-restart',
            '-e', 'class', package+'.ProtectionProcessDeathTest#'+name, '-e', 'death_token', token,
            *args, package+'.test/androidx.test.runner.AndroidJUnitRunner'], 150)
        (report/(name+'.txt')).write_text(result.stdout+result.stderr)
        if result.returncode != 0:
            raise RuntimeError(f'Protection process death instrumentation failed in {name}')
        return result.stdout

    if command('pm', 'clear', package).strip() != 'Success':
        raise RuntimeError('Could not clear the disposable protection test app data')
    command('appops', 'set', package, 'ACTIVATE_VPN', 'allow')
    command('pm', 'grant', package, 'android.permission.POST_NOTIFICATIONS')
    launch()
    victim = command('pidof', package).strip()
    if not re.fullmatch(r'[1-9]\d*', victim):
        raise RuntimeError('Could not identify the original protection test PID')
    status['victim_pid'] = int(victim)
    prepared = phase('prepareAndKill').splitlines()
    if (prepared.count(f'INSTRUMENTATION_STATUS: cor_death_prepared={token}:{victim}') != 1
            or 'INSTRUMENTATION_RESULT: shortMsg=Process crashed.' not in prepared
            or 'INSTRUMENTATION_CODE: 0' not in prepared
            or any(line.strip() in ('OK (1 test)', 'FAILURES!!!') for line in prepared)):
        raise RuntimeError('The preparation phase did not terminate as the expected process death')
    verify_device()
    processes = adb_evidence_command(shell + ['pidof', package], 15)
    if (processes.returncode not in (0, 1) or processes.stderr.strip()
            or (processes.returncode == 0 and not re.fullmatch(r'[1-9]\d*', processes.stdout.strip()))
            or (processes.returncode == 1 and processes.stdout.strip())
            or victim in processes.stdout.split()):
        raise RuntimeError('Could not confirm that the original protection test PID is gone')
    status['prepare_terminated'] = True
    write_json(report/'process-death-status.json', status)

    launch()
    replacement = command('pidof', package).strip()
    if not re.fullmatch(r'[1-9]\d*', replacement) or replacement == victim:
        raise RuntimeError('The recovery Activity did not run in a different process')
    status['recovery_pid'] = int(replacement)
    recovered = phase('recoverAfterDeath', '-e', 'death_pid', victim).splitlines()
    summaries = [line.strip() for line in recovered]
    if (recovered.count(f'INSTRUMENTATION_STATUS: cor_death_observed={token}:{victim}:{replacement}') != 1
            or recovered.count(f'INSTRUMENTATION_STATUS: cor_death_tun={token}:{replacement}') != 1
            or 'OK (1 test)' not in summaries
            or 'INSTRUMENTATION_STATUS_CODE: 0' not in recovered
            or 'INSTRUMENTATION_CODE: -1' not in recovered
            or 'FAILURES!!!' in summaries):
        raise RuntimeError('Recovery instrumentation did not verify both process death and VPN recovery')
    status['tun_verified'] = True
    status['recovery_verified'] = True


@contextmanager
def protection_death_followup(action, *args):
    try:
        yield
    except BaseException as original:
        try:
            action(*args)
        except Exception as error:
            original.add_note(str(error))
        raise
    else:
        action(*args)


def collect_protection_death_evidence(device, report, log_capture, marker, verify_device):
    errors = []
    try:
        captured_log, log_error = finish_test_log(log_capture, report, marker, adb=device,
                                                  verify_device=verify_device)
        runtime = runtime_crash_status(captured_log, log_error is None)
        write_json(report/'runtime-status.json', runtime)
        if log_error is not None:
            errors.append(str(log_error))
        if runtime['application_crashes']:
            errors.append('Android application crashed; inspect runtime-status.json and logcat')
        require_native_test_log(captured_log)
    except Exception as error:
        errors.append(str(error))
    for name, command in ANDROID_TEST_DIAGNOSTICS.items():
        try:
            verify_device()
        except Exception as error:
            errors.append(f'Could not collect Android {name} diagnostics: {error}')
            break
        details = adb_evidence_command(device + ['shell', *command], 15)
        (report/(name+'.txt')).write_text(details.stdout+details.stderr)
        if details.returncode != 0:
            errors.append(f'Android {name} diagnostics failed; inspect {name}.txt')
    if errors:
        raise RuntimeError('; '.join(errors))


def cleanup_protection_death(device, report, verify_device):
    package = 'org.clashofrust.android'
    commands = {
        'force_stop': ['am', 'force-stop', package],
        'clear_fixture': ['pm', 'clear', package],
        'vpn_consent': ['appops', 'set', package, 'ACTIVATE_VPN', 'default'],
        'notifications': ['pm', 'revoke', package, 'android.permission.POST_NOTIFICATIONS'],
    }
    cleanup = {'schema': 1, 'passed': False, 'commands': {}}
    try:
        for name, command in commands.items():
            try:
                verify_device()
            except Exception:
                cleanup['blocked_command'] = name
                raise
            result = adb_evidence_command(device + ['shell', *command], 15)
            passed = result.returncode == 0 and (name != 'clear_fixture' or result.stdout.strip() == 'Success')
            cleanup['commands'][name] = {
                'passed': passed, 'returncode': result.returncode,
                'stdout': result.stdout, 'stderr': result.stderr,
            }
        cleanup['passed'] = all(result['passed'] for result in cleanup['commands'].values())
        if not cleanup['passed']:
            raise RuntimeError('One or more disposable app cleanup commands failed')
    except Exception as error:
        cleanup['error'] = str(error)
        raise RuntimeError(f'Protection process death cleanup failed: {error}') from error
    finally:
        write_json(report/'cleanup-status.json', cleanup)


@contextmanager
def protection_death_report_lock(report):
    """Keep the lock file in place so all runs lock the same file."""
    report.mkdir(parents=True, exist_ok=True)
    with (report/'.process-death.lock').open('a+b') as lock:
        try:
            if os.name == 'nt':
                import msvcrt
                lock.seek(0)
                msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as error:
            if error.errno not in (errno.EACCES, errno.EAGAIN):
                raise
            raise RuntimeError(f'Another Android protection process death test is already using the report directory: '
                               f'{report}') from error
        try:
            yield
        finally:
            if os.name == 'nt':
                lock.seek(0)
                msvcrt.locking(lock.fileno(), msvcrt.LK_UNLCK, 1)


def run_protection_death_test(apk, test_apk, report):
    status = {'schema': 1, 'passed': False}
    def record_failure(error, *paths):
        status['error'] = str(error)
        if notes := getattr(error, '__notes__', None):
            status['notes'] = notes
        for path in paths:
            try:
                write_json(path, status)
            except Exception as receipt_error:
                error.add_note(f'Could not write the process death failure receipt: {receipt_error}')

    with protection_death_report_lock(report), ExitStack() as origin:
        status_path = report/'process-death-status.json'
        write_json(status_path, status)
        try:
            origin.enter_context(protection_death_origin())
        except BaseException as error:
            record_failure(error, status_path, report/f'origin-failure-{uuid.uuid4().hex}.json')
            raise

        try:
            installations = {'install-app.txt': apk, 'install-test.txt': test_apk}
            previous_reports = (
                'prepareAndKill.txt', 'recoverAfterDeath.txt', 'cleanup-status.json',
                'device.json', 'runtime-status.json', 'adb-transport-health.json',
                'logcat.raw', 'logcat-stderr.txt', 'logcat.txt', 'logcat-recovery.raw',
                'logcat-recovered.raw', 'logcat-recovery.txt', 'logcat-recovery-status.json',
            )
            diagnostics = tuple(f'{key}.txt' for key in ANDROID_TEST_DIAGNOSTICS)
            for name in (*previous_reports, *installations, *diagnostics):
                (report/name).unlink(missing_ok=True)
            avd_path = cor_ci_avd_path()
            device, launch_identity = protection_death_device(expected_path=avd_path)
            status['serial'] = device[-1]

            def verify_device():
                protection_death_device(device, launch_identity=launch_identity, expected_path=avd_path)

            for name, path in installations.items():
                verify_device()
                result = adb_evidence_command(device + ['install', '-r', path], 60)
                (report/name).write_text(result.stdout+result.stderr)
                if result.returncode != 0:
                    raise RuntimeError(f'Protection process death APK installation failed; inspect {name}')
            verify_device()
            properties = {name: device_property(name, adb=device) for name in (
                'ro.build.version.sdk', 'ro.build.version.release', 'ro.build.fingerprint',
                'ro.product.manufacturer', 'ro.product.model', 'ro.product.cpu.abi',
            )}
            write_json(report/'device.json', properties)
            marker = f'cor-android-test-{os.getpid()}-{time.time_ns()}'
            log_capture = start_test_log(report, marker, adb=device, verify_device=verify_device)
            with protection_death_followup(cleanup_protection_death, device, report, verify_device):
                with protection_death_followup(collect_protection_death_evidence, device, report,
                                               log_capture, marker, verify_device):
                    test_protection_process_death(device, report, status, verify_device)
            origin.close()
        except BaseException as error:
            record_failure(error, status_path)
            raise
        status['passed'] = True
        write_json(status_path, status)


def test(arch, apk=None, test_apk=None, ime_only=False, ime_probe=False,
         ime_rounds=1, applications_only=False, diagnostics_only=False, basic_only=False,
         routing_only=False, protection_death_only=False):
    if not 1 <= ime_rounds <= 3 or ((ime_probe or ime_rounds != 1) and not ime_only):
        raise ValueError('IME probe options require IME-only testing and one to three rounds')
    if sum((ime_only, applications_only, diagnostics_only, basic_only, routing_only, protection_death_only)) > 1:
        raise ValueError('Application screenshots, diagnostic verification and input-method tests are separate')
    if arch != 'x64':
        return
    apk = apk or ANDROID/'app/build/outputs/apk/release/app-release.apk'
    test_apk = test_apk or ANDROID/'app/build/outputs/apk/androidTest/release/app-release-androidTest.apk'
    report=ROOT/'target/android-report'/arch
    if protection_death_only:
        run_protection_death_test(apk, test_apk, report/'protection-process-death')
        return
    network_tests = not (ime_only or applications_only)
    run([ADB,'install','-r',apk])
    run([ADB,'install','-r',test_apk])
    import http.server
    import threading
    server=None
    if network_tests:
        server=http.server.ThreadingHTTPServer(('0.0.0.0',18081),network_origin_handler())
        threading.Thread(target=server.serve_forever,daemon=True).start()
    # Exercise Android's real first-use dialog; pre-granting consent hides
    # ordering bugs between authorization, protection DNS and VPN startup.
    if network_tests and not routing_only:
        run([ADB,'shell','appops','set','org.clashofrust.android','ACTIVATE_VPN','default'])
    if routing_only:
        report = report / 'application-rules'
    if ime_only:
        api=device_property('ro.build.version.sdk')
        if not api.isdigit():
            raise ValueError('Could not identify the Android IME test API level')
        report=report/('ime-api-'+api)
        report=report/('production' if ime_probe else 'regression')
    if applications_only:
        api = device_property('ro.build.version.sdk')
        if not api.isdigit():
            raise ValueError('Could not identify the Android application preview version')
        report = report / ('applications-api-' + api)
    if diagnostics_only:
        api = device_property('ro.build.version.sdk')
        if not api.isdigit():
            raise ValueError('Could not identify the Android diagnostic verification version')
        report = report / ('diagnostics-api-' + api)
    report.mkdir(parents=True,exist_ok=True)
    properties={}
    for name in ('ro.build.version.sdk','ro.build.version.release','ro.build.fingerprint',
                 'ro.product.manufacturer','ro.product.model','ro.product.cpu.abi'):
        properties[name]=device_property(name)
    ime_service=''
    ime_package=''
    if ime_only:
        ime_service=subprocess.check_output(
            [ADB,'shell','settings','get','secure','default_input_method'],text=True,timeout=15).strip()
        ime_package=ime_service.split('/',1)[0]
        if not re.fullmatch(r'[A-Za-z_][A-Za-z0-9_.]*/[A-Za-z0-9_.$]+',ime_service):
            raise ValueError('Could not identify the active Android input method')
        properties['input_method']=ime_service
    write_json(report/'device.json',properties)
    marker=f'cor-android-test-{os.getpid()}-{time.time_ns()}'
    log_capture=start_test_log(report,marker)
    try:
        outputs=[]
        statuses=[]
        from ci_test_policy import android_cases
        cases=(('applicationRoutingPersistsAndKeepsOtherAppsOnline',) if routing_only else
               ('androidUiPreviewCapturesPagesAndApplications',) if applications_only else
               ('basicSystemProxyAndTun', 'measurementDiagnosticsDistinguishFailuresAndExport')
               if diagnostics_only else
               ('subscriptionKeyboardProbeCapturesBothFields' if ime_probe
                else 'subscriptionFieldsShowImeAcceptTextAndReopenAfterBack',)
               if ime_only else ('basicSystemProxyAndTun',) if basic_only else android_cases())
        selections = [('ProxyTunTest', case) for case in cases]
        if not any((routing_only, applications_only, diagnostics_only, ime_only, basic_only)):
            selections.append(('UpdateInstallerTest', 'updateInstallConsentRestoresOnceAndRejectsReplacedApk'))
        failures=[]
        for iteration in range(1,ime_rounds+1):
            if ime_only:
                animation_scale='0' if iteration==3 else '1'
                for setting in ('window_animation_scale','transition_animation_scale','animator_duration_scale'):
                    run([ADB,'shell','settings','put','global',setting,animation_scale])
                print(f'IME round {iteration}: system animation scale {animation_scale}',flush=True)
            for test_class, case in selections:
                stop_app_before_test()
                started=time.monotonic()
                status={'case':case,'round':iteration,'completed':False,'passed':False}
                statuses.append(status)
                write_json(report/'instrumentation-status.json',{'schema':1,'cases':statuses})
                command = [ADB,'shell','am','instrument','-w','-e','class',
                    f'org.clashofrust.android.{test_class}#{case}',
                    '-e','ime_round',str(iteration),
                    '-e','ime_probe',str(ime_probe).lower(),
                    'org.clashofrust.android.test/androidx.test.runner.AndroidJUnitRunner']
                timeout = 600 if applications_only else 240
                try:
                    result = subprocess.run(command,text=True,capture_output=True,timeout=timeout)
                except subprocess.TimeoutExpired as error:
                    stdout = error.stdout or ''
                    stderr = error.stderr or ''
                    if isinstance(stdout, bytes):
                        stdout = stdout.decode(errors='replace')
                    if isinstance(stderr, bytes):
                        stderr = stderr.decode(errors='replace')
                    result = subprocess.CompletedProcess(command,255,stdout,
                        stderr+f'\nADB command timed out after {timeout}s')
                passed = (result.returncode == 0 and 'FAILURES!!!' not in result.stdout
                          and 'OK (1 test)' in result.stdout)
                status.update(completed=True,returncode=result.returncode,
                              elapsed_seconds=round(time.monotonic()-started,3),passed=passed,
                              transport_error=adb_transport_failed(result))
                write_json(report/'instrumentation-status.json',{'schema':1,'cases':statuses})
                outputs.append(f'Round {iteration}: '+case+'\n'+result.stdout+result.stderr)
                (report/'instrumentation.txt').write_text('\n\n'.join(outputs))
                print(result.stdout,flush=True)
                if not passed:
                    record_adb_health(report)
                    failures.append(f'{case} round {iteration}')
        if failures:
            raise RuntimeError('Android integration test failed: '+', '.join(failures))
        if ime_probe:
            message='Visual IME probe captured; instrumentation success does not verify keyboard rendering or Android compatibility.'
            print(message,flush=True)
            if summary:=os.environ.get('GITHUB_STEP_SUMMARY'):
                with open(summary,'a') as stream:
                    stream.write(f'\nAndroid API {api}, production screenshots: '
                                 f'{ime_rounds} probe rounds. {message}\n')
    finally:
        if server is not None:
            server.shutdown(); server.server_close()
        captured_log,log_error=finish_test_log(log_capture,report,marker)
        runtime_status=runtime_crash_status(captured_log,log_error is None)
        write_json(report/'runtime-status.json',runtime_status)
        if runtime_status['system_ui_crashes']:
            print('::warning::Android emulator SystemUI crashed; inspect runtime-status.json and original logcat. '
                  'Application crashes and window readiness are checked separately.',flush=True)
        crashes=input_method_crashes(captured_log,ime_package) if ime_only and log_error is None else []
        if ime_only:
            write_json(report/'input-method-status.json',
                       {'schema':1,'input_method':ime_service,'log_scoped':log_error is None,'crashes':crashes})
        remote_report='/sdcard/Android/data/org.clashofrust.android/files/ime-report'
        remote_report+='/'+('production' if ime_probe else 'regression')
        capture_error=None
        try:
            if 'measurementDiagnosticsDistinguishFailuresAndExport' in cases:
                pull_device_report('/sdcard/Android/data/org.clashofrust.android/files/measurement-diagnostics',
                                   report, required=True, required_pattern='result.json',
                                   name='measurement-diagnostics')
                validate_measurement_diagnostics(report / 'measurement-diagnostics',
                                                 int(properties['ro.build.version.sdk']))
            if applications_only:
                destination = report / 'application-preview'
                pull_device_report('/sdcard/Android/data/org.clashofrust.android/files/application-preview',
                                   report, required=True, name='application-preview')
                validate_application_preview(destination, int(api))
                if summary := os.environ.get('GITHUB_STEP_SUMMARY'):
                    manifest = json.loads((destination / 'manifest.json').read_text())
                    state = next(case['state'] for case in manifest['cases']
                                 if case['name'] == 'phone-applications-default')
                    with open(summary, 'a') as stream:
                        stream.write(f'\nAndroid {properties["ro.build.version.release"]}: '
                                     f'application list {state["applications_visible"]} / '
                                     f'{state["applications_total"]}; '
                                     f'{len(manifest["cases"])} page screenshots captured. '
                                     'No VPN or traffic-routing test was run by this preview.\n')
            if ime_only:
                pull_device_report(remote_report,report,required=True)
                phases=(('before-focus','name-after-tap','name-settled','source-after-tap',
                         'source-settled','after-back') if ime_probe else
                        ('before-focus','0-name-visible','1-source-visible','2-name-visible',
                         '2-name-hidden','3-source-visible','3-source-hidden','4-name-visible',
                         '4-name-hidden','foreground-hidden','after-resume','landscape-hidden','landscape-name-visible',
                         'landscape-source-visible',
                         'portrait-hidden','final'))
                for iteration in range(1,ime_rounds+1):
                    for phase in phases:
                        for suffix in ('.png','-state.json','-input-method.txt','-windows.txt'):
                            expected=report/'ime-report'/('round-'+str(iteration))/(phase+suffix)
                            if not expected.is_file():
                                raise RuntimeError('IME evidence is incomplete: '+str(expected.relative_to(report)))
        except Exception as error:
            capture_error=error
        diagnostics=ANDROID_TEST_DIAGNOSTICS.copy()
        if ime_only:
            diagnostics['input-method']=['dumpsys','input_method']
        if applications_only:
            diagnostics = {key: diagnostics[key] for key in ('windows', 'activities')}
        for name, command in diagnostics.items():
            details=subprocess.run([ADB,'shell',*command],capture_output=True,text=True,timeout=15)
            (report/(name+'.txt')).write_text(details.stdout+details.stderr)
        errors=[error for error in (capture_error,log_error) if error is not None]
        if runtime_status['application_crashes']:
            errors.append(RuntimeError('Android application crashed; inspect runtime-status.json and logcat'))
        if crashes:
            errors.append(RuntimeError(f'Android input method crashed: {ime_package}; inspect input-method-status.json'))
            if ime_only and not ime_probe and properties.get('ro.build.version.sdk')=='31':
                try:
                    platform_ime_control(report,ime_service)
                except Exception as control_error:
                    errors.append(RuntimeError('Native IME control capture failed: '+str(control_error)))
        if errors:
            if original:=sys.exception():
                for error in errors:
                    original.add_note(str(error))
            else:
                raise RuntimeError('; '.join(map(str,errors)))
    require_native_test_log(captured_log)


def require_native_test_log(captured_log):
    if any(marker in captured_log for marker in
           ('Android startup:', 'Mobile runtime stopped:', 'Recursion detected', 'E RustPanic')):
        raise RuntimeError('Native Android UI/runtime failed; inspect logcat')


def validate_application_preview(directory, api):
    manifest = json.loads((directory / 'manifest.json').read_text())
    if (manifest.get('schema'), manifest.get('scope'), manifest.get('android_api')) != (
            3, 'android-ui-preview', api) or not manifest.get('android_release'):
        raise ValueError('Android preview identity differs from the selected Android version')
    primary = ('home', 'proxies-grid', 'proxies-list', 'subscriptions-grid', 'subscriptions-list',
               'settings-top', 'settings-middle', 'settings-bottom', 'applications-all', 'applications-default',
               'applications-scrolled')
    expected = {f'{layout}-{case}' for layout in ('phone', 'landscape', 'tablet-portrait', 'tablet-landscape')
                for case in primary}
    expected.update('phone-' + case for case in (
        'subscription-editor', 'tools', 'connections', 'rules', 'rule-shortcuts', 'logs', 'network', 'ip', 'ai',
        'applications-system', 'applications-only', 'applications-exclude',
        'applications-unsaved'))
    cases = manifest.get('cases')
    if not isinstance(cases, list):
        raise ValueError('Android preview screenshot manifest is invalid')
    indexed = {}
    for case in cases:
        if not isinstance(case, dict) or not isinstance(case.get('name'), str):
            raise ValueError('Android preview screenshot entry is invalid')
        name = case['name']
        if name in indexed or not re.fullmatch('[a-z0-9-]+', name) or case.get('image') != name + '.png':
            raise ValueError('Android preview screenshot name is invalid or duplicated')
        indexed[name] = case
    if not expected.issubset(indexed):
        raise ValueError('Android preview screenshots are incomplete: ' + ', '.join(sorted(expected - indexed.keys())))
    for name in sorted(expected):
        case = indexed[name]
        image = (directory / case['image']).read_bytes()
        if len(image) < 24 or not image.startswith(b'\x89PNG\r\n\x1a\n') \
                or image[12:16] != b'IHDR':
            raise ValueError('Android preview screenshot is missing or invalid: ' + name)
        size = struct.unpack('>II', image[16:24])
        if min(size) < 320 or size != (case.get('pixel_width'), case.get('pixel_height')):
            raise ValueError('Android preview screenshot dimensions are invalid: ' + name)
        state = case.get('state')
        if not isinstance(state, dict) or state.get('live') is not True \
                or state.get('vpn') is not False or state.get('vpn_pending') is not False:
            raise ValueError('Android preview did not capture the live UI without VPN: ' + name)
        width, height = state.get('width'), state.get('height')
        scale = state.get('scale')
        if (type(scale) not in (int, float) or not math.isfinite(scale) or scale <= 0
                or (name.startswith('tablet-') and abs(scale - 1.0) >= .001)):
            raise ValueError('Android preview did not use the intended display scale: ' + name)
        if (type(width) not in (int, float) or type(height) not in (int, float)
                or not math.isfinite(width) or not math.isfinite(height) or width <= 0 or height <= 0):
            raise ValueError('Android preview has invalid logical dimensions: ' + name)
        black_ratio = case.get('black_sample_ratio')
        if (case.get('capture_method') != 'pixelcopy-window' or type(black_ratio) not in (int, float)
                or not math.isfinite(black_ratio) or not 0 <= black_ratio <= .2
                or not math.isfinite(width * scale) or not math.isfinite(height * scale)
                or abs(size[0] - round(width * scale)) > 2 or abs(size[1] - round(height * scale)) > 2):
            raise ValueError('Android preview window capture is invalid or obscured: ' + name)
        if case.get('screen_image') != name + '-screen.png' \
                or not (directory / case['screen_image']).is_file():
            raise ValueError('Android preview is missing the original display capture: ' + name)
        if ((name.startswith(('phone-', 'tablet-portrait-')) and width >= height)
                or (name.startswith(('landscape-', 'tablet-landscape-')) and width <= height)
                or (name.startswith('tablet-portrait-') and width < 760)
                or (name.startswith('tablet-landscape-') and width < 1024)):
            raise ValueError('Android preview did not use the intended layout: ' + name)
        suffix = name.removeprefix('tablet-landscape-').removeprefix('tablet-portrait-') \
            .removeprefix('landscape-').removeprefix('phone-')
        pages = {'home': 0, 'proxies': 1, 'subscriptions': 2, 'subscription-editor': 2,
                 'connections': 3, 'rules': 4, 'rule-shortcuts': 4, 'logs': 5, 'network': 6,
                 'ip': 7, 'ai': 8, 'settings': 9, 'applications': 9, 'tools': 10}
        page = next((value for prefix, value in pages.items()
                     if suffix == prefix or suffix.startswith(prefix + '-')), None)
        if page is None or state.get('page') != page:
            raise ValueError('Android preview captured the wrong page: ' + name)
        detail = 3 if suffix == 'subscription-editor' else 12 if suffix == 'applications-unsaved' \
            else 18 if suffix.startswith('applications-') else 0
        if state.get('detail') != detail:
            raise ValueError('Android preview captured the wrong detail: ' + name)
        if suffix == 'rule-shortcuts' and state.get('shortcuts_open') is not True:
            raise ValueError('Android preview did not open rule shortcuts')
        for prefix, key in (('proxies-', 'nodes_list'), ('subscriptions-', 'profiles_list')):
            if suffix.startswith(prefix) and state.get(key) is not (suffix.endswith('-list')):
                raise ValueError('Android preview captured the wrong list mode: ' + name)
        if suffix.startswith('applications-') and suffix != 'applications-unsaved':
            total, visible = state.get('applications_total'), state.get('applications_visible')
            all_applications = suffix == 'applications-all'
            if (state.get('applications_loaded') is not True or state.get('applications_loading') is not False
                    or state.get('applications_error') != '' or type(total) is not int or type(visible) is not int
                    or total <= 1 or not (visible == 0 if all_applications else 1 < visible <= total)):
                raise ValueError('Android preview did not show a completed application list: ' + name)
            icons, icon_failures = state.get('applications_icons'), state.get('applications_icon_failures')
            if (type(icons) is not int or not (icons == 0 if all_applications else 0 < icons <= visible)
                    or type(icon_failures) is not int or icon_failures < 0):
                raise ValueError('Android preview did not load application icons: ' + name)
            if (state.get('applications_visibility_limited') is not False
                    or state.get('applications_query_permission') is not True
                    or not isinstance(state.get('applications_saved_summary'), str)
                    or not state['applications_saved_summary'].strip()):
                raise ValueError('Android preview application visibility or saved state is unavailable: ' + name)
            if suffix == 'applications-system' and state.get('applications_system') is not True:
                raise ValueError('Android preview did not show system applications')
            mode = 0 if all_applications else 2 if suffix == 'applications-exclude' else 1
            if type(state.get('applications_mode')) is not int or state['applications_mode'] != mode:
                raise ValueError('Android preview captured the wrong application routing mode')
            selected = state.get('applications_selected')
            if type(selected) is not int or not 0 <= selected <= total:
                raise ValueError('Android preview application selection is invalid: ' + name)
            if suffix == 'applications-scrolled':
                offset = state.get('applications_scroll_y')
                if type(offset) not in (int, float) or not offset < -1:
                    raise ValueError('Android preview application list did not scroll: ' + name)
    only = indexed['phone-applications-only']['state']
    exclude = indexed['phone-applications-exclude']['state']
    if (only['applications_total'] != exclude['applications_total']
            or only['applications_selected'] + exclude['applications_selected'] != only['applications_total']):
        raise ValueError('Android preview application modes did not invert the full selection')


def validate_measurement_diagnostics(directory, api):
    receipt = json.loads((directory / 'result.json').read_text())
    cases = ['without-vpn', 'with-vpn', 'controller-authentication', 'recovered']
    if (receipt.get('schema') != 1 or receipt.get('sdk') != api
            or receipt.get('passed') is not True or receipt.get('cases') != cases):
        raise ValueError('Android measurement diagnostic receipt is incomplete')
    previous = None
    for name in cases:
        text = (directory / (name + '.json')).read_text()
        report = json.loads(text)
        if report.get('schema') != 1 or report.get('command') != 'diagnose':
            raise ValueError('Invalid exported Android diagnostic report')
        context = report['data']['context']
        measurements = context['measurements']
        platform = context['platform']['Ok']
        if not 1 <= len(measurements) <= 4 or platform.get('sdk') != api:
            raise ValueError('Diagnostic history or platform version mismatch')
        latest = measurements[-1]
        if previous is not None and latest['started_at'] <= previous:
            raise ValueError('Diagnostic export reused an earlier measurement')
        previous = latest['started_at']
        if name == 'controller-authentication':
            if (latest.get('completed') is not False or latest.get('node_failures') != 0
                    or latest.get('failure') != {'code': 'controller_authentication', 'http_status': 401}):
                raise ValueError('Controller failure was mistaken for node failure')
        elif (latest.get('completed') is not True or latest.get('attempted') != 2
                or latest.get('succeeded') != 1 or latest.get('node_failures') != 1
                or latest.get('request_failures') != 0):
            raise ValueError('Mixed successful and refused node results did not survive export')
        if platform['vpn'].get('connected') is not (name == 'with-vpn'):
            raise ValueError('Diagnostic VPN state differs from the verified scenario')
        networks = platform['network']['physical_networks']
        if not 1 <= len(networks) <= 8 or any(
                ('mtu' in network) != (api >= 29)
                or ('private_dns_active' in network) != (api >= 28)
                for network in networks):
            raise ValueError('Android diagnostic fields violate the OS compatibility boundary')
        if 'ci-only-url-secret' in text or 'ci-controller-rejected' in text:
            raise ValueError('An isolated test credential leaked into diagnostics')
        if name == 'recovered' and len(measurements) != 4:
            raise ValueError('Measurement history was not bounded to four reports')
    if summary := os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(summary, 'a') as stream:
            stream.write(f'\nAndroid API {api}: measurement success/refusal, controller authentication failure, '
                         'VPN on/off, recovery and actual diagnostic-file writes verified. '
                         'This isolated fixture does not reproduce a reported public subscription failure or a vendor ROM.\n')


def application_preview(build, api, verify_diagnostics=False, compatibility=False,
                        screenshots=False, verify_application_rules=False):
    from workflow_artifacts import extract_build
    actual_api = device_property('ro.build.version.sdk')
    if actual_api != str(api):
        raise ValueError('Application preview device differs from the selected Android version')
    if not (sdk_tools()/'aapt2').is_file():
        raise RuntimeError('Android preview build tools are missing; prepare the selected emulator first')
    archive = build / 'build.tar.gz'
    expected = (build / 'build.tar.gz.sha256').read_text().strip()
    if not re.fullmatch('[a-f0-9]{64}', expected) or sha256(archive) != expected:
        raise ValueError('Application preview CI archive checksum mismatch')
    root = ROOT / 'target/android-application-preview-build'
    root.mkdir(parents=True, exist_ok=False)
    extract_build(archive, root)
    verify_build(root, 'x64', commit())
    if compatibility and not verify_diagnostics:
        test('x64', apk=root / 'payload/app.apk', test_apk=root / 'tests/androidTest.apk',
             basic_only=True)
    if verify_diagnostics:
        test('x64', apk=root / 'payload/app.apk', test_apk=root / 'tests/androidTest.apk',
             applications_only=False, diagnostics_only=True)
    if verify_application_rules:
        test('x64', apk=root / 'payload/app.apk', test_apk=root / 'tests/androidTest.apk',
             routing_only=True)
    if screenshots or (not compatibility and not verify_diagnostics):
        test('x64', apk=root / 'payload/app.apk', test_apk=root / 'tests/androidTest.apk',
             applications_only=True)


def package(arch, run_id, check_only, build=None):
    from workflow_artifacts import fetch_build
    from importlib.util import spec_from_file_location, module_from_spec
    spec=spec_from_file_location('publish_release',ROOT/'scripts/publish-release.py')
    module=module_from_spec(spec); spec.loader.exec_module(module)
    if build is None:
        repo=module.repository(); client=module.GitHub(repo,module.credential(repo))
        root=ROOT/'ci-build'
        fetch_build(client, run_id, 'android', arch, root, commit())
    else:
        if not check_only:
            raise ValueError('Local Android candidate input requires check mode')
        root=build
        verify_build(root,arch,commit())
    output=ROOT/'dist'; output.mkdir(exist_ok=True)
    name=f'Clash-of-Rust-{package_version()}-android-{arch}.apk'
    apk=output/name
    shutil.copy2(root/'payload/app.apk',apk)
    if not check_only:
        key=os.environ.get('ANDROID_KEYSTORE_PATH')
        if not key: raise RuntimeError('Formal Android APKs require a persistent Android signing key')
        tool=sdk_tools()
        # Instrumentation and its target must carry the same signature.
        for path in (apk,root/'tests/androidTest.apk'):
            run([tool/'apksigner','-J-enable-native-access=ALL-UNNAMED','sign','--ks',key,'--ks-key-alias',os.environ['ANDROID_KEY_ALIAS'],
                 '--ks-pass','env:ANDROID_KEYSTORE_PASSWORD','--key-pass','env:ANDROID_KEY_PASSWORD',path])
    sizes=validate_apk(apk,arch)
    tool=sdk_tools()
    run([tool/'apksigner','-J-enable-native-access=ALL-UNNAMED','verify','--verbose',apk])
    run([tool/'zipalign','-c','-P','16','4',apk])
    digest=sha256(apk)
    (output/(name+'.sha256')).write_text(digest+'\n')
    # Collect the normal reports before the process-death test clears app data.
    test(arch,apk,root/'tests/androidTest.apk')
    if arch == 'x64':
        test(arch, apk, root/'tests/androidTest.apk', protection_death_only=True)
    artifact=json.loads((root/'source-artifact.json').read_text())
    from build_inputs import package_contents
    write_json(output/'package-receipt.json',{'schema':1,'commit':commit(),'version':package_version(),
        'ci_run_id':run_id,'system':'android','arch':arch,'package':name,'tests_passed':True,
        'distribution_signed':not check_only,
        'build_artifact':artifact,'sha256':digest,'contents':package_contents(root)})
    summary=os.environ.get('GITHUB_STEP_SUMMARY')
    if summary:
        with open(summary,'a') as stream:
            stream.write(f'\nAndroid {arch}: {sizes["apk_bytes"]/1024**2:.2f} MiB.\n')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation',choices=('build','export','test','package','emulator','application-preview'))
    parser.add_argument('--verify-diagnostics', action='store_true',
                        help='Run explicitly requested diagnostic verification instead of UI screenshots')
    parser.add_argument('--compatibility', action='store_true',
                        help='Verify basic proxy/TUN using the selected CI APK')
    parser.add_argument('--screenshots', action='store_true',
                        help='Also capture pages after compatibility verification')
    parser.add_argument('--verify-application-rules', action='store_true',
                        help='Verify application scope and outbound routing after basic compatibility')
    parser.add_argument('--arch',choices=ABIS,required=True)
    parser.add_argument('--gradle-offline', action='store_true', help='Build using previously resolved Maven dependencies')
    parser.add_argument('--run-id',type=int)
    parser.add_argument('--check',action='store_true')
    parser.add_argument('--build',type=Path,help='CI candidate input for package --check or application-preview')
    parser.add_argument('--legacy',action='store_true',help='Use the approved Android 12 IME regression image')
    parser.add_argument('--ime-only',action='store_true',help='Run only the subscription input-method regression')
    parser.add_argument('--protection-death-only', action='store_true',
                        help='Run only the protection recovery test on the disposable API 36 cor-ci AVD')
    parser.add_argument('--api',type=int,help='Use an approved Android test image')
    args=parser.parse_args()
    if args.gradle_offline and args.operation != 'build':
        parser.error('--gradle-offline requires build')
    if args.verify_diagnostics and args.operation != 'application-preview':
        parser.error('--verify-diagnostics requires application-preview')
    if (args.compatibility or args.screenshots) and args.operation != 'application-preview':
        parser.error('--compatibility and --screenshots require application-preview')
    if args.screenshots and not args.compatibility:
        parser.error('--screenshots requires --compatibility')
    if args.verify_application_rules and (args.operation != 'application-preview' or not args.compatibility):
        parser.error('--verify-application-rules requires application-preview --compatibility')
    if args.build is not None and args.operation != 'application-preview' and (args.operation!='package' or not args.check):
        parser.error('--build requires package --check or application-preview')
    if args.operation == 'application-preview' and (args.arch != 'x64' or args.build is None or args.api is None):
        parser.error('application-preview requires --arch x64, --api and --build DIRECTORY')
    if args.legacy and args.operation!='emulator':
        parser.error('--legacy applies only to emulator preparation')
    if args.api is not None and (args.operation not in ('emulator', 'application-preview') or args.legacy):
        parser.error('--api applies only to emulator preparation or application-preview and cannot be combined with --legacy')
    if args.ime_only and args.operation!='test':
        parser.error('--ime-only applies only to test')
    if args.protection_death_only and (args.operation != 'test' or args.ime_only or args.arch != 'x64'):
        parser.error('--protection-death-only requires test --arch x64 without --ime-only')
    if args.operation=='emulator':
        avd_path=cor_ci_avd_path()
        (avd_path/PROTECTION_DEATH_LAUNCH_FILE).unlink(missing_ok=True)
        avds=avd_path.parent
        user_home=avds.parent
        device_root=user_home.parent
        sdk=Path(os.environ['ANDROID_HOME'])
        manager=prepare_sdk_manager()
        emulator,image=prepare_emulator(sdk,manager,device_root,legacy=args.legacy,api=args.api)
        avds.mkdir(parents=True,exist_ok=True)
        env=dict(os.environ,ANDROID_SDK_ROOT=str(sdk),ANDROID_USER_HOME=str(user_home),
                 ANDROID_AVD_HOME=str(avds),ANDROID_SDK_HOME=str(device_root))
        with sdk_emulator_for_avd(sdk, emulator):
            run([manager/'avdmanager','create','avd','--force','--name','cor-ci','--path',avd_path,
                 '--device','pixel_6','--package',image],input='no\n',text=True,env=env)
        report=ROOT/'target/android-report/x64'; report.mkdir(parents=True,exist_ok=True)
        if args.legacy or args.api is not None:
            report=report/(('ime-api-' if args.legacy else 'api-')+image.split(';')[1].removeprefix('android-'))
            report.mkdir(parents=True,exist_ok=True)
        configure_emulator_storage(avd_path,report)
        cores=min(4,os.cpu_count() or 1)
        print(f'Disposable Android emulator: 4096 MiB RAM, {cores} virtual CPUs',flush=True)
        with (report/'emulator.txt').open('w') as log:
            process=subprocess.Popen([emulator,'-avd','cor-ci','-no-window','-no-audio','-no-boot-anim','-no-snapshot','-no-metrics',
                '-gpu','swiftshader_indirect','-memory','4096','-cores',str(cores)],
                env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        run([ADB,'start-server'],env=env,timeout=30)
        wait_emulator_boot(process,env,report)
        protection_death_image = image.startswith('system-images;android-36;')
        adb = cor_ci_device(avd_path) if protection_death_image else [ADB]
        run(adb+['shell','settings','put','global','private_dns_mode','off'])
        run(adb+['shell','input','keyevent','KEYCODE_WAKEUP'])
        run(adb+['shell','wm','dismiss-keyguard'])
        if args.legacy:
            prepare_emulator_input_method(report)
        if args.legacy or args.api is not None:
            run(adb+['shell','wm','size','1080x2400'])
            run(adb+['shell','wm','density','440'])
        # Standard UI-test setup: the launcher's animated input sink can outlive
        # Activity.onResume. Keep system transitions deterministic in CI only.
        for setting in ('window_animation_scale','transition_animation_scale','animator_duration_scale'):
            run(adb+['shell','settings','put','global',setting,'0'])
        if protection_death_image:
            record_protection_death_launch(avd_path, adb)
    elif args.operation=='build': build(args.arch, gradle_offline=args.gradle_offline)
    elif args.operation=='export': export_build(args.arch)
    elif args.operation=='test':
        test(args.arch, ime_only=args.ime_only, protection_death_only=args.protection_death_only)
    elif args.operation=='application-preview':
        application_preview(args.build, args.api, args.verify_diagnostics,
                            args.compatibility, args.screenshots, args.verify_application_rules)
    else: package(args.arch,args.run_id,args.check,args.build)


if __name__=='__main__':
    main()
