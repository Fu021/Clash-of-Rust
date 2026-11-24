"""Identify disposable local Linux compatibility guests."""
import os
from pathlib import Path
import stat
import subprocess
import sys

LOCAL_GUEST_MARKER = Path('/etc/clash-of-rust-compatibility-vm')
LOCAL_GUEST_CONTENT = b'clash-of-rust disposable compatibility guest v1\n'


def is_local_guest():
    if (sys.platform != 'linux' or os.geteuid() == 0
            or os.environ.get('COR_LINUX_GUEST') != 'true'):
        return False
    try:
        marker = LOCAL_GUEST_MARKER.lstat()
        if (not stat.S_ISREG(marker.st_mode) or stat.S_IMODE(marker.st_mode) != 0o444
                or marker.st_uid != 0 or marker.st_size != len(LOCAL_GUEST_CONTENT)
                or LOCAL_GUEST_MARKER.read_bytes() != LOCAL_GUEST_CONTENT):
            return False
        virtual_machine = subprocess.run(
            ['/usr/bin/systemd-detect-virt', '--vm'], capture_output=True, text=True, timeout=5)
    except (OSError, subprocess.TimeoutExpired):
        return False
    return virtual_machine.returncode == 0 and virtual_machine.stdout.strip() in ('qemu', 'kvm')


def is_disposable_test_environment():
    return os.environ.get('GITHUB_ACTIONS') == 'true' or is_local_guest()
