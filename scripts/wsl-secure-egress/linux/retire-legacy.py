#!/usr/bin/env python3
"""Retire only the known XUAN WSL services; retain original files and secrets.

Preview by default. The journal contains unit metadata only and may reside in
the project's ignored backup directory. The old Docker ExecStop is suppressed:
it launches privileged Docker containers and is not a safe cleanup operation.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

UNITS = (
    'xuan-claude-de-connect.service', 'xuan-claude-de-doh.service',
    'xuan-claude-de.service', 'xuan-docker-firewall.service',
    'xuan-claude-de-redsocks.service', 'xuan-claude-de-dns.service',
)
OVERRIDE = Path('/etc/apt/apt.conf.d/99-zz-cor-egress')
CONTENT = b'Acquire::http::Proxy "DIRECT";\nAcquire::https::Proxy "DIRECT";\n'
RUNTIME = Path('/run/systemd/system/xuan-docker-firewall.service.d/cor-egress-retire.conf')
RUNTIME_CONTENT = b'[Service]\nExecStop=\n'
ENV = {'PATH': '/usr/sbin:/usr/bin:/sbin:/bin', 'LANG': 'C', 'LC_ALL': 'C'}


def run(args, check=True):
    result = subprocess.run(args, env=ENV, capture_output=True, text=True, timeout=40)
    if check and result.returncode:
        raise RuntimeError('Known legacy operation failed; original files retained.')
    return result


def inventory():
    result = []
    for unit in UNITS:
        path = Path('/etc/systemd/system')/unit
        if path.is_symlink():
            raise ValueError('Legacy unit is redirected; review before retirement.')
        if not path.exists():
            continue
        if not path.is_file() or path.stat().st_uid != 0 or path.stat().st_mode & 0o022:
            raise ValueError('Legacy unit ownership is unsafe.')
        result.append(dict(unit=unit, sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                           enabled=run(['systemctl', 'is-enabled', unit], False).stdout.strip(),
                           active=run(['systemctl', 'is-active', unit], False).stdout.strip()))
    return result


def save(path, value, exclusive=False):
    if path.is_symlink():
        raise ValueError('Redirected journal.')
    flags = os.O_WRONLY | os.O_CREAT | os.O_NOFOLLOW | (os.O_EXCL if exclusive else os.O_TRUNC)
    descriptor = os.open(path, flags, 0o600)
    with os.fdopen(descriptor, 'w') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())


def write_owned(path, data):
    if path.exists() or path.is_symlink():
        if path.is_symlink() or path.read_bytes() != data:
            raise ValueError('Owned retirement filename is already occupied.')
        return
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644)
    with os.fdopen(descriptor, 'wb') as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--journal', type=Path, required=True)
    parser.add_argument('--execute', action='store_true')
    parser.add_argument('--rollback', action='store_true')
    args = parser.parse_args()
    if os.geteuid() != 0:
        raise ValueError('Run as Linux root.')
    if not args.execute:
        return dict(outcome='not_run', units=inventory(), original_files_retained=True)
    if args.rollback:
        state = json.loads(args.journal.read_text())
        if state.get('owner') != 'ClashOfRust.LegacyRetirement' or state.get('schema') != 1:
            raise ValueError('Unknown retirement journal.')
        current = {entry['unit']: entry for entry in inventory()}
        for entry in state['units']:
            if current.get(entry['unit'], {}).get('sha256') != entry['sha256']:
                raise ValueError('Legacy unit changed; restore refused.')
        if state.get('apt_override_created'):
            if OVERRIDE.is_symlink() or OVERRIDE.read_bytes() != CONTENT:
                raise ValueError('APT override changed; restore refused.')
            OVERRIDE.unlink()
        for entry in state['units']:
            if entry['enabled'] == 'enabled':
                run(['systemctl', 'enable', entry['unit']])
        if state.get('runtime_override_created') and RUNTIME.exists():
            if RUNTIME.is_symlink() or RUNTIME.read_bytes() != RUNTIME_CONTENT:
                raise ValueError('Runtime override changed; restore refused.')
            RUNTIME.unlink()
        run(['systemctl', 'daemon-reload'])
        # Start the old firewall before its dependent services.
        order = ('xuan-claude-de.service', *(x for x in UNITS if x != 'xuan-claude-de.service'))
        for unit in order:
            entry = next((x for x in state['units'] if x['unit'] == unit), None)
            if entry and entry['active'] == 'active':
                run(['systemctl', 'start', unit])
        state['phase'] = 'restored'
    else:
        state = dict(schema=1, owner='ClashOfRust.LegacyRetirement', phase='prepared',
                     units=inventory(), apt_override_created=not OVERRIDE.exists(),
                     runtime_override_created=not RUNTIME.exists())
        save(args.journal, state, exclusive=True)
        RUNTIME.parent.mkdir(parents=True, exist_ok=True)
        write_owned(RUNTIME, RUNTIME_CONTENT)
        run(['systemctl', 'daemon-reload'])
        for entry in state['units']:
            run(['systemctl', 'disable', entry['unit']])
        # The known firewall unit's ExecStop removes only its exact legacy
        # tuples. Docker ExecStop was cleared above and cannot launch containers.
        for entry in state['units']:
            run(['systemctl', 'stop', entry['unit']])
        write_owned(OVERRIDE, CONTENT)
        after = inventory()
        if any(x['enabled'] == 'enabled' or x['active'] == 'active' for x in after):
            raise RuntimeError('A legacy service remains active or enabled.')
        state['phase'] = 'retired'
    save(args.journal, state)
    return dict(outcome='succeeded', phase=state['phase'], units=len(state['units']),
                original_files_retained=True, credential_files_copied=False)


if __name__ == '__main__':
    try:
        print(json.dumps(main()))
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.SubprocessError):
        print(json.dumps(dict(outcome='failed', message='Inspect the metadata journal; original files retained.')))
        sys.exit(1)
