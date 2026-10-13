"""Maintain offline icons/flags and verify Windows embedded icons.

Only generation needs Pillow; verification uses the standard library. Normal
builds consume committed assets and never regenerate artwork or flags.
"""
import argparse
import concurrent.futures
import io
import json
from pathlib import Path
import re
import struct

from build_support import ROOT, opener


def pack_icons():
    from PIL import Image

    output = ROOT/'resources/icons'
    output.mkdir(exist_ok=True)
    for name, source in {
        'app': 'app-blue-transparent.png',
        'system': 'system-orange-transparent.png',
        'tun': 'tun-green-transparent.png',
    }.items():
        with Image.open(ROOT/'resources/icon-concepts'/source) as image:
            image = image.convert('RGBA')
            assert image.getpixel((0, 0))[3] == 0, f'{source} must have a transparent background'
            thumbnail = image.resize((128, 128), Image.Resampling.LANCZOS)
            (output/f'{name}.rgba').write_bytes(thumbnail.tobytes())
            thumbnail.save(output/f'{name}.png')
            if name == 'app':
                image.save(output/'app.ico', sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)])


def prepare_flags(proxy):
    from PIL import Image

    dest = ROOT/'resources/flags'
    dest.mkdir(parents=True, exist_ok=True)
    client = opener(proxy)

    def fetch(url):
        with client.open(url, timeout=30) as response:
            return response.read()

    twemoji = 'https://raw.githubusercontent.com/twitter/twemoji/v14.0.2/'
    codes = re.search(r'ISO_CODES: &str = "([^"]+)"',
                      (ROOT/'src/probe.rs').read_text(encoding='utf-8'))[1].split()

    def flag(code):
        name = '-'.join(f"{0x1f1e6 + ord(ch) - ord('A'):x}" for ch in code)
        with Image.open(io.BytesIO(fetch(twemoji + f'assets/72x72/{name}.png'))) as image:
            image = image.convert('RGBA')
            assert image.size == (72, 72)
            return code, image.tobytes()

    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        flags = sorted(pool.map(flag, [code for code in codes if code not in ('HK', 'MO', 'TW')]))
    (dest/'flags.rgba').write_bytes(b''.join(pixels for _, pixels in flags))
    (dest/'codes.json').write_text(json.dumps([code for code, _ in flags]), encoding='utf-8')
    (dest/'LICENSE-Twemoji.txt').write_bytes(fetch(twemoji + 'LICENSE-GRAPHICS'))
    cldr = 'https://raw.githubusercontent.com/unicode-org/cldr-json/46.0.0/'
    territories = json.loads(fetch(cldr + 'cldr-json/cldr-localenames-full/main/en/territories.json'))[
        'main']['en']['localeDisplayNames']['territories']
    (dest/'countries.json').write_text(json.dumps(
        {code: territories[code] for code in codes}, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    (dest/'LICENSE-Unicode.txt').write_bytes(fetch(cldr + 'LICENSE'))
    print(f'Prepared {len(flags)} flags; no network access is needed by the application')


def verify_icons(filenames):
    source = (ROOT/'resources/icons/app.ico').read_bytes()
    count = struct.unpack_from('<H', source, 4)[0]
    expected = set()
    for index in range(count):
        size, offset = struct.unpack_from('<II', source, 6 + index * 16 + 8)
        expected.add(source[offset:offset + size])
    for filename in filenames:
        data = Path(filename).read_bytes()
        pe = struct.unpack_from('<I', data, 0x3C)[0]
        assert data[pe:pe + 4] == b'PE\0\0'
        optional = pe + 24
        directory = optional + (112 if struct.unpack_from('<H', data, optional)[0] == 0x20B else 96)
        resource_rva = struct.unpack_from('<I', data, directory + 16)[0]
        section_count = struct.unpack_from('<H', data, pe + 6)[0]
        sections = optional + struct.unpack_from('<H', data, pe + 20)[0]

        def address(rva):
            for index in range(section_count):
                virtual_size, start, size, offset = struct.unpack_from('<IIII', data, sections + index * 40 + 8)
                if start <= rva < start + max(size, virtual_size):
                    return offset + rva - start
            raise ValueError('Resource address is outside PE sections')

        root = address(resource_rva)
        icons = set()
        groups = 0

        def walk(relative=0, kind=None):
            nonlocal groups
            location = root + relative
            named, numbered = struct.unpack_from('<HH', data, location + 12)
            for index in range(named + numbered):
                name, target = struct.unpack_from('<II', data, location + 16 + index * 8)
                entry_kind = name if kind is None else kind
                if target & 0x80000000:
                    walk(target & 0x7FFFFFFF, entry_kind)
                else:
                    rva, size = struct.unpack_from('<II', data, root + target)
                    if entry_kind == 3:
                        start = address(rva)
                        icons.add(data[start:start + size])
                    elif entry_kind == 14:
                        groups += 1

        walk()
        assert groups and expected.issubset(icons), f'Approved ICO frames missing from {filename}'
        print(f'{Path(filename).name}: all {count} approved icon frames embedded')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    commands.add_parser('icons', help='Encode approved PNG artwork as ICO, PNG and RGBA')
    flags = commands.add_parser('flags', help='Prepare pinned Twemoji/CLDR offline assets')
    flags.add_argument('--proxy', default='')
    verify = commands.add_parser('verify-icons', help='Check PE icon resources without executing programs')
    verify.add_argument('files', type=Path, nargs='+')
    args = parser.parse_args()
    if args.command == 'icons':
        pack_icons()
    elif args.command == 'flags':
        prepare_flags(args.proxy)
    else:
        verify_icons(args.files)


if __name__ == '__main__':
    main()
