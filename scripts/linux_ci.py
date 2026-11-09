"""Ubuntu 20.04 build environment and native-kernel package verification.

The host owns Actions credentials/caches. Only the source, Rust directories and
explicit build inputs enter the disposable container; guests receive no tokens.
"""
import argparse
import contextlib
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import socket
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request
import uuid

from build_support import ROOT, download as download_original, host_arch, opener, read_json, run, sha256, write_json
from linux_apt import OPTIONS as APT_OPTIONS

BASELINE = '20.04'
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
    if not re.fullmatch('[0-9a-f]{64}', record.get('sha256', '')):
        raise ValueError('Download requires an upstream SHA256')
    download_original(record['url'], path, digest=record['sha256'])
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
    from linux_apt import matches, source_url
    from upstream_cache import valid_source
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
    print(f'Guest APT inputs: staged {len(archives)} verified originals through host official/private fallback',
          flush=True)


def guest_receipts(ssh):
    from upstream_cache import directory, valid_source
    script = ("import json,pathlib; root=pathlib.Path('/home/ubuntu/cor-runner-temp/cor-upstream-downloads'); "
              "files=sorted(root.glob('*.json')); "
              "assert len(files)<=4096; "
              "print(json.dumps([json.loads(path.read_text()) for path in files]))")
    try:
        payload = subprocess.check_output(
            [*ssh, '/opt/cor-test-python/bin/python3 -c '+shlex.quote(script)], timeout=60)
        if len(payload) > 1024*1024:
            raise ValueError('Guest original receipt exceeds 1 MiB')
        records = json.loads(payload)
        if not isinstance(records, list) or any(
                not isinstance(record, dict) or set(record) != {'url', 'size', 'sha256'}
                or not valid_source(record['url'], record['sha256'], record['size'])
                for record in records):
            raise ValueError('Guest original receipt has invalid metadata')
        write_json(REPORT/'focal-upstream-receipts.json', records)
        for record in records:
            identity = hashlib.sha256((record['url']+'\n'+record['sha256']).encode()).hexdigest()
            write_json(directory()/(identity+'.json'), record)
        print(f'Guest DEB inventory: imported {len(records)} verified original receipts; no archives copied',
              flush=True)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print('::warning::Guest original download receipts unavailable: '
              + type(error).__name__, flush=True)


def build_image():
    require_actions()
    value = inputs()
    archive = download(value['architectures'][host_arch()]['python'],
                       tools_directory()/'python.tar.gz')
    dockerfile = ROOT/'resources/linux/ubuntu20.Dockerfile'
    identity = image_identity(value, host_arch())
    tag = 'cor-ubuntu20:' + identity[:20]
    inspect = ['docker', 'image', 'inspect', '--format={{.Id}}', tag]
    present = subprocess.run(inspect, capture_output=True, text=True)
    if present.returncode == 0:
        return tag
    with tempfile.TemporaryDirectory(prefix='cor-build-image-') as temporary:
        directory = Path(temporary)
        shutil.copy2(dockerfile, directory/'Dockerfile')
        shutil.copy2(archive, directory/'python.tar.gz')
        run(['docker', 'build', '--build-arg', 'BASE_IMAGE='+value['image'],
             '--build-arg', 'USER_ID='+str(os.getuid()), '--build-arg', 'GROUP_ID='+str(os.getgid()),
             '--tag', tag, directory], timeout=900)
    return tag


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
                'GITHUB_REPOSITORY', 'GITHUB_RUN_ID', 'GITHUB_RUN_ATTEMPT', 'RUST_BACKTRACE'):
        if key in os.environ:
            # Runner summary files live outside the source mount.
            if key in ('GITHUB_OUTPUT', 'GITHUB_STEP_SUMMARY'):
                path = Path(os.environ[key])
                arguments.extend(['--volume', f'{path.parent}:{path.parent}'])
            arguments.extend(['--env', key])
    if cache := os.environ.get('COR_LINUX_APT_CACHE'):
        Path(cache).mkdir(parents=True, exist_ok=True)
        arguments.extend(['--volume', cache + ':' + cache, '--env', 'COR_LINUX_APT_CACHE=' + cache])
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
            raise RuntimeError('Ubuntu 20.04 VM exited during boot')
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError('Ubuntu 20.04 VM did not become SSH-ready')
        try:
            ready = subprocess.run([*command, 'true'], stdout=subprocess.DEVNULL,
                                   stderr=subprocess.DEVNULL, timeout=min(15, remaining))
        except subprocess.TimeoutExpired:
            ready = None
        if ready is not None and ready.returncode == 0:
            return
        time.sleep(min(5, max(0, deadline - time.monotonic())))


class VmGuest:
    def __init__(self, base, directory, label):
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
                               'lock_passwd': False, 'ssh_authorized_keys': [public]}],
                     'ssh_pwauth': False, 'package_update': False, 'package_upgrade': False,
                     'chpasswd': {'list': 'ubuntu:cor-ci-only', 'expire': False}}
        (directory/'user-data').write_text('#cloud-config\n'+json.dumps(user_data))
        (directory/'meta-data').write_text(
            'instance-id: cor-focal-' + uuid.uuid4().hex + '\nlocal-hostname: cor-focal-test\n')
        seed = directory/'seed.img'
        self.disk = directory/'disk.qcow2'
        run(['cloud-localds', seed, directory/'user-data', directory/'meta-data'])
        run(['qemu-img', 'create', '-f', 'qcow2', '-F', 'qcow2', '-b', base, self.disk, '24G'])
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            port = listener.getsockname()[1]
        architecture, kvm = self.architecture, self.kvm
        print(f'Ubuntu {BASELINE} VM: {label}, {architecture}, acceleration={"kvm" if kvm else "tcg"}',
              flush=True)
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
        try:
            self.console = (REPORT / ('focal-console-' + label + '.log')).open('w')
            self.process = subprocess.Popen(command, stdout=self.console, stderr=subprocess.STDOUT)
            wait_vm_ssh(self.process, self.ssh)
            self.remote('sudo timeout --signal=TERM --kill-after=30s 300s cloud-init status --wait',
                        timeout=360)
            self.kernel = self.output('uname -r').strip()
            if not self.kernel.startswith('5.4.'):
                raise RuntimeError('Minimum-kernel test requires Ubuntu 20.04 GA 5.4')
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

    def stage(self, archive, python=None):
        with archive.open('rb') as stream:
            self.remote('mkdir -p ~/cor-source ~/cor-apt && tar xzf - -C ~/cor-source',
                        stdin=stream, timeout=300)
        if python:
            with python.open('rb') as stream:
                self.remote('sudo mkdir -p /opt/cor-test-python && sudo tar xzf - '
                            '-C /opt/cor-test-python --strip-components=1', stdin=stream, timeout=120)

    def inspect(self):
        script = ('import json,prepared_vm_environment; '
                  'print(json.dumps(prepared_vm_environment.inspect_environment()))')
        return json.loads(self.output(self.environment + '/opt/cor-test-python/bin/python3 -c '
                                      + shlex.quote("import sys; sys.path.insert(0,'scripts'); " + script)))

    def preserve(self):
        with contextlib.suppress(OSError, subprocess.SubprocessError):
            with (REPORT/'focal-guest-report.tar.gz').open('wb') as output:
                subprocess.run([*self.ssh, 'cd ~/cor-source && tar czf - target/linux-report'],
                               stdout=output, timeout=60)
        guest_receipts(self.ssh)

    def shutdown(self):
        with contextlib.suppress(subprocess.CalledProcessError):
            self.remote('sudo shutdown -h now', timeout=30)
        if self.process.wait(timeout=180) != 0:
            raise RuntimeError('Template guest did not shut down cleanly')
        self.close()

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


def vm_source_archive(directory):
    archive = directory / 'source.tar.gz'
    tracked = subprocess.check_output(['git', 'ls-files', '-z'], cwd=ROOT).decode().split('\0')
    with tarfile.open(archive, 'w:gz', compresslevel=1) as tar:
        for relative in tracked:
            if relative:
                tar.add(ROOT / relative, arcname=relative, recursive=False)
        for relative in ('.git', 'dist', 'target/linux-tun-test', 'dependency-snapshot'):
            path = ROOT / relative
            if path.exists():
                tar.add(path, arcname=relative)
    return archive


def verify_vm_image(path, virtual_size):
    import prepared_vm as protocol
    protocol.qcow2_header(path, virtual_size)
    information = json.loads(subprocess.check_output(
        ['qemu-img', 'info', '--output=json', '-f', 'qcow2', path], timeout=60))
    specific = information.get('format-specific', {}).get('data', {})
    protocol.require(information.get('format') == 'qcow2'
                     and information.get('virtual-size') == virtual_size
                     and not information.get('backing-filename')
                     and not information.get('full-backing-filename')
                     and not information.get('encrypted') and not information.get('snapshots')
                     and not information.get('dirty-flag')
                     and not specific.get('data-file') and not specific.get('corrupt')
                     and not specific.get('dirty-flag'), 'unsupported_image')
    result = json.loads(subprocess.check_output(
        ['qemu-img', 'check', '--output=json', '-f', 'qcow2', path], timeout=180))
    protocol.require(type(result.get('check-errors')) is int and result['check-errors'] == 0
                     and result.get('format') == 'qcow2'
                     and all(result.get(name, 0) == 0 for name in ('corruptions', 'leaks')),
                     'unsupported_image')


def prepare_vm_guest(guest, directory, frozen=None):
    if frozen:
        guest_deb_downloads(guest.ssh, guest.remote, guest.environment, directory/'guest-debs',
                            frozen['lock']['downloads'])
        guest.python("from build_inputs import manifest; "
                     "from prepared_vm_environment import install_frozen, frozen_environments; "
                     "from build_support import host_arch; "
                     "value=manifest(); install_frozen('dependency-snapshot',"
                     "frozen_environments(value)[host_arch()],"
                     "value['upstream']['linux'])", timeout=2100)
    else:
        guest.remote('sudo timeout --signal=TERM --kill-after=30s 300s '
                     + shlex.join([*APT_OPTIONS, 'update']), timeout=360)
        guest_deb_downloads(guest.ssh, guest.remote, guest.environment, directory/'guest-debs')
        guest.python("import runpy; runpy.run_path('scripts/test-release-package.py')"
                     "['prepare_compatibility_environment']()", timeout=2100)
    guest_receipts(guest.ssh)
    try:
        run([sys.executable, ROOT/'scripts/upstream_cache.py', 'submit',
             '--budget-seconds', '20', '--poll-seconds', '0', '--no-summary'], timeout=30)
    except (OSError, subprocess.SubprocessError) as error:
        print('::warning::Early guest download manifest submission unavailable: '
              + type(error).__name__, flush=True)


def verify_vm_package(guest, directory, package_run_id, prepared=False, timings=None):
    timings = timings if timings is not None else {}
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
            with (directory/'package.tar.gz').open('rb') as stream:
                guest.remote('tar xzf - -C ~/cor-source', stdin=stream, timeout=300)
    finally:
        timings['package_wait_and_transfer'] = round(time.monotonic() - started, 3)
    extra = 'COR_PREPARED_VM_VERIFIED=true ' if prepared else ''
    started = time.monotonic()
    try:
        guest.remote(guest.environment + extra + '/opt/cor-test-python/bin/python3 '
                     'scripts/test-release-package.py --linux-compat --expect-ubuntu 20.04',
                     timeout=2400 if guest.architecture == 'aarch64' or not guest.kvm else 1200)
    finally:
        timings['application_tests'] = round(time.monotonic() - started, 3)


def vm_test(package_run_id=None):
    overall_started = time.monotonic()
    timings = {}
    require_actions()
    import prepared_vm as protocol
    import prepared_vm_environment as environment
    from build_inputs import manifest
    REPORT.mkdir(parents=True, exist_ok=True)
    linux = inputs()
    value = linux['architectures'][host_arch()]
    client = protocol.client(REPORT / 'prepared-vm.json')
    snapshot = manifest() or {}
    frozen = environment.frozen_environments(snapshot).get(host_arch())
    enabled = False
    if frozen:
        try:
            enabled = environment.verify_frozen(
                Path(os.environ.get('BUILD_INPUTS_DIR', 'dependency-snapshot')), frozen, linux)
            enabled = enabled and client.available(host_arch())
        except protocol.ERRORS as error:
            client.event('inputs', 'invalid_frozen_inputs', time.monotonic(),
                         error_code=getattr(error, 'code', type(error).__name__))
    temporary_root = Path(os.environ.get('RUNNER_TEMP', tempfile.gettempdir())).resolve()
    protocol.require(not temporary_root.is_relative_to(ROOT), 'unsafe_image_directory')
    with tempfile.TemporaryDirectory(prefix='cor-focal-vm-', dir=temporary_root) as temporary:
        directory = Path(temporary)
        archive = vm_source_archive(directory)
        candidate = directory / 'prepared.qcow2'
        phase_started = time.monotonic()
        saved = client.restore(frozen['descriptor'], candidate, verify_vm_image) if enabled else None
        timings['private_lookup_download_verify'] = round(time.monotonic() - phase_started, 3)
        guest = None
        upload = None
        result = {'distribution': BASELINE, 'arch': host_arch(), 'success': False}
        cache_path = 'cold'
        workers = ThreadPoolExecutor(max_workers=1)
        started = time.monotonic()
        try:
            if saved:
                try:
                    protocol.require(saved['image']['virtual_size'] == 24 * 1024**3,
                                     'environment_mismatch')
                    environment.check_sources(saved['environment'], frozen)
                    guest = VmGuest(candidate, directory, 'warm')
                    guest.stage(archive)
                    environment.check_environment(guest.inspect(), saved['environment'], frozen['lock'])
                    client.event('boot', 'warm_verified', started)
                    cache_path = 'warm'
                except protocol.ERRORS as error:
                    if guest:
                        guest.close()
                    guest = None
                    enabled = False
                    cache_path = 'cold_after_restore_failure'
                    client.event('boot', 'cold_fallback', started,
                                 error_code=getattr(error, 'code', type(error).__name__))
            if guest is None:
                phase_started = time.monotonic()
                base = download(value['cloud'], tools_directory()/'focal.img')
                python = download(value['python'], tools_directory()/'python.tar.gz')
                timings['original_downloads'] = round(time.monotonic() - phase_started, 3)
                guest = VmGuest(base, directory, 'cold')
                guest.stage(archive, python)
                try:
                    phase_started = time.monotonic()
                    prepare_vm_guest(guest, directory, frozen if enabled else None)
                    timings['dependency_preparation'] = round(time.monotonic() - phase_started, 3)
                    if enabled and client.available(host_arch()):
                        phase_started = time.monotonic()
                        actual = guest.inspect()
                        environment.check_environment(actual, None, frozen['lock'])
                        sources = {record['sha256']: record for record in frozen['lock']['downloads'].values()}
                        for name, path in (('cloud', base), ('python', python)):
                            sources[value[name]['sha256']] = {**value[name], 'size': path.stat().st_size}
                        actual['sources'] = [sources[checksum] for checksum in sorted(sources)]
                        guest.python('from prepared_vm_environment import clean_template; clean_template()')
                        guest.shutdown()
                        run(['qemu-img', 'convert', '-f', 'qcow2', '-O', 'qcow2', '-c',
                             '-o', 'compat=1.1', guest.disk, candidate], timeout=600)
                        run(['qemu-img', 'compare', '-f', 'qcow2', '-F', 'qcow2',
                             guest.disk, candidate], timeout=300)
                        verify_vm_image(candidate, 24 * 1024**3)
                        candidate.chmod(0o400)
                        record = {'schema': 1, 'group': protocol.GROUP,
                                  'env_key': protocol.identity(frozen['descriptor']),
                                  'descriptor': frozen['descriptor'],
                                  'image': {'format': 'qcow2', 'size': candidate.stat().st_size,
                                            'virtual_size': 24 * 1024**3, 'sha256': sha256(candidate)},
                                  'environment': actual, 'producer': protocol.producer()}
                        protocol.validate(record)
                        write_json(REPORT / 'prepared-vm-manifest.json', record)
                        guest = VmGuest(candidate, directory, 'template-check')
                        guest.stage(archive)
                        environment.check_environment(guest.inspect(), actual, frozen['lock'])
                        client.event('template', 'second_boot_verified', started)
                        cache_path = 'cold_template'
                        timings['template_and_second_boot'] = round(time.monotonic() - phase_started, 3)
                        if client.available(host_arch()):
                            upload = workers.submit(client.upload, record, candidate)
                except protocol.ERRORS as error:
                    if not enabled:
                        raise
                    client.event('template', 'cold_fallback', started,
                                 error_code=getattr(error, 'code', type(error).__name__))
                    guest.close()
                    guest = VmGuest(base, directory, 'cold-fallback')
                    guest.stage(archive, python)
                    phase_started = time.monotonic()
                    prepare_vm_guest(guest, directory)
                    timings['fallback_dependency_preparation'] = round(time.monotonic() - phase_started, 3)
                    enabled = False
            timings['final_guest_boot'] = guest.boot_seconds
            verify_vm_package(guest, directory, package_run_id, prepared=enabled, timings=timings)
            result.update(kernel=guest.kernel, acceleration='kvm' if guest.kvm else 'tcg',
                          cloud_sha256=value['cloud']['sha256'], success=True)
        finally:
            try:
                if guest:
                    try:
                        guest.preserve()
                    finally:
                        guest.close()
            finally:
                phase_started = time.monotonic()
                try:
                    workers.shutdown(wait=True)
                    if upload is not None:
                        result['template_saved'] = upload.result() is not None
                finally:
                    timings['upload_tail_wait'] = round(time.monotonic() - phase_started, 3)
                    result.update(prepared=enabled, cache_path=cache_path,
                                  seconds=round(time.monotonic() - overall_started, 3),
                                  timings_seconds=timings)
                    write_json(REPORT/'focal-vm.json', result)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('ci', 'package', 'vm', 'cache-keys', 'cache-downloads', 'cache-image'))
    parser.add_argument('--vm', action='store_true', help='Include cloud image downloads')
    parser.add_argument('--package-run-id', type=int, help='Prepare the VM before waiting for this run’s Linux package')
    args, extra = parser.parse_known_args()
    if args.package_run_id is not None and (args.operation != 'vm' or args.package_run_id <= 0):
        parser.error('--package-run-id requires vm and a positive Actions run id')
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
    elif args.operation == 'vm':
        if extra:
            parser.error('vm takes no additional arguments')
        vm_test(args.package_run_id)
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
