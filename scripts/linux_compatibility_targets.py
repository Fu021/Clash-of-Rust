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

    @property
    def distribution(self):
        return self.family.capitalize()

    def matrix_entry(self):
        runner_release = self.release if self.hosted else '24.04'
        runner = f'ubuntu-{runner_release}' + ('-arm' if self.arch == 'arm64' else '')
        qemu = ''
        if not self.hosted:
            qemu = {'x64': 'qemu-system-x86', 'arm64': 'qemu-system-arm qemu-efi-aarch64'}[self.arch]
        return {'distribution': self.distribution, 'release': self.release, 'arch': self.arch,
                'os': runner, 'qemu': qemu}


def debian_target(release, series, build, digest):
    root = f'https://cdimage.debian.org/cdimage/cloud/{series}/{build}'
    return Target('debian', release, 'x64', f'{root}/debian-{release}-generic-amd64-{build}.qcow2',
                  digest, 'sha512')


TARGETS = {
    'ubuntu': (
        Target('ubuntu', UBUNTU_BASELINE, 'x64', kernel_series='5.4'),
        Target('ubuntu', UBUNTU_BASELINE, 'arm64', kernel_series='5.4'),
        *(Target('ubuntu', release, arch, hosted=True)
          for release in ('22.04', '24.04', '26.04') for arch in ('x64', 'arm64')),
    ),
    'debian': (
        debian_target('12', 'bookworm', '20261117-2665',
                      'd597a8b422a0fa7f7cf1a0477d23be7f7514f32e57ccb8d0c69156e1be4a7e1712'
                      'fb64aee6298520d70aa573b801d0cfe3392ab5de39829728760c0cf99ef4ec'),
        debian_target('13', 'trixie', '20261116-2664',
                      'e53a93ef33f828ceed0e7278c2cc2753bdc7d5098fbfae15bd64b6d878db350ea37'
                      '44247763a1c8ecb6b4019a0f623c38caf6038ad6f9c357fb4e01493ddf1a3'),
    ),
}


def target_for(suite, release, arch):
    for target in TARGETS[suite]:
        if (target.release, target.arch) == (release, arch):
            return target
    raise ValueError(f'Unsupported Linux compatibility target: {suite} {release} {arch}')
