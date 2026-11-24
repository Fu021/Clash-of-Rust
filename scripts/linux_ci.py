"""Ubuntu 20.04 builds and Linux package verification.

The host owns Actions credentials/caches. Only the source, Rust directories and
explicit build inputs enter the disposable container; guests receive no tokens.
"""
import argparse
import contextlib
from concurrent.futures import ThreadPoolExecutor
import hashlib
import io
import json
import math
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
from urllib.error import HTTPError, URLError
import urllib.request
import uuid

from build_support import ROOT, download as download_original, host_arch, opener, read_json, run, sha256, write_json
from linux_compatibility_targets import TARGETS, UBUNTU_BASELINE, target_for

BASELINE = UBUNTU_BASELINE
CACHE_ID = 'ubuntu20-v1'
PYTHON_REPO = 'astral-sh/python-build-standalone'
CLOUD_ROOT = 'https://cloud-images.ubuntu.com/focal/'
REPORT = ROOT/'target/linux-report'
KVM_GET_API_VERSION = 0xAE00
KVM_CREATE_VM = 0xAE01
KVM_API_VERSION = 12
PUBLISHED_UPDATE_CLEANUP_SECONDS = 420
PUBLISHED_UPDATE_TRANSPORT_SECONDS = 60


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


def probe_image_mirror(url):
    curl = shutil.which('curl')
    if not curl:
        return None
    sample = 1024*1024
    try:
        result = subprocess.run(
            [curl, '--location', '--fail', '--silent', '--show-error',
             '--proto', '=https', '--proto-redir', '=https',
             '--connect-timeout', '5', '--max-time', '12',
             '--max-filesize', str(sample), '--range', f'0-{sample-1}',
             '--output', os.devnull,
             '--write-out', '%{http_code} %{size_download} %{time_total}', url],
            capture_output=True, text=True, timeout=15)
        status, size, elapsed = result.stdout.split()
        seconds = float(elapsed)
        if result.returncode or status != '206' or int(size) != sample or not math.isfinite(seconds) or seconds <= 0:
            return None
        return sample/seconds
    except (OSError, ValueError, subprocess.SubprocessError):
        return None


def order_image_mirrors(urls, path):
    urls = tuple(dict.fromkeys(urls))
    if len(urls) < 2 or path.suffix != '.iso' or path.is_file():
        return urls
    with ThreadPoolExecutor(max_workers=min(len(urls), 4)) as workers:
        speeds = list(workers.map(probe_image_mirror, urls))
    ordered = sorted(zip(urls, speeds), key=lambda item: (
        item[1] is None, -item[1] if item[1] is not None else 0))
    if ordered[0][1] is not None:
        print(f'Linux ISO mirror sample: selected {ordered[0][0]} '
              f'({ordered[0][1]/1024**2:.1f} MiB/s)', flush=True)
    return tuple(url for url, _speed in ordered)


def download_bounded_iso(record, path):
    curl = shutil.which('curl')
    if not curl or (path.is_file() and sha256(path) == record.get('sha256')):
        return download(record, path)
    expected = record.get('sha256', '')
    if not re.fullmatch(r'[0-9a-f]{64}', expected):
        raise ValueError('Download requires an upstream SHA256')
    path.parent.mkdir(parents=True, exist_ok=True)
    partial = path.with_name(path.name + '.partial')
    try:
        result = subprocess.run(
            [curl, '--location', '--fail', '--silent', '--show-error',
             '--proto', '=https', '--proto-redir', '=https', '--max-redirs', '5',
             '--connect-timeout', '15', '--speed-limit', '262144', '--speed-time', '60',
             '--retry', '2', '--retry-all-errors', '--retry-delay', '1',
             '--output', str(partial), '--', record['url']],
            capture_output=True, text=True)
        if result.returncode:
            raise URLError(f'ISO download failed (curl exit {result.returncode})')
        if sha256(partial) != expected:
            raise ValueError('SHA256 mismatch: ' + path.name)
        partial.replace(path)
        print(f'Linux download: verified {path.name} ({path.stat().st_size / 1024**2:.1f} MiB)',
              flush=True)
        return path
    finally:
        partial.unlink(missing_ok=True)


def download_vm_image(image, mirrors, path):
    urls = order_image_mirrors((image['url'], *mirrors), path)
    for url in urls:
        try:
            record = dict(image, url=url)
            fetch = download_bounded_iso if path.suffix == '.iso' and url != urls[-1] else download
            return fetch(record, path), url
        except HTTPError as error:
            if error.code not in (403, 404, 408, 410, 429, 500, 502, 503, 504) or url == urls[-1]:
                raise
            print(f'Linux image unavailable (HTTP {error.code}): {url}. Trying a listed mirror.',
                  flush=True)
        except (URLError, TimeoutError, ConnectionError) as error:
            if url == urls[-1]:
                raise
            print(f'Linux image download failed ({type(error).__name__}): {url}. '
                  'Trying a listed mirror.', flush=True)


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

    with ThreadPoolExecutor(max_workers=8) as workers:
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


def wait_vm_ssh(process, command, timeout=600, ready_command='true', console=None, console_offset=0):
    deadline = time.monotonic() + timeout
    while True:
        if process.poll() is not None:
            raise RuntimeError('Linux VM exited during boot')
        if console is not None:
            with console.open('rb') as stream:
                end = stream.seek(0, os.SEEK_END)
                stream.seek(max(console_offset, end - 64 * 1024))
                tail = stream.read(64 * 1024)
            if (b'You are in emergency mode.' in tail
                    or b'Entering emergency mode. Exit the shell to continue.' in tail):
                raise RuntimeError('Linux VM entered emergency mode before SSH was ready')
            if b'Kernel panic - not syncing:' in tail:
                raise RuntimeError('Linux VM reported a kernel panic before SSH was ready')
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError('Linux VM did not become SSH-ready')
        try:
            ready = subprocess.run([*command, ready_command], stdout=subprocess.DEVNULL,
                                   stderr=subprocess.DEVNULL, timeout=min(15, remaining))
        except subprocess.TimeoutExpired:
            ready = None
        if ready is not None and ready.returncode == 0:
            return
        time.sleep(min(5, max(0, deadline - time.monotonic())))


def kvm_available():
    import fcntl

    try:
        with open('/dev/kvm', 'r+b') as device:
            if fcntl.ioctl(device, KVM_GET_API_VERSION) != KVM_API_VERSION:
                print('KVM: unexpected kernel API version, using TCG.', flush=True)
                return False
            os.close(fcntl.ioctl(device, KVM_CREATE_VM, 0))
    except OSError as error:
        print(f'KVM: {error}, using TCG.', flush=True)
        return False
    return True


class VmGuest:
    def __init__(self, base, directory, label, target, *, kvm, timings):
        self.target = target
        self.directory = Path(directory) / label
        self.directory.mkdir()
        self.process = None
        self.display = None
        self.console = None
        boot_started = time.monotonic()
        self.architecture = 'x86_64' if host_arch() == 'x64' else 'aarch64'
        self.kvm = kvm
        directory = self.directory
        key = directory / 'id_ed25519'
        run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', key], stdout=subprocess.DEVNULL)
        public = key.with_suffix('.pub').read_text().strip()
        self.disk = directory/'disk.qcow2'
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            port = listener.getsockname()[1]
        architecture, kvm = self.architecture, self.kvm
        machine = 'q35' if architecture == 'x86_64' else 'virt'
        if architecture == 'aarch64' and kvm:
            machine += ',gic-version=host'
        cpu = 'host' if kvm else ('max' if architecture == 'x86_64' else 'cortex-a72')
        cpus = 2
        if kvm or (architecture == 'aarch64' and target.full_desktop):
            cpus = min(4, len(os.sched_getaffinity(0)))
        print(f'{target.distribution} {target.release} VM: {label}, {architecture}, '
              f'acceleration={"kvm" if kvm else "tcg"}, vcpus={cpus}', flush=True)
        command = ['qemu-system-'+architecture, '-machine', machine+',accel='+('kvm' if kvm else 'tcg'),
                   '-cpu', cpu, '-smp', str(cpus), '-m', '4096', '-display', 'none',
                   '-serial', 'stdio', '-monitor', 'none']
        drive = f'file={self.disk},format=qcow2,discard=unmap'
        fedora_uki = target.family == 'fedora' and target.writable_arm_uefi
        if target.family == 'pop':
            from pop_live_install import DISK_DEVICE
            command.extend(['-drive', drive+',if=none,id=compatdisk', '-device', DISK_DEVICE])
        else:
            drive += ',if=virtio'
            if fedora_uki:
                drive += ',id=compatdisk'
            command.extend(['-drive', drive])
        if architecture == 'aarch64':
            if target.writable_arm_uefi:
                firmware = Path('/usr/share/AAVMF')
                variables = directory/'aavmf-vars.fd'
                shutil.copyfile(firmware/'AAVMF_VARS.fd', variables)
                command.extend([
                    '-drive', f'if=pflash,format=raw,readonly=on,file={firmware/"AAVMF_CODE.fd"}',
                    '-drive', f'if=pflash,format=raw,file={variables}',
                ])
                if fedora_uki and not kvm:
                    command.extend([
                        '-smbios',
                        'type=11,value=io.systemd.stub.kernel-cmdline-extra=systemd.default_device_timeout_sec=180',
                    ])
            else:
                command.extend(['-bios', '/usr/share/qemu-efi-aarch64/QEMU_EFI.fd'])
            if target.full_desktop:
                command.extend(['-device', 'virtio-gpu-pci'])
        elif target.package_format != 'deb':
            command.extend(['-vga', 'virtio'])
        cosmic = target.full_desktop and (target.family, target.release) == ('pop', '24.04')
        self.monitor_path = directory/'qmp.sock'
        self.ssh = ['ssh', '-i', str(key), '-o', 'StrictHostKeyChecking=no',
                    '-o', 'UserKnownHostsFile='+str(directory/'known_hosts'),
                    '-o', 'ConnectTimeout=5', '-o', 'BatchMode=yes',
                    '-o', 'ControlMaster=auto', '-o', 'ControlPersist=60',
                    '-o', 'ControlPath='+str(directory/'ssh-control'),
                    '-p', str(port), 'ubuntu@127.0.0.1']
        self.environment = ('cd ~/cor-source && GITHUB_ACTIONS=true COR_LINUX_GUEST=true '
                            f'COR_LINUX_VM_ACCELERATION={"kvm" if kvm else "tcg"} '
                            'BUILD_INPUTS_DIR=dependency-snapshot RUNNER_TEMP=/home/ubuntu/cor-runner-temp '
                            'COR_LINUX_APT_CACHE=/home/ubuntu/cor-apt '
                            'PATH=/opt/cor-test-python/bin:$PATH PYTHONDONTWRITEBYTECODE=1 ')
        extended = target.full_desktop and not target.hosted
        self.environment += f'COR_CI_EXTENDED_TESTS={str(extended).lower()} '
        try:
            ready_command = 'true'
            if target.image_format == 'iso':
                from linux_mint_install import FIRMWARE, FIRMWARE_VARS, INSTALL_SSH, SSH_UNIT
                if architecture == 'x86_64':
                    variables = directory/'ovmf-vars.fd'
                    shutil.copyfile(FIRMWARE_VARS, variables)
                    command.extend(['-vga', 'virtio', '-drive', f'if=pflash,format=raw,readonly=on,file={FIRMWARE}',
                                    '-drive', f'if=pflash,format=raw,file={variables}'])
                run(['qemu-img', 'create', '-f', 'qcow2', self.disk, '24G'])
                installer_console = REPORT/'vm-console-installer.log'
                install_started = time.monotonic()
                try:
                    if target.family == 'pop':
                        from pop_live_install import INSTALLED_READY, install
                        install(base, directory, public, command, installer_console, SSH_UNIT, INSTALL_SSH,
                                architecture=architecture)
                    elif target.family in ('linuxmint', 'zorin'):
                        from linux_mint_install import INSTALLED_READY, install
                        options = {'initrd': '/casper/initrd.zstd'} if target.family == 'zorin' else {}
                        install(base, directory, public, command, installer_console, **options)
                    else:
                        raise ValueError(f'No ISO installer for {target.family}')
                except Exception:
                    timings['failed_iso_install'] = round(time.monotonic() - install_started, 3)
                    raise
                boot_started = time.monotonic()
                timings['iso_install'] = round(boot_started - install_started, 3)
                ready_command = INSTALLED_READY
            else:
                seed = self.cloud_disk(base, public)
                command.extend(['-drive', f'file={seed},if=virtio,format=raw,readonly=on'])
            command.extend(['-netdev', f'user,id=net0,hostfwd=tcp:127.0.0.1:{port}-:22',
                            '-device', 'virtio-net-pci,netdev=net0'])
            environment = None
            command.extend(['-qmp', f'unix:{self.monitor_path},server=on,wait=off'])
            if cosmic:
                from linux_vm_monitor import SoftwareDisplay
                self.display = SoftwareDisplay(REPORT/'vm-xvfb.log')
                environment = self.display.environment
                command[command.index('-display')+1] = 'sdl,gl=on'
                if architecture == 'aarch64':
                    command[command.index('virtio-gpu-pci')] = 'virtio-gpu-gl-pci'
                else:
                    vga = command.index('-vga')
                    command[vga:vga+2] = ['-device', 'virtio-vga-gl']
                command.extend(['-device', 'virtio-tablet-pci', '-name', 'cor-cosmic-compatibility'])
            self.console_path = REPORT / ('vm-console-' + label + '.log')
            self.console = self.console_path.open('w')
            self.process = subprocess.Popen(command, stdout=self.console, stderr=subprocess.STDOUT, env=environment)
            try:
                boot_timeout = 900 if target.image_format == 'iso' else 600
                if target.family == 'ubuntu' and target.full_desktop and not kvm:
                    boot_timeout = 1200
                wait_vm_ssh(self.process, self.ssh, ready_command=ready_command, console=self.console_path,
                            timeout=boot_timeout)
                self.wait_cloud_init()
            except (RuntimeError, TimeoutError):
                with contextlib.suppress(OSError):
                    self.capture_desktop_failure('boot-' + label)
                raise
            actual = self.output('uname -m').strip()
            if actual != architecture:
                raise RuntimeError(f'Linux VM returned {actual} instead of {architecture}')
            identity = self.output('. /etc/os-release && printf "%s\\n" "$ID" "$VERSION_ID"').splitlines()
            expected = [target.os_release_id, target.os_release_version]
            if identity != expected:
                raise RuntimeError(f'Linux VM returned {identity!r} instead of {expected!r}')
            self.kernel = self.output('uname -r').strip()
            if target.kernel_series and not self.kernel.startswith(target.kernel_series + '.'):
                raise RuntimeError(f'{target.distribution} {target.release} requires kernel '
                                   f'{target.kernel_series}, booted {self.kernel}')
            self.boot_seconds = round(time.monotonic() - boot_started, 3)
            if fedora_uki and target.full_desktop:
                self.expand_fedora_uki_disk()
        except BaseException:
            self.close()
            raise

    def cloud_disk(self, base, public):
        directory = self.directory
        user_data = {'ssh_genkeytypes': ['ed25519'],
                     'users': [{'name': 'ubuntu',
                               'sudo': 'ALL=(ALL) NOPASSWD:ALL', 'shell': '/bin/bash',
                               'ssh_authorized_keys': [public]}],
                     'ssh_pwauth': False, 'package_update': False, 'package_upgrade': False}
        if self.target.package_format == 'deb':
            user_data['users'][0]['groups'] = 'adm,sudo'
        if self.target.writable_arm_uefi:
            user_data['growpart'] = {'mode': 'off'}
            user_data['resize_rootfs'] = False
        (directory/'user-data').write_text('#cloud-config\n'+json.dumps(user_data))
        (directory/'meta-data').write_text(
            'instance-id: cor-linux-' + uuid.uuid4().hex + '\nlocal-hostname: cor-linux-test\n')
        seed = directory/'seed.img'
        run(['cloud-localds', seed, directory/'user-data', directory/'meta-data'])
        command = ['qemu-img', 'create', '-f', 'qcow2', '-F', 'qcow2', '-b', base, self.disk]
        if not self.target.writable_arm_uefi:
            command.append('24G')
        run(command)
        return seed

    def expand_fedora_uki_disk(self):
        from linux_vm_monitor import Monitor

        root = self.output('findmnt -nr --nofsroot -o SOURCE,FSTYPE --target /').strip()
        if root != '/dev/vda2 btrfs':
            raise RuntimeError(f'Fedora UKI returned an unexpected root layout: {root!r}')
        size = 24 * 1024**3
        monitor = Monitor(self.monitor_path)
        try:
            monitor.execute('block_resize', {'device': 'compatdisk', 'size': size})
        finally:
            monitor.close()
        self.remote('sudo timeout 120 sh -c '
                    + shlex.quote(f'until [ "$(blockdev --getsize64 /dev/vda)" = {size} ]; do sleep 1; done')
                    + ' && sudo growpart /dev/vda 2 && sudo btrfs filesystem resize max /',
                    timeout=240)

    def wait_cloud_init(self):
        if self.target.image_format == 'iso':
            return
        command = 'sudo env LC_ALL=C timeout --signal=TERM --kill-after=30s 300s cloud-init status --wait --long'
        diagnostics = REPORT/(self.directory.name+'-cloud-init-status.txt')
        try:
            result = self.remote(command, capture_output=True, text=True, timeout=360)
        except subprocess.CalledProcessError as error:
            diagnostics.write_text((error.stdout or '') + (error.stderr or ''))
            if error.returncode != 2 or 'status: done' not in (error.stdout or '').splitlines():
                raise
            print('::warning::cloud-init completed with recoverable errors. See '+diagnostics.name, flush=True)
        else:
            diagnostics.write_text(result.stdout + result.stderr)

    def remote(self, script, **kwargs):
        return subprocess.run([*self.ssh, script], check=True, **kwargs)

    def output(self, script, timeout=60):
        return subprocess.check_output([*self.ssh, script], text=True, timeout=timeout)

    def python(self, script, timeout=120):
        return guest_python(self.remote, self.environment, script, timeout)

    def reboot(self, timeout=600):
        previous_boot = self.output('cat /proc/sys/kernel/random/boot_id').strip()
        console_offset = self.console_path.stat().st_size
        self.remote('sudo systemd-run --on-active=2s /usr/bin/systemctl reboot', timeout=15)
        self.close_ssh()
        ready = ('boot_id=$(cat /proc/sys/kernel/random/boot_id) && '
                 'test -n "$boot_id" && test "$boot_id" != ' + shlex.quote(previous_boot))
        fresh_ssh = [self.ssh[0], '-o', 'ControlPath=none', *self.ssh[1:]]
        wait_vm_ssh(self.process, fresh_ssh, ready_command=ready, console=self.console_path,
                    console_offset=console_offset, timeout=timeout)
        self.wait_cloud_init()

    def capture_desktop_failure(self, stage):
        from linux_vm_monitor import Monitor, capture_screen
        desktop = 'cosmic' if self.display else self.target.family
        snapshot = REPORT/f'{desktop}-failure-{stage}.png'
        try:
            monitor = Monitor(self.monitor_path, handshake_timeout=5)
            try:
                method = capture_screen(monitor, snapshot, self.display.environment if self.display else None)
            finally:
                monitor.close()
            result = {'method': method, 'screenshot': snapshot.name}
        except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
            result = {'error': str(error)}
        write_json(snapshot.with_suffix('.json'), result)

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


    def close_ssh(self):
        with contextlib.suppress(OSError, subprocess.SubprocessError):
            subprocess.run([self.ssh[0], '-O', 'exit', *self.ssh[1:]], timeout=10,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def close(self):
        self.close_ssh()
        if self.process and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=10)
        if self.console:
            self.console.close()
        if self.display:
            self.display.close()


def vm_source_archive(directory, include_frozen_apt=True, include_cosmic_probe=False):
    archive = directory / 'source.tar.gz'
    from build_inputs import FILES
    from workflow_artifacts import commit
    paths = {'Cargo.toml', '.github/build-versions.json', 'apps/cli/src/args.rs', *FILES}
    paths.update(path.relative_to(ROOT).as_posix() for path in (ROOT / 'scripts').glob('*.py'))
    if include_cosmic_probe:
        paths.add('scripts/wayland-window-count.c')
    paths.add('installer/arch/PKGBUILD')
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
    elif guest.target.package_format != 'deb':
        guest.python("from linux_native_guest import refresh; refresh()", timeout=900)
    else:
        guest.python('from linux_apt import update_indexes; update_indexes()', timeout=1050)
        if guest.target.family == 'debian' and guest.architecture == 'aarch64' and not guest.kvm:
            print('Debian ARM64 TCG: using compression level 1 for guest initramfs rebuilds', flush=True)
            guest.remote('sudo tee /etc/initramfs-tools/conf.d/zz-cor-ci-compression >/dev/null',
                         input='COMPRESSLEVEL=1\n', text=True, timeout=15)
        if (guest.target.family, guest.target.release) == ('ubuntu', BASELINE):
            guest_deb_downloads(guest.ssh, guest.remote, guest.environment, directory/'guest-debs')


def fetch_compatibility_package(package_run_id, arch, *, rpm=False):
    from workflow_artifacts import commit, fetch_package_inputs, release_module
    module = release_module()
    repo = module.repository()
    client = module.GitHub(repo, module.credential(repo))
    fetch_package_inputs(client, package_run_id, arch, ROOT, commit(), package_run_id, rpm=rpm)


def compatibility_command(target, python, *, prepare_desktop_login=False):
    command = [python, 'scripts/test-release-package.py', '--linux-compat',
               '--expect-linux', target.os_release_id, target.os_release_version, '--expect-arch', target.arch]
    if prepare_desktop_login:
        command.append('--prepare-desktop-login')
    return command


def published_update_command(python, fixture, deadline):
    return ['/usr/bin/timeout', '--signal=TERM', f'--kill-after={PUBLISHED_UPDATE_CLEANUP_SECONDS}',
            str(deadline), python, 'scripts/linux_update_compatibility.py', fixture]


def hosted_test(package_run_id, target, fixture=None):
    started = time.monotonic()
    result = {'distribution': target.family, 'version_id': target.release, 'arch': target.arch,
              'execution': 'hosted', 'kernel': os.uname().release,
              'runner': target.matrix_entry()['os'], 'runner_image': os.environ.get('ImageVersion'),
              'success': False}
    print(f'{target.distribution} {target.release} GitHub-hosted runner: {result["kernel"]}', flush=True)
    try:
        fetch_compatibility_package(package_run_id, target.arch)
        environment = dict(os.environ)
        environment.pop('GH_TOKEN', None)
        environment.pop('GITHUB_TOKEN', None)
        if target.full_desktop:
            environment['COR_CI_EXTENDED_TESTS'] = 'true'
        run(compatibility_command(target, sys.executable), env=environment,
            timeout=3600 if target.full_desktop else 1500)
        if fixture:
            deadline = 1800
            run(published_update_command(sys.executable, fixture, deadline), env=environment,
                timeout=deadline + PUBLISHED_UPDATE_CLEANUP_SECONDS + PUBLISHED_UPDATE_TRANSPORT_SECONDS)
        if target.full_desktop:
            vm_test(package_run_id, target, package_staged=True)
        result['success'] = True
    finally:
        result['seconds'] = round(time.monotonic() - started, 3)
        write_json(REPORT/'linux-runner.json', result)


def container_test(package_run_id, target, image):
    from workflow_artifacts import commit

    require_actions()
    started = time.monotonic()
    result = {'distribution': target.os_release_id, 'version_id': target.release, 'arch': target.arch,
              'image': image, 'execution': 'container', 'kernel': os.uname().release,
              'runner': target.matrix_entry()['os'], 'success': False}
    print(f'{target.distribution} {target.release} userspace on runner kernel {result["kernel"]}: {image}',
          flush=True)
    try:
        fetch_compatibility_package(package_run_id, target.arch, rpm=target.package_format == 'rpm')
        with tempfile.TemporaryDirectory(prefix='cor-linux-container-') as temporary:
            directory = Path(temporary)
            with tarfile.open(vm_source_archive(directory, include_frozen_apt=False)) as source:
                source.extractall(directory, filter='data')
            if (directory/'source-commit.txt').read_text().strip() != commit():
                raise ValueError('Linux container source commit mismatch')
            shutil.copy2(ROOT/'scripts/linux-compatibility-container.sh', directory/'scripts')
            shutil.copytree(ROOT/'dist', directory/'dist')
            (directory/'target/linux-report').mkdir(parents=True)
            shutil.copy2(ROOT/'target/linux-tun-test', directory/'target/linux-tun-test')
            python = directory/'python'
            python.mkdir()
            archive = download(inputs()['architectures'][target.arch]['python'],
                               tools_directory()/'python.tar.gz')
            run(['tar', '-xzf', archive, '-C', python, '--strip-components=1'], timeout=60)
            REPORT.mkdir(parents=True, exist_ok=True)
            if not Path('/dev/net/tun').is_char_device():
                run(['sudo', 'modprobe', 'tun'], timeout=30)
            name = 'cor-linux-compat-' + uuid.uuid4().hex
            architecture = {'x64': 'amd64', 'arm64': 'arm64'}[target.arch]
            command = [
                'docker', 'run', '--rm', '--name', name, '--init', '--privileged',
                '--platform', f'linux/{architecture}', '--network', 'bridge',
                '--mount', f'type=bind,src={directory},dst=/work',
                '--mount', f'type=bind,src={REPORT},dst=/work/target/linux-report',
                '--workdir', '/work', '--entrypoint', '/bin/sh',
                '--env', 'GITHUB_ACTIONS=true', '--env', 'COR_LINUX_GUEST=true',
                '--env', 'COR_LINUX_COMPAT_CONTAINER=true', '--env', 'PYTHONDONTWRITEBYTECODE=1',
                '--env', 'BUILD_INPUTS_DIR=dependency-snapshot',
                image, 'scripts/linux-compatibility-container.sh', str(os.getuid()), str(os.getgid()),
                *compatibility_command(target, '/work/python/bin/python3'),
            ]
            try:
                run(command, timeout=780)
            except subprocess.TimeoutExpired:
                with contextlib.suppress(subprocess.SubprocessError):
                    run(['docker', 'rm', '--force', name], timeout=15)
                raise
        result['success'] = True
    finally:
        result['seconds'] = round(time.monotonic() - started, 3)
        write_json(REPORT/'linux-container.json', result)


def fetch_vm_package(directory, package_run_id, timings, *, rpm=False, fixture=None, package_staged=False):
    started = time.monotonic()
    try:
        if not package_staged:
            fetch_compatibility_package(package_run_id, host_arch(), rpm=rpm)
        with tarfile.open(directory/'package.tar.gz', 'w:gz', compresslevel=1) as tar:
            for relative in ('dist', 'target/linux-tun-test'):
                tar.add(ROOT/relative, arcname=relative)
            if fixture:
                tar.add(fixture, arcname='target/linux-update-fixture')
        return directory / 'package.tar.gz'
    finally:
        timings['package_download_and_archive'] = round(time.monotonic() - started, 3)


def collect_arch_package(guest, directory, ci_run_id):
    from linux_native_package import arch_filename

    source = json.loads((ROOT/'dist/package-receipt.json').read_text())
    name = arch_filename(source['version'])
    work = directory/'linux-arch'
    work.mkdir()
    for filename in ('arch-receipt.json', name):
        path = work/filename
        with path.open('wb') as output:
            guest.remote('cat -- ' + shlex.quote(f'cor-source/dist/linux-arch/{filename}'),
                         stdout=output, timeout=120)
    receipt = json.loads((work/'arch-receipt.json').read_text())
    if receipt != {'schema': 1, 'commit': source['commit'], 'ci_run_id': ci_run_id,
                   'source_sha256': source['sha256'], 'package': name, 'sha256': sha256(work/name)}:
        raise ValueError('Recovered Arch package differs from the installed candidate')
    shutil.copytree(work, ROOT/'dist/linux-arch')


def verify_vm_package(guest, directory, package_run_id, frozen=False, timings=None,
                      package_future=None, fixture=None):
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
    command = shlex.join(compatibility_command(guest.target, '/opt/cor-test-python/bin/python3',
                                               prepare_desktop_login=guest.target.hosted))
    bookworm_tcg = ((guest.target.family, guest.target.release, guest.target.arch, guest.kvm)
                    == ('debian', '12', 'arm64', False))
    slow_guest = guest.architecture == 'aarch64' or not guest.kvm
    started = time.monotonic()
    try:
        print(f'{guest.target.distribution} {guest.target.release}: verifying installed candidate', flush=True)
        candidate_timeout = 3600 if slow_guest else 1200
        try:
            guest.remote(guest.environment + extra + command,
                         timeout=2520 if bookworm_tcg else candidate_timeout)
        except subprocess.SubprocessError:
            with contextlib.suppress(OSError):
                guest.capture_desktop_failure('installed-candidate')
            raise
        finally:
            timings['installed_candidate'] = round(time.monotonic() - started, 3)
        if guest.target.full_desktop:
            cosmic = (guest.target.family, guest.target.release) == ('pop', '24.04')
            debian = guest.target.family == 'debian'
            phases = ('prepare', 'first-login', 'background-login')
            if guest.target.family == 'linuxmint':
                phases += ('prepare-x11', 'x11', 'prepare-wayland', 'wayland')
            elif cosmic:
                phases += ('prepare-wayland', 'wayland')
            for phase in phases:
                print(f'{guest.target.distribution} {guest.target.release}: desktop {phase}', flush=True)
                phase_started = time.monotonic()
                preparing = phase.startswith('prepare')
                timeout = 1800 if slow_guest else 800
                if debian and not preparing:
                    from linux_desktop_login import DEBIAN_LOGIN_BUDGET
                    timeout = DEBIAN_LOGIN_BUDGET + 120
                elif bookworm_tcg and preparing:
                    from linux_desktop_login import BOOKWORM_TCG_DESKTOP_APT_BUDGET
                    timeout = BOOKWORM_TCG_DESKTOP_APT_BUDGET + 120
                try:
                    if not preparing:
                        guest.reboot(timeout=240 if bookworm_tcg else 600)
                    if phase == 'first-login' and cosmic:
                        from linux_cosmic_authorization import verify_dialogs
                        verify_dialogs(guest, REPORT)
                    guest.remote(guest.environment + '/opt/cor-test-python/bin/python3 '
                                 'scripts/linux_desktop_login.py ' + phase, timeout=timeout)
                except Exception:
                    with contextlib.suppress(OSError):
                        guest.capture_desktop_failure(phase)
                    raise
                finally:
                    timings['desktop_' + phase.replace('-', '_')] = round(
                        time.monotonic() - phase_started, 3)
            operation = ('deb_upgrade_and_uninstall' if guest.target.package_format == 'deb'
                         else 'native_upgrade_and_uninstall')
            print(f'{guest.target.distribution} {guest.target.release}: verifying upgrade and uninstall', flush=True)
            phase_started = time.monotonic()
            lifecycle_timeout = 1800 if slow_guest else 900
            try:
                guest.python("from pathlib import Path; import runpy; "
                             f"runpy.run_path('scripts/test-release-package.py')['{operation}']"
                             "(next(Path('dist').glob('*.deb')).resolve())",
                             timeout=420 if bookworm_tcg else lifecycle_timeout)
            except subprocess.SubprocessError:
                with contextlib.suppress(OSError):
                    guest.capture_desktop_failure('package-lifecycle')
                raise
            finally:
                timings['package_lifecycle'] = round(time.monotonic() - phase_started, 3)
        if fixture:
            print(f'{guest.target.distribution} {guest.target.release}: verifying published update', flush=True)
            phase_started = time.monotonic()
            try:
                deadline = (3600 if guest.architecture == 'aarch64' and not guest.kvm
                            else 2400 if slow_guest else 1800)
                command = published_update_command('/opt/cor-test-python/bin/python3',
                                                   'target/linux-update-fixture', deadline)
                guest.remote(guest.environment + shlex.join(command),
                             timeout=deadline + PUBLISHED_UPDATE_CLEANUP_SECONDS
                             + PUBLISHED_UPDATE_TRANSPORT_SECONDS)
            finally:
                timings['published_update'] = round(time.monotonic() - phase_started, 3)
        if guest.target.family == 'arch':
            collect_arch_package(guest, directory, package_run_id)
    finally:
        timings['application_tests'] = round(time.monotonic() - started, 3)


def vm_test(package_run_id, target, fixture=None, *, package_staged=False):
    overall_started = time.monotonic()
    timings = {}
    from build_inputs import manifest
    from ubuntu_vm_environment import PREPARATION_ERRORS, frozen_environments, verify_frozen

    REPORT.mkdir(parents=True, exist_ok=True)
    architecture = target.arch
    linux = inputs()
    value = linux['architectures'][architecture]
    frozen = None
    if target.image_url:
        image = {'url': target.image_url, target.digest_algorithm: target.image_digest}
    else:
        image = value['cloud']
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
        archive = vm_source_archive(
            directory, include_frozen_apt=bool(frozen),
            include_cosmic_probe=target.full_desktop and (target.family, target.release) == ('pop', '24.04'))
        timings['source_archive'] = round(time.monotonic() - phase_started, 3)
        guest = None
        report_image = 'iso' if target.image_format == 'iso' else 'cloud'
        result = {'distribution': target.family, 'version_id': target.release,
                  'arch': architecture, 'execution': 'vm', 'success': False,
                  'runner': target.matrix_entry()['os'], 'runner_image': os.environ.get('ImageVersion'),
                  'source_archive_bytes': archive.stat().st_size,
                  report_image + '_url': image['url'],
                  report_image + '_' + target.digest_algorithm: image[target.digest_algorithm]}
        with ThreadPoolExecutor(max_workers=1) as workers:
            try:
                stage = 'download_inputs'
                package_future = workers.submit(fetch_vm_package, directory, package_run_id, timings,
                                                rpm=target.package_format == 'rpm', fixture=fixture,
                                                package_staged=package_staged)
                phase_started = time.monotonic()
                filename = (f'{target.family}-{target.release}-{architecture}.{target.image_format}'
                            if target.image_url else 'focal.img')
                try:
                    image_path = (directory if target.image_format == 'iso' else tools_directory())/filename
                    base, used_url = download_vm_image(image, target.image_mirrors, image_path)
                    result[report_image + '_url'] = used_url
                finally:
                    timings['image_download_and_verify'] = round(time.monotonic() - phase_started, 3)
                python_started = time.monotonic()
                try:
                    python = download(value['python'], tools_directory()/'python.tar.gz')
                finally:
                    timings['python_download_and_verify'] = round(time.monotonic() - python_started, 3)
                    timings['original_downloads'] = round(time.monotonic() - phase_started, 3)
                stage = 'guest_boot'
                phase_started = time.monotonic()
                kvm = kvm_available()
                result['acceleration'] = 'kvm' if kvm else 'tcg'
                guest = VmGuest(base, directory, 'official-image', target, kvm=kvm, timings=timings)
                timings['final_guest_boot'] = guest.boot_seconds
                stage = 'guest_source_transfer'
                guest.stage(archive, python)
                stage = 'guest_dependencies'
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
                    guest = None
                    frozen = None
                    stage = 'guest_boot'
                    phase_started = time.monotonic()
                    guest = VmGuest(base, directory, 'official-image-fresh-apt', target, kvm=kvm, timings=timings)
                    timings['final_guest_boot'] = guest.boot_seconds
                    stage = 'guest_source_transfer'
                    guest.stage(archive, python)
                    stage = 'guest_dependencies'
                    phase_started = time.monotonic()
                    prepare_vm_guest(guest, directory)
                timings['dependency_preparation'] = round(time.monotonic() - phase_started, 3)
                timings['source_transfer'] = guest.stage_seconds
                stage = 'installed_candidate'
                verify_vm_package(guest, directory, package_run_id, frozen=bool(frozen), timings=timings,
                                  package_future=package_future, fixture=fixture)
                result['success'] = True
            except Exception:
                if 'failed_iso_install' in timings:
                    stage = 'iso_install'
                elif stage == 'guest_boot':
                    timings['failed_guest_boot'] = round(
                        time.monotonic() - phase_started - timings.get('iso_install', 0), 3)
                result['failed_stage'] = stage
                print(f'Linux compatibility failed at {stage}.', flush=True)
                raise
            finally:
                try:
                    if guest:
                        result['kernel'] = guest.kernel
                        try:
                            guest.preserve()
                        finally:
                            guest.close()
                finally:
                    result.update(package_format=target.package_format,
                                  apt_inputs=('frozen' if frozen else 'current')
                                  if target.package_format == 'deb' else None,
                                  seconds=round(time.monotonic() - overall_started, 3),
                                  timings_seconds=timings)
                    write_json(REPORT/'linux-vm.json', result)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('ci', 'package', 'verify', 'cache-keys', 'cache-downloads', 'cache-image'))
    parser.add_argument('--vm', action='store_true', help='Include cloud image downloads')
    parser.add_argument('--package-run-id', type=int, help='Successful CI run that produced the candidate package')
    parser.add_argument('--suite', choices=tuple(TARGETS))
    parser.add_argument('--release')
    parser.add_argument('--expect-arch', choices=('x64', 'arm64'))
    args, extra = parser.parse_known_args()
    verify_arguments = (args.package_run_id, args.suite, args.release, args.expect_arch)
    if args.operation == 'verify':
        if not all(verify_arguments) or args.package_run_id < 1 or extra:
            parser.error('verify requires a positive --package-run-id, --suite, --release and --expect-arch')
    elif any(value is not None for value in verify_arguments):
        parser.error('--package-run-id, --suite, --release and --expect-arch require verify')
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
    elif args.operation == 'verify':
        require_actions()
        if host_arch() != args.expect_arch:
            raise RuntimeError('Linux compatibility runner architecture mismatch')
        target = target_for(args.suite, args.release, args.expect_arch)
        verify = hosted_test if target.hosted else vm_test
        old_run = os.environ.get('COR_LINUX_UPDATE_FROM_CI', '')
        release_tag = os.environ.get('COR_LINUX_UPDATE_RELEASE', '')
        if bool(old_run) != bool(release_tag) or (old_run and not old_run.isdecimal()):
            raise ValueError('Published update testing requires both a previous CI run and release tag')
        if old_run:
            from linux_update_compatibility import prepare
            from workflow_artifacts import release_module
            module = release_module()
            repo = module.repository()
            client = module.GitHub(repo, module.credential(repo))
            fixture = ROOT/'target/linux-update-fixture'
            prepare(client, int(old_run), release_tag, target.package_format, target.arch, fixture)
            verify(args.package_run_id, target, fixture=fixture)
        elif image := target.basic_container_image():
            container_test(args.package_run_id, target, image)
        else:
            verify(args.package_run_id, target)
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
