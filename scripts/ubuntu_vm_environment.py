"""Freeze disposable Ubuntu guest environments independently of cor builds."""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import runpy
import shutil
import subprocess
import sys
import urllib.parse

from build_support import ROOT, run, sha256
from linux_apt import OPTIONS, archive_records, source_url, valid_source

ARCHITECTURES = {'arm64': 'arm64', 'x64': 'amd64'}
PREPARATION_ERRORS = (OSError, ValueError, RuntimeError, KeyError, TypeError, subprocess.SubprocessError)

TEST_PACKAGES = (
    'git', 'dbus-x11', 'iproute2', 'util-linux', 'libcap2-bin',
    'xvfb', 'xwayland', 'x11-utils', 'xdotool', 'wmctrl', 'openbox',
    'gnome-shell', 'gnome-shell-extension-appindicator', 'policykit-1-gnome',
    'libgl1-mesa-dri', 'libegl1',
)


def require(condition, message='Invalid frozen Ubuntu VM environment'):
    if not condition:
        raise ValueError(message)


def fields(value, names):
    require(type(value) is dict and set(value) == set(names.split()))


def integer(value, minimum=1, maximum=2**53 - 1):
    require(type(value) is int and minimum <= value <= maximum)


def text(value, pattern, minimum=1, maximum=2048):
    require(type(value) is str and minimum <= len(value) <= maximum)
    require(re.fullmatch(pattern, value, flags=re.ASCII) is not None)


def digest(value):
    text(value, '[0-9a-f]{64}', 64, 64)


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True,
                      allow_nan=False).encode('ascii')


def checksum(value):
    return hashlib.sha256(canonical(value)).hexdigest()


def validate_descriptor(descriptor):
    fields(descriptor, 'schema kind distribution release arch kernel_series base_sha256 '
           'python_sha256 environment_lock_sha256 prepare_recipe_sha256')
    require(type(descriptor['schema']) is int and descriptor['schema'] == 1)
    for name, value in {'kind': 'ubuntu-compat', 'distribution': 'ubuntu',
                        'release': '20.04', 'kernel_series': '5.4'}.items():
        require(descriptor[name] == value)
    require(type(descriptor['arch']) is str)
    require(descriptor['arch'] in ARCHITECTURES, 'unsupported_image')
    for name in ('base_sha256', 'python_sha256', 'environment_lock_sha256', 'prepare_recipe_sha256'):
        digest(descriptor[name])
    require(descriptor['base_sha256'] != descriptor['python_sha256'])


def validate_packages(packages, arch='arm64'):
    require(arch in ARCHITECTURES, 'unsupported_image')
    require(type(packages) is list and 1 <= len(packages) <= 4096)
    identities = []
    for package in packages:
        fields(package, 'name version architecture')
        text(package['name'], '[a-z0-9][a-z0-9+.-]+', 2, 128)
        text(package['version'], '[0-9][A-Za-z0-9.+:~-]*', maximum=256)
        require(package['architecture'] in (ARCHITECTURES[arch], 'all'))
        identities.append((package['name'], package['architecture']))
    require(identities == sorted(set(identities)))


def sources(arch):
    require(arch in ARCHITECTURES, 'unsupported_image')
    base = ('https://ports.ubuntu.com/ubuntu-ports' if arch == 'arm64'
            else 'https://archive.ubuntu.com/ubuntu')
    result = []
    for suite in ('focal', 'focal-updates', 'focal-security'):
        origin = 'https://security.ubuntu.com/ubuntu' if arch == 'x64' and suite == 'focal-security' else base
        result.append(f'deb [arch={ARCHITECTURES[arch]} '
                      f'signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] '
                      f'{origin} {suite} main universe restricted multiverse\n')
    return ''.join(result)


def recipe(arch='arm64'):
    return checksum({'schema': 1, 'recipe_sha256': sha256(Path(__file__)),
                              'test_packages': list(TEST_PACKAGES), 'sources': sources(arch)})


def package_record(filename, arch='arm64'):
    parts = urllib.parse.unquote(filename.removesuffix('.deb')).split('_')
    require(len(parts) == 3)
    name, version, architecture = parts
    text(name, '[a-z0-9][a-z0-9+.-]+', 2, 128)
    text(version, '[0-9][A-Za-z0-9.+:~-]*', maximum=256)
    require(architecture in (ARCHITECTURES[arch], 'all'))
    return {'name': name, 'version': version, 'architecture': architecture}


def packages_from_plan(records, arch='arm64'):
    packages = [package_record(name, arch) for name in records]
    result = sorted(packages, key=lambda package: (package['name'], package['architecture']))
    require(len({(entry['name'], entry['architecture']) for entry in result}) == len(result))
    return result


def frozen_environments(snapshot):
    if 'ubuntu_vm_environments' not in snapshot:
        return {}
    value = snapshot['ubuntu_vm_environments']
    require(type(value) is dict and set(value) == set(ARCHITECTURES))
    return value


def freeze(directory, linux, arch='arm64'):
    require(arch in ARCHITECTURES, 'unsupported_image')
    if os.environ.get('COR_UBUNTU_VM_INPUTS') != 'true':
        return {'schema': 1, 'status': 'disabled', 'reason': 'not_requested'}
    if os.environ.get('GITHUB_ACTIONS') != 'true' or sys.platform != 'linux':
        raise RuntimeError('VM environment resolution requires a disposable Actions Linux runner')
    directory = Path(directory)
    if not linux:
        return {'schema': 1, 'status': 'disabled', 'reason': 'linux_inputs_unavailable'}
    try:
        staging = directory / 'vm-environment' / arch
        apt = staging / 'apt'
        (apt / 'lists/partial').mkdir(parents=True, exist_ok=True)
        (apt / 'archives/partial').mkdir(parents=True, exist_ok=True)
        (apt / 'status').write_text('')
        (apt / 'sources.list').write_text(sources(arch))
        native = ARCHITECTURES[arch]
        (apt / 'isolated.conf').write_text(
            'Dir::Etc::parts "-";\nDir::Etc::main "-";\n'
            f'APT::Architecture "{native}";\nAPT::Architectures {{ "{native}"; }};\n')
        apt_environment = {**os.environ, 'APT_CONFIG': str((apt / 'isolated.conf').resolve()),
                           'LC_ALL': 'C'}
        command = [
            *OPTIONS, '-o', 'APT::Architecture=' + native, '-o', 'APT::Architectures::=' + native,
            '-o', 'Dir::Etc::sourcelist=' + str((apt / 'sources.list').resolve()),
            '-o', 'Dir::Etc::sourceparts=-', '-o', 'Dir::Etc::preferencesparts=-',
            '-o', 'Dir::Etc::preferences=-',
            '-o', 'Dir::State::lists=' + str((apt / 'lists').resolve()),
            '-o', 'Dir::State::status=' + str((apt / 'status').resolve()),
            '-o', 'Dir::Cache::archives=' + str((apt / 'archives').resolve()),
            '-o', 'Dir::Cache::pkgcache=', '-o', 'Dir::Cache::srcpkgcache=',
            '-o', 'Acquire::GzipIndexes=false',
            '-o', 'Acquire::IndexTargets::deb::DEP-11::DefaultEnabled=false',
        ]
        run([*command, '-o', 'APT::Update::Error-Mode=any', 'update'],
            timeout=300, env=apt_environment)
        installer = runpy.run_path(str(ROOT / 'scripts/build-installer.py'))
        download_packages = list(dict.fromkeys([
            *TEST_PACKAGES,
            *(entry.split('|', 1)[0].strip() for entry in installer['GUI_DEPENDENCIES']),
        ]))

        def plan(packages):
            result = run([*command, '-o', 'Acquire::ForceHash=SHA256', '--print-uris',
                          'install', '-y', '--no-install-recommends', *packages],
                         stdout=subprocess.PIPE, text=True, timeout=120, env=apt_environment)
            records = archive_records(result.stdout)
            require(0 < len(records) <= 4096)
            for record in records.values():
                record['url'] = source_url(record['url'])
                require(valid_source(record['url'], record['sha256'], record['size']))
            resolved = {entry['name'] for entry in packages_from_plan(records, arch)}
            require(set(packages).issubset(resolved))
            return records

        installed = plan(TEST_PACKAGES)
        downloads = plan(download_packages)
        for name, record in installed.items():
            require(name not in downloads or downloads[name] == record)
            downloads[name] = record
        lock = {'schema': 1, 'arch': arch, 'packages': packages_from_plan(installed, arch),
                'downloads': downloads, 'sources_list': sources(arch)}
        descriptor = {'schema': 1, 'kind': 'ubuntu-compat', 'distribution': 'ubuntu',
                      'release': '20.04', 'arch': arch, 'kernel_series': '5.4',
                      'base_sha256': linux['architectures'][arch]['cloud']['sha256'],
                      'python_sha256': linux['architectures'][arch]['python']['sha256'],
                      'environment_lock_sha256': checksum(lock),
                      'prepare_recipe_sha256': recipe(arch)}
        paths = [apt / 'sources.list', *(apt / 'lists').glob('*')]
        files = {path.relative_to(directory).as_posix(): sha256(path)
                 for path in paths if path.is_file() and path.name != 'lock'}
        require(len(files) >= 4 and sum((directory / name).stat().st_size
                                               for name in files) <= 256 * 1024**2)
        return {'schema': 1, 'status': 'frozen', 'descriptor': descriptor, 'lock': lock, 'files': files}
    except PREPARATION_ERRORS as error:
        shutil.rmtree(directory / 'vm-environment' / arch, ignore_errors=True)
        print('Ubuntu VM inputs: ' + arch + ': cold fallback: ' + type(error).__name__, flush=True)
        return {'schema': 1, 'status': 'disabled', 'reason': 'resolution_failed'}


def verify_frozen(directory, frozen, linux):
    require(type(frozen) is dict and type(frozen.get('schema')) is int and frozen['schema'] == 1)
    if frozen.get('status') == 'disabled':
        return False
    require(frozen.get('status') == 'frozen')
    fields(frozen, 'schema status descriptor lock files')
    descriptor, lock = frozen['descriptor'], frozen['lock']
    validate_descriptor(descriptor)
    arch = descriptor['arch']
    fields(lock, 'schema arch packages downloads sources_list')
    require(type(lock['schema']) is int and lock['schema'] == 1)
    validate_packages(lock['packages'], arch)
    require(type(lock['downloads']) is dict and 0 < len(lock['downloads']) <= 4096)
    for name, record in lock['downloads'].items():
        text(name, r'[A-Za-z0-9.+_%~:-]+\.deb', maximum=512)
        fields(record, 'url size sha256')
        integer(record['size'])
        digest(record['sha256'])
        require(valid_source(record['url'], record['sha256'], record['size']))
    downloaded = {(entry['name'], entry['version'], entry['architecture'])
                  for entry in packages_from_plan(lock['downloads'], arch)}
    require(all((entry['name'], entry['version'], entry['architecture']) in downloaded
                         for entry in lock['packages']))
    require(descriptor['environment_lock_sha256'] == checksum(lock)
                     and descriptor['prepare_recipe_sha256'] == recipe(arch)
                     and descriptor['base_sha256'] == linux['architectures'][arch]['cloud']['sha256']
                     and descriptor['python_sha256'] == linux['architectures'][arch]['python']['sha256'])
    require(lock['arch'] == arch and lock['sources_list'] == sources(arch)
                     and 0 < len(lock['packages']) <= 4096 and type(frozen['files']) is dict
                     and 4 <= len(frozen['files']) <= 64)
    directory = Path(directory).resolve()
    for name, expected in frozen['files'].items():
        relative = PurePosixPath(name)
        require(str(relative) == name and not relative.is_absolute()
                         and '..' not in relative.parts
                         and relative.parts[:2] == ('vm-environment', arch))
        target = directory / name
        require(not target.is_symlink() and target.resolve().is_relative_to(directory)
                         and target.is_file() and sha256(target) == expected)
    source = f'vm-environment/{arch}/apt/sources.list'
    require(source in frozen['files']
                     and (directory / source).read_text() == lock['sources_list'])
    return True


def guest_only(arch=None):
    machine = platform.machine().lower()
    if (os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('COR_LINUX_GUEST') != 'true'
            or machine not in ('arm64', 'aarch64', 'x86_64', 'amd64')):
        raise RuntimeError('Ubuntu VM commands require the disposable Actions guest')
    if arch is not None:
        require(arch == ('arm64' if machine in ('arm64', 'aarch64') else 'x64'),
                         'environment_mismatch')


def install_frozen(directory, frozen, linux):
    guest_only(frozen['descriptor']['arch'])
    verify_frozen(directory, frozen, linux)
    apt = Path(directory) / 'vm-environment' / frozen['descriptor']['arch'] / 'apt'
    run(['sudo', 'sh', '-c',
         'for unit in apt-daily.timer apt-daily-upgrade.timer apt-daily.service '
         'apt-daily-upgrade.service unattended-upgrades.service; do '
         'if systemctl cat "$unit" >/dev/null 2>&1; then '
         'systemctl mask --now "$unit" || exit "$?"; fi; done'], timeout=180)
    run(['sudo', 'mkdir', '-p', '/etc/apt/cor-sources-disabled'])
    run(['sudo', 'sh', '-c',
         'set -eu; for entry in /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; '
         'do [ ! -f \"$entry\" ] || mv \"$entry\" /etc/apt/cor-sources-disabled/; done'])
    run(['sudo', 'cp', str(apt / 'sources.list'), '/etc/apt/sources.list'])
    run(['sudo', 'find', '/var/lib/apt/lists', '-maxdepth', '1', '-type', 'f', '-delete'])
    for name in frozen['files']:
        path = Path(directory) / name
        if path.parent.name == 'lists':
            if path.name.endswith('_InRelease'):
                run(['gpgv', '--keyring', '/usr/share/keyrings/ubuntu-archive-keyring.gpg', path],
                    timeout=30)
            run(['sudo', 'cp', path, '/var/lib/apt/lists/' + path.name])
    module = runpy.run_path(str(ROOT / 'scripts/test-release-package.py'))
    module['prepare_guest_package_environment']()
    pins = [package['name'] + ':' + package['architecture'] + '=' + package['version']
            if package['architecture'] != 'all' else package['name'] + '=' + package['version']
            for package in frozen['lock']['packages']]
    from linux_apt import install
    install(pins, 1800, no_recommends=True)


def inspect_environment():
    guest_only()
    release = dict(line.split('=', 1) for line in Path('/etc/os-release').read_text().splitlines()
                   if '=' in line)
    require(release.get('ID', '').strip('\"') == 'ubuntu'
                     and release.get('VERSION_ID', '').strip('\"') == '20.04')
    output = subprocess.check_output(
        ['dpkg-query', '-W', '-f=${Package}\\t${Version}\\t${Architecture}\\t${db:Status-Status}\\n'],
        text=True)
    packages = []
    for line in output.splitlines():
        name, version, architecture, status = line.split('\t')
        if status == 'installed':
            require(name not in ('clash-of-rust', 'cor'), 'application_present')
            packages.append({'name': name, 'version': version, 'architecture': architecture})
    packages.sort(key=lambda package: (package['name'], package['architecture']))
    return {'kernel_release': platform.release(),
            'python_version': subprocess.check_output(
                ['/opt/cor-test-python/bin/python3', '-c',
                 'import platform; print(platform.python_version())'], text=True).strip(),
            'packages': packages, 'packages_sha256': checksum(packages)}


def check_environment(actual, lock):
    require(actual['kernel_release'].startswith('5.4.')
                     and actual['packages_sha256'] == checksum(actual['packages']),
                     'environment_mismatch')
    installed = {(entry['name'], entry['architecture']): entry['version'] for entry in actual['packages']}
    require(all(installed.get((entry['name'], entry['architecture'])) == entry['version']
                         for entry in lock['packages']), 'environment_mismatch')
