"""Build and check the optional Debian ARM64 guest dependency image."""
from datetime import datetime, timezone
import hashlib
import json
import re
import runpy
import subprocess
import time

from build_support import ROOT, run, sha256

MAX_BYTES = 2 * 1024**3
IMAGE = 'debian-12-arm64.qcow2'
CHECKSUM = IMAGE + '.sha256'
RECIPE = ('linux_apt.py', 'linux_vm_cache.py')


def supported(target):
    return (target.family, target.release, target.arch) == ('debian', '12', 'arm64')


def key(target):
    if not supported(target):
        raise ValueError('This target has no prepared guest cache')
    from linux_ci import inputs
    from linux_desktop_login import GNOME, login_dependencies

    python = inputs()['architectures'][target.arch]['python']['sha256']
    release = {'ID': target.os_release_id, 'VERSION_ID': target.os_release_version}
    package = runpy.run_path(str(ROOT/'scripts/test-release-package.py'))
    recipe = [target.image_digest, python, package['GUEST_PACKAGE_CONFIG'],
              package['compatibility_dependencies'](release=release, extended=True),
              login_dependencies(GNOME, release), *(sha256(ROOT/'scripts'/name) for name in RECIPE)]
    identity = hashlib.sha256(json.dumps(recipe).encode('utf-8')).hexdigest()
    week = datetime.now(timezone.utc).strftime('%G-%V')
    return f'linux-prepared-v2-debian-12-arm64-{week}-{identity}'


def check_image(path):
    if not path.is_file() or path.is_symlink() or not 0 < path.stat().st_size <= MAX_BYTES:
        raise ValueError('Prepared guest image is missing or outside its storage budget')
    info = json.loads(subprocess.check_output(
        ['qemu-img', 'info', '-f', 'qcow2', '--output=json', path], timeout=30))
    details = info.get('format-specific', {}).get('data', {})
    if (info.get('format') != 'qcow2' or info.get('virtual-size') != 24 * 1024**3
            or any(name.endswith('backing-filename') for name in info)
            or details.get('data-file') or details.get('corrupt') or info.get('snapshots')):
        raise ValueError('Prepared guest must be a standalone clean QCOW2 disk')
    return path


def restore(directory, timings):
    if not directory.is_dir():
        return None
    started = time.monotonic()
    try:
        checksum = directory/CHECKSUM
        if checksum.is_symlink() or checksum.stat().st_size != 65:
            raise ValueError('Invalid prepared guest checksum')
        expected = checksum.read_text().strip()
        image = check_image(directory/IMAGE)
        if not re.fullmatch(r'[0-9a-f]{64}', expected) or sha256(image) != expected:
            raise ValueError('Prepared guest checksum does not match the disk')
        return image
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f'::warning::Prepared guest cache could not be read: {error}', flush=True)
        return None
    finally:
        timings['prepared_disk_verify'] = round(time.monotonic() - started, 3)


def export(guest, directory):
    guest.remote('sudo systemctl set-default multi-user.target', timeout=15)
    guest.python(
        "import platform, runpy; "
        "from linux_desktop_login import GNOME, install_login_dependencies; "
        "runpy.run_path('scripts/test-release-package.py')['prepare_compatibility_environment'](); "
        "install_login_dependencies(GNOME, platform.freedesktop_os_release())",
        timeout=1200,
    )
    guest.remote(
        'sudo systemctl stop gdm.service && '
        'sudo rm -rf -- /home/ubuntu/cor-source /home/ubuntu/cor-apt /opt/cor-test-python && '
        'sudo rm -f -- /home/ubuntu/.ssh/authorized_keys && '
        'sudo shred -u -- /etc/ssh/ssh_host_* && '
        'sudo rm -f -- /etc/netplan/50-cloud-init.yaml && '
        'sudo apt-get clean && sudo cloud-init clean --logs --machine-id --seed && sudo fstrim / && '
        'sudo systemd-run --on-active=2s /usr/bin/systemctl poweroff',
        timeout=120,
    )
    guest.close_ssh()
    if guest.process.wait(timeout=180):
        raise RuntimeError('Prepared guest did not shut down cleanly')
    candidate = guest.directory/'prepared.qcow2'
    run(['qemu-img', 'convert', '-f', 'qcow2', '-O', 'qcow2',
         '-c', '-o', 'compression_type=zstd', guest.disk, candidate], timeout=240)
    check_image(candidate)
    digest = sha256(candidate)
    directory.mkdir(parents=True, exist_ok=True)
    image = directory/IMAGE
    candidate.replace(image)
    (directory/CHECKSUM).write_text(digest+'\n')
    return image
