#!/usr/bin/python3
"""Install pre-staged, hash-pinned payload. Does not start services or networking.

The trusted Windows administrator supplies an expected SHA256 manifest; it is
not a code signature. Do not accept an untrusted manifest alongside untrusted code.
Existing installation is accepted only if every installed digest still matches
its root-private receipt. No package download, WSL import or service start occurs.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import sys
import tempfile

FILES = ('manager', 'manager.py', 'secure-egress', 'cor-egress-manager')
DEST = Path('/usr/local/lib/cor-egress')
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--payload', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, required=True,
                        help='root-owned JSON object mapping the four exact filenames to SHA256')
    args = parser.parse_args()
    try:
        if sys.platform != 'linux' or os.geteuid() != 0:
            raise ValueError('Linux root required.')
        os.umask(0o077)
        source = safe(args.payload, directory=True)
        manifest = json.loads(safe(args.manifest).read_bytes())
        if set(manifest) != set(FILES):
            raise ValueError('Manifest must contain exactly the four runtime payload files.')
        blobs = {}
        for name in FILES:
            data = safe(source / name).read_bytes()
            if len(data) > 1024 * 1024 or digest(data) != manifest[name]:
                raise ValueError('Payload size/hash mismatch.')
            if b'\r' in data:
                raise ValueError('Linux payload must use LF line endings; pin hashes after normalization.')
            blobs[name] = data
        DEST.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
        safe(DEST.parent, directory=True)
        if DEST.exists() or DEST.is_symlink():
            safe(DEST, directory=True)
            receipt = json.loads(safe(DEST / RECEIPT).read_bytes())
            if receipt.get('owner') != 'cor-egress-payload-v1' or set(receipt.get('files', {})) != set(FILES):
                raise ValueError('Existing installation ownership not recognized.')
            for name in FILES:
                if digest(safe(DEST / name).read_bytes()) != receipt['files'][name]:
                    raise ValueError('Installed payload drift; no existing file overwritten.')
            # Update requires an independent code review and no active manager.
            # Keep bootstrap idempotent but never do an implicit live upgrade.
            if receipt['files'] != manifest:
                raise ValueError('Different payload already installed; explicit reviewed upgrade required.')
        else:
            stage = Path(tempfile.mkdtemp(prefix='.cor-egress-', dir=DEST.parent))
            try:
                os.chmod(stage, 0o755)
                for name, data in blobs.items():
                    write(stage / name, data, 0o755 if name != 'manager.py' else 0o644)
                write(stage / RECEIPT, json.dumps({'owner': 'cor-egress-payload-v1', 'files': manifest}, sort_keys=True).encode(), 0o600)
                os.rename(stage, DEST)  # publish only a complete, verified directory
            except BaseException:
                # Leave an incomplete private stage for diagnosis; never recursive-delete.
                raise
        for name in ('secure-egress', 'cor-egress-manager'):
            link = Path('/usr/local/bin') / name
            link.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
            safe(link.parent, directory=True)
            expected = str(DEST / name)
            if link.is_symlink():
                if os.readlink(link) != expected or link.lstat().st_uid != 0:
                    raise ValueError('Executable alias collision; existing entry preserved.')
            elif link.exists():
                raise ValueError('Executable alias collision; existing entry preserved.')
            else:
                os.symlink(expected, link)
        print(json.dumps({'schema': 1, 'schema_version': 1, 'action': 'install-payload',
                          'outcome': 'succeeded', 'protection': 'Unverified',
                          'message': 'Hash-pinned payload installed. No networking or service was started.'}))
        return 0
    except (OSError, ValueError, KeyError):
        print(json.dumps({'schema': 1, 'schema_version': 1, 'action': 'install-payload',
                          'outcome': 'blocked', 'protection': 'Blocked',
                          'message': 'Payload installation blocked by ownership/hash/platform check; existing files preserved.'}))
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
