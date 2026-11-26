"""Fixed runner and guest releases for installed Linux package checks."""
from dataclasses import dataclass
import hashlib

UBUNTU_BASELINE = '20.04'
OPENSUSE_MIRROR = 'https://ftp.gwdg.de/pub/opensuse'
JAMMY_CONTAINER = 'docker.io/library/ubuntu@sha256:b000e69a50ff940390233025e7a337fb8469b509e2334fa4d018ac30f3e18cdb'
NOBLE_CONTAINER = 'docker.io/library/ubuntu@sha256:562111b88caa79c71391c0c4d3c2114a7d5c35915c20561fd893cb1e07929e6b'
UBUNTU_ARM_RUNNER_CONTAINERS = {
    '22.04': JAMMY_CONTAINER,
    '24.04': NOBLE_CONTAINER,
    '26.04': 'docker.io/library/ubuntu@sha256:d66b366dac61b575bee2aef07e9c26e8070096fbe84a3d4d3eb0da069b5d76db',
}
BASIC_CONTAINER_IMAGES = {
    ('arch', '20261115.625457'): 'docker.io/library/archlinux@sha256:fcee73ea1b96e236248f53a213f71276cc9072d9025b58bce9915e32f19963ae',
    ('ubuntu', '20.04'): 'docker.io/library/ubuntu@sha256:8feb4d8ca5354def3d8fce243717141ce31e2c428701f6682bd2fafe15388214',
    ('debian', '12'): 'docker.io/library/debian@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc',
    ('debian', '13'): 'docker.io/library/debian@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2',
    ('fedora', '44'): 'quay.io/fedora/fedora@sha256:c2311603edaa76512f78e9edbb2b5b3903b3ad5bbc43a94ecb2e6c5c501d5f0c',
    ('fedora', '45'): 'quay.io/fedora/fedora@sha256:92837d4834c1cff45cc2d534b655489b8e536271de1897932117e396cc9980c5',
    ('linuxmint', '21.3'): JAMMY_CONTAINER,
    ('linuxmint', '22.3'): NOBLE_CONTAINER,
    ('pop', '22.04'): JAMMY_CONTAINER,
    ('pop', '24.04'): NOBLE_CONTAINER,
    ('zorin', '17.3'): JAMMY_CONTAINER,
    ('zorin', '18.1'): NOBLE_CONTAINER,
    ('opensuse', '16.0'): 'registry.opensuse.org/opensuse/leap@sha256:9c88da273b41600dfd12790849dfb198ddefed3a109eaf42ada944afb4282a9d',
    ('opensuse', '16.1'): 'registry.opensuse.org/opensuse/leap@sha256:755746c7eec92fbdd3c4b02be194bfded4a8d4fde0abedf1835f945d403f43c8',
    ('opensuse', '20261123'): 'registry.opensuse.org/opensuse/tumbleweed@sha256:e81375320e178f55d205d5d1abdec3ebf42ad43e680f757e2d45ba536617f468',
}
MINT_CONTAINER_KEYRING = {
    'url': 'https://fastly.linuxmint.io/pool/main/l/linuxmint-keyring/linuxmint-keyring_2022.06.21_all.deb',
    'sha256': 'b71be690c543112ea7b65f43e9bbce9a3f17dd5cc784074858f0824154942a99',
}
ZORIN_PATCHES_KEY = {
    'url': 'https://keyserver.ubuntu.com/pks/lookup?op=get&search=0xB2228B8CCA5C00A2C59D35E530FCF8F64F71B61C',
    'sha256': '1c656c6d675d8854da073f5c192d3a156bf0da3613e7f9ada5b20bd1565a32ca',
}
DEB_CONTAINER_INPUTS = {
    ('linuxmint', '21.3'): {
        'release-keyring.deb': MINT_CONTAINER_KEYRING,
        'base-files.deb': {
            'url': 'https://fastly.linuxmint.io/pool/upstream/b/base-files/base-files_21.3.0_amd64.deb',
            'sha256': '45a2838973e859a74ee80156bc4dfa6dfe4d63a4f5e15be813bd699badc8c008',
        },
    },
    ('linuxmint', '22.3'): {
        'release-keyring.deb': MINT_CONTAINER_KEYRING,
        'base-files.deb': {
            'url': 'https://fastly.linuxmint.io/pool/upstream/b/base-files/base-files_13ubuntu10mint22.3.0_amd64.deb',
            'sha256': '9d1e93b0578c4d57e0b873b78ea0926b9a0f75bada90deb9b24599fefd0135db',
        },
    },
    ('pop', '22.04'): {
        'release-keyring.deb': {
            'url': 'https://apt.pop-os.org/release/pool/jammy/keyring/'
                   '356a75e405d00a23889998d2e8f2508a9e67b535/'
                   'pop-keyring_1.0.0~1641313934~22.04~356a75e_all.deb',
            'sha256': '9af097d894cf4549484f506c46c22d3139dc095cd0f1185cb7d589bcebbfba39',
        },
    },
    ('pop', '24.04'): {
        'release-keyring.deb': {
            'url': 'https://apt.pop-os.org/release/pool/noble/keyring/'
                   '356a75e405d00a23889998d2e8f2508a9e67b535/'
                   'pop-keyring_1.0.0~1641313934~24.04~356a75e_all.deb',
            'sha256': 'c259ecb956c0eda137b3f71e2d547d5e3440b518f96b9fe7f925644039129081',
        },
    },
    ('zorin', '17.3'): {
        'patches.asc': ZORIN_PATCHES_KEY,
        'base-files.deb': {
            'url': 'https://ppa.launchpadcontent.net/zorinos/patches/ubuntu/pool/main/b/base-files/'
                   'base-files_12ubuntu4.7+zorin2_amd64.deb',
            'sha256': '452d8e3135efaa75eea2e7c98aa107dfeccc5d34ce704ef6f1c0ab7b73cb0bc5',
        },
    },
    ('zorin', '18.1'): {
        'patches.asc': ZORIN_PATCHES_KEY,
        'base-files.deb': {
            'url': 'https://ppa.launchpadcontent.net/zorinos/patches/ubuntu/pool/main/b/base-files/'
                   'base-files_13ubuntu10.4+zorin2_amd64.deb',
            'sha256': '22cf137b5ac4a9e6a0a08faa928166b67256f99f1213113f111ca19f2ea50114',
        },
    },
}
TUMBLEWEED_REPOMD = {
    'x86_64': {'20261123': '3e3d5ec33d14143ad87e19aa3359df2e7a85f67b53d0e7be9d2dbb03b36a48bd'},
    'aarch64': {'20261123': '7c34a91594f384e6c483a06d9ffe635aee95a00c536a55c7f26fdb0f4b513b44'},
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

    def basic_container_image(self, published_update=False):
        if (published_update and self.arch == 'x64') or self.full_desktop:
            return ''
        if self.hosted:
            return UBUNTU_ARM_RUNNER_CONTAINERS[self.release] if self.arch == 'arm64' else ''
        return BASIC_CONTAINER_IMAGES.get((self.family, self.release), '')

    def matrix_entry(self, published_update=False):
        runner_release = self.release if self.hosted else self.vm_runner_release
        runner = f'ubuntu-{runner_release}' + ('-arm' if self.arch == 'arm64' else '')
        if self.basic_container_image(published_update):
            return {'distribution': self.distribution, 'release': self.release, 'arch': self.arch,
                    'os': runner, 'qemu': '', 'timeout': 15, 'vm_cache_prefix': ''}
        qemu = ''
        if not self.hosted or self.full_desktop:
            qemu = {'x64': 'qemu-system-x86',
                    'arm64': 'qemu-system-arm qemu-efi-aarch64 ipxe-qemu'}[self.arch]
        if self.family in ('linuxmint', 'zorin'):
            qemu += ' ovmf xorriso'
        elif self.family == 'pop':
            qemu += ' ovmf libarchive-tools' if self.arch == 'x64' else ' libarchive-tools'
            if self.release == '24.04' and self.full_desktop:
                qemu += (' qemu-system-gui qemu-system-modules-opengl libgl1-mesa-dri'
                         ' xvfb python3-pil xdotool tesseract-ocr tesseract-ocr-eng')
        if self.family == 'pop' and self.arch == 'arm64':
            timeout = 180 if self.full_desktop else 120
        elif self.full_desktop:
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
        Target('ubuntu', UBUNTU_BASELINE, 'arm64', kernel_series='5.4'),
        *(Target('ubuntu', release, 'x64', hosted=True) for release in ('22.04', '24.04', '26.04')),
        *(Target('ubuntu', release, 'arm64', hosted=True) for release in ('22.04', '24.04', '26.04')),
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
        pop_target('24.04', 3, '7b4cce0e92dc5c903464e7e7c33760c4417f1400165e9ecfcd064fbceb68ef22',
                   arch='arm64'),
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
        leap_target('16.0', '18.97', 'x64', '6f41376091084719bdf0a7854e8498cddc0ec8fbbe45b0f4d6317da9ac368c70'),
        leap_target('16.1', '3.23', 'x64', '563932158571f5e7cd42834aa54f8027be198653844125b7467756a5ec5aa651'),
        tumbleweed_target('20261123', 'x64', '9a47d43636bea57f0f8b4c4843f3f5e294076c08c17f59ea757b5b3d70838ca7'),
        leap_target('16.0', '18.97', 'arm64', 'b2da386b7af09f5ece0681c6659cf1f7a6f034799f0c04029c7890c75816c18f'),
        leap_target('16.1', '3.23', 'arm64', 'ffb95d3c9bb38c5fccb73324f857c2921c24081c1bfdb910981c655e54ecb3cb'),
        tumbleweed_target('20261123', 'arm64', '4758863fb624f5b2c9116633d4f1d3b1f5990639001660c5686c737cac681b02'),
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
