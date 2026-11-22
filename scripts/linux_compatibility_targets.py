"""Fixed runner and guest releases for installed Linux package checks."""
from dataclasses import dataclass
import hashlib

UBUNTU_BASELINE = '20.04'
OPENSUSE_MIRROR = 'https://ftp.gwdg.de/pub/opensuse'
TUMBLEWEED_ARM64_REPOMD = {
    '20261117': '781d163094bd76fc89ff4fea07636bca733f4fdd51600ed1f6c0eb31baf853ad',
}


@dataclass(frozen=True)
class Target:
    family: str
    release: str
    arch: str
    image_url: str = ''
    image_digest: str = ''
    digest_algorithm: str = 'sha256'
    kernel_series: str = ''
    hosted: bool = False
    image_format: str = 'qcow2'
    writable_arm_uefi: bool = False
    vm_runner_release: str = '24.04'
    full_desktop: bool = False
    image_mirrors: tuple[str, ...] = ()

    @property
    def distribution(self):
        return {'linuxmint': 'Linux Mint', 'pop': 'Pop!_OS',
                'zorin': 'Zorin OS', 'opensuse': 'openSUSE',
                'arch': 'Arch Linux'}.get(self.family, self.family.capitalize())

    @property
    def package_format(self):
        return {'fedora': 'rpm', 'opensuse': 'rpm', 'arch': 'pkg'}.get(self.family, 'deb')

    @property
    def os_release_id(self):
        if self.family == 'opensuse':
            return 'opensuse-tumbleweed' if self.release.isdecimal() else 'opensuse-leap'
        return self.family

    @property
    def os_release_version(self):
        if self.family == 'arch':
            return ''
        return self.release.split('.')[0] if self.family == 'zorin' else self.release

    def matrix_entry(self):
        runner_release = self.release if self.hosted else self.vm_runner_release
        runner = f'ubuntu-{runner_release}' + ('-arm' if self.arch == 'arm64' else '')
        qemu = ''
        if not self.hosted:
            qemu = {'x64': 'qemu-system-x86',
                    'arm64': 'qemu-system-arm qemu-efi-aarch64 ipxe-qemu'}[self.arch]
        if self.family in ('linuxmint', 'zorin'):
            qemu += ' ovmf xorriso'
        elif self.family == 'pop':
            qemu += ' ovmf libarchive-tools' if self.arch == 'x64' else ' libarchive-tools'
            if self.release == '24.04' and self.full_desktop:
                qemu += ' qemu-system-gui libgl1-mesa-dri xvfb python3-pil xdotool tesseract-ocr tesseract-ocr-eng'
        if self.hosted:
            timeout = 30
        elif self.full_desktop:
            timeout = 120
        else:
            timeout = 75 if self.image_format == 'iso' else 45
        cache_prefix = ''
        if qemu:
            profile = hashlib.sha256(qemu.encode()).hexdigest()[:16]
            cache_prefix = f'linux-host-apt-v1-{runner}-{profile}-'
        return {'distribution': self.distribution, 'release': self.release, 'arch': self.arch,
                'os': runner, 'qemu': qemu, 'timeout': timeout, 'vm_cache_prefix': cache_prefix}


def debian_target(release, series, build, arch, digest):
    root = f'https://cdimage.debian.org/cdimage/cloud/{series}/{build}'
    image_arch = {'x64': 'amd64', 'arm64': 'arm64'}[arch]
    return Target('debian', release, arch, f'{root}/debian-{release}-generic-{image_arch}-{build}.qcow2',
                  digest, 'sha512')


def mint_target(release, digest):
    url = f'https://pub.linuxmint.io/stable/{release}/linuxmint-{release}-cinnamon-64bit.iso'
    return Target('linuxmint', release, 'x64', url, digest, image_format='iso')


def pop_target(release, build, digest, *, arch='x64'):
    image_arch, variant = {'x64': ('amd64', 'intel'), 'arm64': ('arm64', 'generic')}[arch]
    root = f'https://iso.pop-os.org/{release}/{image_arch}/{variant}/{build}'
    return Target('pop', release, arch, f'{root}/pop-os_{release}_{image_arch}_{variant}_{build}.iso',
                  digest, image_format='iso', writable_arm_uefi=arch == 'arm64')


def zorin_target(release, filename, digest):
    root = 'https://mirrors.edge.kernel.org/zorinos-isos'
    return Target('zorin', release, 'x64', f'{root}/{release.split(".")[0]}/{filename}',
                  digest, image_format='iso')


def fedora_target(build, arch, digest, *, variant='Generic', runner_release='24.04'):
    release = build.split('-')[0]
    image_arch = {'x64': 'x86_64', 'arm64': 'aarch64'}[arch]
    root = f'https://dl.fedoraproject.org/pub/fedora/linux/releases/{release}/Cloud/{image_arch}/images'
    return Target('fedora', release, arch, f'{root}/Fedora-Cloud-Base-{variant}-{build}.{image_arch}.qcow2',
                  digest, writable_arm_uefi=arch == 'arm64' and variant == 'UEFI-UKI',
                  vm_runner_release=runner_release)


def leap_target(release, build, arch, digest):
    image_arch = {'x64': 'x86_64', 'arm64': 'aarch64'}[arch]
    path = (f'/distribution/leap/{release}/appliances/'
            f'Leap-{release}-Minimal-VM.{image_arch}-Cloud-Build{build}.qcow2')
    return Target('opensuse', release, arch, 'https://download.opensuse.org' + path, digest,
                  image_mirrors=(OPENSUSE_MIRROR + path,))


def tumbleweed_target(snapshot, arch, digest):
    port, image_arch = {'x64': ('', 'x86_64'), 'arm64': ('/ports/aarch64', 'aarch64')}[arch]
    path = (f'{port}/tumbleweed/appliances/'
            f'openSUSE-Tumbleweed-Minimal-VM.{image_arch}-1.0.0-Cloud-Snapshot{snapshot}.qcow2')
    return Target('opensuse', snapshot, arch, 'https://downloadcontent.opensuse.org' + path, digest,
                  image_mirrors=(OPENSUSE_MIRROR + path,))


TARGETS = {
    'ubuntu': (
        Target('ubuntu', UBUNTU_BASELINE, 'x64', kernel_series='5.4'),
        *(Target('ubuntu', release, arch, hosted=True)
          for release in ('22.04', '24.04', '26.04') for arch in ('x64', 'arm64')),
    ),
    'debian': (
        debian_target('12', 'bookworm', '20261117-2665', 'x64',
                      'd597a8b422a0fa7f7cf1a0477d23be7f7514f32e57ccb8d0c69156e1be4a7e1712'
                      'fb64aee6298520d70aa573b801d0cfe3392ab5de39829728760c0cf99ef4ec'),
        debian_target('13', 'trixie', '20261116-2664', 'x64',
                      'e53a93ef33f828ceed0e7278c2cc2753bdc7d5098fbfae15bd64b6d878db350ea37'
                      '44247763a1c8ecb6b4019a0f623c38caf6038ad6f9c357fb4e01493ddf1a3'),
    ),
    'linuxmint': (
        mint_target('21.3', '5aa24abbc616807ab754a6a3b586f24460b0c213b6cacb0bf8b9a80b65013ecc'),
        mint_target('22.3', 'a081ab202cfda17f6924128dbd2de8b63518ac0531bcfe3f1a1b88097c459bd4'),
    ),
    'pop': (
        pop_target('22.04', 58, '4e1c5e391062c79dc611ce383c2a709fecac36798ebd81444a734fd41252608e'),
        pop_target('24.04', 20, 'a0ef3842ab710db4f4407cf3499560b59dddbbcd59bee17beab7b0e99dc22b4c'),
    ),
    'zorin': (
        zorin_target('17.3', 'Zorin-OS-17.3-Core-64-bit-r2.iso',
                     'c1510d1e16ec50c884faeb4e32326ba8d2e38d2cf22698b649168011d00220f6'),
        zorin_target('18.1', 'Zorin-OS-18.1-Core-64-bit.iso',
                     '44649b97bd307fc4c8529205d098ebbf98575a1e1ba2ee7d7005b697af1721d5'),
    ),
    'fedora': (
        fedora_target('44-1.7', 'x64', '28680fe5b371a5a82ebf43a31926e086a168e59949d03969c5093e7071f90b7f'),
        fedora_target('45-1.4', 'x64', '50bdc5e70f6a24f540932d15411c1c6ae9fcfa40fb990d876d83c5cc823b2ad0'),
    ),
    'opensuse': (
        leap_target('16.0', '18.95', 'x64', 'cdf1e5eefc20b7479059eb7ef12694e395d787168a11792997a0209499084ae5'),
        leap_target('16.1', '3.21', 'x64', 'd30c848752e54d3e05d1c07de8438c4f0cbd23fc690e402942ae61e08d00c911'),
        tumbleweed_target('20261120', 'x64', '4a2be0e8122445baeb2c7a81aa352aab5c60ee4d7f29fcc2547a4e2ef2574cde'),
    ),
    'arch': (
        Target('arch', '20261115.625457', 'x64',
               'https://geo.mirror.pkgbuild.com/images/v20261115.625457/'
               'Arch-Linux-x86_64-cloudimg-20261115.625457.qcow2',
               'dbd02830f6eb0fe07ef6e5edaddbae8b577ce4296d9e79118904704cce0a1f60'),
    ),
}


def target_for(suite, release, arch):
    for target in TARGETS[suite]:
        if (target.release, target.arch) == (release, arch):
            return target
    raise ValueError(f'Unsupported Linux compatibility target: {suite} {release} {arch}')
