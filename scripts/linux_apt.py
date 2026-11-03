"""Reuse DEB downloads only after checking the current signed APT indexes."""
import os
from pathlib import Path
import re
import shlex
import shutil
import stat
import subprocess
import tempfile

from build_support import run, sha256

OPTIONS = ['apt-get', '-o', 'Acquire::http::Timeout=30', '-o',
           'Acquire::https::Timeout=30', '-o', 'Acquire::Retries=2', '-o',
           'Acquire::Languages=none', '-o', 'Dpkg::Use-Pty=0']
MAX_CACHE_BYTES = 512 * 1024**2


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
        if len(fields) != 4:
            raise ValueError('Unexpected APT download record')
        uri, name, size, checksum = fields
        if not uri.startswith(('http://', 'https://')):
            continue  # Local application packages are checked by their receipt.
        if (Path(name).name != name or not name.endswith('.deb')
                or not re.fullmatch(r'[A-Za-z0-9.+_%~:-]+\.deb', name)
                or not size.isdecimal()):
            raise ValueError('Invalid APT archive name or size')
        if not re.fullmatch(r'SHA256:[0-9a-fA-F]{64}', checksum):
            continue  # No strong repository hash: let APT fetch it normally.
        record = {'size': int(size), 'sha256': checksum.split(':', 1)[1].lower()}
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


def trim(directory):
    size = 0
    files = [path for path in directory.glob('*.deb')
             if stat.S_ISREG(path.lstat().st_mode)]
    for path in sorted(files, key=lambda value: value.stat().st_mtime, reverse=True):
        length = path.stat().st_size
        if size + length > MAX_CACHE_BYTES:
            path.unlink()
        else:
            size += length


def install(packages, timeout=900, no_recommends=False):
    arguments = ['install', '-y']
    if no_recommends:
        arguments.append('--no-install-recommends')
    arguments.extend(map(str, packages))
    configured = os.environ.get('COR_LINUX_APT_CACHE')
    if not configured:
        return privileged([*OPTIONS, *arguments], timeout)
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('APT cache installation requires a disposable Actions environment')
    cache = Path(configured)
    try:
        cache.mkdir(parents=True, exist_ok=True)
    except OSError:
        print('::warning::APT cache is unavailable; installing without it.', flush=True)
        return privileged([*OPTIONS, *arguments], timeout)
    # A separate empty archive directory makes print-uris include every needed
    # package, including downloads already present in our untrusted cache.
    directory = Path(tempfile.mkdtemp(prefix='cor-apt-archives-'))
    directory.chmod(0o755)
    command = [*OPTIONS, '-o', 'Dir::Cache::archives='+str(directory),
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
            return privileged([*OPTIONS, *arguments], timeout)
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
            trim(cache)
        except OSError as error:
            print('::warning::APT download cache export unavailable: '
                  + type(error).__name__, flush=True)
        try:
            privileged(['rm', '-rf', '--', str(directory)], 60)
        except (OSError, subprocess.SubprocessError):
            print('::warning::Temporary APT archive cleanup failed.', flush=True)
