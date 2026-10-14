"""Scan Windows installers with native Defender; fail on threats or scan errors."""
import argparse
import os
from pathlib import Path
import re
import subprocess
import time

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


def update_signatures(timeout=300):
    deadline = time.monotonic() + timeout
    # MMPC is Microsoft's direct security-intelligence source. Avoid the
    # runner's Windows Update queue, and wait only for its explicit MSI-busy error.
    for delay in (15, 30, 45, 60, None):
        command = [str(defender_command()), '-SignatureUpdate', '-MMPC']
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise subprocess.TimeoutExpired(command, timeout)
        print('Updating Defender security intelligence:', command[0], flush=True)
        result = subprocess.run(
            command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, encoding='utf-8', errors='replace', timeout=remaining,
        )
        output = result.stdout or ''
        if output:
            print(output, end='' if output.endswith('\n') else '\n', flush=True)
        if result.returncode == 0:
            # The platform itself may have upgraded while we waited.
            return defender_command()
        busy = re.search(r'\b0x80070652\b', output.replace('\0', ''), re.IGNORECASE)
        if not busy or delay is None:
            raise subprocess.CalledProcessError(result.returncode, command, output=output)
        remaining = deadline - time.monotonic()
        if remaining <= delay:
            raise subprocess.TimeoutExpired(command, timeout, output=output)
        print(f'Defender installation is busy; retrying in {delay}s', flush=True)
        time.sleep(delay)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('installers', type=Path, nargs='+')
    parser.add_argument('--update-signatures', action='store_true')
    args = parser.parse_args()
    installers = [path.resolve(strict=True) for path in args.installers]
    if any(not path.is_file() or path.suffix.lower() != '.exe' for path in installers):
        raise ValueError('Expected Windows .exe installer files')
    command = update_signatures() if args.update_signatures else defender_command()
    print('Windows Defender:', command, flush=True)
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
