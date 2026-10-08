"""Prepare verified mihomo and Geo assets for Windows/Linux; Python 3.11+."""
import argparse
import gzip
import os
import shutil
import time
import zipfile
from build_support import ROOT, bundle_directory, copy_static, download_asset, host_arch, read_json, sha256, write_json


def prepare(system, arch, proxy='', core_version='v1.19.32'):
    if system not in ('windows', 'linux') or arch not in ('x64', 'arm64'):
        raise ValueError('Supported targets: windows/linux, x64/arm64')
    bundle = bundle_directory(system, arch)
    resources = bundle/'resources'
    resources.mkdir(parents=True, exist_ok=True)
    architecture = 'amd64' if arch == 'x64' else 'arm64'
    asset_name = (f'mihomo-windows-{architecture}'+('-compatible' if arch == 'x64' else '')+f'-{core_version}.zip') if system == 'windows' else (f'mihomo-linux-{architecture}'+('-compatible' if arch == 'x64' else '')+f'-{core_version}.gz')
    release = read_json(f'https://api.github.com/repos/MetaCubeX/mihomo/releases/tags/{core_version}', proxy)
    asset = next((a for a in release['assets'] if a['name'] == asset_name), None)
    if asset is None:
        raise ValueError('Official core release missing '+asset_name)
    archive = ROOT/'bin'/asset_name
    download_asset(asset, archive, proxy)
    core = resources/('mihomo.exe' if system == 'windows' else 'mihomo')
    temporary = core.with_name(core.name+'.partial')
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
    release = read_json('https://api.github.com/repos/MetaCubeX/meta-rules-dat/releases/latest', proxy)
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
    write_json(resources/'geodata.json', {'version':release['tag_name']+' | '+release['published_at'], 'updated':int(time.time()), 'files':records})
    write_json(resources/'core.json', {'version':core_version,'system':system,'arch':arch,'source':asset['browser_download_url'],'archive_sha256':asset['digest'][7:],'exe_sha256':sha256(core)})
    copy_static(bundle)
    print('Verified core, four Geo databases and native detector notices prepared:', resources)
    return resources


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--system', choices=('windows','linux'), default='windows' if os.name == 'nt' else 'linux')
    parser.add_argument('--arch', choices=('x64','arm64'), default=host_arch())
    parser.add_argument('--proxy', default='')
    parser.add_argument('--core-version', default='v1.19.32')
    args = parser.parse_args()
    prepare(args.system, args.arch, args.proxy, args.core_version)


if __name__ == '__main__':
    main()
