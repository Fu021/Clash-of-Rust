#!/usr/bin/python3
"""Install pre-staged, hash-pinned payload. Does not start services or networking.

The trusted Windows administrator supplies an expected SHA256 manifest; it is
not a code signature. Do not accept an untrusted manifest alongside untrusted code.
Existing installation is accepted only if every installed digest still matches
its root-private receipt. No package download, WSL import or service start occurs.
"""
import argparse
from contextlib import contextmanager
import ctypes
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys
import tempfile

FILES = ('manager', 'manager.py', 'secure-egress', 'cor-egress-manager')
DEST = Path('/usr/local/lib/cor-egress')
BIN = Path('/usr/local/bin')
STATE = Path('/var/lib/cor-egress')
NETNS = Path('/run/netns')
RECEIPT = 'payload-receipt.json'


def digest(data):
    return hashlib.sha256(data).hexdigest()


def safe(path, directory=False):
    path = Path(path)
    if not path.is_absolute():
        raise ValueError('Absolute owned path required.')
    for item in [path, *path.parents]:
        st = item.lstat()
        if stat.S_ISLNK(st.st_mode) or st.st_uid != 0 or st.st_mode & 0o022:
            raise ValueError('Path is not root-owned and protected against replacement.')
    st = path.stat()
    if directory and not stat.S_ISDIR(st.st_mode):
        raise ValueError('Expected directory.')
    if not directory and (not stat.S_ISREG(st.st_mode) or st.st_nlink != 1):
        raise ValueError('Expected singly-linked regular file.')
    return path


def write(path, data, mode):
    fd, temp = tempfile.mkstemp(prefix='.cor-payload-', dir=path.parent)
    try:
        with os.fdopen(fd, 'wb') as out:
            os.fchmod(out.fileno(), mode)
            out.write(data)
            out.flush()
            os.fsync(out.fileno())
        os.replace(temp, path)
    finally:
        if os.path.exists(temp):
            os.unlink(temp)


def sync_directory(path):
    fd = os.open(path, os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def hashes(value):
    if (not isinstance(value, dict) or set(value) != set(FILES) or
            any(not isinstance(item, str) or not re.fullmatch(r'[0-9a-f]{64}', item)
                for item in value.values())):
        raise ValueError('Expected exactly four SHA256 payload digests.')
    return value


def private_file(path):
    path = safe(path)
    if path.stat().st_mode & 0o077:
        raise ValueError('Receipt, journal and lock files must be root-private.')
    return path


def installation(path):
    """Return a validated receipt and CAS token; archive code and receipt only."""
    safe(path, directory=True)
    if {entry.name for entry in path.iterdir()} != set(FILES) | {RECEIPT}:
        raise ValueError('Unexpected installed entries; code-only upgrade refused.')
    receipt_path = private_file(path / RECEIPT)
    if receipt_path.stat().st_size > 16384:
        raise ValueError('Oversized payload receipt.')
    raw = receipt_path.read_bytes()
    receipt = json.loads(raw)
    if not isinstance(receipt, dict) or receipt.get('owner') != 'cor-egress-payload-v1':
        raise ValueError('Existing installation ownership not recognized.')
    expected = hashes(receipt.get('files'))
    directory_stat = path.stat()
    token = [(directory_stat.st_dev, directory_stat.st_ino)]
    for name in (*FILES, RECEIPT):
        entry = private_file(path / name) if name == RECEIPT else safe(path / name)
        st = entry.stat()
        if st.st_size > 1024 * 1024:
            raise ValueError('Oversized installed payload.')
        data = entry.read_bytes()
        if name != RECEIPT and digest(data) != expected[name]:
            raise ValueError('Installed payload drift; no existing file overwritten.')
        if name == RECEIPT and data != raw:
            raise ValueError('Receipt changed during validation.')
        token.append((name, st.st_dev, st.st_ino, st.st_mode, st.st_uid, st.st_gid,
                      st.st_nlink, digest(data)))
    return expected, token


@contextmanager
def manager_lock():
    """Serialize installers and exclude every manager action through publication."""
    import fcntl
    safe(STATE.parent, directory=True)
    STATE.mkdir(mode=0o700, exist_ok=True)
    safe(STATE, directory=True)
    if STATE.stat().st_mode & 0o077:
        raise ValueError('Manager state directory must be root-private.')
    path = STATE / 'manager.lock'
    fd = os.open(path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    try:
        st = os.fstat(fd)
        if (not stat.S_ISREG(st.st_mode) or st.st_uid != 0 or
                st.st_mode & 0o077 or st.st_nlink != 1):
            raise ValueError('Unsafe manager lock.')
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        current = private_file(path).stat()
        if (current.st_dev, current.st_ino) != (st.st_dev, st.st_ino):
            raise ValueError('Manager lock changed during acquisition.')
        yield
    finally:
        os.close(fd)


def quiescent_state():
    path = private_file(STATE / 'state.json')
    if path.stat().st_size > 1024 * 1024:
        raise ValueError('Oversized manager journal.')
    raw = path.read_bytes()
    state = json.loads(raw)
    if (not isinstance(state, dict) or state.get('schema') != 1 or
            not isinstance(state.get('owner'), str) or not state['owner'] or
            state.get('phase') not in ('Guarded', 'RolledBack', 'DistroConfigured', 'DistroRestored') or
            not isinstance(state.get('namespaces', {}), dict) or state.get('namespaces', {})):
        raise ValueError('Upgrade requires a stable stopped manager with no workload namespaces.')
    if state['phase'] in ('Guarded', 'RolledBack') and 'namespaces' not in state:
        raise ValueError('Manager namespace inventory missing.')
    for name in ('cor-egress-gw', 'cor-egress-transport'):
        if os.path.lexists(NETNS / name):
            raise ValueError('Runtime namespace still exists; reviewed upgrade refused.')
    st = path.stat()
    return st.st_dev, st.st_ino, digest(raw)


def exchange_directories(stage, destination):
    # No two-rename fallback: readers must always see one complete generation.
    libc = ctypes.CDLL(None, use_errno=True)
    exchange = getattr(libc, 'renameat2', None)
    if exchange is None:
        raise ValueError('Atomic directory exchange is unavailable; upgrade refused.')
    exchange.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    exchange.restype = ctypes.c_int
    if exchange(-100, os.fsencode(stage), -100, os.fsencode(destination), 2):
        raise OSError(ctypes.get_errno(), 'Atomic payload exchange failed.')


def aliases():
    BIN.mkdir(mode=0o755, parents=True, exist_ok=True)
    safe(BIN, directory=True)
    missing = []
    for name in ('secure-egress', 'cor-egress-manager'):
        link = BIN / name
        expected = str(DEST / name)
        if link.is_symlink():
            if os.readlink(link) != expected or link.lstat().st_uid != 0:
                raise ValueError('Executable alias collision; existing entry preserved.')
        elif link.exists():
            raise ValueError('Executable alias collision; existing entry preserved.')
        else:
            missing.append((link, expected))
    return missing


def install(blobs, manifest, reviewed_upgrade=False):
    DEST.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
    safe(DEST.parent, directory=True)
    previous = None
    with manager_lock():
        aliases()  # Refuse existing collisions before publishing any code.
        exists = DEST.exists() or DEST.is_symlink()
        if exists:
            expected, token = installation(DEST)
            if expected != manifest and not reviewed_upgrade:
                raise ValueError('Different payload already installed; explicit reviewed upgrade required.')
            state_token = quiescent_state() if expected != manifest else None
        if not exists or expected != manifest:
            prefix = '.cor-egress-previous-' if exists else '.cor-egress-'
            stage = Path(tempfile.mkdtemp(prefix=prefix, dir=DEST.parent))
            # Failed stages are retained for diagnosis. Never copy profiles,
            # keys, manager state, or arbitrary installed directory contents.
            for name, data in blobs.items():
                write(stage / name, data, 0o755 if name != 'manager.py' else 0o644)
            write(stage / RECEIPT, json.dumps({'owner': 'cor-egress-payload-v1', 'files': manifest},
                                            sort_keys=True).encode(), 0o600)
            os.chmod(stage, 0o755)
            installation(stage)
            sync_directory(stage)
            aliases()
            if exists:
                if installation(DEST)[1] != token or quiescent_state() != state_token:
                    raise ValueError('Installation or manager state changed; CAS upgrade refused.')
                exchange_directories(stage, DEST)
                previous = str(stage)  # The same atomic operation retains the old code here.
            else:
                if DEST.exists() or DEST.is_symlink():
                    raise ValueError('Installation appeared during staging; no overwrite.')
                os.rename(stage, DEST)
            sync_directory(DEST.parent)
        for link, expected_target in aliases():
            os.symlink(expected_target, link)
        sync_directory(BIN)
    return previous


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--payload', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, required=True,
                        help='root-owned JSON object mapping the four exact filenames to SHA256')
    parser.add_argument('--reviewed-upgrade', action='store_true',
                        help='explicitly allow a reviewed version change while the manager is stopped')
    args = parser.parse_args(argv)
    try:
        if sys.platform != 'linux' or os.geteuid() != 0:
            raise ValueError('Linux root required.')
        os.umask(0o077)
        source = safe(args.payload, directory=True)
        manifest = hashes(json.loads(safe(args.manifest).read_bytes()))
        blobs = {}
        for name in FILES:
            data = safe(source / name).read_bytes()
            if len(data) > 1024 * 1024 or digest(data) != manifest[name]:
                raise ValueError('Payload size/hash mismatch.')
            if b'\r' in data:
                raise ValueError('Linux payload must use LF line endings; pin hashes after normalization.')
            blobs[name] = data
        previous = install(blobs, manifest, args.reviewed_upgrade)
        print(json.dumps({'schema': 1, 'schema_version': 1, 'action': 'install-payload',
                          'outcome': 'succeeded', 'protection': 'Unverified',
                          'previous_payload': previous,
                          'message': 'Hash-pinned payload installed. No networking or service was started.'}))
        return 0
    except (OSError, ValueError, KeyError, TypeError):
        print(json.dumps({'schema': 1, 'schema_version': 1, 'action': 'install-payload',
                          'outcome': 'blocked', 'protection': 'Blocked',
                          'message': 'Payload installation blocked. Owned directories retained; inspect the active receipt before retrying.'}))
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
