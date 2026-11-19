"""Inspect COSMIC Wayland windows in an installed Pop!_OS compatibility guest."""
import json
from pathlib import Path
import re
import tempfile

from build_support import download, run
from linux_gui import output

PROBE = Path('/usr/local/bin/cor-cosmic-window-probe')
TITLE = 'Clash of Rust'
PROTOCOLS = '8e84152fedf350d2756a2c1c90e07313acb9cdf6'
COSMIC_XML = {
    'cosmic-toplevel-info-unstable-v1.xml': 'f1e9db30d1b69b7e9db1ff5401b1ae9bb13a13b089ad02c05c9163182fa83d91',
    'cosmic-toplevel-management-unstable-v1.xml': 'a625174bbf82304405c919bebcc43af7bf8720bb87243893e94707708fd0b224',
    'cosmic-workspace-unstable-v1.xml': 'c740e275fc0e7f893d6142eaa9322cde00c790d732c6364909f5b7b77498f3e4',
}


def prepare():
    staging = Path('/usr/share/wayland-protocols/staging')
    standard = [staging/name/f'{name}-v1.xml'
                for name in ('ext-foreign-toplevel-list', 'ext-workspace')]
    source = Path(__file__).resolve().with_name('wayland-window-count.c')
    base_url = f'https://raw.githubusercontent.com/pop-os/cosmic-protocols/{PROTOCOLS}/unstable'
    with tempfile.TemporaryDirectory(prefix='cor-cosmic-window-probe-') as temporary:
        directory = Path(temporary)
        protocols = list(standard)
        for name, digest in COSMIC_XML.items():
            xml = directory/name
            download(f'{base_url}/{name}', xml, digest=digest)
            xml.write_bytes(re.sub(rb' deprecated-since="\d+"', b'', xml.read_bytes()))
            protocols.append(xml)
        implementations = []
        for protocol in protocols:
            header = directory/(protocol.stem+'-client.h')
            code = directory/(protocol.stem+'.c')
            run(['wayland-scanner', '--strict', 'client-header', protocol, header], timeout=15)
            run(['wayland-scanner', '--strict', 'private-code', protocol, code], timeout=15)
            implementations.append(code)
        binary = directory/PROBE.name
        run(['cc', '-std=c11', '-Wall', '-Wextra', '-Werror', '-I', directory, source,
             *implementations, '-lwayland-client', '-o', binary], timeout=60)
        run(['sudo', 'install', '-m', '0755', binary, PROBE])


def windows(call=output):
    return [window | {'title': TITLE, 'pid': 0, 'wayland': True}
            for window in json.loads(call([PROBE, 'inspect', TITLE]))]


def close(call=output):
    call([PROBE, 'close', TITLE])
