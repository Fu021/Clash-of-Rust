"""Version summaries and validated download snapshot shared by every site page."""
from datetime import date, datetime
from html import escape
import re

SUFFIXES = ('windows-x64-setup.exe', 'windows-arm64-setup.exe', 'linux-amd64.deb',
            'linux-arm64.deb', 'macos-arm64.dmg', 'macos-x64.dmg',
            'android-arm64.apk', 'android-armv7.apk', 'android-x64.apk')
OPTIONAL_SUFFIXES = ('linux-x86_64.rpm', 'linux-aarch64.rpm', 'linux-x86_64.pkg.tar.zst')

def validate(data):
    date.fromisoformat(data['checked'])
    series = data.get('series', '')
    if not re.fullmatch(r'\d+\.\d+', series):
        raise ValueError('Expected a major.minor release series')
    seen = set()
    for release in data['releases']:
        version = release['version']
        if not re.fullmatch(r'\d+\.\d+\.\d+', version) or version in seen:
            raise ValueError('Invalid or duplicate version: ' + version)
        seen.add(version)
        if not version.startswith(series + '.'):
            raise ValueError('Version outside the current release series: ' + version)
        if release['channel'] != 'stable':
            raise ValueError('Version/channel mismatch: ' + version)
        date.fromisoformat(release['date'])
        for lang in ('en','zh'):
            copy = release[lang]
            if not copy['title'].strip() or not copy['highlights'] or not all(isinstance(x,str) and x.strip() for x in copy['highlights']):
                raise ValueError('Missing translated highlights: ' + version)
        if len(release['en']['highlights']) != len(release['zh']['highlights']):
            raise ValueError('Translated release highlights are out of sync: ' + version)
        for field in ('summary', 'known'):
            if bool(release['en'].get(field)) != bool(release['zh'].get(field)):
                raise ValueError('Missing translated release field: ' + field)
    for release in data['history']:
        version = release['version']
        if not re.fullmatch(r'\d+\.\d+\.\d+', version) or version in seen:
            raise ValueError('Invalid or duplicate historical version: ' + version)
        seen.add(version)
        date.fromisoformat(release['date'])
        for lang in ('en', 'zh'):
            summary = release.get(lang, {}).get('summary')
            if not isinstance(summary, str) or not summary.strip():
                raise ValueError('Missing translated historical summary: ' + version)
    snapshot = data['downloads']['stable']
    release = next((r for r in data['releases'] if 'v'+r['version'] == snapshot['tag']), None)
    if release is None:
        raise ValueError('Download has no matching stable release')
    if datetime.fromisoformat(snapshot['published'].replace('Z','+00:00')).date().isoformat() != release['date']:
        raise ValueError('Release date mismatch: stable')
    sizes = snapshot['sizes']
    if (not set(SUFFIXES) <= sizes.keys() <= set(SUFFIXES + OPTIONAL_SUFFIXES)
            or not all(type(size) is int and size > 0 for size in sizes.values())):
        raise ValueError('Expected valid required and optional package sizes: stable')

def render(data, lang, repo, compact=False, root=''):
    zh = lang == 'zh'
    labels = {'stable':'正式版' if zh else 'Stable'}
    rows = data['releases']
    if compact:
        rows = sorted(rows, key=lambda r: r['date'], reverse=True)[:2]
    else:
        rows = sorted(rows,key=lambda r:r['date'],reverse=True)
    parts = []
    for r in rows:
        copy = r[lang]; version = escape(r['version']); channel = r['channel']
        target = f'releases.html#v{version}' if compact else f'{repo}/releases/tag/v{version}'
        highlights = [copy.get('summary',copy['highlights'][0])] if compact else copy['highlights']
        notes = ''.join(f'<li>{escape(x)}</li>' for x in highlights)
        known = f'<p class="release-known">{escape(copy["known"])}</p>' if copy.get('known') and not compact else ''
        heading = 'h3' if compact else 'h2'
        link_label = ('看看更新' if zh else 'What’s new') if compact else ('GitHub 完整说明 ↗' if zh else 'Full notes on GitHub ↗')
        download = '' if compact else f'<a href="{repo}/releases/tag/v{version}">{"查看此版本安装包" if zh else "Packages for this version"} ↗</a>'
        parts.append(f'<article class="version-entry" id="v{version}"><p class="version-label">{labels[channel]} · <time datetime="{r["date"]}">{r["date"]}</time></p><{heading}>{escape(copy["title"])}</{heading}><p class="version-number">v{version}</p><ul>{notes}</ul>{known}<div class="version-links"><a href="{target}">{link_label}</a>{download}</div></article>')
    content = '<div class="version-grid">'+''.join(parts)+'</div>'
    if compact:
        return '<section class="part versions"><div class="wrap"><div class="version-heading"><h2>'+('一路更新' if zh else 'Along the way')+'</h2><a href="releases.html">'+('查看版本记录 →' if zh else 'All version notes →')+'</a></div>'+content+'</div></section>'
    history = ''.join(
        f'<li id="v{escape(item["version"])}"><h3><a href="{repo}/releases/tag/v{escape(item["version"])}">'
        f'v{escape(item["version"])}</a></h3><time datetime="{item["date"]}">{item["date"]}</time>'
        f'<p>{escape(item[lang]["summary"])}</p></li>'
        for item in data['history'])
    earlier = ('历史正式版' if zh else 'Earlier stable releases')
    archive = f'<section class="release-history"><h2>{earlier}</h2><ul class="release-archive">{history}</ul></section>' if history else ''
    alt = '蓝猫在车站张贴新的线路告示。' if zh else 'The blue cat puts up a new railway notice at the station.'
    banner = f'<div class="banner scene"><picture><source media="(max-width: 640px)" srcset="{root}assets/versions-small.webp"><img src="{root}assets/versions.webp" width="2172" height="724" alt="{alt}"></picture></div>'
    return '<section class="page-head"><div class="wrap"><h1>'+('版本记录' if zh else 'Release notes')+'</h1><p class="lead">'+('历次正式版更新。' if zh else 'Stable releases through the years.')+'</p></div>'+banner+'</section><section class="wrap doc versions"><p>'+('完整发行说明见 ' if zh else 'Full release notes: ')+f'<a href="{repo}/releases">GitHub Releases ↗</a>'+('。' if zh else '.')+'</p>'+content+archive+'</section>'
