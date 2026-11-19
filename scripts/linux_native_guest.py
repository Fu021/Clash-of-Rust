"""Native package operations for disposable non-DEB compatibility guests."""
from datetime import datetime
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import zipfile

from build_support import download, run
from linux_compatibility_targets import TARGETS

DISTROS = ('fedora', 'opensuse-leap', 'opensuse-tumbleweed', 'arch')
RPM = ('fedora', 'opensuse-leap', 'opensuse-tumbleweed')
APPINDICATOR = 'appindicatorsupport@rgcjonas.gmail.com'
APPINDICATOR_URL = ('https://github.com/ubuntu/gnome-shell-extension-appindicator/releases/download/'
                    'v66/appindicatorsupport%40rgcjonas.gmail.com.zip')
APPINDICATOR_SHA256 = 'f2b17c7c5590f729853cd353f20a9e59b6e0c32e9c0a561976e09f5a97487730'


def distribution():
    distro = platform.freedesktop_os_release().get('ID')
    if (distro not in DISTROS or os.environ.get('COR_LINUX_GUEST') != 'true'
            or os.environ.get('GITHUB_ACTIONS') != 'true' or os.geteuid() == 0):
        raise RuntimeError('Native package commands require an ordinary user in an isolated compatibility VM')
    return distro


def manager():
    return {
        'fedora': ('dnf', '-y', '--setopt=install_weak_deps=False'),
        'opensuse-leap': ('zypper', '--non-interactive'),
        'opensuse-tumbleweed': ('zypper', '--non-interactive'),
        'arch': ('pacman', '--noconfirm'),
    }[distribution()]


def privileged(*arguments, timeout=900):
    run(['sudo', 'timeout', '--signal=TERM', '--kill-after=30s', f'{timeout}s',
         *manager(), *arguments], timeout=timeout+45)


def refresh():
    distro = distribution()
    if distro == 'fedora':
        privileged('makecache', timeout=300)
    elif distro in RPM:
        if distro == 'opensuse-tumbleweed':
            snapshot = platform.freedesktop_os_release()['VERSION_ID']
            datetime.strptime(snapshot, '%Y%m%d')
            privileged('modifyrepo', '--all', '--disable', timeout=60)
            privileged('addrepo', '--check', '--gpgcheck',
                       f'https://download.opensuse.org/history/{snapshot}/tumbleweed/repo/oss/',
                       'cor-tumbleweed-snapshot', timeout=90)
        privileged('refresh', timeout=300)
    else:
        mirrorlist = '/etc/pacman.d/mirrorlist'
        snapshot = datetime.strptime(TARGETS['arch'][0].release.split('.')[0], '%Y%m%d')
        run(['sudo', 'tee', mirrorlist],
            input=f'Server = https://archive.archlinux.org/repos/{snapshot:%Y/%m/%d}/$repo/os/$arch\n',
            text=True, stdout=subprocess.DEVNULL)
        privileged('-Syyu')


def install_dependencies(packages):
    distro = distribution()
    if distro == 'arch':
        privileged('-S', '--needed', *packages)
    else:
        flags = ('--no-recommends',) if distro.startswith('opensuse-') else ()
        privileged('install', *flags, *packages)


def prepare():
    distro = distribution()
    common = ('git', 'gdm', 'gnome-session', 'gnome-shell',
              'gsettings-desktop-schemas', 'polkit', 'mate-polkit', 'binutils', 'sudo')
    if distro == 'fedora':
        packages = ('xorg-x11-server-Xvfb', 'xorg-x11-server-Xwayland', 'xprop', 'xdpyinfo',
                    'xwininfo', 'xdotool', 'wmctrl', 'openbox', 'gtk3', 'iproute', 'systemd-resolved',
                    'libcap', 'rpm-build', 'glib2', 'glib-networking', 'google-noto-sans-cjk-fonts')
    elif distro in RPM:
        packages = ('xorg-x11-server-Xvfb', 'xwayland', 'xprop', 'xdpyinfo', 'xwininfo', 'xdotool',
                    'wmctrl', 'openbox', 'gtk3-tools', 'iproute2', 'systemd-resolved', 'libcap-progs',
                    'rpm-build', 'glib2-tools', 'glib-networking', 'google-noto-sans-cjk-fonts')
    else:
        packages = ('xorg-server-xvfb', 'xorg-xwayland', 'xorg-xprop', 'xorg-xdpyinfo', 'xorg-xwininfo',
                    'xdotool', 'wmctrl', 'openbox', 'gtk3', 'iproute2', 'systemd-resolvconf', 'libcap',
                    'fakeroot', 'libarchive', 'glib2', 'glib-networking', 'noto-fonts-cjk')
    install_dependencies((*common, *packages))
    install_appindicator()


def install_appindicator():
    from workflow_artifacts import safe_name

    distribution()
    target = Path('/usr/share/gnome-shell/extensions')/APPINDICATOR
    if target.is_dir():
        return
    with tempfile.TemporaryDirectory(prefix='cor-appindicator-') as temporary:
        directory = Path(temporary)
        archive = directory/'extension.zip'
        download(APPINDICATOR_URL, archive, digest=APPINDICATOR_SHA256)
        source = directory/'extension'
        source.mkdir()
        with zipfile.ZipFile(archive) as zipped:
            entries = zipped.infolist()
            if (len({entry.filename for entry in entries}) != len(entries)
                    or sum(entry.file_size for entry in entries) > 16*1024*1024
                    or any(not safe_name(entry.filename.rstrip('/'))
                           or (entry.external_attr >> 16) & 0o170000 == 0o120000
                           for entry in entries)):
                raise ValueError('Invalid AppIndicator extension archive')
            for entry in entries:
                path = source/entry.filename
                if entry.is_dir():
                    path.mkdir(parents=True, exist_ok=True)
                else:
                    path.parent.mkdir(parents=True, exist_ok=True)
                    with zipped.open(entry) as stream, path.open('xb') as output:
                        shutil.copyfileobj(stream, output)
        if not (source/'metadata.json').is_file():
            raise ValueError('AppIndicator archive is missing extension metadata')
        run(['sudo', 'install', '-d', '-m', '0755', target])
        run(['sudo', 'cp', '-R', str(source) + '/.', target])


def install_package(package):
    if not package.is_absolute() or not package.is_file():
        raise ValueError('Expected a verified local native package')
    distro = distribution()
    if distro == 'arch':
        privileged('-U', package)
    else:
        flags = ('--allow-unsigned-rpm', '--no-recommends') if distro.startswith('opensuse-') else ()
        privileged('install', *flags, package)


def remove_package():
    command = '-R' if distribution() == 'arch' else 'remove'
    privileged(command, 'clash-of-rust', timeout=300)


def package_version(package=None):
    distro = distribution()
    if distro == 'arch':
        arguments = ['pacman', '-Qp', package] if package else ['pacman', '-Q', 'clash-of-rust']
        value = subprocess.check_output(arguments, text=True).strip().split()
        if len(value) != 2 or value[0] != 'clash-of-rust':
            raise ValueError('Incorrect Arch package identity')
        return value[1]
    arguments = ['rpm', '-qp'] if package else ['rpm', '-q']
    name, version = subprocess.check_output(
        [*arguments, '--qf', '%{NAME}\n%{VERSION}-%{RELEASE}', package or 'clash-of-rust'],
        text=True).splitlines()
    if name != 'clash-of-rust':
        raise ValueError('Incorrect RPM package identity')
    return version


def verify_upgrade_order(previous, candidate):
    if compare_versions(previous, candidate) >= 0:
        raise RuntimeError(f'Native package manager did not order {previous} before {candidate}')


def compare_versions(previous, candidate):
    if distribution() == 'arch':
        result = subprocess.check_output(['vercmp', previous, candidate], text=True).strip()
    else:
        import re
        if not all(re.fullmatch(r'[0-9a-zA-Z.~+-]+', value) for value in (previous, candidate)):
            raise ValueError('Invalid RPM version')
        result = subprocess.check_output(
            ['rpm', '--eval', f'%{{lua:print(rpm.vercmp("{previous}","{candidate}"))}}'],
            text=True).strip()
    return int(result)
