#!/usr/bin/env python3
"""Prepare a dedicated VPS service and a private Windows WireGuard profile.

Uses an existing SSH alias with strict known-host verification. Dependencies
must already exist. No keys or endpoint values are printed. Preview by default.
Run on Windows with --execute; keep the output profile outside the repository.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent

# This bootstrap is transported over authenticated SSH. Its stdin contains the
# two reviewed server files, their hashes, and the client's public key only.
REMOTE = r'''
import base64,hashlib,ipaddress,json,os,pathlib,shlex,subprocess,sys
os.umask(0o077)
env={'PATH':'/usr/sbin:/usr/bin:/sbin:/bin','LANG':'C','LC_ALL':'C'}
def run(args,data=None):
    p=subprocess.run(args,input=data,capture_output=True,env=env,timeout=45)
    if p.returncode: raise RuntimeError('Dedicated deployment command failed')
    return p.stdout
def write(path,data,mode):
    if path.is_symlink() or path.exists(): raise ValueError('Existing object preserved')
    fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,mode)
    with os.fdopen(fd,'wb') as f: f.write(data); f.flush(); os.fsync(f.fileno())
request=json.load(sys.stdin)
root=pathlib.Path('/var/lib/cor-wsl-egress')
library=pathlib.Path('/usr/local/lib/cor-wsl-egress')
unit=pathlib.Path('/etc/systemd/system/cor-wsl-egress.service')
for parent in (pathlib.Path('/var/lib'),pathlib.Path('/usr/local/lib'),pathlib.Path('/etc/systemd/system')):
    if parent.is_symlink() or parent.stat().st_uid!=0 or parent.stat().st_mode&0o022:
        raise ValueError('Unsafe deployment parent')
if request.get('action')=='rollback':
    journal=root/'deployment.json'
    if root.is_symlink() or journal.is_symlink() or not journal.is_file():
        raise ValueError('Missing trusted deployment journal')
    record=json.loads(journal.read_text())
    if record.get('owner')!='ClashOfRust.WslEgress.Deployment' or record.get('schema')!=1:
        raise ValueError('Unknown deployment journal')
    for name,expected in record['files'].items():
        path=pathlib.Path(name)
        if path not in (library/'manage.py',unit) or path.is_symlink() or hashlib.sha256(path.read_bytes()).hexdigest()!=expected:
            raise ValueError('Deployed service changed; preserve later changes')
    run(['systemctl','disable','--now','cor-wsl-egress.service'])
    if (root/'state.json').exists():
        run(['python3','-I',str(library/'manage.py'),'rollback','--execute'])
    added=[shlex.split(line)[1:] for line in run(['ufw','show','added']).decode().splitlines() if line.startswith('ufw ')]
    rules=list(record['ufw_added'])
    if record.get('ufw_pending') and record['ufw_pending'] not in rules:
        rules.append(record['ufw_pending'])
    for rule in reversed(rules):
        if rule not in added: continue
        if not rule[-1].startswith('cor-wsl-egress-'): raise ValueError('Unowned UFW rule')
        delete=(['route','delete',*rule[1:]] if rule[0]=='route' else ['delete',*rule])
        run(['ufw','--force',*delete])
    record['phase']='restored'
    journal.write_text(json.dumps(record,indent=2)+'\n')
    print(json.dumps({'outcome':'succeeded','phase':'restored','private_files_retained':True}))
    sys.exit(0)
if root.exists() or library.exists() or unit.exists() or any(p.is_symlink() for p in (root,library,unit)):
    raise ValueError('Existing deployment requires its own recovery journal')
decoded={}
for name in ('manage.py','cor-wsl-egress.service'):
    record=request['files'][name]
    data=base64.b64decode(record['content'],validate=True)
    if len(data)>1048576 or b'\r' in data or hashlib.sha256(data).hexdigest()!=record['sha256']:
        raise ValueError('Server payload hash/format mismatch')
    decoded[name]=data
route=json.loads(run(['ip','-4','-j','route','get','1.1.1.1']))[0]
endpoint=str(ipaddress.IPv4Address(route['prefsrc']))
if not ipaddress.IPv4Address(endpoint).is_global: raise ValueError('VPS needs a global fixed IPv4')
uplink=route['dev']
if not uplink.replace('-','').replace('_','').replace('.','').isalnum() or len(uplink)>15:
    raise ValueError('Unsupported uplink')
root.mkdir(mode=0o700)
library.mkdir(mode=0o755)
write(library/'manage.py',decoded['manage.py'],0o755)
write(unit,decoded['cor-wsl-egress.service'],0o644)
record={'schema':1,'owner':'ClashOfRust.WslEgress.Deployment','phase':'prepared',
        'files':{str(library/'manage.py'):hashlib.sha256(decoded['manage.py']).hexdigest(),
                 str(unit):hashlib.sha256(decoded['cor-wsl-egress.service']).hexdigest()},
        'ufw_added':[],'service_was_active':False}
def save():
    (root/'deployment.json').write_text(json.dumps(record,indent=2)+'\n')
save()
profile={'schema':1,'server_address':'10.77.0.1/24','peer_address':'10.77.0.2',
         'resolver_upstream':'1.1.1.1','listen_port':request['port'],
         'peer_public_key':request['peer_public_key']}
run(['python3','-I',str(library/'manage.py'),'install','--execute'],json.dumps(profile).encode())
if 'Status: active' in run(['ufw','status']).decode():
    rules=[
      ['allow',str(request['port'])+'/udp','comment','cor-wsl-egress-endpoint'],
      ['allow','in','on','cor-wsl0','from','10.77.0.2','to','10.77.0.1','port','53','proto','udp','comment','cor-wsl-egress-dns-udp'],
      ['allow','in','on','cor-wsl0','from','10.77.0.2','to','10.77.0.1','port','53','proto','tcp','comment','cor-wsl-egress-dns-tcp'],
      ['route','allow','in','on','cor-wsl0','out','on',uplink,'from','10.77.0.2','to','any','comment','cor-wsl-egress-forward'],
    ]
    before=run(['ufw','show','added']).decode()
    if 'cor-wsl-egress-' in before: raise ValueError('Owned UFW comment collision')
    for rule in rules:
        record['ufw_pending']=rule; save()
        result=run(['ufw',*rule]).decode()
        if 'Skipping' in result: raise ValueError('Unowned equivalent UFW rule preserved; review required')
        record['ufw_added'].append(rule); record.pop('ufw_pending',None); save()
run(['systemctl','daemon-reload'])
record['service_was_active']='active' in run(['systemctl','is-active','cor-wsl-egress.service'],False).decode()
save()
run(['systemctl','enable','--now','cor-wsl-egress.service'])
state=json.loads((root/'state.json').read_text())
record['phase']='running'; save()
print(json.dumps({'schema':1,'endpoint_ipv4':endpoint,'peer_public_key':state['server_public_key'],
                  'ssh_active':subprocess.run(['systemctl','is-active','--quiet','ssh'],env=env).returncode==0,
                  'proxy_active':subprocess.run(['systemctl','is-active','--quiet','sing-box'],env=env).returncode==0}))
'''


def run(args, data=None, timeout=90, env=None):
    process = subprocess.run(args, input=data, capture_output=True, timeout=timeout, env=env)
    if process.returncode:
        raise RuntimeError('Preparation failed; inspect private local/VPS journals. No private output disclosed.')
    return process.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ssh-alias', required=True)
    parser.add_argument('--identity-file', type=Path, required=True)
    parser.add_argument('--distro')
    parser.add_argument('--profile', type=Path)
    parser.add_argument('--port', type=int, default=51820)
    parser.add_argument('--execute', action='store_true')
    parser.add_argument('--rollback-server', action='store_true')
    parser.add_argument('--update-service', action='store_true')
    args = parser.parse_args()
    if not args.execute:
        return dict(outcome='not_run', message='Preview: prepare dedicated VPS service and a new private profile.')
    if sys.platform != 'win32' or not args.identity_file.is_file():
        raise ValueError('Use Windows and an existing local SSH identity.')
    if not args.ssh_alias or args.ssh_alias.startswith('-') or any(x.isspace() for x in args.ssh_alias):
        raise ValueError('Use a single existing SSH alias.')
    if not 1 <= args.port <= 65535:
        raise ValueError('Invalid endpoint port.')
    ssh = str(Path(os.environ['SystemRoot'])/'System32/OpenSSH/ssh.exe')
    remote = "python3 -c '" + REMOTE.replace("'", "'\"'\"'") + "'"
    ssh_args = [
        ssh, '-6', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=10',
        '-o', 'StrictHostKeyChecking=yes', '-o', 'IdentitiesOnly=yes',
        '-o', 'KexAlgorithms=+diffie-hellman-group14-sha256',
        '-i', str(args.identity_file), args.ssh_alias, remote,
    ]
    if args.rollback_server:
        return json.loads(run(ssh_args, json.dumps(dict(action='rollback')).encode()))
    if args.update_service:
        # This path does not need a client profile or key generation. It
        # preserves the existing server key, peer and exact UFW rules.
        files = {}
        for name in ('manage.py', 'cor-wsl-egress.service'):
            content = (HERE/'server'/name).read_bytes()
            files[name] = dict(content=base64.b64encode(content).decode(),
                               sha256=hashlib.sha256(content).hexdigest())
        journal = '/var/lib/cor-wsl-egress/deployment.json'
        stage = f'/var/lib/cor-wsl-egress/.manage-update-{os.urandom(8).hex()}'
        remote_update = (
            "set -eu; test ! -e " + stage + "; "
            "install -o root -g root -m 0755 /dev/stdin " + stage + "; "
            "python3 -c 'import hashlib,json,os,pathlib,sys;"
            "stage=pathlib.Path(sys.argv[1]);journal=pathlib.Path(sys.argv[2]);"
            "old=pathlib.Path(sys.argv[3]);data=stage.read_bytes();"
            "record=json.loads(journal.read_text());expected=record[\"files\"][str(old)];"
            "assert record[\"owner\"]==\"ClashOfRust.WslEgress.Deployment\";"
            "assert hashlib.sha256(old.read_bytes()).hexdigest()==expected;"
            "assert len(data)<1048576 and b\"\\r\" not in data;"
            "os.replace(stage,old);record[\"files\"][str(old)]=hashlib.sha256(data).hexdigest();"
            "journal.write_text(json.dumps(record,indent=2)+\"\\n\")' " + stage + " " + journal + " /usr/local/lib/cor-wsl-egress/manage.py; "
            "systemctl restart cor-wsl-egress.service; "
            "systemctl is-active --quiet ssh; systemctl is-active --quiet cor-wsl-egress.service"
        )
        update_args = ssh_args[:-1] + [remote_update]
        run(update_args, (HERE/'server/manage.py').read_bytes())
        return dict(outcome='succeeded', service='restarted', keys_changed=False,
                    ssh_active=True, runtime_acceptance='NOT_RUN')
    if not args.distro or not args.profile:
        raise ValueError('New deployments require --distro and --profile.')
    profile = args.profile.resolve()
    repository = HERE.parents[1]
    if profile.is_relative_to(repository):
        raise ValueError('Private profiles must be outside the repository.')
    expected_parent = Path(os.environ['LOCALAPPDATA'])/'ClashOfRust/WslEgress'
    if profile.parent != expected_parent.resolve():
        raise ValueError('Use the protected per-user ClashOfRust/WslEgress profile directory.')
    for path in (expected_parent, *expected_parent.parents):
        if path.exists() and path.is_symlink():
            raise ValueError('Redirected private profile directory.')
    if profile.exists() or profile.is_symlink() or profile.with_suffix('.pending.json').exists():
        raise ValueError('Existing private profile preserved; inspect the existing deployment.')
    profile.parent.mkdir(parents=True, exist_ok=True)
    # Set a complete ACL: /grant:r would retain unlisted explicit permissions.
    # Only this fixed directory path reaches the child environment; no key does.
    acl_script = r'''
$ErrorActionPreference='Stop'
$path=[Environment]::GetEnvironmentVariable('COR_EGRESS_PROFILE_DIRECTORY')
$acl=New-Object Security.AccessControl.DirectorySecurity
$acl.SetAccessRuleProtection($true,$false)
$user=[Security.Principal.WindowsIdentity]::GetCurrent().User
foreach($sid in @($user.Value,'S-1-5-18','S-1-5-32-544')){
  $rule=New-Object Security.AccessControl.FileSystemAccessRule(
    (New-Object Security.Principal.SecurityIdentifier($sid)),'FullControl',
    'ContainerInherit,ObjectInherit','None','Allow')
  $acl.AddAccessRule($rule)
}
$acl.SetOwner($user)
Set-Acl -LiteralPath $path -AclObject $acl
'''
    powershell = Path(os.environ['SystemRoot'])/'System32/WindowsPowerShell/v1.0/powershell.exe'
    run([str(powershell), '-NoLogo', '-NoProfile', '-NonInteractive', '-Command', acl_script],
        env={**os.environ, 'COR_EGRESS_PROFILE_DIRECTORY': str(profile.parent)})
    wsl = str(Path(os.environ['SystemRoot'])/'System32/wsl.exe')
    prefix = [wsl, '-d', args.distro, '-u', 'root', '--exec']
    key = run([*prefix, '/usr/bin/wg', 'genkey']).strip()
    public = run([*prefix, '/usr/bin/wg', 'pubkey'], key+b'\n').decode().strip()
    pending = profile.with_suffix('.pending.json')
    value = dict(schema=1, backend='WireGuard', manager_distro=args.distro,
                 endpoint_port=args.port, private_key=key.decode(),
                 tunnel_address='10.77.0.2/32', resolver_ipv4='10.77.0.1',
                 uid=1000, gid=1000)
    with pending.open('x', encoding='utf-8') as stream:
        json.dump(value, stream)
        stream.flush()
        os.fsync(stream.fileno())
    files = {}
    for name in ('manage.py', 'cor-wsl-egress.service'):
        content = (HERE/'server'/name).read_bytes()
        files[name] = dict(content=base64.b64encode(content).decode(),
                           sha256=hashlib.sha256(content).hexdigest())
    request = dict(schema=1, action='install', port=args.port, peer_public_key=public, files=files)
    # Quoting is for the remote POSIX command. No private values enter argv.
    result = json.loads(run(ssh_args, json.dumps(request).encode()))
    if result.get('schema') != 1 or not result.get('ssh_active') or not result.get('proxy_active'):
        raise ValueError('Remote service health check failed; preserve pending profile and recovery journal.')
    value.update(endpoint_ipv4=result['endpoint_ipv4'], peer_public_key=result['peer_public_key'])
    with profile.open('x', encoding='utf-8') as stream:
        json.dump(value, stream)
        stream.flush()
        os.fsync(stream.fileno())
    pending.unlink()
    return dict(outcome='succeeded', profile_created=True, vps_service='running',
                management_services_preserved=True, runtime_acceptance='NOT_RUN')


if __name__ == '__main__':
    try:
        print(json.dumps(main()))
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.SubprocessError):
        print(json.dumps(dict(outcome='failed', message='Preparation failed; retain private recovery journals.')))
        sys.exit(1)
