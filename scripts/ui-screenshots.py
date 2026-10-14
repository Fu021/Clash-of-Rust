"""Select verified CI inputs, then render its source UI in isolated native VMs."""
import argparse
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import zlib

from build_support import ROOT, cargo_messages, host_arch, host_system, write_json, sha256
import workflow_artifacts

PLATFORMS = [
    {'os': 'windows-latest', 'system': 'windows', 'arch': 'x64', 'target': 'x86_64-pc-windows-msvc', 'python': '3.11'},
    {'os': 'windows-11-arm', 'system': 'windows', 'arch': 'arm64', 'target': 'aarch64-pc-windows-msvc', 'python': '3.13'},
    {'os': 'ubuntu-22.04', 'system': 'linux', 'arch': 'x64', 'target': 'x86_64-unknown-linux-gnu', 'python': '3.11'},
    {'os': 'ubuntu-22.04-arm', 'system': 'linux', 'arch': 'arm64', 'target': 'aarch64-unknown-linux-gnu', 'python': '3.11'},
    {'os': 'macos-15-intel', 'system': 'macos', 'arch': 'x64', 'target': 'x86_64-apple-darwin', 'python': '3.11'},
    {'os': 'macos-latest', 'system': 'macos', 'arch': 'arm64', 'target': 'aarch64-apple-darwin', 'python': '3.11'},
]
PAGES = ('home', 'proxies-ascending', 'profiles', 'connections', 'rules', 'logs',
         'diagnostics', 'ip-summary', 'settings', 'claude', 'claude-risk', 'claude-normal',
         'claude-unknown', 'profiles-edit', 'connections-details', 'rules-editor',
         'rules-collapsed', 'profiles-delete-active', 'profiles-discard')


def github():
    release = workflow_artifacts.release_module()
    repo = release.repository()
    return release.GitHub(repo, release.credential(repo))


def resolve(run_id, platform, architecture):
    client = github()
    if not run_id:
        candidates = client('/actions/workflows/ci.yml/runs?branch=main&event=push&status=success&per_page=10')['workflow_runs']
        if not candidates:
            raise ValueError('No successful main CI available')
        run_id = candidates[0]['id']
    run = client(f'/actions/runs/{run_id}')
    if (run['path'].split('@', 1)[0] != '.github/workflows/ci.yml'
        or run['head_repository']['full_name'].lower() != client.repo.lower()
        or (run['status'], run['conclusion']) != ('completed', 'success')):
        raise ValueError('Select a successful build CI from this repository')
    jobs = client(f'/actions/runs/{run_id}/jobs?filter=latest&per_page=100')['jobs']
    expected = {'build-'+p['system']+'-'+p['arch'] for p in PLATFORMS}
    if ({job['name'] for job in jobs} != {name.removeprefix('build-') for name in expected}
        or len(jobs) != 6 or any(job['conclusion'] != 'success' for job in jobs)):
        raise ValueError('Selected CI must pass all six native platforms')
    artifacts = client(f'/actions/runs/{run_id}/artifacts?per_page=100')['artifacts']
    builds = [a for a in artifacts if a['name'] in expected]
    if len(builds) != 6 or {a['name'] for a in builds} != expected or any(a['expired'] for a in builds):
        raise ValueError('Selected CI build artifacts are incomplete or expired')
    matrix = [p for p in PLATFORMS if platform in ('all', p['system']) and architecture in ('all', p['arch'])]
    if not matrix:
        raise ValueError('No selected native screenshot platform')
    result = {'commit': run['head_sha'], 'ci_run_id': run_id, 'matrix': {'include': matrix}}
    print(json.dumps(result))
    if output := os.environ.get('GITHUB_OUTPUT'):
        with open(output, 'a') as stream:
            for name, value in result.items():
                stream.write(name+'='+json.dumps(value, separators=(',', ':'))+'\n' if name == 'matrix' else name+'='+str(value)+'\n')


def png(rgba, width, height, destination):
    if width < 400 or height < 300 or len(rgba) != width*height*4:
        raise ValueError('Invalid renderer pixel buffer')
    # Detect missing/blank rendering instead of uploading an empty success image.
    if len({rgba[index:index+4] for index in range(0, len(rgba), 64)}) < 8:
        raise ValueError('Renderer returned a blank image')
    def chunk(kind, body):
        return struct.pack('>I', len(body))+kind+body+struct.pack('>I', zlib.crc32(kind+body))
    pixels = b''.join(b'\0'+rgba[y*width*4:(y+1)*width*4] for y in range(height))
    destination.write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0))
                           +chunk(b'IDAT', zlib.compress(pixels))+chunk(b'IEND', b''))


def render(source, run_id, system, arch, output, sizes, scenes=None, report_file=None):
    if (host_system(), host_arch()) != (system, arch):
        raise ValueError('Screenshots require a matching native runner')
    client = github()
    source = source.resolve(strict=True)
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=source, text=True).strip()
    # The selected CI's version and dependency lock, not the workflow branch,
    # are the identity against which the immutable payload is checked.
    import build_support
    build_support.ROOT = source
    workflow_artifacts.ROOT = source
    build = output/'ci-build'
    workflow_artifacts.fetch_build(client, run_id, system, arch, build, commit)
    manifest = json.loads((build/'manifest.json').read_text())
    adapter = ROOT/'scripts/ui_capture.rs'
    preview = source/'src/ui_preview.rs'
    harness = source/'tests/ui_preview.rs'
    original_preview, original_harness = preview.read_bytes(), harness.read_bytes()
    adapter_destination = source/'src/ui_capture_adapter.rs'
    if adapter_destination.exists():
        raise ValueError('Selected source already has a conflicting capture adapter')
    shutil.copy2(adapter, adapter_destination)
    preview.write_text(original_preview.decode()+'\n#[path = "ui_capture_adapter.rs"]\nmod render_capture;\npub(crate) use render_capture::run_render_capture;\n')
    harness.write_text('#[allow(dead_code, unused_imports)]\n#[path = "../src/main.rs"]\nmod app;\nfn main() { app::ui_preview::run_render_capture().expect("native render capture"); }\n')
    report = {'schema': 1, 'ci_run_id': run_id, 'commit': commit, 'version': manifest['version'],
              'system': system, 'arch': arch, 'source_artifact': json.loads((build/'source-artifact.json').read_text()),
              'ci_gui_sha256': manifest['files']['payload/clash-of-rust'+('.exe' if system == 'windows' else '')]['sha256'],
              'adapter_sha256': sha256(adapter), 'data': 'Fixed demonstration data; no account or live network requests',
              'rendering': 'Native Iced pixels from the selected source; screenshot-only harness compiled in this workflow',
              'status': 'running', 'images': []}
    write_json(output/'metadata.json', report)
    try:
        with (output/'build.log').open('w') as log:
            messages = cargo_messages(['cargo', 'test', '--release', '--locked', '--features', 'ui-preview',
                                      '--test', 'ui_preview', '--no-run', '--message-format=json'],
                                     cwd=source, stderr=log, log=output/'build.jsonl', timeout=2400)
        entries = [json.loads(line) for line in messages.splitlines()]
        binaries = [e['executable'] for e in entries if e.get('reason') == 'compiler-artifact'
                    and e.get('executable') and e['target']['name'] == 'ui_preview']
        if len(binaries) != 1:
            raise ValueError('Screenshot harness executable missing')
        report['capture_executable_sha256'] = sha256(binaries[0])
        suffixes = ('', '-light') if sizes == 'normal' else ('-compact', '-compact-light') if sizes == 'compact' else ('', '-light', '-compact', '-compact-light')
        if report_file:
            report['data']='Actual native test results; controlled HTTPS responses are explicitly labelled, not real subscription exits'
            report['validation_report_sha256']=sha256(report_file)
        for page in (scenes or PAGES):
            for suffix in suffixes:
                scene = page+suffix
                with tempfile.TemporaryDirectory(prefix='clash-ui-capture-') as temporary:
                    directory = Path(temporary)
                    capture = directory/'pixels'
                    env = dict(os.environ, CLASH_UI_PREVIEW_SCENE=scene, CLASH_UI_PREVIEW_OUTPUT=str(capture),
                               CLASH_OF_RUST_DATA_DIR=str(directory/'data'), GSETTINGS_BACKEND='memory')
                    if report_file:
                        env['CLASH_CLAUDE_VALIDATION_REPORT']=str(Path(report_file).resolve(strict=True))
                    if system == 'linux':
                        env.update(WINIT_UNIX_BACKEND='x11', WINIT_X11_SCALE_FACTOR='1')
                        env.pop('WAYLAND_DISPLAY', None)
                    with (output/(scene+'.log')).open('w') as log:
                        subprocess.run([binaries[0]], cwd=source, env=env, stdout=log, stderr=log, check=True, timeout=45)
                    width, height = json.loads(capture.with_suffix('.json').read_text())
                    destination = output/(scene+'.png')
                    png(capture.with_suffix('.rgba').read_bytes(), width, height, destination)
                    report['images'].append({'scene': scene, 'width': width, 'height': height, 'sha256': sha256(destination)})
                    write_json(output/'metadata.json', report)
        report['status'] = 'passed'
    except BaseException as error:
        report.update(status='failed', error=str(error))
        raise
    finally:
        preview.write_bytes(original_preview)
        harness.write_bytes(original_harness)
        adapter_destination.unlink(missing_ok=True)
        # Do not re-upload large CI binaries in a screenshot artifact.
        shutil.rmtree(build)
        write_json(output/'metadata.json', report)
        (output/'README.md').write_text(f'# {system} {arch} UI screenshots\n\nCI `{run_id}`, source `{commit}`, version `{manifest["version"]}`.\n\nStatus: **{report["status"]}**. {report["data"]}.\n\n'+''.join(f'- [{image["scene"]}]({image["scene"]}.png)\n' for image in report['images']))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('resolve', 'render'))
    parser.add_argument('--ci-run-id', type=int)
    parser.add_argument('--platform', choices=('all', 'windows', 'linux', 'macos'), default='all')
    parser.add_argument('--architecture', choices=('all', 'x64', 'arm64'), default='all')
    parser.add_argument('--source', type=Path)
    parser.add_argument('--system', choices=('windows', 'linux', 'macos'))
    parser.add_argument('--arch', choices=('x64', 'arm64'))
    parser.add_argument('--output', type=Path)
    parser.add_argument('--sizes', choices=('normal', 'compact', 'both'), default='both')
    parser.add_argument('--scenes', nargs='+')
    parser.add_argument('--report-file', type=Path)
    args = parser.parse_args()
    if args.operation == 'resolve':
        resolve(args.ci_run_id, args.platform, args.architecture)
    else:
        if any(value is None for value in (args.source, args.ci_run_id, args.system, args.arch, args.output)):
            parser.error('render requires --source, --ci-run-id, --system, --arch and --output')
        args.output.mkdir(parents=True, exist_ok=False)
        render(args.source, args.ci_run_id, args.system, args.arch, args.output, args.sizes, args.scenes, args.report_file)


if __name__ == '__main__':
    main()
