"""Freeze disposable Ubuntu guest environments independently of cor builds."""
import os
from pathlib import Path, PurePosixPath
import platform
import runpy
import shutil
import subprocess
import sys
import urllib.parse

from build_support import ROOT, run, sha256, write_json
from linux_apt import OPTIONS, archive_records, source_url
import prepared_vm as protocol

TEST_PACKAGES = (
    'git', 'dbus-x11', 'iproute2', 'util-linux', 'libcap2-bin',
    'xvfb', 'xwayland', 'x11-utils', 'xdotool', 'wmctrl', 'openbox',
    'gnome-shell', 'gnome-shell-extension-appindicator', 'policykit-1-gnome',
    'libgl1-mesa-dri', 'libegl1',
)
def sources(arch):
    protocol.require(arch in protocol.ARCHITECTURES, 'unsupported_image')
    base = ('https://ports.ubuntu.com/ubuntu-ports' if arch == 'arm64'
            else 'https://archive.ubuntu.com/ubuntu')
    result = []
    for suite in ('focal', 'focal-updates', 'focal-security'):
        origin = 'https://security.ubuntu.com/ubuntu' if arch == 'x64' and suite == 'focal-security' else base
        result.append(f'deb [arch={protocol.ARCHITECTURES[arch]} '
                      f'signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] '
                      f'{origin} {suite} main universe restricted multiverse\n')
    return ''.join(result)


def recipe(arch='arm64'):
    return protocol.checksum({'schema': 1, 'recipe_sha256': sha256(Path(__file__)),
                              'test_packages': list(TEST_PACKAGES), 'sources': sources(arch)})


def package_record(filename, arch='arm64'):
    parts = urllib.parse.unquote(filename.removesuffix('.deb')).split('_')
    protocol.require(len(parts) == 3)
    name, version, architecture = parts
    protocol.text(name, '[a-z0-9][a-z0-9+.-]+', 2, 128)
    protocol.text(version, '[0-9][A-Za-z0-9.+:~-]*', maximum=256)
    protocol.require(architecture in (protocol.ARCHITECTURES[arch], 'all'))
    return {'name': name, 'version': version, 'architecture': architecture}


def packages_from_plan(records, arch='arm64'):
    packages = [package_record(name, arch) for name in records]
    result = sorted(packages, key=lambda package: (package['name'], package['architecture']))
    protocol.require(len({(entry['name'], entry['architecture']) for entry in result}) == len(result))
    return result


def frozen_environments(snapshot):
    if 'prepared_vms' in snapshot:
        value = snapshot['prepared_vms']
        protocol.require(type(value) is dict and set(value) == set(protocol.ARCHITECTURES))
        return value
    return {'arm64': snapshot['prepared_vm']} if 'prepared_vm' in snapshot else {}


def freeze(directory, linux, arch='arm64'):
    protocol.require(arch in protocol.ARCHITECTURES, 'unsupported_image')
    if os.environ.get('COR_PREPARED_VM_INPUTS') != 'true':
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
        native = protocol.ARCHITECTURES[arch]
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
            protocol.require(0 < len(records) <= 4096)
            for record in records.values():
                record['url'] = source_url(record['url'])
                protocol.require(protocol.valid_source(record['url'], record['sha256'], record['size']))
            resolved = {entry['name'] for entry in packages_from_plan(records, arch)}
            protocol.require(set(packages).issubset(resolved))
            return records

        installed = plan(TEST_PACKAGES)
        downloads = plan(download_packages)
        for name, record in installed.items():
            protocol.require(name not in downloads or downloads[name] == record)
            downloads[name] = record
        lock = {'schema': 1, 'arch': arch, 'packages': packages_from_plan(installed, arch),
                'downloads': downloads, 'sources_list': sources(arch)}
        descriptor = {'schema': 1, 'kind': 'ubuntu-compat', 'distribution': 'ubuntu',
                      'release': '20.04', 'arch': arch, 'kernel_series': '5.4',
                      'base_sha256': linux['architectures'][arch]['cloud']['sha256'],
                      'python_sha256': linux['architectures'][arch]['python']['sha256'],
                      'environment_lock_sha256': protocol.checksum(lock),
                      'prepare_recipe_sha256': recipe(arch)}
        paths = [apt / 'sources.list', *(apt / 'lists').glob('*')]
        files = {path.relative_to(directory).as_posix(): sha256(path)
                 for path in paths if path.is_file() and path.name != 'lock'}
        protocol.require(len(files) >= 4 and sum((directory / name).stat().st_size
                                               for name in files) <= 256 * 1024**2)
        return {'schema': 1, 'status': 'frozen', 'descriptor': descriptor, 'lock': lock, 'files': files}
    except protocol.ERRORS as error:
        shutil.rmtree(directory / 'vm-environment' / arch, ignore_errors=True)
        print('Prepared VM inputs: ' + arch + ': cold fallback: ' + type(error).__name__, flush=True)
        return {'schema': 1, 'status': 'disabled', 'reason': 'resolution_failed'}


def verify_frozen(directory, frozen, linux):
    protocol.require(type(frozen) is dict and type(frozen.get('schema')) is int and frozen['schema'] == 1)
    if frozen.get('status') == 'disabled':
        return False
    protocol.require(frozen.get('status') == 'frozen')
    protocol.fields(frozen, 'schema status descriptor lock files')
    descriptor, lock = frozen['descriptor'], frozen['lock']
    protocol.validate_descriptor(descriptor)
    arch = descriptor['arch']
    protocol.fields(lock, 'schema arch packages downloads sources_list')
    protocol.require(type(lock['schema']) is int and lock['schema'] == 1)
    protocol.validate_packages(lock['packages'], arch)
    protocol.require(type(lock['downloads']) is dict and 0 < len(lock['downloads']) <= 4096)
    for name, record in lock['downloads'].items():
        protocol.text(name, r'[A-Za-z0-9.+_%~:-]+\.deb', maximum=512)
        protocol.fields(record, 'url size sha256')
        protocol.integer(record['size'])
        protocol.digest(record['sha256'])
        protocol.require(protocol.valid_source(record['url'], record['sha256'], record['size']))
    downloaded = {(entry['name'], entry['version'], entry['architecture'])
                  for entry in packages_from_plan(lock['downloads'], arch)}
    protocol.require(all((entry['name'], entry['version'], entry['architecture']) in downloaded
                         for entry in lock['packages']))
    protocol.require(descriptor['environment_lock_sha256'] == protocol.checksum(lock)
                     and descriptor['prepare_recipe_sha256'] == recipe(arch)
                     and descriptor['base_sha256'] == linux['architectures'][arch]['cloud']['sha256']
                     and descriptor['python_sha256'] == linux['architectures'][arch]['python']['sha256'])
    protocol.require(lock['arch'] == arch and lock['sources_list'] == sources(arch)
                     and 0 < len(lock['packages']) <= 4096 and type(frozen['files']) is dict
                     and 4 <= len(frozen['files']) <= 64)
    directory = Path(directory).resolve()
    for name, expected in frozen['files'].items():
        relative = PurePosixPath(name)
        protocol.require(str(relative) == name and not relative.is_absolute()
                         and '..' not in relative.parts
                         and relative.parts[:2] == ('vm-environment', arch))
        target = directory / name
        protocol.require(not target.is_symlink() and target.resolve().is_relative_to(directory)
                         and target.is_file() and sha256(target) == expected)
    source = f'vm-environment/{arch}/apt/sources.list'
    protocol.require(source in frozen['files']
                     and (directory / source).read_text() == lock['sources_list'])
    return True


def guest_only(arch=None):
    machine = platform.machine().lower()
    if (os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('COR_LINUX_GUEST') != 'true'
            or machine not in ('arm64', 'aarch64', 'x86_64', 'amd64')):
        raise RuntimeError('Prepared VM operations require the disposable Actions guest')
    if arch is not None:
        protocol.require(arch == ('arm64' if machine in ('arm64', 'aarch64') else 'x64'),
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
    protocol.require(release.get('ID', '').strip('\"') == 'ubuntu'
                     and release.get('VERSION_ID', '').strip('\"') == '20.04')
    output = subprocess.check_output(
        ['dpkg-query', '-W', '-f=${Package}\\t${Version}\\t${Architecture}\\t${db:Status-Status}\\n'],
        text=True)
    packages = []
    for line in output.splitlines():
        name, version, architecture, status = line.split('\t')
        if status == 'installed':
            protocol.require(name not in ('clash-of-rust', 'cor'), 'application_present')
            packages.append({'name': name, 'version': version, 'architecture': architecture})
    packages.sort(key=lambda package: (package['name'], package['architecture']))
    return {'kernel_release': platform.release(),
            'python_version': subprocess.check_output(
                ['/opt/cor-test-python/bin/python3', '-c',
                 'import platform; print(platform.python_version())'], text=True).strip(),
            'packages': packages, 'packages_sha256': protocol.checksum(packages)}


def check_environment(actual, expected, lock):
    for name in ('kernel_release', 'python_version', 'packages', 'packages_sha256'):
        if expected is not None:
            protocol.require(actual[name] == expected[name], 'environment_mismatch')
    protocol.require(actual['kernel_release'].startswith('5.4.')
                     and actual['packages_sha256'] == protocol.checksum(actual['packages']),
                     'environment_mismatch')
    installed = {(entry['name'], entry['architecture']): entry['version'] for entry in actual['packages']}
    protocol.require(all(installed.get((entry['name'], entry['architecture'])) == entry['version']
                         for entry in lock['packages']), 'environment_mismatch')


def check_sources(environment, frozen):
    sources = {entry['sha256']: entry['size'] for entry in environment['sources']}
    downloads = {entry['sha256']: entry['size'] for entry in frozen['lock']['downloads'].values()}
    originals = {frozen['descriptor'][name] for name in ('base_sha256', 'python_sha256')}
    protocol.require(set(sources) == set(downloads) | originals
                     and all(sources.get(checksum) == size for checksum, size in downloads.items()),
                     'environment_mismatch')


def clean_template():
    guest_only()
    protocol.require(not Path('/opt/clash-of-rust').exists(), 'application_present')
    run(['sudo', 'cloud-init', 'clean', '--logs'], timeout=60)
    run(['sudo', 'passwd', '-l', 'ubuntu'], timeout=30)
    run(['sudo', 'sh', '-c',
         'set -eu; cd /; rm -rf /home/ubuntu/cor-source /home/ubuntu/cor-runner-temp '
         '/home/ubuntu/.ssh /home/ubuntu/.cache /home/ubuntu/.bash_history '
         '/home/ubuntu/cor-apt-plan* /root/.ssh /root/.bash_history '
         '/var/lib/cloud/seed /tmp/* /var/tmp/*; '
         'rm -f /etc/ssh/ssh_host_* /var/lib/systemd/random-seed; '
         'rm -f /var/lib/dbus/machine-id; ln -s /etc/machine-id /var/lib/dbus/machine-id; '
         'truncate -s 0 /etc/machine-id; '
         'find /var/log -type f -exec truncate -s 0 {} +; fstrim /'], timeout=270)
