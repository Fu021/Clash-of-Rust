"""Fixed guest releases and official cloud images for installed DEB checks."""
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
    uefi: bool = False

    @property
    def distribution(self):
        return self.family.capitalize()

    def matrix_entry(self):
        qemu = {'x64': 'qemu-system-x86', 'arm64': 'qemu-system-arm qemu-efi-aarch64'}[self.arch]
        if self.arch == 'x64' and self.uefi:
            qemu += ' ovmf'
        return {'distribution': self.distribution, 'release': self.release, 'arch': self.arch,
                'os': {'x64': 'ubuntu-24.04', 'arm64': 'ubuntu-22.04-arm'}[self.arch],
                'qemu': qemu}


def ubuntu_targets(release, series, build, amd64, arm64):
    root = f'https://cloud-images.ubuntu.com/releases/{series}/release-{build}'
    return tuple(Target('ubuntu', release, arch,
                        f'{root}/ubuntu-{release}-server-cloudimg-{deb_arch}.img', digest,
                        uefi=True)
                 for arch, deb_arch, digest in (('x64', 'amd64', amd64), ('arm64', 'arm64', arm64)))


def debian_target(release, series, build, digest):
    root = f'https://cdimage.debian.org/cdimage/cloud/{series}/{build}'
    return Target('debian', release, 'x64', f'{root}/debian-{release}-generic-amd64-{build}.qcow2',
                  digest, 'sha512')


TARGETS = {
    'ubuntu': (
        Target('ubuntu', UBUNTU_BASELINE, 'x64', kernel_series='5.4'),
        Target('ubuntu', UBUNTU_BASELINE, 'arm64', kernel_series='5.4'),
        *ubuntu_targets('22.04', 'jammy', '20261109',
                        '77d19e53656f87c95f1f6c6be59c789083b3f77fa08f9d88011ee04085c61f87',
                        'e429865ad84a083a84fdbd51bf6fd6e4174bbb1610ba0d72ca01014c656d45bc'),
        *ubuntu_targets('24.04', 'noble', '20261017',
                        '04e8f2461a16a44e78a370284471f29616cdd9baf1893df25807b71f985b6c94',
                        '50e24e1fb79e89116e939d90aa7738f7af48c442fa1bc2510131e0b39c50348e'),
        *ubuntu_targets('26.04', 'resolute', '20261108',
                        '7b90593f5532b49b7e3fe3d2743f0d1f0d822e5b07cebcd8e00c0df8069ecab9',
                        'cf3b8472b84e7de37cdc2d2e9a9bf82b3edf0ab97113a1422eff048f26688e2f'),
        *ubuntu_targets('26.10', 'stonking', '20261105',
                        '9ef2243a4319e43164ab11d61e324efa0dd19ed850979d10c3a6e44feb5684c5',
                        '90c07f4b58d7a06bc5525a41db4d32501e36fbab3e6ae05405b81e38c406d7a8'),
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
