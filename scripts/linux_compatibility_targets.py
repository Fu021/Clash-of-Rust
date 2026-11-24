"""Fixed runner and guest releases for installed Linux package checks."""
from dataclasses import dataclass
import hashlib

UBUNTU_BASELINE = '20.04'
OPENSUSE_MIRROR = 'https://ftp.gwdg.de/pub/opensuse'
TUMBLEWEED_REPOMD = {
    'x86_64': {'20261121': 'd4e2a3bf9bf999e5fab5546f75388a27aac7c1f5384879d26b4f15b988f4a772'},
    'aarch64': {'20261121': 'eb4a8dc11f34b7db4c419ce9b2f9c6130fe2b45d7b7fb91aee7757439ca86500'},
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
        if not self.hosted or self.full_desktop:
            qemu = {'x64': 'qemu-system-x86',
                    'arm64': 'qemu-system-arm qemu-efi-aarch64 ipxe-qemu'}[self.arch]
        if self.family in ('linuxmint', 'zorin'):
            qemu += ' ovmf xorriso'
        elif self.family == 'pop':
            qemu += ' ovmf libarchive-tools' if self.arch == 'x64' else ' libarchive-tools'
            if self.release == '24.04' and self.full_desktop:
                qemu += ' qemu-system-gui libgl1-mesa-dri xvfb python3-pil xdotool tesseract-ocr tesseract-ocr-eng'
        if self.full_desktop:
            timeout = 180 if self.family == 'ubuntu' else 120
        elif self.hosted:
            timeout = 30
        else:
            timeout = 75 if self.image_format == 'iso' else 45
        cache_prefix = ''
        if qemu:
            profile = hashlib.sha256(qemu.encode()).hexdigest()[:16]
            cache_prefix = f'linux-host-apt-v1-{runner}-{profile}-'
        return {'distribution': self.distribution, 'release': self.release, 'arch': self.arch,
                'os': runner, 'qemu': qemu, 'timeout': timeout, 'vm_cache_prefix': cache_prefix}


def ubuntu_arm_desktop_target(release, series, build, digest):
    root = f'https://cloud-images.ubuntu.com/releases/{series}/release-{build}'
    return Target('ubuntu', release, 'arm64', f'{root}/ubuntu-{release}-server-cloudimg-arm64.img',
                  digest, hosted=True, full_desktop=True)


def debian_target(release, series, build, arch, digest):
    root = f'https://cdimage.debian.org/cdimage/cloud/{series}/{build}'
    image_arch = {'x64': 'amd64', 'arm64': 'arm64'}[arch]
    return Target('debian', release, arch, f'{root}/debian-{release}-generic-{image_arch}-{build}.qcow2',
                  digest, 'sha512')


def mint_target(release, digest):
    path = f'/stable/{release}/linuxmint-{release}-cinnamon-64bit.iso'
    return Target('linuxmint', release, 'x64', 'https://pub.linuxmint.io' + path, digest,
                  image_format='iso', image_mirrors=('https://mirrors.edge.kernel.org/linuxmint' + path,))


def pop_target(release, build, digest, *, arch='x64'):
    image_arch, variant = {'x64': ('amd64', 'intel'), 'arm64': ('arm64', 'generic')}[arch]
    root = f'https://iso.pop-os.org/{release}/{image_arch}/{variant}/{build}'
    return Target('pop', release, arch, f'{root}/pop-os_{release}_{image_arch}_{variant}_{build}.iso',
                  digest, image_format='iso', writable_arm_uefi=arch == 'arm64')


def zorin_target(release, filename, digest):
    path = f'/{release.split(".")[0]}/{filename}'
    return Target('zorin', release, 'x64', 'https://mirrors.edge.kernel.org/zorinos-isos' + path,
                  digest, image_format='iso',
                  image_mirrors=('https://distro.ibiblio.org/zorinos' + path,
                                 'https://mirror.clarkson.edu/zorinos/isos' + path))


def fedora_target(build, arch, digest, *, variant='Generic', runner_release='24.04'):
    release = build.split('-')[0]
    image_arch = {'x64': 'x86_64', 'arm64': 'aarch64'}[arch]
    root = f'https://dl.fedoraproject.org/pub/fedora/linux/releases/{release}/Cloud/{image_arch}/images'
    return Target('fedora', release, arch, f'{root}/Fedora-Cloud-Base-{variant}-{build}.{image_arch}.qcow2',
                  digest, writable_arm_uefi=arch == 'arm64' and variant == 'UEFI-UKI',
                  vm_runner_release=runner_release, full_desktop=arch == 'arm64')


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
        Target('ubuntu', UBUNTU_BASELINE, 'arm64', kernel_series='5.4'),
        *(Target('ubuntu', release, 'x64', hosted=True) for release in ('22.04', '24.04', '26.04')),
        Target('ubuntu', '22.04', 'arm64', hosted=True),
        ubuntu_arm_desktop_target('24.04', 'noble', '20261017',
                                  '50e24e1fb79e89116e939d90aa7738f7af48c442fa1bc2510131e0b39c50348e'),
        ubuntu_arm_desktop_target('26.04', 'resolute', '20261108',
                                  'cf3b8472b84e7de37cdc2d2e9a9bf82b3edf0ab97113a1422eff048f26688e2f'),
    ),
    'debian': (
        debian_target('12', 'bookworm', '20261117-2665', 'x64',
                      'd597a8b422a0fa7f7cf1a0477d23be7f7514f32e57ccb8d0c69156e1be4a7e1712'
                      'fb64aee6298520d70aa573b801d0cfe3392ab5de39829728760c0cf99ef4ec'),
        debian_target('13', 'trixie', '20261116-2664', 'x64',
                      'e53a93ef33f828ceed0e7278c2cc2753bdc7d5098fbfae15bd64b6d878db350ea37'
                      '44247763a1c8ecb6b4019a0f623c38caf6038ad6f9c357fb4e01493ddf1a3'),
        debian_target('12', 'bookworm', '20261117-2665', 'arm64',
                      '1e8651e41b5cd07cd0f9ee63aa72dffe28afe8f2f197f0b340576f43df8f10c4bf'
                      '8352d3f8630e9a99b225802f2c2fdb25ac17e7182310a4a181fa4a7880c889'),
        debian_target('13', 'trixie', '20261116-2664', 'arm64',
                      '3d4655799e64ca93f51832da31101188f28e01cb14fec2164ac9a7a5f70f303b80f'
                      '72e9632498dd9cb0099ad04b12c186bffe4f0cd50c5e739faa3ee85007eb0'),
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
        fedora_target('44-1.7', 'arm64', '55c60a3b80d3616a08705afd0459e75fe9f03c54aba7a46e4002a41a72fa0d5b'),
        fedora_target('45-1.4', 'arm64', 'f51f556104434e66df591d647d3e54254cc722378c5ac31c4b1c37b15075e4f1',
                      variant='UEFI-UKI', runner_release='26.04'),
    ),
    'opensuse': (
        leap_target('16.0', '18.96', 'x64', '05637a09744708dde5148b49f703bc3192e4fe5f81ea4b104f586b1514b5421c'),
        leap_target('16.1', '3.22', 'x64', '89d4638c55cb045042fbdd1be57952f9919eeb034caf993389116da118bc6157'),
        tumbleweed_target('20261121', 'x64', '7ba3b9ed4ea8e86dbf5a5f538870f834d37a2ac30d763e0268f3186dd03ddb95'),
        leap_target('16.0', '18.96', 'arm64', 'd9094828e22bd0b1a6ff6290d338750cf694dbacf2a191744f49d2f2a0e6b389'),
        leap_target('16.1', '3.22', 'arm64', '5927563db4344f157209ce68f1bb07f311c83cf504e7b3f66d5eed82c348c555'),
        tumbleweed_target('20261121', 'arm64', 'bef781e7c43d1cbe184e1b72242a7809d58c7440df7380946169c391e19ae28b'),
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
