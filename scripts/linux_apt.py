"""Reuse DEB downloads only after checking the current signed APT indexes."""
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import stat
import subprocess
import tempfile
import time
from urllib.parse import urlsplit, urlunsplit

from build_support import host_arch, run, sha256

OPTIONS = ['apt-get', '-o', 'Acquire::http::Timeout=30', '-o',
           'Acquire::https::Timeout=30', '-o', 'Acquire::Retries=2', '-o',
           'Acquire::Languages=none', '-o', 'Dpkg::Use-Pty=0', '-o',
           'DPkg::Lock::Timeout=120']
UBUNTU_ARCHIVES = {'archive.ubuntu.com', 'security.ubuntu.com', 'ports.ubuntu.com'}
INDEX_RETRY_DELAYS = (5, 15)


def privileged(command, timeout, **kwargs):
    return run(['sudo', 'env', 'DEBIAN_FRONTEND=noninteractive', 'timeout',
                '--signal=TERM', '--kill-after=30s', f'{timeout}s', *command], **kwargs)


def archive_records(output):
    """APT --print-uris uses hashes from its authenticated package lists."""
    records = {}
    for line in output.splitlines():
        if not line.startswith("'"):
            continue
        fields = shlex.split(line)
        if fields and not fields[0].startswith(('http://', 'https://')):
            continue
        if len(fields) != 4:
            raise ValueError('Unexpected APT download record')
        uri, name, size, checksum = fields
        if (Path(name).name != name or not name.endswith('.deb')
                or not re.fullmatch(r'[A-Za-z0-9.+_%~:-]+\.deb', name)
                or not size.isdecimal()):
            raise ValueError('Invalid APT archive name or size')
        if not re.fullmatch(r'SHA256:[0-9a-fA-F]{64}', checksum):
            continue  # No strong repository hash: let APT fetch it normally.
        record = {'size': int(size), 'sha256': checksum.split(':', 1)[1].lower(), 'url': uri}
        if name in records and records[name] != record:
            raise ValueError('Conflicting APT archive records')
        records[name] = record
    return records


def matches(path, record):
    try:
        return (stat.S_ISREG(path.lstat().st_mode)
                and path.stat().st_size == record['size']
                and sha256(path) == record['sha256'])
    except OSError:
        return False


def install(packages, timeout=900, no_recommends=False, *, options=OPTIONS):
    arguments = ['install', '-y']
    if no_recommends:
        arguments.append('--no-install-recommends')
    arguments.extend(map(str, packages))
    configured = os.environ.get('COR_LINUX_APT_CACHE')
    if not configured:
        return privileged([*options, *arguments], timeout)
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('APT cache installation requires a disposable Actions environment')
    cache = Path(configured)
    try:
        cache.mkdir(parents=True, exist_ok=True)
    except OSError:
        print('::warning::APT cache is unavailable; installing without it.', flush=True)
        return privileged([*options, *arguments], timeout)
    # A separate empty archive directory makes print-uris include every needed
    # package, including downloads already present in our untrusted cache.
    directory = Path(tempfile.mkdtemp(prefix='cor-apt-archives-'))
    directory.chmod(0o755)
    command = [*options, '-o', 'Dir::Cache::archives='+str(directory),
               '-o', 'Acquire::ForceHash=SHA256',
               '-o', 'APT::Keep-Downloaded-Packages=true']
    records = {}
    try:
        try:
            query = privileged([*command, '--print-uris', *arguments], 120,
                               stdout=subprocess.PIPE, text=True)
            records = archive_records(query.stdout)
        except (OSError, ValueError, subprocess.SubprocessError):
            print('::warning::APT cache metadata unavailable; installing fresh downloads.',
                  flush=True)
            return privileged([*options, *arguments], timeout)
        restored, reused_bytes = 0, 0
        for name, record in records.items():
            source = cache/name
            try:
                if matches(source, record):
                    shutil.copyfile(source, directory/name)
                    restored += 1
                    reused_bytes += record['size']
                elif source.exists() or source.is_symlink():
                    source.unlink()
            except OSError:
                (directory/name).unlink(missing_ok=True)
                print('::warning::APT cached archive unavailable; downloading again.',
                      flush=True)
        print(f'APT download cache: {restored}/{len(records)} packages verified; '
              f'{reused_bytes / 1024**2:.1f} MiB reused', flush=True)
        return privileged([*command, *arguments], timeout)
    finally:
        # Never cache the application DEB or anything lacking a repository hash.
        # Failures in optional cache export must not hide installation results.
        try:
            for name, record in records.items():
                source = directory/name
                if matches(source, record):
                    temporary = cache/(name+'.partial')
                    shutil.copyfile(source, temporary)
                    temporary.replace(cache/name)
        except OSError as error:
            print('::warning::APT download cache export unavailable: '
                  + type(error).__name__, flush=True)
        try:
            privileged(['rm', '-rf', '--', str(directory)], 60)
        except (OSError, subprocess.SubprocessError):
            print('::warning::Temporary APT archive cleanup failed.', flush=True)


def update_indexes(timeout=300, *, options=OPTIONS):
    command = [*options, '-o', 'APT::Update::Error-Mode=any', 'update']
    for delay in (*INDEX_RETRY_DELAYS, 0):
        try:
            privileged(command, timeout)
            return
        except subprocess.CalledProcessError:
            if not delay:
                raise
            print(f'::warning::APT index refresh failed; retrying in {delay} seconds.', flush=True)
            time.sleep(delay)


def install_vm_tools(packages):
    release = platform.freedesktop_os_release()
    codename = {'24.04': 'noble', '26.04': 'resolute'}.get(release.get('VERSION_ID'))
    if (os.environ.get('GITHUB_ACTIONS') != 'true'
            or release.get('ID') != 'ubuntu' or codename is None):
        raise RuntimeError('VM tools require an Ubuntu 24.04 or 26.04 compatibility runner')
    arm = host_arch() == 'arm64'
    use_ports = arm and codename == 'noble'
    architecture = 'arm64' if arm else 'amd64'
    archive = 'https://ports.ubuntu.com/ubuntu-ports' if use_ports else 'https://archive.ubuntu.com/ubuntu'
    security = archive if use_ports else 'https://security.ubuntu.com/ubuntu'
    signing = '/usr/share/keyrings/ubuntu-archive-keyring.gpg'
    with tempfile.TemporaryDirectory(prefix='cor-host-apt-') as temporary:
        directory = Path(temporary)
        directory.chmod(0o755)
        sources = directory/'ubuntu.list'
        sources.write_text(''.join(
            f'deb [arch={architecture} signed-by={signing}] {origin} {suite} main universe restricted multiverse\n'
            for suite, origin in ((codename, archive), (f'{codename}-updates', archive),
                                  (f'{codename}-security', security))))
        options = [*OPTIONS, '-o', 'Dir::Etc::sourcelist='+str(sources),
                   '-o', 'Dir::Etc::sourceparts=-']
        update_indexes(360, options=options)
        install([*packages, 'qemu-utils', 'cloud-image-utils'], 1200, True, options=options)


def source_url(url):
    parsed = urlsplit(url)
    official = UBUNTU_ARCHIVES | {'azure.archive.ubuntu.com'}
    if (parsed.scheme in ('http', 'https') and parsed.hostname in official
            and parsed.netloc == parsed.hostname):
        hostname = 'archive.ubuntu.com' if parsed.hostname == 'azure.archive.ubuntu.com' else parsed.hostname
        return urlunsplit(('https', hostname, parsed.path, parsed.query, parsed.fragment))
    return url


def valid_source(url, checksum, size):
    try:
        parsed = urlsplit(url)
        return (parsed.scheme == 'https' and parsed.hostname in UBUNTU_ARCHIVES
                and not parsed.username and not parsed.password
                and not parsed.query and not parsed.fragment and parsed.port in (None, 443)
                and parsed.path.startswith('/') and not re.search(r'[\s\\|]', url)
                and re.fullmatch(r'[0-9a-f]{64}', checksum) is not None
                and type(size) is int and 0 < size <= 4 * 1024**3)
    except (AttributeError, TypeError, ValueError):
        return False
