"""Isolated upstream for Linux native TUN tests; never contacts the Internet."""
import asyncio
import base64
import ipaddress
import json
from pathlib import Path
import socket
import ssl
import struct
import sys

# These are the real production resolver endpoints, assigned inside the peer
# namespace. TLS verification stays enabled using a namespace-local test CA.
BOOTSTRAP = ('223.5.5.5', '223.6.6.6', '1.0.0.1')
PUBLIC = ('1.1.1.1', '8.8.8.8', '2606:4700:4700::1111', '2001:4860:4860::8888')
NODE = ('203.0.113.10', '2001:db8:2::10')
WEBSITE = ('203.0.113.20', '2001:db8:2::20')
METRICS = {'bootstrap': 0, 'proxied_dns': 0, 'direct_public_dns': 0,
           'node_connections': 0, 'web': 0, 'plaintext_dns': 0,
           'bootstrap_public_lookup': 0, 'resolver_bootstrap': 0}
BLOCK_FIXED_BOOTSTRAP = False
BLOCK_PUBLIC_DNS = False


def dns_response(query, bootstrap):
    end, labels = 12, []
    while query[end]:
        length = query[end]
        labels.append(query[end + 1:end + 1 + length].decode('ascii'))
        end += length + 1
    end += 1
    kind, _ = struct.unpack('!HH', query[end:end + 4])
    end += 4
    name = '.'.join(labels)
    if name == 'resolver.fixture.test':
        address = ('192.0.2.254', '2001:db8:1::fe')
    else:
        address = NODE if name.startswith(('node.', 'node6.')) else WEBSITE
    # Bootstrap may resolve only node names. A public lookup sent here would
    # constitute a privacy regression even if it received a usable answer.
    allowed = not bootstrap or name.startswith(('node.', 'node6.'))
    if not allowed:
        METRICS['bootstrap_public_lookup'] += 1
    if name.startswith('node6.') and kind == 1:
        allowed = False
    answer = allowed and kind in (1, 28)
    response = query[:2] + struct.pack('!HHHHH', 0x8180, 1, int(answer), 0, 0) + query[12:end]
    if answer:
        value = ipaddress.ip_address(address[kind == 28]).packed
        response += b'\xc0\x0c' + struct.pack('!HHIH', kind, 1, 0, len(value)) + value
    return response


async def http(reader, writer):
    global BLOCK_FIXED_BOOTSTRAP, BLOCK_PUBLIC_DNS
    try:
        head = await asyncio.wait_for(reader.readuntil(b'\r\n\r\n'), 8)
        path = head.split(b' ', 2)[1].decode('ascii')
        local = writer.get_extra_info('sockname')[0]
        peer = writer.get_extra_info('peername')[0]
        if path.startswith('/block-fixed-bootstrap/'):
            BLOCK_FIXED_BOOTSTRAP = path.endswith('/on')
            body, content_type = b'ok', 'text/plain'
        elif path.startswith('/block-public-dns/'):
            BLOCK_PUBLIC_DNS = path.endswith('/on')
            body, content_type = b'ok', 'text/plain'
        elif path == '/metrics':
            body, content_type = json.dumps(METRICS).encode(), 'application/json'
        elif path.startswith('/dns-query?dns='):
            fixed = local in BOOTSTRAP or local == PUBLIC[2]
            bootstrap = fixed or local in ('192.0.2.254', '2001:db8:1::fe')
            direct = peer in ('192.0.2.1', '2001:db8:1::1')
            if fixed and direct and BLOCK_FIXED_BOOTSTRAP:
                # Model a network where the chosen public HTTPS IPs cannot be
                # reached. The subscription's own encrypted bootstrap remains
                # available at the peer IP, without changing production URLs.
                return
            if not direct and BLOCK_PUBLIC_DNS:
                return
            if bootstrap and direct:
                METRICS['bootstrap'] += 1
            elif not direct:
                METRICS['proxied_dns'] += 1
                bootstrap = False
            else:
                METRICS['direct_public_dns'] += 1
            encoded = path.split('dns=', 1)[1].split('&', 1)[0]
            query = base64.urlsafe_b64decode(encoded + '=' * (-len(encoded) % 4))
            body, content_type = dns_response(query, bootstrap), 'application/dns-message'
        else:
            METRICS['web'] += 1
            if path == '/auto-check':
                # Keep loopback measurements nonzero on fast CI hosts.
                await asyncio.sleep(0.03)
            body, content_type = b'protected-ok', 'text/plain'
        writer.write(f'HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n'.encode() + body)
        await writer.drain()
    except (OSError, ValueError, asyncio.IncompleteReadError, asyncio.TimeoutError):
        pass
    finally:
        writer.close()


async def socks(reader, writer):
    remote = None
    try:
        if writer.get_extra_info('sockname')[1] == 1081:
            # A second, deliberately slower route makes Auto re-selection
            # deterministic without relying on public Internet latency.
            await asyncio.sleep(0.2)
        version, size = await reader.readexactly(2)
        assert version == 5
        await reader.readexactly(size)
        writer.write(b'\x05\x00')
        await writer.drain()
        version, command, _, kind = await reader.readexactly(4)
        assert version == 5 and command == 1
        if kind == 1:
            host = socket.inet_ntop(socket.AF_INET, await reader.readexactly(4))
        elif kind == 4:
            host = socket.inet_ntop(socket.AF_INET6, await reader.readexactly(16))
        else:
            assert kind == 3
            host = (await reader.readexactly((await reader.readexactly(1))[0])).decode('ascii')
        port = struct.unpack('!H', await reader.readexactly(2))[0]
        # Only the fixtures are reachable. Domain requests have no OS lookup.
        if host not in (*BOOTSTRAP, *PUBLIC, *WEBSITE, '192.0.2.254'):
            host = WEBSITE[0]
        incoming, remote = await asyncio.open_connection(host, port)
        METRICS['node_connections'] += 1
        writer.write(b'\x05\x00\x00\x01\x7f\x00\x00\x01\x00\x00')
        await writer.drain()

        async def relay(source, target):
            while data := await source.read(16384):
                target.write(data)
                await target.drain()
            target.close()
        await asyncio.gather(relay(reader, remote), relay(incoming, writer))
    except (OSError, AssertionError, asyncio.IncompleteReadError):
        pass
    finally:
        writer.close()
        if remote:
            remote.close()


class PlaintextDNS(asyncio.DatagramProtocol):
    def connection_made(self, transport):
        self.transport = transport

    def datagram_received(self, data, peer):
        # This one infrastructure name is intentionally bootstrapped by the
        # subscription's original default-nameserver, never a public query.
        infrastructure = b'\x08resolver\x07fixture\x04test\x00' in data
        METRICS['resolver_bootstrap' if infrastructure else 'plaintext_dns'] += 1
        self.transport.sendto(dns_response(data, False), peer)


async def main(directory):
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(directory/'server.crt', directory/'server.key')
    servers = []
    for host, family in [('0.0.0.0', socket.AF_INET), ('::', socket.AF_INET6)]:
        for port in (80, 443, 18080):
            servers.append(await asyncio.start_server(http, host, port, family=family,
                                                     ssl=context if port == 443 else None))
        for port in (1080,1081):
            servers.append(await asyncio.start_server(socks, host, port, family=family))
    loop = asyncio.get_running_loop()
    transport, _ = await loop.create_datagram_endpoint(PlaintextDNS, local_addr=('0.0.0.0', 53))
    (directory/'ready').write_text('ready')
    try:
        await asyncio.Future()
    finally:
        transport.close()
        for server in servers:
            server.close()


if __name__ == '__main__':
    asyncio.run(main(Path(sys.argv[1])))
