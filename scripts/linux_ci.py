"""Ubuntu 20.04 build environment and native-kernel package verification.

The host owns Actions credentials/caches. Only the source, Rust directories and
explicit build inputs enter the disposable container; guests receive no tokens.
"""
import argparse
import contextlib
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

from build_support import ROOT, host_arch, opener, read_json, run, sha256, write_json

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


def resolve_inputs():
    """Resolve metadata only; every Linux job consumes the frozen URLs/digests."""
    token = read_json('https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/ubuntu:pull')['token']
    headers = {'Authorization': 'Bearer '+token,
               'Accept': 'application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json'}
    request = urllib.request.Request(
        'https://registry-1.docker.io/v2/library/ubuntu/manifests/20.04', headers=headers)
    with opener().open(request, timeout=60) as response:
        document = response.read()
        digest = response.headers.get('Docker-Content-Digest', '')
    if (not re.fullmatch(r'sha256:[0-9a-f]{64}', digest)
            or hashlib.sha256(document).hexdigest() != digest.removeprefix('sha256:')):
        raise ValueError('Ubuntu registry manifest digest is missing or incorrect')
    release = read_json('https://api.github.com/repos/'+PYTHON_REPO+'/releases/latest')
    assets = release['assets']
    dates = re.findall(r'href="(\d{8}(?:\.\d+)?)/"', text(CLOUD_ROOT))
    if not dates:
        raise ValueError('No dated Ubuntu 20.04 cloud image found')
    cloud = CLOUD_ROOT+max(dates, key=lambda value: tuple(map(int, value.split('.'))))+'/'
    sums = text(cloud+'SHA256SUMS')
    result = {'version': BASELINE, 'image': 'ubuntu@'+digest, 'architectures': {}}
    for arch, triple, deb in [('x64', 'x86_64', 'amd64'), ('arm64', 'aarch64', 'arm64')]:
        pattern = rf'cpython-3\.11\.[0-9]+\+[^/]+-{triple}-unknown-linux-gnu-install_only\.tar\.gz'
        selected = [asset for asset in assets if re.fullmatch(pattern, asset['name'])]
        if len(selected) != 1:
            raise ValueError('Expected one portable Python 3.11 archive for '+arch)
        asset = selected[0]
        checksum = (asset.get('digest') or '').removeprefix('sha256:')
        if not re.fullmatch('[0-9a-f]{64}', checksum):
            checksum = text(asset['browser_download_url']+'.sha256').split()[0]
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


def download(record, path):
    if not re.fullmatch('[0-9a-f]{64}', record.get('sha256', '')):
        raise ValueError('Download requires an upstream SHA256')
    if path.is_file() and sha256(path) == record['sha256']:
        return path
    path.parent.mkdir(parents=True, exist_ok=True)
    partial = path.with_suffix(path.suffix+'.partial')
    try:
        with opener().open(record['url'], timeout=120) as response, partial.open('wb') as output:
            shutil.copyfileobj(response, output, 1024*1024)
        if sha256(partial) != record['sha256']:
            raise ValueError('Upstream download checksum mismatch: '+path.name)
        partial.replace(path)
    finally:
        partial.unlink(missing_ok=True)
    return path


def require_actions():
    if os.environ.get('GITHUB_ACTIONS') != 'true' or sys.platform != 'linux':
        raise RuntimeError('Linux integration requires a disposable Actions Linux runner')


def build_image():
    require_actions()
    value = inputs()
    archive = download(value['architectures'][host_arch()]['python'],
                       ROOT/'target/linux-tools/python.tar.gz')
    dockerfile = ROOT/'resources/linux/ubuntu20.Dockerfile'
    identity = hashlib.sha256((value['image']+sha256(archive)+sha256(dockerfile)).encode()).hexdigest()[:20]
    tag = 'cor-ubuntu20:'+identity
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
                'GITHUB_REPOSITORY', 'GITHUB_RUN_ID', 'GITHUB_RUN_ATTEMPT'):
        if key in os.environ:
            # Runner summary files live outside the source mount.
            if key in ('GITHUB_OUTPUT', 'GITHUB_STEP_SUMMARY'):
                path = Path(os.environ[key])
                arguments.extend(['--volume', f'{path.parent}:{path.parent}'])
            arguments.extend(['--env', key])
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


def vm_test():
    require_actions()
    REPORT.mkdir(parents=True, exist_ok=True)
    value = inputs()['architectures'][host_arch()]
    base = download(value['cloud'], ROOT/'target/linux-tools/focal.img')
    python = download(value['python'], ROOT/'target/linux-tools/python.tar.gz')
    with tempfile.TemporaryDirectory(prefix='cor-focal-vm-') as temporary:
        directory = Path(temporary)
        key = directory/'id_ed25519'
        run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', key])
        public = key.with_suffix('.pub').read_text().strip()
        # This password exists only in the disposable guest and its GUI agent test.
        user_data = {'users': [{'name': 'ubuntu', 'groups': 'adm,sudo',
                               'sudo': 'ALL=(ALL) NOPASSWD:ALL', 'shell': '/bin/bash',
                               'lock_passwd': False, 'ssh_authorized_keys': [public]}],
                     'ssh_pwauth': False, 'chpasswd': {'list': 'ubuntu:cor-ci-only', 'expire': False}}
        (directory/'user-data').write_text('#cloud-config\n'+json.dumps(user_data))
        (directory/'meta-data').write_text('instance-id: cor-focal-test\nlocal-hostname: cor-focal-test\n')
        seed = directory/'seed.img'
        disk = directory/'disk.qcow2'
        run(['cloud-localds', seed, directory/'user-data', directory/'meta-data'])
        run(['qemu-img', 'create', '-f', 'qcow2', '-F', 'qcow2', '-b', base, disk, '24G'])
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            port = listener.getsockname()[1]
        kvm = os.access('/dev/kvm', os.R_OK | os.W_OK)
        architecture = 'x86_64' if host_arch() == 'x64' else 'aarch64'
        print(f'Ubuntu {BASELINE} VM: {architecture}, acceleration={"kvm" if kvm else "tcg"}',
              flush=True)
        machine = 'q35' if architecture == 'x86_64' else 'virt'
        cpu = 'host' if kvm else ('max' if architecture == 'x86_64' else 'cortex-a72')
        command = ['qemu-system-'+architecture, '-machine', machine+',accel='+('kvm' if kvm else 'tcg'),
                   '-cpu', cpu, '-smp', '2', '-m', '4096', '-display', 'none',
                   '-serial', 'stdio', '-monitor', 'none',
                   '-drive', f'file={disk},if=virtio,format=qcow2',
                   '-drive', f'file={seed},if=virtio,format=raw,readonly=on',
                   '-netdev', f'user,id=net0,hostfwd=tcp:127.0.0.1:{port}-:22',
                   '-device', 'virtio-net-pci,netdev=net0']
        if architecture == 'aarch64':
            command.extend(['-bios', '/usr/share/qemu-efi-aarch64/QEMU_EFI.fd'])
        ssh = ['ssh', '-i', str(key), '-p', str(port), '-o', 'StrictHostKeyChecking=no',
               '-o', 'UserKnownHostsFile='+str(directory/'known_hosts'),
               '-o', 'ConnectTimeout=5', '-o', 'BatchMode=yes', 'ubuntu@127.0.0.1']
        archive = directory/'source.tar.gz'
        tracked = subprocess.check_output(['git', 'ls-files', '-z'], cwd=ROOT).decode().split('\0')
        with tarfile.open(archive, 'w:gz', compresslevel=1) as tar:
            for relative in tracked:
                if relative:
                    tar.add(ROOT/relative, arcname=relative, recursive=False)
            for relative in ('.git', 'dist', 'target/linux-tun-test', 'dependency-snapshot'):
                path = ROOT/relative
                if path.exists():
                    tar.add(path, arcname=relative)
        with (REPORT/'focal-console.log').open('w') as console:
            guest = subprocess.Popen(command, stdout=console, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic()+600
                while True:
                    if guest.poll() is not None:
                        raise RuntimeError('Ubuntu 20.04 VM exited during boot')
                    ready = subprocess.run([*ssh, 'true'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    if ready.returncode == 0:
                        break
                    if time.monotonic() >= deadline:
                        raise TimeoutError('Ubuntu 20.04 VM did not become SSH-ready')
                    time.sleep(5)
                remote = lambda script, **kwargs: subprocess.run([*ssh, script], check=True, **kwargs)
                remote('sudo timeout --signal=TERM --kill-after=30s 300s '
                       'cloud-init status --wait', timeout=360)
                kernel = subprocess.check_output([*ssh, 'uname -r'], text=True).strip()
                if not kernel.startswith('5.4.'):
                    raise RuntimeError('Minimum-kernel test requires Ubuntu 20.04 GA 5.4; got '+kernel)
                remote('mkdir -p ~/cor-source && tar xzf - -C ~/cor-source',
                       stdin=archive.open('rb'), timeout=300)
                remote('sudo mkdir -p /opt/cor-test-python && sudo tar xzf - -C /opt/cor-test-python --strip-components=1',
                       stdin=python.open('rb'), timeout=120)
                remote('sudo timeout --signal=TERM --kill-after=30s 300s apt-get update',
                       timeout=360)
                remote('sudo env DEBIAN_FRONTEND=noninteractive timeout '
                       '--signal=TERM --kill-after=30s 300s apt-get install -y git',
                       timeout=360)
                remote('cd ~/cor-source && GITHUB_ACTIONS=true COR_LINUX_GUEST=true '
                       'PATH=/opt/cor-test-python/bin:$PATH PYTHONDONTWRITEBYTECODE=1 '
                       '/opt/cor-test-python/bin/python3 scripts/test-release-package.py '
                       '--linux-compat --expect-ubuntu 20.04',
                       timeout=2400 if architecture == 'aarch64' or not kvm else 1200)
                write_json(REPORT/'focal-vm.json', {'distribution': BASELINE, 'kernel': kernel,
                           'acceleration': 'kvm' if kvm else 'tcg', 'arch': host_arch(),
                           'cloud_sha256': value['cloud']['sha256'], 'success': True})
            finally:
                with contextlib.suppress(subprocess.SubprocessError):
                    subprocess.run([*ssh, 'cd ~/cor-source && tar czf - target/linux-report'],
                                   stdout=(REPORT/'focal-guest-report.tar.gz').open('wb'), timeout=60)
                guest.terminate()
                try:
                    guest.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    guest.kill()
                    guest.wait(timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('ci', 'package', 'vm'))
    args, extra = parser.parse_known_args()
    if args.operation == 'vm':
        if extra:
            parser.error('vm takes no additional arguments')
        vm_test()
    elif args.operation == 'ci':
        container(['python3', 'scripts/run-ci.py', '--system', 'linux', '--arch', host_arch()])
    else:
        container(['python3', 'scripts/test-release-package.py', '--system', 'linux',
                   '--arch', host_arch(), *extra])


if __name__ == '__main__':
    main()
