"""Fixed runner and guest releases for installed DEB checks."""
from dataclasses import dataclass

UBUNTU_BASELINE = '20.04'


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

    @property
    def distribution(self):
        return 'Linux Mint' if self.family == 'linuxmint' else self.family.capitalize()

    @property
    def full_desktop(self):
        return self.family in ('debian', 'linuxmint')

    def matrix_entry(self):
        runner_release = self.release if self.hosted else '24.04'
        runner = f'ubuntu-{runner_release}' + ('-arm' if self.arch == 'arm64' else '')
        qemu = ''
        if not self.hosted:
            qemu = {'x64': 'qemu-system-x86', 'arm64': 'qemu-system-arm qemu-efi-aarch64'}[self.arch]
        if self.image_format == 'iso':
            qemu += ' ovmf xorriso'
        return {'distribution': self.distribution, 'release': self.release, 'arch': self.arch,
                'os': runner, 'qemu': qemu}


def debian_target(release, series, build, arch, digest):
    root = f'https://cdimage.debian.org/cdimage/cloud/{series}/{build}'
    image_arch = {'x64': 'amd64', 'arm64': 'arm64'}[arch]
    return Target('debian', release, arch, f'{root}/debian-{release}-generic-{image_arch}-{build}.qcow2',
                  digest, 'sha512')


def mint_target(release, digest):
    url = f'https://pub.linuxmint.io/stable/{release}/linuxmint-{release}-cinnamon-64bit.iso'
    return Target('linuxmint', release, 'x64', url, digest, image_format='iso')


TARGETS = {
    'ubuntu': (
        Target('ubuntu', UBUNTU_BASELINE, 'x64', kernel_series='5.4'),
        Target('ubuntu', UBUNTU_BASELINE, 'arm64', kernel_series='5.4'),
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
}


def target_for(suite, release, arch):
    for target in TARGETS[suite]:
        if (target.release, target.arch) == (release, arch):
            return target
    raise ValueError(f'Unsupported Linux compatibility target: {suite} {release} {arch}')
