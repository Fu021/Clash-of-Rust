"""Build Android on Actions and package the exact APK tested by its CI."""
import argparse
from collections import deque
import io
import json
import math
import os
import re
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tarfile
import time
import zipfile

from build_support import ROOT, mihomo_go_version, package_version, sha256, write_json, write_mihomo_notice
from build_inputs import android_versions

ANDROID = ROOT / 'apps/android'
ADB = str(Path(os.environ.get('ANDROID_HOME', '/nonexistent')) / 'platform-tools/adb')
ABIS = {
    'arm64': ('arm64-v8a', 'aarch64-linux-android', 'aarch64-linux-android', 'arm64'),
    'armv7': ('armeabi-v7a', 'armv7-linux-androideabi', 'armv7a-linux-androideabi', 'arm'),
    'x64': ('x86_64', 'x86_64-linux-android', 'x86_64-linux-android', 'amd64'),
}
MOBILE_TOOL = '19777044486779222400e9ca16c4306f9c6dbfa9'


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


def cargo_build(command, **kwargs):
    """Retry transient dependency downloads, while preserving compiler failures."""
    transient = re.compile(r'(?:HTTP(?:/[0-9.]+)?\s+|returned error:\s*)(?:429|50[0234])\b'
                           r'|Could not resolve host|Failed to connect to|Connection timed out', re.I)
    for attempt in range(3):
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


def verify_emulator_versions(sdk, legacy=False, api=None):
    versions = android_versions()
    image, _, source_revision = emulator_image(legacy, api)
    def revision(value):
        parts = value.split('.')
        return tuple(int(part) for part in parts + ['0'] * (3 - len(parts)))
    for relative, expected in (('emulator', versions['emulator']),
                               (image.replace(';', '/'), source_revision)):
        properties = (sdk / relative / 'source.properties').read_text()
        match = re.search(r'(?m)^\s*Pkg\.Revision\s*=\s*([0-9.]+)\s*$', properties)
        if not match or revision(match[1]) != revision(expected):
            actual = match[1] if match else 'missing'
            raise ValueError(f'Android runtime revision differs from the approved version: {relative}: '
                             f'expected {expected}, got {actual}')
        print('Verified Android runtime:', relative, match[1], flush=True)


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


def gradle(*args, arch=None):
    code = 1_000_000 + int(os.environ.get('GITHUB_RUN_NUMBER', '1'))
    command = ['gradle', '--no-daemon', '--warning-mode', 'all', '-p', ANDROID,
               '-PappVersion=' + package_version(), '-PappVersionCode=' + str(code),
               '-PappNdkVersion=' + android_versions()['ndk']]
    if os.environ.get('RUNNER_DEBUG') == '1':
        command.extend(['--info', '-Dorg.gradle.deprecation.trace=true'])
    if arch:
        command += ['-PappAbi=' + ABIS[arch][0]]
    return [*command, *args]


def seed_assets():
    from build_support import read_json, download_asset
    from build_inputs import resource_release
    target = ANDROID/'app/offline-assets'
    target.mkdir(parents=True, exist_ok=True)
    # Reuse the approved desktop artwork; Android applies its own launcher mask.
    logo=ANDROID/'app/generated-res/drawable-nodpi/app_logo.png'
    logo.parent.mkdir(parents=True,exist_ok=True)
    shutil.copy2(ROOT/'resources/icon-concepts/app-blue-transparent.png',logo)
    shutil.copy2(ROOT/'resources/icons/app.png',logo.with_name('notification_logo.png'))
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



def build(arch):
    from action_cache import workspace_sources
    from ci_test_policy import extended_tests
    workspace_sources(ROOT)
    packages = ['-p', 'clash-mobile-runtime', '-p', 'clash-android']
    targets = ['--all-targets'] if extended_tests() else ['--lib']
    cargo_build(['cargo', 'clippy', *packages, '--release', *targets, '--locked', '--', '-D', 'warnings'], cwd=ROOT)
    if extended_tests():
        run(['cargo', 'test', *packages, '--release', '--locked', '--lib',
             '--no-fail-fast', '--', '--test-threads=1'], cwd=ROOT)
    else:
        run(['cargo', 'test', '-p', 'clash-android', '--release', '--locked', '--lib',
             '--', '--test-threads=1'], cwd=ROOT)
    abi, target, clang_target, go_arch = ABIS[arch]
    ndk = Path(os.environ['ANDROID_NDK_ROOT'])
    toolchain = ndk/'toolchains/llvm/prebuilt/linux-x86_64/bin'
    clang = toolchain/(clang_target+'26-clang')
    env = dict(os.environ)
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
    cargo_build(['cargo', 'clippy', '-p', 'clash-android', '--target', target, '--release', '--locked', '--', '-D', 'warnings'], cwd=ROOT, env=env)
    cargo_build(['cargo', 'build', '-p', 'clash-android', '--target', target, '--release', '--locked'], cwd=ROOT, env=env)
    destination = ANDROID/'app/native-libs'/abi
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ROOT/'target'/target/'release/libclash_mobile.so', destination)
    # Android's supported backend is Skia, not the desktop software renderer.
    ensure_binding_tool('gomobile')
    ensure_binding_tool('gobind')
    env['ANDROID_NDK_HOME'] = str(ndk)
    core = ANDROID/'core'
    run(['go', 'mod', 'tidy', '-diff'], cwd=core, env=env)
    aar = ANDROID/'app/libs/mihomo.aar'
    aar.parent.mkdir(parents=True, exist_ok=True)
    run(['gomobile', 'bind', '-target', 'android/'+go_arch, '-androidapi', '26',
         # VpnService owns routing and package exclusions. The default Android
         # listener tries to read root-only /data/system/packages.xml; cmfa
         # selects the upstream app-hosted path. Our TUN explicitly uses gVisor.
         '-tags', 'cmfa,with_gvisor',
         '-javapkg', 'org.clashofrust.nativecore', '-ldflags=-s -w -extldflags=-Wl,-z,max-page-size=16384',
         '-o', aar, '.'], cwd=core, env=env)
    seed_assets()
    try:
        run(gradle('assembleRelease', 'assembleReleaseAndroidTest', 'lintRelease', arch=arch), cwd=ROOT)
    finally:
        report=ROOT/'target/android-report'/arch
        report.mkdir(parents=True,exist_ok=True)
        for path in (ANDROID/'app/build/reports').glob('lint-results-release.*'):
            shutil.copy2(path,report/path.name)
    apk = ANDROID/'app/build/outputs/apk/release/app-release.apk'
    sizes = validate_apk(apk, arch)
    report = {'commit':commit(), 'arch':arch, 'version':package_version(), **sizes}
    write_json(ROOT/'target/android-report'/arch/'build.json', report)
    print(json.dumps(report), flush=True)
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


def pull_device_report(remote, report, required=False, required_pattern='*.png', name='ime-report'):
    destination=report/name
    destination.mkdir(exist_ok=True)
    command=[ADB,'pull',remote.rstrip('/')+'/.',str(destination)]
    result=subprocess.run(command,capture_output=True,text=True,timeout=30)
    output=[result.stdout+result.stderr]
    if result.returncode and required:
        properties=[subprocess.run([ADB,'shell','getprop',name],capture_output=True,text=True,timeout=15)
                    for name in ('ro.kernel.qemu','ro.debuggable')]
        if all(value.returncode==0 and value.stdout.strip()=='1' for value in properties):
            restart=subprocess.run([ADB,'root'],capture_output=True,text=True,timeout=30)
            output.append(restart.stdout+restart.stderr)
            if restart.returncode==0:
                connected=subprocess.run([ADB,'wait-for-device'],capture_output=True,text=True,timeout=30)
                output.append(connected.stdout+connected.stderr)
                if connected.returncode==0:
                    result=subprocess.run(command,capture_output=True,text=True,timeout=30)
                    output.append(result.stdout+result.stderr)
    pull_log = name.removesuffix('-report') + '-pull.txt'
    (report/pull_log).write_text('\n'.join(output))
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


def start_test_log(report, marker):
    with (report/'logcat.raw').open('wb') as output, (report/'logcat-stderr.txt').open('wb') as errors:
        process=subprocess.Popen([ADB,'logcat','-b','main,system,crash','-v','threadtime'],
                                 stdout=output,stderr=errors)
    try:
        run([ADB,'shell','log','-t','CorAndroidTest',marker],timeout=15)
        wait_test_log_marker(process,report,marker)
    except BaseException:
        stop_test_log(process)
        raise
    return process


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


def finish_test_log(process, report, marker):
    error=None
    end_marker=marker+'-end'
    try:
        if process.poll() is not None:
            raise RuntimeError('Android logcat stream exited before test completion')
        run([ADB,'shell','log','-t','CorAndroidTest',end_marker],timeout=15)
        wait_test_log_marker(process,report,end_marker)
    except Exception as capture_error:
        error=capture_error
    finally:
        try:
            stop_test_log(process)
        except Exception as cleanup_error:
            error=error or cleanup_error
    raw=(report/'logcat.raw').read_bytes()
    try:
        captured_log=scoped_test_log(raw,marker,end_marker)
    except RuntimeError as scope_error:
        error=error or scope_error
        captured_log=raw.decode('utf-8',errors='replace')
    (report/'logcat.txt').write_text(captured_log)
    return captured_log,error


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


def test(arch, apk=None, test_apk=None, ime_only=False, ime_probe=False,
         ime_rounds=1, applications_only=False):
    if not 1 <= ime_rounds <= 3 or ((ime_probe or ime_rounds != 1) and not ime_only):
        raise ValueError('IME probe options require IME-only testing and one to three rounds')
    if applications_only and ime_only:
        raise ValueError('Application screenshots and input-method tests are separate')
    network_tests = not (ime_only or applications_only)
    if arch != 'x64':
        return
    apk = apk or ANDROID/'app/build/outputs/apk/release/app-release.apk'
    test_apk = test_apk or ANDROID/'app/build/outputs/apk/androidTest/release/app-release-androidTest.apk'
    run([ADB,'install','-r',apk])
    run([ADB,'install','-r',test_apk])
    import http.server
    import threading
    class Origin(http.server.BaseHTTPRequestHandler):
        protocol_version = 'HTTP/1.1'
        def do_CONNECT(self):
            # Only the test core can reach this origin for the reserved target.
            # The emulator's directly reachable host alias cannot prove TUN
            # capture: use a negative control and a real HTTP proxy handshake.
            if self.path != '198.51.100.1:18081':
                self.send_error(403)
                return
            self.send_response(200, 'Connection established'); self.end_headers(); self.wfile.flush()
            self.close_connection = False
            self.handle_one_request()
        def do_GET(self):
            body=b'cor-android-network-ok'
            self.send_response(200); self.send_header('Content-Length',str(4096 if self.path=='/hold' else len(body))); self.end_headers()
            self.wfile.write(body); self.wfile.flush()
            if self.path=='/hold': time.sleep(15)
        def log_message(self,*args): pass
    server=None
    if network_tests:
        server=http.server.ThreadingHTTPServer(('0.0.0.0',18081),Origin)
        threading.Thread(target=server.serve_forever,daemon=True).start()
    # Exercise Android's real first-use dialog; pre-granting consent hides
    # ordering bugs between authorization, protection DNS and VPN startup.
    if network_tests:
        run([ADB,'shell','appops','set','org.clashofrust.android','ACTIVATE_VPN','default'])
    report=ROOT/'target/android-report'/arch
    if ime_only:
        api=subprocess.check_output([ADB,'shell','getprop','ro.build.version.sdk'],text=True,timeout=15).strip()
        if not api.isdigit():
            raise ValueError('Could not identify the Android IME test API level')
        report=report/('ime-api-'+api)
        report=report/('production' if ime_probe else 'regression')
    if applications_only:
        api = subprocess.check_output([ADB, 'shell', 'getprop', 'ro.build.version.sdk'],
                                      text=True, timeout=15).strip()
        if not api.isdigit():
            raise ValueError('Could not identify the Android application preview version')
        report = report / ('applications-api-' + api)
    report.mkdir(parents=True,exist_ok=True)
    properties={}
    for name in ('ro.build.version.sdk','ro.build.version.release','ro.build.fingerprint',
                 'ro.product.manufacturer','ro.product.model','ro.product.cpu.abi'):
        properties[name]=subprocess.check_output([ADB,'shell','getprop',name],text=True,timeout=15).strip()
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
        from ci_test_policy import android_cases
        cases=(('androidUiPreviewCapturesPagesAndApplications',) if applications_only else
               ('subscriptionKeyboardProbeCapturesBothFields' if ime_probe
                else 'subscriptionFieldsShowImeAcceptTextAndReopenAfterBack',)
               if ime_only else android_cases())
        failures=[]
        for iteration in range(1,ime_rounds+1):
            if ime_only:
                animation_scale='0' if iteration==3 else '1'
                for setting in ('window_animation_scale','transition_animation_scale','animator_duration_scale'):
                    run([ADB,'shell','settings','put','global',setting,animation_scale])
                print(f'IME round {iteration}: system animation scale {animation_scale}',flush=True)
            for case in cases:
                run([ADB,'shell','am','force-stop','org.clashofrust.android'])
                result = subprocess.run([ADB,'shell','am','instrument','-w','-e','class',
                    'org.clashofrust.android.ProxyTunTest#'+case,
                    '-e','ime_round',str(iteration),
                    '-e','ime_probe',str(ime_probe).lower(),
                    'org.clashofrust.android.test/androidx.test.runner.AndroidJUnitRunner'],
                    text=True,capture_output=True,timeout=600 if applications_only else 240)
                outputs.append(f'Round {iteration}: '+case+'\n'+result.stdout+result.stderr)
                (report/'instrumentation.txt').write_text('\n\n'.join(outputs))
                print(result.stdout,flush=True)
                if result.returncode or 'FAILURES!!!' in result.stdout or 'OK (' not in result.stdout:
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
        diagnostics={'connectivity':['dumpsys','connectivity'],
                     'routes':['ip','route','show','table','all'],
                     'vpn':['dumpsys','vpn_management'],
                     'windows':['dumpsys','window','windows'],
                     'activities':['dumpsys','activity','activities'],
                     'power':['dumpsys','power']}
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


def application_preview(build, api):
    from workflow_artifacts import extract_build
    actual_api = subprocess.check_output([ADB, 'shell', 'getprop', 'ro.build.version.sdk'],
                                         text=True, timeout=15).strip()
    if actual_api != str(api):
        raise ValueError('Application preview device differs from the selected Android version')
    archive = build / 'build.tar.gz'
    expected = (build / 'build.tar.gz.sha256').read_text().strip()
    if not re.fullmatch('[a-f0-9]{64}', expected) or sha256(archive) != expected:
        raise ValueError('Application preview CI archive checksum mismatch')
    root = ROOT / 'target/android-application-preview-build'
    root.mkdir(parents=True, exist_ok=False)
    extract_build(archive, root)
    prepare_sdk(package_only=True)
    verify_build(root, 'x64', commit())
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
    # x64 receives the same emulator proxy/TUN test after final signing.
    test(arch,apk,root/'tests/androidTest.apk')
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
    parser.add_argument('--arch',choices=ABIS,required=True)
    parser.add_argument('--run-id',type=int)
    parser.add_argument('--check',action='store_true')
    parser.add_argument('--build',type=Path,help='CI candidate input for package --check or application-preview')
    parser.add_argument('--legacy',action='store_true',help='Use the approved Android 12 IME regression image')
    parser.add_argument('--ime-only',action='store_true',help='Run only the subscription input-method regression')
    parser.add_argument('--api',type=int,help='Use an approved Android test image')
    args=parser.parse_args()
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
    if args.operation=='emulator':
        sdk=Path(os.environ['ANDROID_HOME'])
        manager=prepare_sdk_manager()
        device_root=Path(os.environ['RUNNER_TEMP'])/'cor-emulator'
        versions=android_versions()
        image,image_revision,_=emulator_image(args.legacy,args.api)
        install_sdk_packages(sdk_install_command(manager, [
            'platform-tools@'+versions['platform_tools'], 'emulator@'+versions['emulator'],
            image+'@'+image_revision]), device_root)
        verify_emulator_versions(sdk,legacy=args.legacy,api=args.api)
        user_home=device_root/'.android'; avds=user_home/'avd'
        avds.mkdir(parents=True,exist_ok=True)
        env=dict(os.environ,ANDROID_USER_HOME=str(user_home),ANDROID_AVD_HOME=str(avds),ANDROID_SDK_HOME=str(device_root))
        run([manager/'avdmanager','create','avd','--force','--name','cor-ci','--path',avds/'cor-ci.avd',
             '--device','pixel_6','--package',image],input='no\n',text=True,env=env)
        report=ROOT/'target/android-report/x64'; report.mkdir(parents=True,exist_ok=True)
        if args.legacy or args.api is not None:
            report=report/(('ime-api-' if args.legacy else 'api-')+image.split(';')[1].removeprefix('android-'))
            report.mkdir(parents=True,exist_ok=True)
        configure_emulator_storage(avds/'cor-ci.avd',report)
        cores=min(4,os.cpu_count() or 1)
        print(f'Disposable Android emulator: 4096 MiB RAM, {cores} virtual CPUs',flush=True)
        with (report/'emulator.txt').open('w') as log:
            process=subprocess.Popen([sdk/'emulator/emulator','-avd','cor-ci','-no-window','-no-audio','-no-boot-anim','-no-snapshot','-no-metrics',
                '-gpu','swiftshader_indirect','-memory','4096','-cores',str(cores)],
                env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        run([ADB,'start-server'],env=env,timeout=30)
        deadline=time.monotonic()+180
        while time.monotonic()<deadline:
            if process.poll() is not None:
                raise RuntimeError('Android emulator exited: '+(report/'emulator.txt').read_text()[-4000:])
            ready=subprocess.run([ADB,'shell','getprop','sys.boot_completed'],env=env,capture_output=True,text=True,timeout=10)
            if ready.returncode==0 and ready.stdout.strip()=='1':break
            time.sleep(2)
        else: raise RuntimeError('Android emulator did not finish booting')
        run([ADB,'shell','settings','put','global','private_dns_mode','off'])
        run([ADB,'shell','input','keyevent','KEYCODE_WAKEUP'])
        run([ADB,'shell','wm','dismiss-keyguard'])
        if args.legacy:
            prepare_emulator_input_method(report)
        if args.legacy or args.api is not None:
            run([ADB,'shell','wm','size','1080x2400'])
            run([ADB,'shell','wm','density','440'])
        # Standard UI-test setup: the launcher's animated input sink can outlive
        # Activity.onResume. Keep system transitions deterministic in CI only.
        for setting in ('window_animation_scale','transition_animation_scale','animator_duration_scale'):
            run([ADB,'shell','settings','put','global',setting,'0'])
    elif args.operation=='build': build(args.arch)
    elif args.operation=='export': export_build(args.arch)
    elif args.operation=='test': test(args.arch,ime_only=args.ime_only)
    elif args.operation=='application-preview': application_preview(args.build, args.api)
    else: package(args.arch,args.run_id,args.check,args.build)


if __name__=='__main__':
    main()
