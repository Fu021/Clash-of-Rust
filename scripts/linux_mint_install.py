"""Install a pinned Ubiquity desktop ISO on an empty CI VM disk."""
import re
import secrets
import subprocess

from build_support import run

FIRMWARE = '/usr/share/OVMF/OVMF_CODE_4M.fd'
FIRMWARE_VARS = '/usr/share/OVMF/OVMF_VARS_4M.fd'
INSTALL_MARKER = 'cor-mint-install-complete'
INSTALL_FAILURES = {
    'cor-mint-installer-prompt': 'Mint installer required manual input',
    'cor-mint-installer-failure': 'Mint installer reported an unrecoverable error',
    'cor-mint-finish-failure': 'Mint installer could not provision the installed guest',
}
INSTALLED_READY = (
    'test -f /var/lib/cor-mint-install-complete && '
    'test "$(findmnt -n -o FSTYPE --target /)" = ext4'
)

PRESEED = """\
d-i debian-installer/locale string en_US.UTF-8
d-i console-setup/ask_detect boolean false
d-i keyboard-configuration/layoutcode string us
d-i time/zone string Etc/UTC
d-i clock-setup/utc boolean true
d-i clock-setup/ntp boolean false
d-i netcfg/enable boolean false
d-i netcfg/get_hostname string cor-linux-test
d-i netcfg/get_domain string local
d-i passwd/user-fullname string Compatibility Test
d-i passwd/username string ubuntu
d-i passwd/user-password-crypted password {password}
d-i passwd/auto-login boolean false
d-i user-setup/encrypt-home boolean false
d-i partman-auto/disk string /dev/vda
d-i partman-auto/method string regular
d-i partman-auto/choose_recipe select atomic
d-i partman-partitioning/confirm_write_new_label boolean true
d-i partman/choose_partition select finish
d-i partman/confirm boolean true
d-i partman/confirm_nooverwrite boolean true
d-i grub-installer/bootdev string /dev/vda
d-i apt-setup/use_mirror boolean false
ubiquity ubiquity/use_nonfree boolean false
ubiquity ubiquity/download_updates boolean false
ubiquity ubiquity/reboot boolean true
ubiquity ubiquity/automation_failure_command string /bin/sh /cdrom/preseed/cor-install-failed cor-mint-installer-prompt
ubiquity ubiquity/failure_command string /bin/sh /cdrom/preseed/cor-install-failed cor-mint-installer-failure
ubiquity ubiquity/success_command string /bin/sh /cdrom/preseed/cor-install-done
"""

FINISH = """\
set -eu
trap 'if [ "$?" -ne 0 ]; then /bin/sh /cdrom/preseed/cor-install-failed cor-mint-finish-failure; fi' EXIT
target=/target
seed=/cdrom/preseed
mountpoint -q "$target"
distribution="$(. "$target/etc/os-release"; printf '%s' "$ID")"
if [ "$distribution" = linuxmint ]; then
    mint_sources="$target/etc/apt/sources.list.d/official-package-repositories.list"
    sed -i -E 's@^(deb(-src)?[[:space:]]+)http://packages\\.linuxmint\\.com/?([[:space:]])@\\1https://fastly.linuxmint.io/\\3@' "$mint_sources"
    grep -qE '^deb[[:space:]]+https://fastly\\.linuxmint\\.io/[[:space:]]' "$mint_sources"
elif [ "$distribution" = zorin ]; then
    suite="$(. "$target/etc/os-release"; printf '%s' "$UBUNTU_CODENAME")"
    case "$suite" in
        jammy|noble) ;;
        *) printf 'Unexpected Zorin Ubuntu base: %s\\n' "$suite" >&2; exit 1 ;;
    esac
    keyring=/usr/share/keyrings/ubuntu-archive-keyring.gpg
    test -r "$target$keyring"
    for pocket in "$suite" "$suite-updates"; do
        printf 'deb [signed-by=%s] https://archive.ubuntu.com/ubuntu %s main restricted universe multiverse\\n' \\
            "$keyring" "$pocket"
    done > "$target/etc/apt/sources.list.d/cor-ubuntu-base.list"
fi
install -d -m 0755 "$target/etc/ssh/sshd_config.d" "$target/etc/sudoers.d" \\
    "$target/etc/systemd/system/multi-user.target.wants" "$target/usr/local/sbin"
chroot "$target" install -d -m 0700 -o ubuntu -g ubuntu /home/ubuntu/.ssh
install -m 0600 "$seed/cor-authorized-key" "$target/home/ubuntu/.ssh/authorized_keys"
chroot "$target" chown ubuntu:ubuntu /home/ubuntu/.ssh/authorized_keys
printf '%s\\n' 'PermitRootLogin no' 'PasswordAuthentication no' 'KbdInteractiveAuthentication no' \\
    > "$target/etc/ssh/sshd_config.d/00-cor-compatibility.conf"
printf '%s\\n' 'ubuntu ALL=(ALL) NOPASSWD:ALL' > "$target/etc/sudoers.d/80-cor-compatibility"
chmod 0440 "$target/etc/sudoers.d/80-cor-compatibility"
install -m 0700 "$seed/cor-install-ssh" "$target/usr/local/sbin/cor-install-ssh"
install -m 0644 "$seed/cor-guest-ssh.service" "$target/etc/systemd/system/cor-guest-ssh.service"
ln -s ../cor-guest-ssh.service "$target/etc/systemd/system/multi-user.target.wants/cor-guest-ssh.service"
touch "$target/var/lib/cor-mint-install-complete"
printf '\\n%s\\n' 'cor-mint-install-complete' > /dev/ttyS0
"""

INSTALL_FAILED = """\
printf '\\n%s\\n' "$1" > /dev/ttyS0
systemctl --no-block reboot
"""

SSH_UNIT = """\
[Unit]
Description=Provision SSH for the disposable compatibility guest
Wants=network-online.target
After=network-online.target
ConditionPathExists=!/var/lib/cor-guest-ssh-ready

[Service]
Type=oneshot
ExecStart=/usr/local/sbin/cor-install-ssh
TimeoutStartSec=600
StandardOutput=tty
StandardError=inherit
TTYPath=/dev/ttyS0
"""

INSTALL_SSH = """\
#!/bin/sh
set -eu
export DEBIAN_FRONTEND=noninteractive
apt-get -o Acquire::Retries=3 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20 \\
    -o APT::Update::Error-Mode=any update
apt-get -o Acquire::Retries=3 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20 \\
    install -y --no-install-recommends openssh-server
if ! systemctl is-active --quiet ssh.service ssh.socket; then
    systemctl enable --now ssh.service
fi
touch /var/lib/cor-guest-ssh-ready
"""


def automatic_grub(source, initrd='/casper/initrd.lz'):
    if initrd not in ('/casper/initrd.lz', '/casper/initrd.zstd'):
        raise ValueError('Unsupported Ubiquity initramfs')
    expected = 'maybe-ubiquity' if initrd.endswith('.zstd') else 'boot=casper'
    for line in source.splitlines():
        kernel = re.fullmatch(r'\s*linux\s+/casper/vmlinuz\s+(.+?)\s+--+\s*', line)
        if kernel and expected in kernel[1].split():
            break
    else:
        raise ValueError('ISO does not expose the expected Casper boot menu')
    arguments = [value for value in kernel[1].split()
                 if value not in {'quiet', 'splash', 'maybe-ubiquity'} and not value.startswith('iso-scan/')]
    if 'boot=casper' not in arguments:
        arguments.insert(0, 'boot=casper')
    return ('set default=0\nset timeout=0\nmenuentry "Install compatibility guest" {\n'
            f'  linux /casper/vmlinuz {" ".join(arguments)} automatic-ubiquity only-ubiquity '
            'file=/cdrom/preseed/cor.cfg noprompt console=tty0 console=ttyS0,115200n8 '
            'systemd.journald.forward_to_console=1 --\n'
            f'  initrd {initrd}\n}}\n')


def installation_media(iso, directory, public_key, initrd='/casper/initrd.lz'):
    source_grub = directory/'source-grub.cfg'
    grub = directory/'grub.cfg'
    run(['xorriso', '-osirrox', 'on', '-indev', iso, '-extract', '/boot/grub/grub.cfg',
         source_grub], timeout=120)
    grub.write_text(automatic_grub(source_grub.read_text(), initrd))
    seed = directory/'preseed'
    seed.mkdir()
    password = subprocess.check_output(['openssl', 'passwd', '-6', '-stdin'],
                                       text=True, input=secrets.token_hex(24)+'\n', timeout=10).strip()
    (seed/'cor.cfg').write_text(PRESEED.format(password=password))
    (seed/'cor-install-done').write_text(FINISH)
    (seed/'cor-install-failed').write_text(INSTALL_FAILED)
    (seed/'cor-install-ssh').write_text(INSTALL_SSH)
    (seed/'cor-guest-ssh.service').write_text(SSH_UNIT)
    (seed/'cor-authorized-key').write_text(public_key+'\n')
    media = directory/'installer.iso'
    run(['xorriso', '-indev', iso, '-outdev', media, '-map', grub, '/boot/grub/grub.cfg',
         '-map', seed, '/preseed', '-boot_image', 'any', 'replay'], timeout=300)
    if media.stat().st_size == 0:
        raise RuntimeError('The derived Mint installer ISO is empty')
    return media


def install(iso, directory, public_key, command, console, *, initrd='/casper/initrd.lz'):
    media = installation_media(iso, directory, public_key, initrd)
    iso.unlink()
    problem = None
    with console.open('w') as log:
        try:
            run([*command, '-cdrom', media, '-boot', 'order=d', '-no-reboot', '-nic', 'none'],
                stdout=log, stderr=subprocess.STDOUT, timeout=2100)
        except subprocess.SubprocessError as error:
            problem = error
    completed = False
    with console.open(errors='replace') as log:
        for line in log:
            marker = line.strip()
            if marker in INSTALL_FAILURES:
                raise RuntimeError(INSTALL_FAILURES[marker]) from problem
            completed |= marker == INSTALL_MARKER
    if problem is not None:
        raise problem
    if not completed:
        raise RuntimeError('Mint installer did not finish writing the VM disk')
    media.unlink()
