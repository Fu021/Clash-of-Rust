"""Build a Windows x64 installer or a native Debian Linux package."""
import argparse
import os
import platform
import shutil
import sys
import json
import subprocess
from build_support import ROOT, copy_static, file_version, nsis, package_version, run, sha256, stage_resources, staging_directory, validate_version
import importlib.util

_spec = importlib.util.spec_from_file_location('prepare_resources', ROOT/'scripts/prepare-resources.py')
_module = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_module)
prepare = _module.prepare


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--proxy', default='')
    parser.add_argument('--skip-prepare', action='store_true')
    parser.add_argument('--version', default=package_version())
    parser.add_argument('--system', choices=('windows','linux'), default='windows' if os.name == 'nt' else 'linux')
    parser.add_argument('--arch', choices=('x64','arm64'), default='arm64' if platform.machine().lower() in ('aarch64','arm64') else 'x64')
    args = parser.parse_args()
    version = validate_version(args.version)
    host = 'windows' if os.name == 'nt' else 'linux' if sys.platform.startswith('linux') else 'unsupported'
    host_arch = 'arm64' if platform.machine().lower() in ('aarch64','arm64') else 'x64'
    if (args.system,args.arch) != (host,host_arch):
        raise ValueError('Packaging requires a native build on the selected system/architecture')
    if args.system == 'windows' and args.arch != 'x64':
        raise ValueError('The NSIS installer currently supports Windows x64 only')
    if args.system == 'linux':
        from build_deb import require_tools
        require_tools()
    resources = ROOT/'bundle/resources' if args.system == 'windows' else ROOT/f'bundle/{args.system}-{args.arch}/resources'
    if not args.skip_prepare:
        resources = prepare(args.system,args.arch,args.proxy)
    copy_static(resources.parent)
    env = dict(os.environ)
    if version == package_version():
        env.pop('CLASH_OF_RUST_BUILD_VERSION',None)
    else:
        env['CLASH_OF_RUST_BUILD_VERSION'] = version
    run(['cargo','build','--release','--locked'],env=env)
    metadata = json.loads(subprocess.check_output(['cargo','metadata','--format-version','1','--no-deps','--offline'],cwd=ROOT))
    from pathlib import Path
    binary = Path(metadata['target_directory'])/'release'/('clash-of-rust.exe' if args.system == 'windows' else 'clash-of-rust')
    if args.system == 'windows' and file_version(binary) != version:
        raise ValueError('Application PE version differs from package version')
    # Maintain the developer bundle for installer tests; installer inputs always
    # come from a fresh whitelist stage, so old MSYS/Bash files cannot leak in.
    shutil.copy2(binary, resources.parent/binary.name)
    (ROOT/'dist').mkdir(exist_ok=True)
    artifacts = []
    with staging_directory('release') as stage:
        stage_resources(resources,stage,args.system)
        shutil.copy2(binary,stage/binary.name)
        if args.system == 'windows':
            artifact = ROOT/f'dist/Clash-of-Rust-{version}-windows-x64-setup.exe'
            run([nsis(args.proxy),'/V2','/INPUTCHARSET','UTF8',f'/DAPP_VERSION={version}',f'/DPAYLOAD={stage}',f'/DOUTPUT={artifact}',ROOT/'installer/clash-of-rust.nsi'])
            if file_version(artifact) != version:
                raise ValueError('Installer PE version differs from package version')
            artifacts.append(artifact)
        else:
            from build_deb import build_deb
            artifacts.append(build_deb(stage,version,args.arch))
    for artifact in artifacts:
        artifact.with_name(artifact.name+'.sha256').write_text(sha256(artifact)+'\n',encoding='ascii')
        print('Package ready:',artifact)
    if args.system == 'windows':
        run([sys.executable, ROOT/'scripts/verify-icon-resources.py',binary,artifact])


if __name__ == '__main__':
    main()
