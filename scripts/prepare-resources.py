"""Prepare verified mihomo and Geo assets for Windows/Linux; Python 3.11+."""
import argparse
import gzip
import os
import shutil
import time
import zipfile
from build_support import host_system, ROOT, bundle_directory, copy_static, download_asset, host_arch, read_json, sha256, write_json
from build_inputs import resource_release


def prepare(system, arch, proxy='', core_version=None):
    if system not in ('windows', 'linux', 'macos') or arch not in ('x64', 'arm64'):
        raise ValueError('Supported targets: windows/linux/macos, x64/arm64')
    bundle = bundle_directory(system, arch)
    resources = bundle/'resources'
    resources.mkdir(parents=True, exist_ok=True)
    release = resource_release('mihomo', resources/'core.json', proxy, core_version)
    core_version = release['tag_name']
    architecture = 'amd64' if arch == 'x64' else 'arm64'
    asset_name = (f'mihomo-windows-{architecture}'+('-compatible' if arch == 'x64' else '')+f'-{core_version}.zip') if system == 'windows' else (f"mihomo-{'darwin' if system == 'macos' else 'linux'}-{architecture}"+('-compatible' if arch == 'x64' else '')+f'-{core_version}.gz')
    asset = next((a for a in release['assets'] if a['name'] == asset_name), None)
    if asset is None:
        raise ValueError('Official core release missing '+asset_name)
    archive = ROOT/'bin'/asset_name
    core = resources/('mihomo.exe' if system == 'windows' else 'mihomo')
    import json
    old = json.loads((resources/'core.json').read_text()) if (resources/'core.json').is_file() else {}
    valid = (core.is_file() and old.get('version') == core_version
             and old.get('archive_sha256') == asset.get('digest', '')[7:]
             and old.get('exe_sha256') == sha256(core))
    if not valid:
        temporary = core.with_name(core.name+'.partial')
        try:
            download_asset(asset, archive, proxy)
            if system == 'windows':
                with zipfile.ZipFile(archive) as package:
                    members = [i for i in package.infolist() if i.filename.rsplit('/',1)[-1].startswith('mihomo') and i.filename.endswith('.exe')]
                    if len(members) != 1:
                        raise ValueError('Core archive must contain one executable')
                    with package.open(members[0]) as source, temporary.open('wb') as output:
                        shutil.copyfileobj(source, output)
            else:
                with gzip.open(archive,'rb') as source, temporary.open('wb') as output:
                    shutil.copyfileobj(source, output)
                temporary.chmod(0o755)
            temporary.replace(core)
        finally:
            temporary.unlink(missing_ok=True)
            archive.unlink(missing_ok=True)
    core_release = {key: release[key] for key in ('tag_name', 'published_at')}
    core_release['assets'] = [asset]
    release = resource_release('geo', resources/'geodata.json', proxy)
    records = []
    for remote, local in {'geoip.dat':'GeoIP.dat', 'geosite.dat':'GeoSite.dat', 'country.mmdb':'Country.mmdb', 'GeoLite2-ASN.mmdb':'ASN.mmdb'}.items():
        entry = next((a for a in release['assets'] if a['name'] == remote), None)
        if entry is None:
            raise ValueError('Geo release missing '+remote)
        # Reuse a verified database already prepared for a different platform.
        if not (resources/local).is_file():
            candidates = [ROOT/'bundle/resources'/local, *list((ROOT/'bundle').glob(f'*/resources/{local}'))]
            for cached in candidates:
                if cached.is_file() and sha256(cached) == entry.get('digest','')[7:].lower():
                    shutil.copy2(cached, resources/local)
                    break
        download_asset(entry, resources/local, proxy)
        records.append({'name':local,'source':entry['browser_download_url'],'sha256':entry['digest'][7:].lower(),'size':entry['size']})
    write_json(resources/'geodata.json', {'version':release['tag_name']+' | '+release['published_at'], 'updated':int(time.time()), 'files':records, 'release': release})
    write_json(resources/'core.json', {'version':core_version,'system':system,'arch':arch,'source':asset['browser_download_url'],'archive_sha256':asset['digest'][7:],'exe_sha256':sha256(core), 'release': core_release})
    copy_static(bundle)
    print('Verified core, four Geo databases and native detector notices prepared:', resources)
    return resources


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--system', choices=('windows','linux','macos'), default=host_system())
    parser.add_argument('--arch', choices=('x64','arm64'), default=host_arch())
    parser.add_argument('--proxy', default='')
    parser.add_argument('--core-version')
    args = parser.parse_args()
    prepare(args.system, args.arch, args.proxy, args.core_version)


if __name__ == '__main__':
    main()
