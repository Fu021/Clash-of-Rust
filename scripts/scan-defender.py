"""Scan Windows installers with native Defender; fail on threats or scan errors."""
import argparse
import os
from pathlib import Path
import re
import subprocess

from build_support import sha256


def defender_command():
    if os.name != 'nt':
        raise RuntimeError('Windows Defender scans require Windows')
    platform = Path(os.environ['ProgramData'])/'Microsoft/Windows Defender/Platform'
    candidates = sorted(
        platform.glob('*/MpCmdRun.exe'),
        key=lambda path: tuple(int(n) for n in re.findall(r'\d+', path.parent.name)),
        reverse=True,
    )
    candidates.append(Path(os.environ['ProgramFiles'])/'Windows Defender/MpCmdRun.exe')
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    raise RuntimeError('Windows Defender MpCmdRun.exe is unavailable; scan cannot be skipped')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('installers', type=Path, nargs='+')
    parser.add_argument('--update-signatures', action='store_true')
    args = parser.parse_args()
    installers = [path.resolve(strict=True) for path in args.installers]
    if any(not path.is_file() or path.suffix.lower() != '.exe' for path in installers):
        raise ValueError('Expected Windows .exe installer files')
    command = defender_command()
    print('Windows Defender:', command, flush=True)
    if args.update_signatures:
        subprocess.run([str(command), '-SignatureUpdate'], check=True, timeout=300)
    for installer in installers:
        digest = sha256(installer)
        print('Scanning:', installer.name, flush=True)
        # A custom scan ignores exclusions and scans archives. Keeping detected
        # files unmodified makes a threat return nonzero and blocks CI upload.
        subprocess.run([
            str(command), '-Scan', '-ScanType', '3', '-File', str(installer),
            '-DisableRemediation',
        ], check=True, timeout=300)
        if sha256(installer) != digest:
            raise RuntimeError('Installer changed during Defender scan: '+installer.name)
        print('PASS: Defender scan completed:', installer.name, digest, flush=True)


if __name__ == '__main__':
    main()
