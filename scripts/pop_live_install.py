"""Locate the live kernel and stage a disposable Pop!_OS disk installation."""
import os
from pathlib import PurePosixPath
import secrets
import shlex
import subprocess
import time

from linux_vm_console import Console


DISK_SERIAL = 'cor-pop-install'
DISK_DEVICE = f'virtio-blk-pci,drive=compatdisk,serial={DISK_SERIAL}'
INSTALLED_READY = (
    'test -f /var/lib/cor-pop-install-complete && '
    'test "$(findmnt -n -o SOURCE --target /)" = /dev/vda2 && '
    'test "$(findmnt -n -o FSTYPE --target /)" = ext4'
)


def extract_casper_boot(iso, directory):
    members = subprocess.check_output(['bsdtar', '-tf', iso], text=True).splitlines()
    kernels = [member for member in members
               if PurePosixPath(member).name in ('vmlinuz', 'vmlinuz.efi')
               and PurePosixPath(member).parent.name.startswith('casper')]
    if len(kernels) != 1:
        raise ValueError('Expected one Pop!_OS live kernel')
    media = PurePosixPath(kernels[0]).parent
    initrds = [member for member in members
               if PurePosixPath(member).parent == media
               and PurePosixPath(member).name in ('initrd', 'initrd.gz', 'initrd.lz')]
    if len(initrds) != 1:
        raise ValueError('Expected one initramfs alongside the Pop!_OS live kernel')
    kernel, initrd = directory/'vmlinuz', directory/'initrd'
    for member, output in zip((kernels[0], initrds[0]), (kernel, initrd)):
        with output.open('wb') as stream:
            subprocess.run(['bsdtar', '-xOf', iso, member], stdout=stream, check=True)
        if not output.stat().st_size:
            raise ValueError(f'Empty Pop!_OS boot file: {member}')
    return kernel, initrd, '/' + media.as_posix().lstrip('/')


INSTALL = """\
set -eu
expected_serial="$1"
media="$2"
seed=/mnt/cor-test
disk=/dev/vda
target=/mnt/cor-installed
fail() {
    printf 'Pop!_OS installer: %s\\n' "$*" >&2
    exit 1
}
test "$(id -u)" = 0 || fail 'Live shell is not running as root'
test -d /sys/firmware/efi || fail 'Live system did not boot in UEFI mode'
grep -qw boot=casper /proc/cmdline || fail 'Live system is missing boot=casper'
test "$(lsblk --raw --noheadings --nodeps --output TYPE "$disk")" = disk || fail "$disk is not a disk"
serial="$(lsblk --raw --noheadings --nodeps --output SERIAL "$disk")" || fail "Cannot read $disk serial"
test "$serial" = "$expected_serial" || fail "$disk serial '$serial' does not match '$expected_serial'"
test "$(lsblk --raw --noheadings --nodeps --output RO "$disk")" = 0 || fail "$disk is not writable"
signatures="$(wipefs --no-act --noheadings --output TYPE "$disk")" || fail "Cannot inspect $disk signatures"
test -z "$signatures" || fail "$disk is not blank"
squashfs="/cdrom$media/filesystem.squashfs"
test -f "$squashfs" && test -r "$squashfs" || fail "Cannot read $squashfs"
remove="/cdrom$media/filesystem.manifest-remove"
test -r "$remove" || fail "Cannot read $remove"
printf 'Pop!_OS installer: starting distinst\\n'
distinst -s "$squashfs" -r "$remove" -h cor-linux-test -k us -l en_US.UTF-8 \\
    -b "$disk" -t "$disk:gpt" \\
    -n "$disk:primary:start:512M:fat32:mount=/boot/efi:flags=esp" \\
    -n "$disk:primary:512M:-4096M:ext4:mount=/" \\
    -n "$disk:primary:-4096M:end:swap" \\
    --username ubuntu --realname 'Compatibility Test' \\
    --profile_icon /usr/share/pixmaps/faces/penguin.png --tz Etc/UTC < "$seed/installer-password" \\
    || fail 'distinst failed'
printf 'Pop!_OS installer: preparing installed system\\n'
sync
install -d -m 0755 "$target"
mount -t ext4 "${disk}2" "$target"
trap 'umount "$target"' EXIT
test "$(chroot "$target" /bin/sh -c '. /etc/os-release; printf "%s" "$ID"')" = pop
chroot "$target" getent passwd ubuntu >/dev/null
install -d -m 0755 "$target/etc/ssh/sshd_config.d" "$target/etc/sudoers.d" \\
    "$target/etc/systemd/system/multi-user.target.wants" "$target/usr/local/sbin"
chroot "$target" install -d -m 0700 -o ubuntu -g ubuntu /home/ubuntu/.ssh
install -m 0600 "$seed/authorized-key" "$target/home/ubuntu/.ssh/authorized_keys"
chroot "$target" chown ubuntu:ubuntu /home/ubuntu/.ssh/authorized_keys
printf '%s\\n' 'PermitRootLogin no' 'PasswordAuthentication no' 'KbdInteractiveAuthentication no' \\
    > "$target/etc/ssh/sshd_config.d/00-cor-compatibility.conf"
printf '%s\\n' 'ubuntu ALL=(ALL) NOPASSWD:ALL' > "$target/etc/sudoers.d/80-cor-compatibility"
chmod 0440 "$target/etc/sudoers.d/80-cor-compatibility"
install -m 0700 "$seed/cor-install-ssh" "$target/usr/local/sbin/cor-install-ssh"
install -m 0644 "$seed/cor-guest-ssh.service" "$target/etc/systemd/system/cor-guest-ssh.service"
ln -s ../cor-guest-ssh.service "$target/etc/systemd/system/multi-user.target.wants/cor-guest-ssh.service"
systemctl --root="$target" set-default multi-user.target
touch "$target/var/lib/cor-pop-install-complete"
sync
umount "$target"
trap - EXIT
printf '%s\\n' cor-pop-install-complete
"""


def installer_media(iso, directory, public_key, ssh_unit, install_ssh):
    kernel, initrd, media = extract_casper_boot(iso, directory)
    share = directory/'pop-install-share'
    share.mkdir(mode=0o700)
    for name, value in (
        ('authorized-key', public_key + '\n'),
        ('installer-password', secrets.token_hex(32) + '\n'),
        ('install.sh', INSTALL),
        ('cor-guest-ssh.service', ssh_unit),
        ('cor-install-ssh', install_ssh),
    ):
        descriptor = os.open(share/name, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, 'w') as stream:
            stream.write(value)
    return kernel, initrd, media, share


def install(iso, directory, public_key, command, console_log, ssh_unit, install_ssh, *, architecture):
    if command.count('-serial') != 1 or DISK_DEVICE not in command:
        raise ValueError('Pop!_OS installer requires an isolated serial console and its disposable disk')
    serial_device = {'x86_64': 'ttyS0', 'aarch64': 'ttyAMA0'}[architecture]
    media_args = ['-cdrom', iso]
    kernel_console = ''
    installer_mask = ''
    if architecture == 'aarch64':
        ssh_unit = ssh_unit.replace('TTYPath=/dev/ttyS0', f'TTYPath=/dev/{serial_device}')
        kernel_console = f'console={serial_device},115200 '
        installer_mask = ' systemd.mask=distinst-v2.service'
        media_args = ['-device', 'virtio-scsi-pci,id=compat-scsi',
                      '-drive', f'file={iso},if=none,media=cdrom,readonly=on,id=compat-iso',
                      '-device', 'scsi-cd,drive=compat-iso,bus=compat-scsi.0']
    kernel, initrd, media, share = installer_media(
        iso, directory, public_key, ssh_unit, install_ssh)
    serial = directory/'pop-installer.sock'
    installer_args = [
        '-kernel', kernel, '-initrd', initrd, *media_args,
        '-append', f'boot=casper live-media-path={media} noprompt {kernel_console}'
                   f'systemd.unit=multi-user.target systemd.debug_shell={serial_device} '
                   f'systemd.mask=serial-getty@{serial_device}.service{installer_mask}',
        '-virtfs', f'local,path={share},mount_tag=cor-test,security_model=none,readonly=on',
        '-serial', f'unix:{serial},server=on,wait=off', '-no-reboot',
        '-nic', 'none',
    ]
    serial_option = command.index('-serial')
    command = [*command[:serial_option], *command[serial_option+2:], *installer_args]
    console = None
    with console_log.with_suffix('.qemu.log').open('w') as output:
        process = subprocess.Popen(command, stdout=output, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + 30
            while not serial.is_socket():
                if process.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError('Pop!_OS installer serial socket did not start')
                time.sleep(.2)
            console = Console(serial, console_log)
            console.wait_shell(process)
            if architecture == 'aarch64':
                console.command('systemctl start dbus.service systemd-logind.service && '
                                'test -S /run/dbus/system_bus_socket && '
                                'udevadm wait --timeout=45 /dev/vda', timeout=120)
            console.command('mkdir -p /mnt/cor-test && mount -t 9p '
                            '-o trans=virtio,version=9p2000.L,ro cor-test /mnt/cor-test')
            install_command = shlex.join(['sh', '/mnt/cor-test/install.sh', DISK_SERIAL, media])
            evidence = console.command(install_command, timeout=2100)
            if 'cor-pop-install-complete' not in evidence.splitlines():
                raise RuntimeError('Pop!_OS installer did not complete disk provisioning')
            console.command('systemd-run --on-active=2s /usr/bin/systemctl poweroff')
            if process.wait(timeout=90) != 0:
                raise RuntimeError('Pop!_OS installer did not shut down cleanly')
        finally:
            if console is not None:
                console.close()
            if process.poll() is None:
                process.kill()
                process.wait(timeout=15)
