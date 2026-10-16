"""Build Android on Actions and package the exact APK tested by its CI."""
import argparse
from collections import deque
import io
import json
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

from build_support import ROOT, package_version, sha256, write_json

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


def cargo_build(command, **kwargs):
    """Retry transient dependency downloads, while preserving compiler failures."""
    transient = re.compile(r'(?:HTTP(?:/[0-9.]+)?\s+|returned error:\s*)(?:429|50[0234])\b'
                           r'|Could not resolve host|Failed to connect to|Connection timed out', re.I)
    for attempt in range(3):
        print('+ ' + ' '.join(map(str, command)), flush=True)
        tail = deque(maxlen=160)
        with subprocess.Popen(list(map(str, command)), stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True, **kwargs) as process:
            for line in process.stdout:
                print(line, end='', flush=True)
                tail.append(line)
            status = process.wait()
        if status == 0:
            return
        if attempt == 2 or not transient.search(''.join(tail)):
            raise subprocess.CalledProcessError(status, command)
        delay = 15 * (attempt + 1)
        print(f'Transient dependency download failure; retrying in {delay}s', flush=True)
        time.sleep(delay)


def commit():
    return subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()


def ensure_binding_tool(name):
    package = 'golang.org/x/mobile/cmd/' + name
    executable = shutil.which(name)
    if executable:
        metadata = subprocess.run(['go', 'version', '-m', executable], text=True, capture_output=True)
        records = [line.split() for line in metadata.stdout.splitlines()]
        if metadata.returncode == 0 and ['path', package] in records and any(
                len(record) >= 3 and record[:2] == ['mod', 'golang.org/x/mobile']
                and record[2].endswith('-' + MOBILE_TOOL[:12]) for record in records):
            print('Reusing verified binding tool:', name, flush=True)
            return
    run(['go', 'install', package + '@' + MOBILE_TOOL])


def gradle(*args, arch=None):
    code = 1_000_000 + int(os.environ.get('GITHUB_RUN_NUMBER', '1'))
    command = ['gradle', '--no-daemon', '-p', ANDROID, '-PappVersion=' + package_version(), '-PappVersionCode=' + str(code)]
    if arch:
        command += ['-PappAbi=' + ABIS[arch][0]]
    return [*command, *args]


def seed_assets():
    from build_support import read_json, download_asset
    target = ANDROID/'app/offline-assets'
    target.mkdir(parents=True, exist_ok=True)
    # Reuse the approved desktop artwork; Android applies its own launcher mask.
    logo=ANDROID/'app/generated-res/drawable-nodpi/app_logo.png'
    logo.parent.mkdir(parents=True,exist_ok=True)
    shutil.copy2(ROOT/'resources/icon-concepts/app-blue-transparent.png',logo)
    shutil.copy2(ROOT/'resources/icons/app.png',logo.with_name('notification_logo.png'))
    release = read_json('https://api.github.com/repos/MetaCubeX/meta-rules-dat/releases/latest')
    records = []
    for remote, local in {'geoip.dat':'GeoIP.dat', 'geosite.dat':'GeoSite.dat', 'country.mmdb':'Country.mmdb', 'GeoLite2-ASN.mmdb':'ASN.mmdb'}.items():
        item = next(a for a in release['assets'] if a['name'] == remote)
        download_asset(item, target/local)
        records.append({'name': local, 'source': item['browser_download_url'],
                        'sha256': item['digest'].removeprefix('sha256:'), 'size': item['size']})
    write_json(target/'geodata.json', {'version':release['tag_name']+' | '+release['published_at'],
                                     'updated':int(time.time()), 'files':records})
    shutil.copy2(ROOT/'LICENSE', target/'LICENSE')
    shutil.copy2(ROOT/'resources/THIRD-PARTY-NOTICES.txt', target/'THIRD-PARTY-NOTICES.txt')
    for directory in ('fonts', 'flags', 'ip-check'):
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
    tool=Path(os.environ['ANDROID_HOME'])/'build-tools/36.0.0/aapt2'
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
    run(['go', 'mod', 'tidy'], cwd=core, env=env)
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
    with tarfile.open(output/ARCHIVE_NAME, 'w:gz', compresslevel=1) as tar:
        for name,path in records.items():
            tar.add(path, arcname=name)
        content=json.dumps(manifest).encode()
        entry=tarfile.TarInfo('manifest.json'); entry.size=len(content)
        tar.addfile(entry, io.BytesIO(content))
    (output/(ARCHIVE_NAME+'.sha256')).write_text(sha256(output/ARCHIVE_NAME)+'\n')


def verify_build(root, arch, expected_commit):
    manifest = json.loads((root/'manifest.json').read_text())
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


def test(arch, apk=None, test_apk=None):
    if arch != 'x64':
        return
    apk = apk or ANDROID/'app/build/outputs/apk/release/app-release.apk'
    test_apk = test_apk or ANDROID/'app/build/outputs/apk/androidTest/release/app-release-androidTest.apk'
    run([ADB,'install','-r',apk])
    run([ADB,'install','-r',test_apk])
    import http.server
    import threading
    class Origin(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            body=b'cor-android-network-ok'
            self.send_response(200); self.send_header('Content-Length',str(4096 if self.path=='/hold' else len(body))); self.end_headers()
            self.wfile.write(body); self.wfile.flush()
            if self.path=='/hold': time.sleep(15)
        def log_message(self,*args): pass
    server=http.server.ThreadingHTTPServer(('0.0.0.0',18081),Origin)
    threading.Thread(target=server.serve_forever,daemon=True).start()
    run([ADB,'shell','appops','set','org.clashofrust.android','ACTIVATE_VPN','allow'])
    report=ROOT/'target/android-report'/arch
    report.mkdir(parents=True,exist_ok=True)
    try:
        result = subprocess.run([ADB,'shell','am','instrument','-w','org.clashofrust.android.test/androidx.test.runner.AndroidJUnitRunner'],
                                text=True,capture_output=True,timeout=240)
        (report/'instrumentation.txt').write_text(result.stdout+result.stderr)
    finally:
        server.shutdown(); server.server_close()
        log = subprocess.run([ADB,'logcat','-d','-v','threadtime'],capture_output=True,text=True,timeout=30)
        (report/'logcat.txt').write_text(log.stdout)
    print(result.stdout, flush=True)
    if 'Android startup:' in log.stdout or 'Mobile runtime stopped:' in log.stdout:
        raise RuntimeError('Native Android UI/runtime failed; inspect logcat')
    if result.returncode or 'FAILURES!!!' in result.stdout or 'OK (' not in result.stdout:
        raise RuntimeError('Android real proxy/TUN integration test failed')


def package(arch, run_id, check_only):
    from workflow_artifacts import fetch_build
    from importlib.util import spec_from_file_location, module_from_spec
    spec=spec_from_file_location('publish_release',ROOT/'scripts/publish-release.py')
    module=module_from_spec(spec); spec.loader.exec_module(module)
    repo=module.repository(); client=module.GitHub(repo,module.credential(repo))
    root=ROOT/'ci-build'
    fetch_build(client, run_id, 'android', arch, root, commit())
    output=ROOT/'dist'; output.mkdir(exist_ok=True)
    name=f'Clash-of-Rust-{package_version()}-android-{arch}.apk'
    apk=output/name
    shutil.copy2(root/'payload/app.apk',apk)
    if not check_only:
        key=os.environ.get('ANDROID_KEYSTORE_PATH')
        if not key: raise RuntimeError('Formal Android APKs require a persistent Android signing key')
        tool=Path(os.environ['ANDROID_HOME'])/'build-tools/36.0.0'
        # Instrumentation and its target must carry the same signature.
        for path in (apk,root/'tests/androidTest.apk'):
            run([tool/'apksigner','sign','--ks',key,'--ks-key-alias',os.environ['ANDROID_KEY_ALIAS'],
                 '--ks-pass','env:ANDROID_KEYSTORE_PASSWORD','--key-pass','env:ANDROID_KEY_PASSWORD',path])
    sizes=validate_apk(apk,arch)
    tool=Path(os.environ['ANDROID_HOME'])/'build-tools/36.0.0'
    run([tool/'apksigner','verify','--verbose',apk])
    run([tool/'zipalign','-c','-P','16','4',apk])
    digest=sha256(apk)
    (output/(name+'.sha256')).write_text(digest+'  '+name+'\n')
    # x64 receives the same emulator proxy/TUN test after final signing.
    test(arch,apk,root/'tests/androidTest.apk')
    artifact=json.loads((root/'source-artifact.json').read_text())
    write_json(output/'package-receipt.json',{'schema':1,'commit':commit(),'version':package_version(),
        'ci_run_id':run_id,'system':'android','arch':arch,'package':name,'tests_passed':True,
        'build_artifact':artifact,'sha256':digest})
    summary=os.environ.get('GITHUB_STEP_SUMMARY')
    if summary:
        with open(summary,'a') as stream:
            stream.write(f'\nAndroid {arch}: {sizes["apk_bytes"]/1024**2:.2f} MiB.\n')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation',choices=('build','export','test','package','emulator'))
    parser.add_argument('--arch',choices=ABIS,required=True)
    parser.add_argument('--run-id',type=int)
    parser.add_argument('--check',action='store_true')
    args=parser.parse_args()
    if args.operation=='emulator':
        sdk=Path(os.environ['ANDROID_HOME'])
        manager=sdk/'cmdline-tools/latest/bin'
        run([manager/'sdkmanager','platform-tools','system-images;android-35;google_apis;x86_64'])
        device_root=Path(os.environ['RUNNER_TEMP'])/'cor-emulator'
        user_home=device_root/'.android'; avds=user_home/'avd'
        avds.mkdir(parents=True,exist_ok=True)
        env=dict(os.environ,ANDROID_USER_HOME=str(user_home),ANDROID_AVD_HOME=str(avds),ANDROID_SDK_HOME=str(device_root))
        run([manager/'avdmanager','create','avd','--force','--name','cor-ci','--path',avds/'cor-ci.avd',
             '--device','pixel_6','--package','system-images;android-35;google_apis;x86_64'],input='no\n',text=True,env=env)
        report=ROOT/'target/android-report/x64'; report.mkdir(parents=True,exist_ok=True)
        with (report/'emulator.txt').open('w') as log:
            process=subprocess.Popen([sdk/'emulator/emulator','-avd','cor-ci','-no-window','-no-audio','-no-boot-anim','-no-snapshot',
                '-gpu','swiftshader_indirect','-memory','2048'],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
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
        # Standard UI-test setup: the launcher's animated input sink can outlive
        # Activity.onResume. Keep system transitions deterministic in CI only.
        for setting in ('window_animation_scale','transition_animation_scale','animator_duration_scale'):
            run([ADB,'shell','settings','put','global',setting,'0'])
    elif args.operation=='build': build(args.arch)
    elif args.operation=='export': export_build(args.arch)
    elif args.operation=='test': test(args.arch)
    else: package(args.arch,args.run_id,args.check)


if __name__=='__main__':
    main()
