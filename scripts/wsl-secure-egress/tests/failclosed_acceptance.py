#!/usr/bin/env python3
"""Opt-in controlled fail-closed check; receiver and client use only the VPS.

The caller closes the Windows endpoint after this script prints READY. The
script then attempts one ordinary-WSL connection and requires both client
failure and an explicit VPS receiver NO_RECEIPT. It never changes networking.
"""
import json
import ipaddress
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys

RECEIVER = r'''
import json,socket,time
s=socket.socket();s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)
s.bind(("0.0.0.0",47889));s.listen(1);s.settimeout(45)
print(json.dumps({"ready":True}),flush=True)
try:
 c,address=s.accept()
 print(json.dumps({"received":True,"source_redacted":True}),flush=True)
 c.close()
except socket.timeout:
 print(json.dumps({"received":False,"receipt":"NO_RECEIPT"}),flush=True)
'''


def validate_endpoint_ipv4(value):
    try:
        endpoint = ipaddress.IPv4Address(value)
    except ipaddress.AddressValueError as error:
        raise ValueError('A numeric IPv4 endpoint is required.') from error
    if not endpoint.is_global:
        raise ValueError('The configured endpoint must be a public IPv4 address.')
    return str(endpoint)


def main():
    if sys.platform != 'win32':
        raise ValueError('Windows host required.')
    alias = os.environ.get('COR_SSH_ALIAS', '')
    identity = Path(os.environ.get('COR_SSH_IDENTITY', ''))
    distro = os.environ.get('COR_WSL_DISTRO', '')
    endpoint = validate_endpoint_ipv4(os.environ.get('COR_WSL_ENDPOINT_IPV4', ''))
    if not alias or alias.startswith('-') or any(c.isspace() for c in alias) or not identity.is_file() or not distro:
        raise ValueError('COR_SSH_ALIAS, COR_SSH_IDENTITY and COR_WSL_DISTRO are required.')
    remote = "python3 -u -c '" + RECEIVER.replace("'", "'\"'\"'") + "'"
    system = Path(os.environ['SystemRoot'])/'System32'
    process = subprocess.Popen([
        str(system/'OpenSSH/ssh.exe'), '-6', '-o', 'BatchMode=yes',
        '-o', 'StrictHostKeyChecking=yes', '-o', 'ConnectTimeout=10',
        '-o', 'IdentitiesOnly=yes', '-o', 'KexAlgorithms=+diffie-hellman-group14-sha256',
        '-i', str(identity), alias, remote,
    ], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
       text=True, encoding='utf-8')
    try:
        line = process.stdout.readline()
        if not json.loads(line).get('ready'):
            raise RuntimeError('Controlled VPS receiver did not become ready.')
        print(json.dumps(dict(stage='READY', receiver='owned-vps')), flush=True)
        if sys.stdin.readline().strip() != 'CLOSED':
            raise RuntimeError('Endpoint closure confirmation was not received.')
        client = "\n".join([
            "import json,socket",
            "s=socket.socket();s.settimeout(5);r={}",
            "try:",
            " s.connect((" + repr(endpoint) + ",47889));r['connected']=True",
            "except Exception as e:",
            " r['connected']=False;r['error_type']=type(e).__name__",
            "print(json.dumps(r))",
        ])
        result = subprocess.run([
            str(system/'wsl.exe'), '-d', distro, '-u', 'root', '--exec',
            '/usr/bin/env', '-i', 'PATH=/usr/bin:/bin', '/usr/bin/python3', '-c', client,
        ], capture_output=True, text=True, timeout=20)
        try:
            connected = json.loads(result.stdout).get('connected')
        except ValueError:
            connected = None
        receipt_line = process.stdout.readline()
        try:
            receipt = json.loads(receipt_line)
        except ValueError:
            receipt = None
        process.wait(timeout=10)
        report = dict(suite='controlled-fail-closed', outcome='passed' if (
            connected is False and receipt and receipt.get('received') is False and
            receipt.get('receipt') == 'NO_RECEIPT') else 'failed',
            client=dict(connected=connected, exit=result.returncode,
                        error_type=(json.loads(result.stdout).get('error_type')
                                    if result.stdout else None)),
            receiver=receipt, endpoint_closure_required=True,
            receiver_source='owned-vps')
        print(json.dumps(report), flush=True)
        return 0 if report['outcome'] == 'passed' else 1
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=10)


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(json.dumps(dict(outcome='failed', message=re.sub(
            r'(?:\d{1,3}\.){3}\d{1,3}', '<address>', str(error))[:1000])))
        sys.exit(1)
