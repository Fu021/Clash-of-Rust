"""Ubuntu 20.04 builds and native-kernel Linux package verification.

The host owns Actions credentials/caches. Only the source, Rust directories and
explicit build inputs enter the disposable container; guests receive no tokens.
"""
import argparse
import contextlib
from concurrent.futures import ThreadPoolExecutor
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import socket
import ssl
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request
import uuid

from build_support import ROOT, download as download_original, host_arch, opener, read_json, run, sha256, write_json
from linux_apt import OPTIONS as APT_OPTIONS
from linux_compatibility_targets import TARGETS, UBUNTU_BASELINE, target_for

BASELINE = UBUNTU_BASELINE
CACHE_ID = 'ubuntu20-v1'
PYTHON_REPO = 'astral-sh/python-build-standalone'
CLOUD_ROOT = 'https://cloud-images.ubuntu.com/focal/'
REPORT = ROOT/'target/linux-report'


def text(url, headers=None):
    with opener().open(urllib.request.Request(url, headers=headers or {}), timeout=60) as response:
        return response.read().decode()


def checksum_entry(sums, name):
    matches = re.findall(r'(?m)^([0-9a-fA-F]{64})\s+\*?' + re.escape(name) + r'$', sums)
    if len(matches) != 1:
        raise ValueError('Missing or ambiguous upstream SHA256: ' + name)
    return matches[0].lower()


def resolve_inputs(fetch=read_json, read=text):
    """Resolve metadata only; every Linux job consumes the frozen URLs/digests."""
    token = fetch('https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/ubuntu:pull')['token']
    headers = {'Authorization': 'Bearer '+token,
               'Accept': 'application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json'}
    request = urllib.request.Request(
        'https://registry-1.docker.io/v2/library/ubuntu/manifests/20.04', headers=headers)
    with opener().open(request, timeout=8) as response:
        document = response.read()
        digest = response.headers.get('Docker-Content-Digest', '')
    if (not re.fullmatch(r'sha256:[0-9a-f]{64}', digest)
            or hashlib.sha256(document).hexdigest() != digest.removeprefix('sha256:')):
        raise ValueError('Ubuntu registry manifest digest is missing or incorrect')
    release = fetch('https://api.github.com/repos/'+PYTHON_REPO+'/releases/latest')
    assets = release['assets']
    dates = re.findall(r'href="(\d{8}(?:\.\d+)?)/"', read(CLOUD_ROOT))
    if not dates:
        raise ValueError('No dated Ubuntu 20.04 cloud image found')
    cloud = CLOUD_ROOT+max(dates, key=lambda value: tuple(map(int, value.split('.'))))+'/'
    sums = read(cloud+'SHA256SUMS')
    result = {'version': BASELINE, 'image': 'ubuntu@'+digest, 'architectures': {}}
    from build_inputs import tool_versions
    python_version = re.escape(tool_versions()['python'])
    for arch, triple, deb in [('x64', 'x86_64', 'amd64'), ('arm64', 'aarch64', 'arm64')]:
        pattern = rf'cpython-{python_version}\+[^/]+-{triple}-unknown-linux-gnu-install_only\.tar\.gz'
        selected = [asset for asset in assets if re.fullmatch(pattern, asset['name'])]
        if len(selected) != 1:
            raise ValueError('Expected one approved portable Python archive for '+arch)
        asset = selected[0]
        checksum = (asset.get('digest') or '').removeprefix('sha256:')
        if not re.fullmatch('[0-9a-f]{64}', checksum):
            checksum = read(asset['browser_download_url']+'.sha256').split()[0]
        image = f'focal-server-cloudimg-{deb}.img'
        result['architectures'][arch] = {
            'python': {'url': asset['browser_download_url'], 'sha256': checksum},
            'cloud': {'url': cloud+image, 'sha256': checksum_entry(sums, image)},
        }
    return result


def inputs():
    from build_inputs import component
    value = component('linux')
    if not value or value.get('version') != BASELINE:
        raise ValueError('Verified CI snapshot lacks the Ubuntu 20.04 environment')
    if not re.fullmatch(r'ubuntu@sha256:[0-9a-f]{64}', value.get('image', '')):
        raise ValueError('Ubuntu build image must be pinned by digest')
    return value


def tools_directory():
    return Path(os.environ.get('COR_LINUX_TOOLS_DIR', ROOT/'target/linux-tools'))


def cache_keys(value=None, arch=None):
    value = value or inputs()
    arch = arch or host_arch()
    if arch not in ('x64', 'arm64'):
        raise ValueError('Unsupported Linux cache architecture')
    records = value['architectures'][arch]
    result = {'arch': arch, 'apt': f'linux-apt-ubuntu20-v1-{arch}'}
    for name in ('python', 'cloud'):
        checksum = records[name]['sha256']
        if not re.fullmatch('[0-9a-f]{64}', checksum):
            raise ValueError('Linux cache identity requires an upstream SHA256')
        result[name] = f'linux-{name}-ubuntu20-v1-{arch}-{checksum}'
    result['image'] = 'linux-image-ubuntu20-v1-' + arch + '-' + image_identity(value, arch)
    return result


def image_identity(value, arch):
    dockerfile = ROOT / 'resources/linux/ubuntu20.Dockerfile'
    return hashlib.sha256((value['image'] + value['architectures'][arch]['python']['sha256']
                           + sha256(dockerfile) + str(os.getuid()) + ':' + str(os.getgid())).encode()).hexdigest()


def download(record, path):
    algorithm = 'sha512' if 'sha512' in record else 'sha256'
    length = 128 if algorithm == 'sha512' else 64
    if not re.fullmatch(rf'[0-9a-f]{{{length}}}', record.get(algorithm, '')):
        raise ValueError(f'Download requires an upstream {algorithm.upper()}')
    download_original(record['url'], path, digest=record[algorithm], digest_algorithm=algorithm)
    print(f'Linux download: verified {path.name} ({path.stat().st_size / 1024**2:.1f} MiB)', flush=True)
    return path


def require_actions():
    if os.environ.get('GITHUB_ACTIONS') != 'true' or sys.platform != 'linux':
        raise RuntimeError('Linux integration requires a disposable Actions Linux runner')


def cache_downloads(vm=False):
    require_actions()
    records = inputs()['architectures'][host_arch()]
    files = [('python', 'python.tar.gz')]
    if vm:
        files.append(('cloud', 'focal.img'))
    for name, filename in files:
        download(records[name], tools_directory()/filename)


def guest_python(remote, environment, script, timeout):
    bootstrap = "import sys; sys.path.insert(0,'scripts'); "
    remote(environment+'/opt/cor-test-python/bin/python3 -c '+shlex.quote(
        bootstrap+script), timeout=timeout)


def guest_deb_downloads(ssh, remote, environment, staging, records=None):
    script = (
        "import json,pathlib,runpy,subprocess; "
        "from linux_apt import OPTIONS,archive_records,privileged; "
        "packages=runpy.run_path('scripts/test-release-package.py')['compatibility_download_dependencies'](); "
        "pathlib.Path('/home/ubuntu/cor-apt-plan-archives').mkdir(exist_ok=True); "
        "result=privileged([*OPTIONS,'-o','Acquire::ForceHash=SHA256','--print-uris',"
        "'-o','Dir::Cache::archives=/home/ubuntu/cor-apt-plan-archives',"
        "'install','-y','--no-install-recommends',*packages],120,stdout=subprocess.PIPE,text=True); "
        "pathlib.Path('/home/ubuntu/cor-apt-plan.json').write_text(json.dumps(archive_records(result.stdout)))"
    )
    if records is None:
        guest_python(remote, environment, script, timeout=180)
        records = json.loads(subprocess.check_output([*ssh, 'cat ~/cor-apt-plan.json'], timeout=60))
    from linux_apt import matches, source_url, valid_source
    if not isinstance(records, dict) or len(records) > 4096:
        raise ValueError('Guest APT download plan is invalid')
    for name, record in records.items():
        if (not isinstance(name, str) or not re.fullmatch(r'[A-Za-z0-9.+_%~:-]+\.deb', name)
                or not isinstance(record, dict) or set(record) != {'url', 'size', 'sha256'}
                or not isinstance(record['url'], str)
                or not valid_source(source_url(record['url']), record['sha256'], record['size'])):
            raise ValueError('Guest APT download plan has invalid metadata')
    staging.mkdir(parents=True, exist_ok=True)

    def fetch(entry):
        name, record = entry
        destination = staging/name
        download_original(source_url(record['url']), destination, digest=record['sha256'])
        if not matches(destination, record):
            raise ValueError('Guest DEB download differs from signed APT metadata')
        return destination

    with ThreadPoolExecutor(max_workers=4) as workers:
        archives = list(workers.map(fetch, records.items()))
    archive = staging.parent/'guest-debs.tar'
    with tarfile.open(archive, 'w') as tar:
        for path in archives:
            tar.add(path, arcname=path.name, recursive=False)
    with archive.open('rb') as stream:
        remote('mkdir -p ~/cor-apt && tar xf - -C ~/cor-apt', stdin=stream, timeout=600)
    print(f'Guest APT inputs: staged {len(archives)} verified archives from the official Ubuntu repositories',
          flush=True)


def system_ca_bundle():
    for candidate in (ssl.get_default_verify_paths().cafile, '/etc/ssl/certs/ca-certificates.crt'):
        if candidate and Path(candidate).is_file():
            return Path(candidate)
    raise RuntimeError('Ubuntu build requires the runner system CA bundle')


def build_image():
    require_actions()
    value = inputs()
    dockerfile = ROOT/'resources/linux/ubuntu20.Dockerfile'
    identity = image_identity(value, host_arch())
    tag = 'cor-ubuntu20:' + identity[:20]
    inspect = ['docker', 'image', 'inspect', '--format={{.Id}}', tag]
    present = subprocess.run(inspect, capture_output=True, text=True)
    if present.returncode == 0:
        return tag
    cache = Path(os.environ['COR_LINUX_BUILD_CACHE']) if os.environ.get('COR_LINUX_BUILD_CACHE') else None
    if cache and restore_build_image(cache, identity, inspect):
        return tag
    archive = download(value['architectures'][host_arch()]['python'],
                       tools_directory()/'python.tar.gz')
    with tempfile.TemporaryDirectory(prefix='cor-build-image-') as temporary:
        directory = Path(temporary)
        certificates = system_ca_bundle()
        shutil.copyfile(certificates, directory/'bootstrap-ca.crt')
        shutil.copy2(dockerfile, directory/'Dockerfile')
        shutil.copy2(archive, directory/'python.tar.gz')
        run(['docker', 'build', '--build-arg', 'BASE_IMAGE='+value['image'],
             '--build-arg', 'USER_ID='+str(os.getuid()), '--build-arg', 'GROUP_ID='+str(os.getgid()),
             '--tag', tag, directory], timeout=900)
    if cache:
        try:
            cache.mkdir(parents=True, exist_ok=True)
            image = cache/'image.tar'
            temporary = cache/'image.partial'
            run(['docker', 'image', 'save', '--output', temporary, tag], timeout=120)
            temporary.replace(image)
            identifier = subprocess.check_output(inspect, text=True).strip()
            write_json(cache/'image.json', {
                'schema': 1, 'identity': identity, 'id': identifier,
                'size': image.stat().st_size, 'sha256': sha256(image),
            })
            print(f'Ubuntu compiler container: archived {image.stat().st_size/1024**2:.1f} MiB', flush=True)
        except (OSError, ValueError, subprocess.SubprocessError):
            (cache/'image.json').unlink(missing_ok=True)
            print('::warning::Ubuntu compiler container archive failed; the prepared build remains usable',
                  flush=True)
    return tag


def restore_build_image(cache, identity, inspect):
    try:
        image, receipt = cache/'image.tar', cache/'image.json'
        if receipt.stat().st_size > 1024**2:
            raise ValueError('Build image receipt exceeds limit')
        value = json.loads(receipt.read_text())
        if (not isinstance(value, dict) or value.get('schema') != 1 or value.get('identity') != identity
                or not isinstance(value.get('id'), str)
                or not re.fullmatch(r'sha256:[0-9a-f]{64}', value['id'])
                or image.is_symlink() or receipt.is_symlink()
                or image.stat().st_size != value.get('size')
                or sha256(image) != value.get('sha256')):
            raise ValueError('Build image identity mismatch')
        run(['docker', 'image', 'load', '--input', image], timeout=120)
        actual = subprocess.check_output(inspect, text=True).strip()
        if actual != value['id']:
            raise ValueError('Loaded Docker image identity mismatch')
        print('Ubuntu build environment: verified GitHub cache; no VM or application data', flush=True)
        return True
    except (OSError, ValueError, subprocess.SubprocessError):
        print('Ubuntu build environment cache unavailable; preparing the frozen environment', flush=True)
        return False


def container(command):
    tag = build_image()
    home = Path.home()
    cargo = Path(os.environ.get('CARGO_HOME', home/'.cargo')).resolve()
    rustup = Path(os.environ.get('RUSTUP_HOME', home/'.rustup')).resolve()
    arguments = ['docker', 'run', '--rm', '--privileged',
                 '--user', f'{os.getuid()}:{os.getgid()}', '--workdir', ROOT,
                 '--volume', f'{ROOT}:{ROOT}', '--volume', f'{cargo}:{cargo}',
                 '--volume', f'{rustup}:{rustup}',
                 '--env', 'HOME=/home/builder', '--env', 'CARGO_HOME='+str(cargo),
                 '--env', 'RUSTUP_HOME='+str(rustup),
                 '--env', 'PATH=/opt/python/bin:'+str(cargo/'bin')+':/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',
                 '--env', 'GITHUB_ACTIONS=true', '--env', 'PYTHONDONTWRITEBYTECODE=1',
                 '--env', 'COR_LINUX_BASELINE='+BASELINE]
    for key in ('BUILD_INPUTS_DIR', 'GITHUB_OUTPUT', 'GITHUB_STEP_SUMMARY',
                'GITHUB_REPOSITORY', 'GITHUB_RUN_ID', 'GITHUB_RUN_ATTEMPT', 'RUST_BACKTRACE',
                'COR_CI_EXTENDED_TESTS', 'COR_CI_LINUX_USER_TESTS', 'COR_CI_DESKTOP_UI_TESTS',
                'COR_CI_UPDATE_TESTS', 'COR_CI_PROXY_BENCHMARK'):
        if key in os.environ:
            # Runner summary files live outside the source mount.
            if key in ('GITHUB_OUTPUT', 'GITHUB_STEP_SUMMARY'):
                path = Path(os.environ[key])
                arguments.extend(['--volume', f'{path.parent}:{path.parent}'])
            arguments.extend(['--env', key])
    if cache := os.environ.get('COR_LINUX_APT_CACHE'):
        Path(cache).mkdir(parents=True, exist_ok=True)
        arguments.extend(['--volume', cache + ':' + cache, '--env', 'COR_LINUX_APT_CACHE=' + cache])
    if snapshot := os.environ.get('WORKSPACE_SOURCE_CACHE'):
        directory = Path(snapshot).parent
        directory.mkdir(parents=True, exist_ok=True)
        arguments.extend(['--volume', f'{directory}:{directory}', '--env', 'WORKSPACE_SOURCE_CACHE'])
    run([*arguments, tag, *command], timeout=3300)


def glibc_requirements(output):
    names = set(re.findall(r'\bName:\s+(GLIBC_[A-Za-z0-9_.]+)', output))
    unsupported = [name for name in names if not re.fullmatch(r'GLIBC_\d+(?:\.\d+)+', name)
                   or tuple(map(int, name[6:].split('.'))) > (2, 31)]
    if unsupported:
        raise ValueError('ELF exceeds Ubuntu 20.04 glibc ABI: '+', '.join(sorted(unsupported)))
    return sorted(names)


def audit_elf(directory):
    """Check every shipped/test ELF, including separately downloaded mihomo."""
    records = {}
    for path in sorted(directory.rglob('*')):
        if not path.is_file() or path.is_symlink():
            continue
        with path.open('rb') as stream:
            if stream.read(4) != b'\x7fELF':
                continue
        versions = subprocess.check_output(['readelf', '--version-info', '--wide', path], text=True)
        records[path.relative_to(directory).as_posix()] = {
            'sha256': sha256(path), 'glibc': glibc_requirements(versions)}
        linked = subprocess.run(['ldd', path], capture_output=True, text=True)
        output = linked.stdout+linked.stderr
        static = 'not a dynamic executable' in output or 'statically linked' in output
        if 'not found' in output or (linked.returncode and not static):
            raise RuntimeError(f'Unresolved ELF dependencies for {path.name}:\n{output}')
    if not records:
        raise ValueError('No Linux ELF binaries found')
    return records


def baseline_report():
    release = dict(line.split('=', 1) for line in Path('/etc/os-release').read_text().splitlines() if '=' in line)
    if release.get('ID', '').strip('"') != 'ubuntu' or release.get('VERSION_ID', '').strip('"') != BASELINE:
        raise RuntimeError('Linux export/packaging must run inside Ubuntu 20.04')
    return {'distribution': BASELINE, 'image': inputs()['image'],
            'libc': subprocess.check_output(['getconf', 'GNU_LIBC_VERSION'], text=True).strip(),
            'packages': subprocess.check_output(['dpkg-query', '-W', '-f=${Package}=${Version}\\n'], text=True).splitlines()}


def wait_vm_ssh(process, command, timeout=600):
    deadline = time.monotonic() + timeout
    while True:
        if process.poll() is not None:
            raise RuntimeError('Linux VM exited during boot')
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError('Linux VM did not become SSH-ready')
        try:
            ready = subprocess.run([*command, 'true'], stdout=subprocess.DEVNULL,
                                   stderr=subprocess.DEVNULL, timeout=min(15, remaining))
        except subprocess.TimeoutExpired:
            ready = None
        if ready is not None and ready.returncode == 0:
            return
        time.sleep(min(5, max(0, deadline - time.monotonic())))


class VmGuest:
    def __init__(self, base, directory, label, target):
        self.target = target
        self.directory = Path(directory) / label
        self.directory.mkdir()
        self.process = None
        self.console = None
        boot_started = time.monotonic()
        self.architecture = 'x86_64' if host_arch() == 'x64' else 'aarch64'
        self.kvm = os.access('/dev/kvm', os.R_OK | os.W_OK)
        directory = self.directory
        key = directory / 'id_ed25519'
        run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', key], stdout=subprocess.DEVNULL)
        public = key.with_suffix('.pub').read_text().strip()
        user_data = {'ssh_genkeytypes': ['ed25519'],
                     'users': [{'name': 'ubuntu', 'groups': 'adm,sudo',
                               'sudo': 'ALL=(ALL) NOPASSWD:ALL', 'shell': '/bin/bash',
                               'ssh_authorized_keys': [public]}],
                     'ssh_pwauth': False, 'package_update': False, 'package_upgrade': False}
        (directory/'user-data').write_text('#cloud-config\n'+json.dumps(user_data))
        (directory/'meta-data').write_text(
            'instance-id: cor-linux-' + uuid.uuid4().hex + '\nlocal-hostname: cor-linux-test\n')
        seed = directory/'seed.img'
        self.disk = directory/'disk.qcow2'
        run(['cloud-localds', seed, directory/'user-data', directory/'meta-data'])
        run(['qemu-img', 'create', '-f', 'qcow2', '-F', 'qcow2', '-b', base, self.disk, '24G'])
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            port = listener.getsockname()[1]
        architecture, kvm = self.architecture, self.kvm
        print(f'{target.distribution} {target.release} VM: {label}, {architecture}, '
              f'acceleration={"kvm" if kvm else "tcg"}', flush=True)
        machine = 'q35' if architecture == 'x86_64' else 'virt'
        cpu = 'host' if kvm else ('max' if architecture == 'x86_64' else 'cortex-a72')
        command = ['qemu-system-'+architecture, '-machine', machine+',accel='+('kvm' if kvm else 'tcg'),
                   '-cpu', cpu, '-smp', '2', '-m', '4096', '-display', 'none',
                   '-serial', 'stdio', '-monitor', 'none',
                   '-drive', f'file={self.disk},if=virtio,format=qcow2,discard=unmap',
                   '-drive', f'file={seed},if=virtio,format=raw,readonly=on',
                   '-netdev', f'user,id=net0,hostfwd=tcp:127.0.0.1:{port}-:22',
                   '-device', 'virtio-net-pci,netdev=net0']
        if architecture == 'aarch64':
            command.extend(['-bios', '/usr/share/qemu-efi-aarch64/QEMU_EFI.fd'])
        elif target.uefi:
            firmware = Path('/usr/share/OVMF')
            code = firmware/'OVMF_CODE_4M.fd'
            variables = directory/'OVMF_VARS.fd'
            shutil.copyfile(firmware/'OVMF_VARS_4M.fd', variables)
            command.extend(['-drive', f'if=pflash,unit=0,format=raw,readonly=on,file={code}',
                            '-drive', f'if=pflash,unit=1,format=raw,file={variables}'])
        self.ssh = ['ssh', '-i', str(key), '-o', 'StrictHostKeyChecking=no',
                    '-o', 'UserKnownHostsFile='+str(directory/'known_hosts'),
                    '-o', 'ConnectTimeout=5', '-o', 'BatchMode=yes',
                    '-o', 'ControlMaster=auto', '-o', 'ControlPersist=60',
                    '-o', 'ControlPath='+str(directory/'ssh-control'),
                    '-p', str(port), 'ubuntu@127.0.0.1']
        self.environment = ('cd ~/cor-source && GITHUB_ACTIONS=true COR_LINUX_GUEST=true '
                            'BUILD_INPUTS_DIR=dependency-snapshot RUNNER_TEMP=/home/ubuntu/cor-runner-temp '
                            'COR_LINUX_APT_CACHE=/home/ubuntu/cor-apt '
                            'PATH=/opt/cor-test-python/bin:$PATH PYTHONDONTWRITEBYTECODE=1 ')
        from ci_test_policy import extended_tests, linux_user_tests
        self.environment += (
            f'COR_CI_EXTENDED_TESTS={str(extended_tests()).lower()} '
            f'COR_CI_LINUX_USER_TESTS={str(linux_user_tests()).lower()} ')
        try:
            self.console = (REPORT / ('vm-console-' + label + '.log')).open('w')
            self.process = subprocess.Popen(command, stdout=self.console, stderr=subprocess.STDOUT)
            wait_vm_ssh(self.process, self.ssh)
            self.remote('sudo timeout --signal=TERM --kill-after=30s 300s cloud-init status --wait',
                        timeout=360)
            actual = self.output('uname -m').strip()
            if actual != architecture:
                raise RuntimeError(f'Linux VM returned {actual} instead of {architecture}')
            identity = self.output('. /etc/os-release && printf "%s\\n" "$ID" "$VERSION_ID"').splitlines()
            if identity != [target.family, target.release]:
                raise RuntimeError(f'Linux VM returned {identity!r} instead of {target.family} {target.release}')
            self.kernel = self.output('uname -r').strip()
            if target.kernel_series and not self.kernel.startswith(target.kernel_series + '.'):
                raise RuntimeError(f'{target.distribution} {target.release} requires kernel '
                                   f'{target.kernel_series}, booted {self.kernel}')
            self.boot_seconds = round(time.monotonic() - boot_started, 3)
        except BaseException:
            self.close()
            raise

    def remote(self, script, **kwargs):
        return subprocess.run([*self.ssh, script], check=True, **kwargs)

    def output(self, script, timeout=60):
        return subprocess.check_output([*self.ssh, script], text=True, timeout=timeout)

    def python(self, script, timeout=120):
        return guest_python(self.remote, self.environment, script, timeout)

    def stage(self, archive, python):
        started = time.monotonic()
        try:
            with archive.open('rb') as stream:
                self.remote('mkdir -p ~/cor-source ~/cor-apt && tar xzf - -C ~/cor-source',
                            stdin=stream, timeout=300)
            with python.open('rb') as stream:
                self.remote('sudo mkdir -p /opt/cor-test-python && sudo tar xzf - '
                            '-C /opt/cor-test-python --strip-components=1', stdin=stream, timeout=120)
        finally:
            self.stage_seconds = round(time.monotonic() - started, 3)


    def preserve(self):
        for name, command in (('cloud-init', 'cloud-init analyze show'),
                              ('boot-chain', 'systemd-analyze critical-chain')):
            try:
                output = self.output(command, timeout=5)
            except (OSError, subprocess.SubprocessError) as error:
                output = 'Diagnostic unavailable: ' + type(error).__name__ + '\n'
            (REPORT / (self.directory.name + '-' + name + '.txt')).write_text(output)
        state = 'not_generated'
        try:
            with tempfile.TemporaryDirectory(prefix='guest-report-', dir=REPORT) as temporary:
                archive = Path(temporary)/'report.tar.gz'
                with archive.open('wb') as output:
                    result = subprocess.run(
                        [*self.ssh, 'cd ~/cor-source && if test -d target/linux-report; '
                                    'then tar czf - target/linux-report; fi'],
                        stdout=output, stderr=subprocess.PIPE, timeout=60)
                if result.returncode:
                    state = 'transfer_failed'
                    print(f'::warning::Guest diagnostic transfer failed (exit {result.returncode})', flush=True)
                elif archive.stat().st_size:
                    archive.replace(REPORT/'vm-guest-report.tar.gz')
                    state = 'saved'
        except (OSError, subprocess.SubprocessError) as error:
            state = 'transfer_failed'
            print(f'::warning::Guest diagnostic transfer failed ({type(error).__name__})', flush=True)
        write_json(REPORT/'vm-guest-report-status.json', {'schema': 1, 'state': state})


    def close(self):
        with contextlib.suppress(OSError, subprocess.SubprocessError):
            subprocess.run([self.ssh[0], '-O', 'exit', *self.ssh[1:]], timeout=10,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if self.process and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=10)
        if self.console:
            self.console.close()


def vm_source_archive(directory, include_frozen_apt=True):
    archive = directory / 'source.tar.gz'
    from build_inputs import FILES
    from workflow_artifacts import commit
    paths = {'Cargo.toml', '.github/build-versions.json', 'apps/cli/src/args.rs', *FILES}
    paths.update(path.relative_to(ROOT).as_posix() for path in (ROOT / 'scripts').glob('*.py'))
    snapshot = Path(os.environ.get('BUILD_INPUTS_DIR', 'dependency-snapshot'))
    if not snapshot.is_absolute():
        snapshot = ROOT / snapshot
    with tarfile.open(archive, 'w:gz', compresslevel=1) as tar:
        for relative in sorted(paths):
            tar.add(ROOT / relative, arcname=relative, recursive=False)
        tar.add(snapshot / 'inputs.json', arcname='dependency-snapshot/inputs.json')
        environment = snapshot / 'vm-environment' / host_arch()
        if include_frozen_apt and environment.exists():
            tar.add(environment, arcname='dependency-snapshot/vm-environment/' + host_arch())
        data = (commit() + '\n').encode('ascii')
        identity = tarfile.TarInfo('source-commit.txt')
        identity.size = len(data)
        identity.mode = 0o644
        tar.addfile(identity, io.BytesIO(data))
    return archive


def prepare_vm_guest(guest, directory, frozen=None):
    if frozen:
        guest_deb_downloads(guest.ssh, guest.remote, guest.environment, directory/'guest-debs',
                            frozen['lock']['downloads'])
        guest.python("from build_inputs import manifest; "
                     "from ubuntu_vm_environment import install_frozen, frozen_environments; "
                     "from build_support import host_arch; "
                     "value=manifest(); install_frozen('dependency-snapshot',"
                     "frozen_environments(value)[host_arch()],"
                     "value['upstream']['linux'])", timeout=2100)
    else:
        guest.remote('sudo timeout --signal=TERM --kill-after=30s 300s '
                     + shlex.join([*APT_OPTIONS, '-o', 'APT::Update::Error-Mode=any', 'update']),
                     timeout=360)
        if (guest.target.family, guest.target.release) == ('ubuntu', BASELINE):
            guest_deb_downloads(guest.ssh, guest.remote, guest.environment, directory/'guest-debs')


def fetch_vm_package(directory, package_run_id, timings):
    started = time.monotonic()
    try:
        if package_run_id:
            from workflow_artifacts import commit, fetch_package_inputs, release_module
            module = release_module()
            repo = module.repository()
            client = module.GitHub(repo, module.credential(repo))
            fetch_package_inputs(client, package_run_id, host_arch(), ROOT, commit(),
                                 int(os.environ['CI_RUN_ID']))
        with tarfile.open(directory/'package.tar.gz', 'w:gz', compresslevel=1) as tar:
            for relative in ('dist', 'target/linux-tun-test'):
                tar.add(ROOT/relative, arcname=relative)
        return directory / 'package.tar.gz'
    finally:
        timings['package_download_and_archive'] = round(time.monotonic() - started, 3)


def verify_vm_package(guest, directory, package_run_id, frozen=False, timings=None,
                      package_future=None):
    timings = timings if timings is not None else {}
    started = time.monotonic()
    try:
        archive = (package_future.result() if package_future is not None
                   else fetch_vm_package(directory, package_run_id, timings))
    finally:
        timings['package_remaining_wait'] = round(time.monotonic() - started, 3)
    started = time.monotonic()
    try:
        with archive.open('rb') as stream:
            guest.remote('tar xzf - -C ~/cor-source', stdin=stream, timeout=300)
    finally:
        timings['package_transfer'] = round(time.monotonic() - started, 3)
    extra = 'COR_UBUNTU_VM_FROZEN=true ' if frozen else ''
    target = guest.target
    command = shlex.join(['/opt/cor-test-python/bin/python3', 'scripts/test-release-package.py',
                          '--linux-compat', '--expect-linux', target.family, target.release,
                          '--expect-arch', target.arch])
    started = time.monotonic()
    try:
        guest.remote(guest.environment + extra + command,
                     timeout=2400 if guest.architecture == 'aarch64' or not guest.kvm else 1200)
    finally:
        timings['application_tests'] = round(time.monotonic() - started, 3)


def vm_test(package_run_id=None, suite='ubuntu', release=BASELINE):
    overall_started = time.monotonic()
    timings = {}
    require_actions()
    from build_inputs import manifest
    from ubuntu_vm_environment import PREPARATION_ERRORS, frozen_environments, verify_frozen

    REPORT.mkdir(parents=True, exist_ok=True)
    architecture = host_arch()
    target = target_for(suite, release, architecture)
    linux = inputs()
    value = linux['architectures'][architecture]
    frozen = None
    if target.image_url:
        cloud = {'url': target.image_url, target.digest_algorithm: target.image_digest}
    else:
        cloud = value['cloud']
        frozen = frozen_environments(manifest() or {}).get(architecture)
    if frozen and not verify_frozen(
            Path(os.environ.get('BUILD_INPUTS_DIR', 'dependency-snapshot')), frozen, linux):
        frozen = None
    temporary_root = Path(os.environ.get('RUNNER_TEMP', tempfile.gettempdir())).resolve()
    if temporary_root.is_relative_to(ROOT):
        raise ValueError('VM files must stay outside the checkout')
    with tempfile.TemporaryDirectory(prefix='cor-linux-vm-', dir=temporary_root) as temporary:
        directory = Path(temporary)
        phase_started = time.monotonic()
        archive = vm_source_archive(directory, include_frozen_apt=bool(frozen))
        timings['source_archive'] = round(time.monotonic() - phase_started, 3)
        guest = None
        result = {'distribution': target.family, 'version_id': target.release,
                  'arch': architecture, 'success': False,
                  'source_archive_bytes': archive.stat().st_size,
                  'cloud_url': cloud['url'],
                  'cloud_' + target.digest_algorithm: cloud[target.digest_algorithm]}
        with ThreadPoolExecutor(max_workers=1) as workers:
            try:
                package_future = workers.submit(fetch_vm_package, directory, package_run_id, timings)
                phase_started = time.monotonic()
                filename = f'{target.family}-{target.release}-{architecture}.qcow2' if target.image_url else 'focal.img'
                base = download(cloud, tools_directory()/filename)
                python = download(value['python'], tools_directory()/'python.tar.gz')
                timings['original_downloads'] = round(time.monotonic() - phase_started, 3)
                guest = VmGuest(base, directory, 'official-image', target)
                guest.stage(archive, python)
                phase_started = time.monotonic()
                try:
                    prepare_vm_guest(guest, directory, frozen)
                except PREPARATION_ERRORS as error:
                    if frozen is None:
                        raise
                    timings['frozen_preparation'] = round(time.monotonic() - phase_started, 3)
                    print('::warning::Frozen Ubuntu APT inputs unavailable; retrying on the same '
                          'official image with current signed indexes: ' + type(error).__name__, flush=True)
                    guest.close()
                    guest = VmGuest(base, directory, 'official-image-fresh-apt', target)
                    guest.stage(archive, python)
                    frozen = None
                    phase_started = time.monotonic()
                    prepare_vm_guest(guest, directory)
                timings['dependency_preparation'] = round(time.monotonic() - phase_started, 3)
                timings['final_guest_boot'] = guest.boot_seconds
                timings['source_transfer'] = guest.stage_seconds
                verify_vm_package(guest, directory, package_run_id, frozen=bool(frozen), timings=timings,
                                  package_future=package_future)
                result.update(kernel=guest.kernel, acceleration='kvm' if guest.kvm else 'tcg', success=True)
            finally:
                try:
                    if guest:
                        try:
                            guest.preserve()
                        finally:
                            guest.close()
                finally:
                    result.update(apt_inputs='frozen' if frozen else 'current',
                                  seconds=round(time.monotonic() - overall_started, 3),
                                  timings_seconds=timings)
                    write_json(REPORT/'linux-vm.json', result)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('ci', 'package', 'vm', 'cache-keys', 'cache-downloads', 'cache-image'))
    parser.add_argument('--vm', action='store_true', help='Include cloud image downloads')
    parser.add_argument('--package-run-id', type=int, help='Prepare the VM before waiting for this run’s Linux package')
    parser.add_argument('--suite', choices=tuple(TARGETS))
    parser.add_argument('--release')
    args, extra = parser.parse_known_args()
    if args.package_run_id is not None and (args.operation != 'vm' or args.package_run_id <= 0):
        parser.error('--package-run-id requires vm and a positive Actions run id')
    if (args.suite is None) != (args.release is None) or (args.suite and args.operation != 'vm'):
        parser.error('--suite and --release must be used together with vm')
    if args.operation in ('cache-keys', 'cache-downloads'):
        if extra or (args.vm and args.operation != 'cache-downloads'):
            parser.error('Unexpected cache arguments')
        require_actions()
        if args.operation == 'cache-keys':
            with Path(os.environ['GITHUB_OUTPUT']).open('a') as output:
                for name, key in cache_keys().items():
                    output.write(f'{name}={key}\n')
        else:
            cache_downloads(args.vm)
    elif args.vm:
        parser.error('--vm is only valid with cache-downloads')
    elif args.operation == 'cache-image':
        if extra:
            parser.error('cache-image takes no additional arguments')
        build_image()
        if (cache := os.environ.get('COR_LINUX_BUILD_CACHE')) and (output := os.environ.get('GITHUB_OUTPUT')):
            with Path(output).open('a') as stream:
                stream.write('ready=' + str((Path(cache)/'image.json').is_file()).lower() + '\n')
    elif args.operation == 'vm':
        if extra:
            parser.error('vm takes no additional arguments')
        vm_test(args.package_run_id, args.suite or 'ubuntu', args.release or BASELINE)
    elif args.operation == 'ci':
        require_actions()
        run([sys.executable, ROOT/'scripts/prepare-resources.py',
             '--system', 'linux', '--arch', host_arch()])
        container(['python3', 'scripts/run-ci.py', '--system', 'linux', '--arch', host_arch()])
    else:
        container(['python3', 'scripts/test-release-package.py', '--system', 'linux',
                   '--arch', host_arch(), *extra])


if __name__ == '__main__':
    main()
