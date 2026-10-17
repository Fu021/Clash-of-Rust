#!/usr/bin/env python3
"""Opt-in WSL/VPS acceptance using the owner's existing strict SSH connection.

No public IP-echo or geolocation service is contacted. DNS nonces and egress
packet headers are observed on the owned VPS, and HTTPS uses example.com.
No routes, firewall rules, services, or keys are changed by this test.
"""
import argparse
import concurrent.futures
import ipaddress
import json
import os
from pathlib import Path
import secrets
import re
import subprocess
import sys

MONITOR = r'''
import ipaddress,json,select,socket,struct,subprocess,sys,time
request=json.loads(sys.stdin.readline())
route=json.loads(subprocess.check_output(['ip','-4','-j','route','get','1.1.1.1']))[0]
expected=route['prefsrc']; uplink=route['dev']
nonces=request['nonces']; ports=request['ports']
observed={nonce:{'tunnel_dns':0,'upstream_dns':0,'clear_uplink':0} for nonce in nonces}
https={str(port):{'syn':0,'source_matches_vps':False,'uplink_synack':0,
                  'tunnel_syn':0,'tunnel_synack':0,'uplink_ack':0} for port in ports}
sock=socket.socket(socket.AF_PACKET,socket.SOCK_RAW,socket.htons(3))
sock.setblocking(False)
print(json.dumps({'ready':True}),flush=True)
deadline=time.monotonic()+120
while time.monotonic()<deadline:
    ready,_,_=select.select([sock,sys.stdin],[],[],0.2)
    if sys.stdin in ready:
        sys.stdin.readline(); break
    if sock not in ready: continue
    frame,address=sock.recvfrom(65535)
    iface,_,kind=address[:3]
    offset=0
    if len(frame)>=14 and frame[12:14] in (b'\x08\x00',b'\x81\x00',b'\x88\xa8'):
        offset=14
        if frame[12:14]!=b'\x08\x00':
            if len(frame)<18 or frame[16:18]!=b'\x08\x00': continue
            offset=18
    if len(frame)<offset+20 or frame[offset]>>4!=4: continue
    ihl=(frame[offset]&15)*4
    if len(frame)<offset+ihl+8: continue
    protocol=frame[offset+9]
    source=socket.inet_ntoa(frame[offset+12:offset+16])
    payload=frame[offset+ihl:]
    if protocol not in (6,17): continue
    sport,dport=struct.unpack('!HH',payload[:4])
    if (protocol==6 and len(payload)>=14 and iface==uplink and kind==4
            and sport in ports and dport==443 and payload[13]&2):
        https[str(sport)]['syn']+=1
        https[str(sport)]['source_matches_vps'] |= source==expected
    if protocol==6 and len(payload)>=14:
        flags=payload[13]
        if sport==443 and dport in ports and flags&0x12==0x12:
            if iface==uplink and kind!=4: https[str(dport)]['uplink_synack']+=1
            if iface=='cor-wsl0' and kind==4: https[str(dport)]['tunnel_synack']+=1
        if sport in ports and dport==443:
            if iface=='cor-wsl0' and kind!=4 and flags&2: https[str(sport)]['tunnel_syn']+=1
            if iface==uplink and kind==4 and flags&16: https[str(sport)]['uplink_ack']+=1
    for nonce in nonces:
        if nonce.encode() not in payload: continue
        if iface=='cor-wsl0' and dport==53:
            observed[nonce]['tunnel_dns']+=1
        if iface==uplink and kind==4 and dport==53 and source==expected:
            observed[nonce]['upstream_dns']+=1
        # Replies from the upstream resolver legitimately echo the DNS nonce.
        # Only a clear nonce arriving on the encrypted transport port is a
        # possible direct client-to-VPS escape.
        if iface==uplink and kind!=4 and dport==51820:
            observed[nonce]['clear_uplink']+=1
print(json.dumps({'observed':observed,'https':https,
                  'source_address_redacted':True,'observer':'owned-vps-packet-receiver'}),flush=True)
'''

CLIENT = r'''
import ipaddress,json,os,pathlib,re,socket,struct,subprocess,sys
request=json.load(sys.stdin)
nonce=request['nonce']
name=nonce+'.example.com'
question=b''.join(bytes([len(x)])+x.encode() for x in name.split('.'))+b'\0'
ident=bytes.fromhex(nonce[:4])
packet=ident+struct.pack('!HHHHH',0x100,1,0,0,0)+question+struct.pack('!HH',1,1)
sock=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);sock.settimeout(8)
dns=False; error=None
try:
    sock.sendto(packet,('10.77.0.1',53))
    response,peer=sock.recvfrom(4096)
    dns=(peer==('10.77.0.1',53) and response[:2]==ident
         and len(response)>12 and response[2]&0x80 and nonce.encode() in response)
except OSError as exc: error=type(exc).__name__
sock.close()
curl=subprocess.run(['/usr/bin/curl','--noproxy','*','--ipv4','--fail',
                     '--connect-timeout','8','--max-time','20',
                     '--local-port',str(request['port']),'-sS','-o','/dev/null',
                     '-w','%{http_code}','https://example.com/'],
                    capture_output=True,text=True,timeout=25)
ipv6_error=None;ipv6_connected=None
try:
    target=socket.getaddrinfo('example.com',443,socket.AF_INET6,socket.SOCK_STREAM)[0][4]
    probe=socket.socket(socket.AF_INET6,socket.SOCK_STREAM);probe.settimeout(5)
    probe.connect(target);ipv6_connected=True;probe.close()
except OSError as exc:
    ipv6_connected=False;ipv6_error=type(exc).__name__
interfaces=json.loads(subprocess.check_output(['/usr/sbin/ip','-j','addr','show']))
addresses=[]
for item in interfaces:
    for value in item.get('addr_info',[]):
        addresses.append(value.get('local'))
public_ipv6=any(ipaddress.ip_address(x).version==6 and
                ipaddress.ip_address(x).is_global for x in addresses)
status=pathlib.Path('/proc/self/status').read_text()
caps={line.split(':')[0]:line.split(':')[1].strip() for line in status.splitlines()
      if line.startswith(('CapEff:','CapPrm:','CapAmb:','CapBnd:','NoNewPrivs:'))}
hosts_text=pathlib.Path('/etc/hosts').read_text()
hosts_bad=[]
for line in hosts_text.splitlines():
    line=line.partition('#')[0].strip()
    if not line: continue
    try: address=ipaddress.ip_address(line.split()[0])
    except ValueError: continue
    if address.version==4 and not address.is_loopback: hosts_bad.append(str(address))
    if address.version==6 and not (address.is_loopback or address.is_link_local or address.is_multicast):
        hosts_bad.append(str(address))
for name in ('host.docker.internal','gateway.docker.internal','kubernetes.docker.internal'):
    if name in hosts_text: hosts_bad.append(name)
interop=pathlib.Path('/proc/sys/fs/binfmt_misc/WSLInterop')
interop_disabled=not interop.exists() or interop.read_text().splitlines()[0].strip()=='disabled'
print(json.dumps({'dns_reply':bool(dns),'dns_error':error,'https_exit':curl.returncode,
                  'https_status':curl.stdout.strip(),'uid':os.getuid(),'caps':caps,
                  'https_error':re.sub(r'(?:\d{1,3}\.){3}\d{1,3}','<address>',curl.stderr.strip())[:700],
                  'ipv6_connected':ipv6_connected,'ipv6_error':ipv6_error,
                  'public_ipv6_interface':public_ipv6,
                  'hosts_leak_free':not hosts_bad,'hosts_violations':hosts_bad,
                  'interop_disabled':bool(interop_disabled),
                  'interfaces':[item['ifname'] for item in interfaces],
                  'windows_mount_hidden':not pathlib.Path('/mnt/c').exists(),
                  'windows_ipconfig_hidden':not pathlib.Path('/mnt/c/Windows/System32/ipconfig.exe').exists(),
                  'proxy_environment_clear':not any(x.lower().endswith('proxy') for x in os.environ)}))
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ssh-alias', required=True)
    parser.add_argument('--identity-file', type=Path, required=True)
    parser.add_argument('--distro', required=True)
    parser.add_argument('--generation', type=int, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--attestation', default='/var/lib/cor-egress/scope-attestation.json')
    parser.add_argument('--endpoint-ipv4')
    parser.add_argument('--execute', action='store_true')
    parser.add_argument('--fail-closed', action='store_true')
    args = parser.parse_args()
    if not args.execute:
        print(json.dumps(dict(outcome='not_run', message='No network test executed.')))
        return 0
    if sys.platform != 'win32' or args.generation <= 0:
        raise ValueError('Windows with an installed generation required.')
    endpoint = None
    if args.fail_closed:
        try:
            endpoint = ipaddress.IPv4Address(args.endpoint_ipv4)
        except (ipaddress.AddressValueError, TypeError) as error:
            raise ValueError('Fail-closed acceptance requires a numeric IPv4 endpoint.') from error
        if not endpoint.is_global:
            raise ValueError('The configured endpoint must be a public IPv4 address.')
    if not args.ssh_alias or args.ssh_alias.startswith('-') or any(c.isspace() for c in args.ssh_alias):
        raise ValueError('One existing SSH alias required.')
    attestation = args.attestation
    system = Path(os.environ['SystemRoot'])/'System32'
    nonces = [secrets.token_hex(16) for _ in range(2)]
    first_port = 49152 + secrets.randbelow(10000)
    ports = [first_port, first_port+1]
    monitor = MONITOR
    if args.fail_closed:
        monitor = MONITOR.replace(
            "nonces=request['nonces']; ports=request['ports']",
            "nonces=request['nonces']; ports=request['ports']\n"
            "s.bind(('0.0.0.0',47889)); s.listen(2); s.settimeout(15)")
        monitor = monitor.replace(
            "print(json.dumps({'observed':observed,'https':https,",
            "print(json.dumps({'observed':observed,'https':https,'receiver':receiver,")
        monitor = monitor.replace(
            "deadline=time.monotonic()+120",
            "receiver=0\ntry:\n s.accept(); receiver+=1\nexcept socket.timeout: pass\n"
            "deadline=time.monotonic()+60")
    remote = "python3 -u -c '" + monitor.replace("'", "'\"'\"'") + "'"
    ssh = [str(system/'OpenSSH/ssh.exe'), '-6', '-o', 'BatchMode=yes',
           '-o', 'StrictHostKeyChecking=yes', '-o', 'ConnectTimeout=10',
           '-o', 'IdentitiesOnly=yes', '-o', 'KexAlgorithms=+diffie-hellman-group14-sha256',
           '-i', str(args.identity_file), args.ssh_alias, remote]
    process = subprocess.Popen(ssh, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True, encoding='utf-8')
    results = []
    observer = None
    try:
        process.stdin.write(json.dumps(dict(nonces=nonces, ports=ports))+'\n')
        process.stdin.flush()
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
            ready = pool.submit(process.stdout.readline)
            try:
                line = ready.result(timeout=20)
            except concurrent.futures.TimeoutError:
                process.kill()
                raise RuntimeError('Owned receiver did not become ready.') from None
        if json.loads(line).get('ready') is not True:
            raise ValueError('Invalid owned receiver readiness.')
        for index, scope in enumerate(('ordinary-wsl', 'isolated-launcher')):
            prefix = [str(system/'wsl.exe'), '-d', args.distro, '-u', 'root', '--exec']
            if args.fail_closed:
                prefix += ['python3', '-c',
                           "import json,socket,sys; s=socket.socket(); s.settimeout(3);"
                           "r={};\ntry: s.connect((" + repr(str(endpoint)) + ",47889)); r['connected']=True\nexcept Exception: r['connected']=False\n"
                           "print(json.dumps(r))"]
            elif scope == 'ordinary-wsl':
                prefix += ['setpriv', '--reuid', '1000', '--regid', '1000', '--clear-groups']
            else:
                prefix += ['/usr/local/lib/cor-egress/manager', 'run',
                           '--generation', str(args.generation)]
                prefix += ['--scope-attestation', attestation]
                prefix += ['--']
            # No inherited Windows proxy or user shell startup participates.
            prefix += ['/usr/bin/env', '-i', 'PATH=/usr/bin:/bin', 'LANG=C.UTF-8',
                       '/usr/bin/python3', '-I', '-c', CLIENT]
            response = subprocess.run(prefix, input=json.dumps(dict(nonce=nonces[index], port=ports[index])),
                                      capture_output=True, text=True, encoding='utf-8', timeout=80)
            try:
                value = json.loads(response.stdout)
            except ValueError:
                value = dict(error='No valid client response', helper_exit=response.returncode)
                import re
                value['diagnostic'] = re.sub(r'(?:\d{1,3}\.){3}\d{1,3}', '<address>',
                                             response.stderr.strip())[:1800]
                if scope == 'isolated-launcher':
                    # Manager errors are deliberately redacted structured JSON.
                    events = response.stderr.splitlines()
                    if events:
                        try:
                            event = json.loads(events[-1])
                            value['helper_message'] = event.get('message')
                        except ValueError:
                            pass
            value.update(scope=scope, nonce=nonces[index], port=ports[index], helper_exit=response.returncode)
            results.append(value)
        process.stdin.write('stop\n')
        process.stdin.flush()
        final, _ = process.communicate(timeout=15)
        observer = json.loads(final)
        for result in results:
            received = observer['observed'][result['nonce']]
            egress = observer['https'][str(result['port'])]
            passed = (result.get('dns_reply') and result.get('https_exit') == 0
                      and result.get('https_status') == '200'
                      and result.get('ipv6_connected') is False
                      and result.get('public_ipv6_interface') is False
                      and result.get('hosts_leak_free') is True
                      and result.get('interop_disabled') is True
                      and result.get('helper_exit') == 0
                      and received['tunnel_dns'] > 0 and received['upstream_dns'] > 0
                      and received['clear_uplink'] == 0
                      and egress['syn'] > 0 and egress['source_matches_vps'])
            if result['scope'] == 'isolated-launcher':
                passed = (passed and set(result.get('interfaces', [])) == {'lo', 'wg0', 'cor-bg0'}
                          and result.get('uid') == 1000 and result.get('windows_mount_hidden')
                          and result.get('windows_ipconfig_hidden')
                          and all(int(result['caps'].get(key, '1'), 16) == 0
                                  for key in ('CapEff', 'CapPrm', 'CapAmb', 'CapBnd'))
                          and result['caps'].get('NoNewPrivs') == '1')
            result['result'] = 'PASS' if passed else 'FAIL'
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=10)
    report = dict(suite='live-wsl-owned-vps', generation=args.generation,
                  outcome='passed' if results and all(x.get('result') == 'PASS' for x in results) else 'failed',
                  cases=results, observer=observer,
                  not_run=['independent geolocation', 'physical network switch', 'sleep/resume'])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
    print(json.dumps(report))
    return 0 if report['outcome'] == 'passed' else 1


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        message = re.sub(r'(?:\d{1,3}\.){3}\d{1,3}', '<address>',
                         f'{type(error).__name__}: {error}')[:1200]
        print(json.dumps(dict(outcome='failed', message=message)))
        sys.exit(1)
