"""Compress APK resources before alignment and signing with the same identity."""
import copy
import hashlib
import os
from pathlib import Path
import re
import subprocess
import tempfile
import zipfile


GEO_ASSETS = {f'assets/{name}' for name in ('GeoIP.dat', 'GeoSite.dat', 'Country.mmdb', 'ASN.mmdb')}


def certificate(path, tools):
    result = subprocess.check_output(
        [str(tools/'apksigner'), '-J-enable-native-access=ALL-UNNAMED', 'verify',
         '--print-certs', str(path)], text=True)
    digests = re.findall(
        r'(?m)^Signer (?:#\d+|\(minSdkVersion=\d+(?: \(dev release=true\))?, '
        r'maxSdkVersion=\d+\)) certificate SHA-256 digest: ([0-9a-fA-F]{64})[ \t]*\r?$',
        result)
    if not digests:
        raise ValueError('APK signing certificate is unavailable')
    return sorted({digest.lower() for digest in digests})


def recompress(source, destination):
    records = {}
    with zipfile.ZipFile(source) as original, zipfile.ZipFile(
            destination, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=9) as output:
        names = original.namelist()
        if len(names) != len(set(names)):
            raise ValueError('APK contains duplicate archive members')
        for entry in original.infolist():
            name = entry.filename
            if (name.startswith('/') or '..' in Path(name).parts or '\\' in name
                    or entry.compress_type not in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED)):
                raise ValueError('Invalid APK member')
            if re.fullmatch(r'META-INF/(?:MANIFEST\.MF|[^/]+\.(?:SF|RSA|DSA|EC))', name, re.I):
                continue
            member = copy.copy(entry)
            member.extra = b''
            if name in GEO_ASSETS:
                member.compress_type = zipfile.ZIP_DEFLATED
            member._compresslevel = 9
            checksum = hashlib.sha256()
            with original.open(entry) as incoming, output.open(member, 'w') as outgoing:
                while block := incoming.read(256*1024):
                    outgoing.write(block)
                    checksum.update(block)
            records[name] = (entry.file_size, checksum.hexdigest())
    with zipfile.ZipFile(destination) as archive:
        for name, (length, digest) in records.items():
            with archive.open(name) as stream:
                if archive.getinfo(name).file_size != length or hashlib.file_digest(stream, 'sha256').hexdigest() != digest:
                    raise ValueError('APK member changed during compression')
    return records


def optimize(path, tools):
    path, tools = Path(path), Path(tools)
    before = path.stat().st_size
    identity = certificate(path, tools)
    key = os.environ.get('ANDROID_KEYSTORE_PATH')
    if key:
        signing = ['--ks', key, '--ks-key-alias', os.environ['ANDROID_KEY_ALIAS'],
                   '--ks-pass', 'env:ANDROID_KEYSTORE_PASSWORD',
                   '--key-pass', 'env:ANDROID_KEY_PASSWORD']
    elif os.environ.get('ANDROID_CI_TEST') == '1':
        key = Path(os.environ.get('ANDROID_USER_HOME', Path.home()/'.android'))/'debug.keystore'
        signing = ['--ks', str(key), '--ks-key-alias', 'androiddebugkey',
                   '--ks-pass', 'pass:android', '--key-pass', 'pass:android']
    else:
        raise ValueError('APK compression requires its original signing identity')
    if not Path(key).is_file():
        raise ValueError('APK signing keystore does not exist')
    with tempfile.TemporaryDirectory(prefix='.apk-compress-', dir=path.parent) as temporary:
        stage = Path(temporary)
        unsigned, aligned = stage/'unsigned.apk', stage/'aligned.apk'
        recompress(path, unsigned)
        subprocess.run([str(tools/'zipalign'), '-f', '-P', '16', '4',
                        str(unsigned), str(aligned)], check=True)
        subprocess.run([str(tools/'apksigner'), '-J-enable-native-access=ALL-UNNAMED',
                        'sign', *signing, '--v4-signing-enabled', 'false', str(aligned)], check=True)
        subprocess.run([str(tools/'zipalign'), '-c', '-P', '16', '4', str(aligned)], check=True)
        if certificate(aligned, tools) != identity:
            raise ValueError('APK compression changed the signing identity')
        after = aligned.stat().st_size
        if after < before:
            aligned.replace(path)
            path.with_name(path.name+'.idsig').unlink(missing_ok=True)
        else:
            after = before
    print(f'APK resource compression: {before} -> {after} bytes; saved {before-after}', flush=True)
    return {'before_bytes': before, 'after_bytes': after, 'saved_bytes': before-after}
