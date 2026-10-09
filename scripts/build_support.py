"""Shared standard-library build helpers; never installed with the application."""
import contextlib
import ctypes
import hashlib
import json
import os
import platform
from pathlib import Path
import re
import shutil
import subprocess
import struct
import time
import urllib.parse
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parent.parent
RESOURCE_NAMES = ('default.yaml', 'settings-defaults.json', 'THIRD-PARTY-NOTICES.txt')
GEO_NAMES = ('GeoIP.dat', 'GeoSite.dat', 'Country.mmdb', 'ASN.mmdb')


def run(args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, cwd=ROOT, **kwargs)


def sha256(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def package_version():
    text = (ROOT/'Cargo.toml').read_text(encoding='utf-8')
    return re.search(r'^version\s*=\s*"([^\"]+)"', text, re.M)[1]


def host_arch():
    machine = platform.machine().lower()
    if machine in ('aarch64', 'arm64'):
        return 'arm64'
    if machine in ('amd64', 'x86_64', 'x64'):
        return 'x64'
    raise ValueError('Unsupported host architecture: '+machine)


def bundle_directory(system, arch):
    if system not in ('windows', 'linux') or arch not in ('x64', 'arm64'):
        raise ValueError('Supported targets: windows/linux, x64/arm64')
    return ROOT/'bundle' if (system, arch) == ('windows', 'x64') else ROOT/f'bundle/{system}-{arch}'


def validate_binary_arch(path, system, arch):
    with Path(path).open('rb') as stream:
        header = stream.read(64)
        if system == 'windows' and header[:2] == b'MZ' and len(header) == 64:
            stream.seek(struct.unpack_from('<I', header, 0x3c)[0])
            pe = stream.read(6)
            if len(pe) != 6 or pe[:4] != b'PE\0\0':
                raise ValueError('Invalid PE header')
            machine = struct.unpack_from('<H', pe, 4)[0]
            expected = {'x64': 0x8664, 'arm64': 0xaa64}[arch]
        elif system == 'linux' and header[:6] == b'\x7fELF\x02\x01' and len(header) == 64:
            machine = struct.unpack_from('<H', header, 18)[0]
            expected = {'x64': 62, 'arm64': 183}[arch]
        else:
            raise ValueError('Expected a native 64-bit '+system+' binary')
        if machine != expected:
            raise ValueError(f'Binary architecture does not match {system}/{arch}: {Path(path).name}')


def validate_version(version):
    number = r'(?:0|[1-9][0-9]*)'
    if not re.fullmatch(rf'{number}\.{number}\.{number}(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?', version):
        raise ValueError('Version must be SemVer with an optional prerelease suffix')
    base, separator, prerelease = version.partition('-')
    if any(int(p) > 65535 for p in base.split('.')) or (separator and any(
            p.isdigit() and len(p) > 1 and p.startswith('0') for p in prerelease.split('.'))):
        raise ValueError('Version components must fit in 16 bits and use no leading zeroes')
    return version


def numeric_version(version):
    return validate_version(version).partition('-')[0]+'.0'


def deb_version(version):
    # Debian must consider a development build older than its final release.
    return validate_version(version).replace('-', '~', 1)


def file_version(path):
    if os.name != 'nt':
        raise RuntimeError('PE version validation requires Windows')
    from ctypes import wintypes
    library = ctypes.WinDLL('version', use_last_error=True)
    library.GetFileVersionInfoSizeW.argtypes = [wintypes.LPCWSTR, ctypes.POINTER(wintypes.DWORD)]
    library.GetFileVersionInfoSizeW.restype = wintypes.DWORD
    library.GetFileVersionInfoW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p]
    library.VerQueryValueW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR, ctypes.POINTER(ctypes.c_void_p), ctypes.POINTER(wintypes.UINT)]
    handle = wintypes.DWORD()
    size = library.GetFileVersionInfoSizeW(str(path), ctypes.byref(handle))
    if not size:
        raise ctypes.WinError(ctypes.get_last_error())
    data = ctypes.create_string_buffer(size)
    if not library.GetFileVersionInfoW(str(path), 0, size, data):
        raise ctypes.WinError(ctypes.get_last_error())
    pointer, length = ctypes.c_void_p(), wintypes.UINT()
    if library.VerQueryValueW(data, r'\VarFileInfo\Translation', ctypes.byref(pointer), ctypes.byref(length)):
        translations = ctypes.cast(pointer, ctypes.POINTER(wintypes.WORD))
        pairs = [(translations[i], translations[i+1]) for i in range(0, length.value // 2, 2)]
        for language, codepage in pairs:
            field = f'\\StringFileInfo\\{language:04x}{codepage:04x}\\FileVersion'
            if library.VerQueryValueW(data, field, ctypes.byref(pointer), ctypes.byref(length)):
                return validate_version(ctypes.wstring_at(pointer, length.value).rstrip('\0'))
    if not library.VerQueryValueW(data, '\\', ctypes.byref(pointer), ctypes.byref(length)):
        raise ctypes.WinError(ctypes.get_last_error())
    words = ctypes.cast(pointer, ctypes.POINTER(wintypes.DWORD))
    return f'{words[2] >> 16}.{words[2] & 65535}.{words[3] >> 16}'


class SafeRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        redirected = super().redirect_request(req, fp, code, msg, headers, newurl)
        if redirected:
            origin = urllib.parse.urlparse(req.full_url)
            target = urllib.parse.urlparse(newurl)
            if (origin.scheme, origin.netloc) != (target.scheme, target.netloc):
                redirected.remove_header('Authorization')
        return redirected


def opener(proxy=''):
    return urllib.request.build_opener(urllib.request.ProxyHandler({'http': proxy, 'https': proxy} if proxy else {}), SafeRedirect())


def read_json(url, proxy=''):
    headers = {'User-Agent': 'Clash-of-Rust-build', 'Accept': 'application/vnd.github+json'}
    target = urllib.parse.urlparse(url)
    token = os.environ.get('GH_TOKEN') or os.environ.get('GITHUB_TOKEN')
    if token and target.scheme == 'https' and target.netloc == 'api.github.com':
        headers['Authorization'] = 'Bearer ' + token
    request = urllib.request.Request(url, headers=headers)
    with opener(proxy).open(request, timeout=30) as response:
        return json.load(response)


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')


def download(url, destination, proxy='', digest=None):
    destination = Path(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    if digest and destination.is_file() and sha256(destination) == digest:
        return
    partial = destination.with_name(destination.name+'.partial')
    for attempt in range(3):
        try:
            request = urllib.request.Request(url, headers={'User-Agent': 'Clash-of-Rust-build'})
            with opener(proxy).open(request, timeout=90) as response, partial.open('wb') as stream:
                shutil.copyfileobj(response, stream, length=1024*1024)
            if digest and sha256(partial) != digest:
                raise ValueError('SHA256 mismatch: '+destination.name)
            partial.replace(destination)
            return
        except Exception:
            partial.unlink(missing_ok=True)
            if attempt == 2:
                raise
            time.sleep(attempt+1)


def download_asset(asset, destination, proxy=''):
    digest = asset.get('digest', '')
    if not re.fullmatch(r'sha256:[a-fA-F0-9]{64}', digest):
        raise ValueError('Missing official SHA256: '+asset['name'])
    download(asset['browser_download_url'], destination, proxy, digest[7:].lower())


def copy_static(bundle):
    resources = bundle/'resources'
    resources.mkdir(parents=True, exist_ok=True)
    for name in RESOURCE_NAMES:
        shutil.copy2(ROOT/'resources'/name, resources/name)
    shutil.copy2(ROOT/'LICENSE', bundle/'LICENSE')
    licenses = {'SourceHanSans-LICENSE.txt':'fonts/LICENSE.txt', 'Twemoji-LICENSE.txt':'flags/LICENSE-Twemoji.txt', 'Unicode-LICENSE.txt':'flags/LICENSE-Unicode.txt'}
    for name, source in licenses.items():
        shutil.copy2(ROOT/'resources'/source, resources/name)
    ip = resources/'ip-check'
    ip.mkdir(exist_ok=True)
    for name in ('LICENSE', 'SOURCE.md'):
        shutil.copy2(ROOT/'vendor/region-restriction-check'/name, ip/name)


def validate_resources(resources, system):
    core = resources/('mihomo.exe' if system == 'windows' else 'mihomo')
    record = json.loads((resources/'core.json').read_text(encoding='utf-8'))
    if sha256(core) != record['exe_sha256']:
        raise ValueError('Core checksum mismatch')
    records = json.loads((resources/'geodata.json').read_text(encoding='utf-8'))['files']
    if len(records) != len(GEO_NAMES) or {r['name'] for r in records} != set(GEO_NAMES):
        raise ValueError('Geo manifest does not cover the four required databases')
    for record in records:
        path = resources/record['name']
        if path.stat().st_size != record['size'] or sha256(path) != record['sha256']:
            raise ValueError('Geo checksum mismatch: '+path.name)


def stage_resources(source, stage, system):
    validate_resources(source, system)
    destination = stage/'resources'
    destination.mkdir(parents=True)
    for name in (*GEO_NAMES, 'geodata.json', 'core.json', 'mihomo.exe' if system == 'windows' else 'mihomo'):
        shutil.copy2(source/name, destination/name)
    copy_static(stage)


def nsis(proxy=''):
    compiler = ROOT/'tools/nsis/nsis-3.13/makensis.exe'
    if compiler.is_file():
        return compiler
    archive = ROOT/'tools/nsis-3.13.zip'
    download('https://downloads.sourceforge.net/project/nsis/NSIS%203/3.13/nsis-3.13.zip', archive, proxy)
    destination = ROOT/'tools/nsis'
    with zipfile.ZipFile(archive) as bundle:
        for member in bundle.infolist():
            target = (destination/member.filename).resolve()
            if not target.is_relative_to(destination.resolve()):
                raise ValueError('Unsafe NSIS archive path')
        bundle.extractall(destination)
    if not compiler.is_file():
        raise FileNotFoundError(compiler)
    return compiler


@contextlib.contextmanager
def staging_directory(name):
    # Only remove our newly-created, unique staging directory under this repo.
    import tempfile
    base = ROOT/'target/package-staging'
    base.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=name+'-', dir=base) as directory:
        yield Path(directory)
