"""Inventory, verify and combine the inputs and outputs of the memory runners."""
import json
import os
import platform
import shutil
from pathlib import Path

from build_support import sha256, write_json
from memory_lifecycle import COOLDOWN_SECONDS, owner_pair


VISIBLE = ('home', 'proxies-collapsed', 'proxies-search', 'home-after-proxies')
TRAY = ('tray-open', 'tray-hidden', 'tray-reopened', 'tray-search', 'tray-search-hidden',
        'tray-search-reopened', 'tray-after-cycles', 'tray-after-cycles-hidden')
PAGINATION_PHASES = ('visible', 'scrolled', 'hidden', 'cycles-hidden')


def runner():
    return {'system': platform.system(), 'machine': platform.machine(), 'cpus': os.cpu_count(),
            'os': platform.platform(), 'image': os.getenv('ImageOS'), 'image_version': os.getenv('ImageVersion')}


def compatible_runner(prepared, measured):
    return all(prepared[key] == measured[key] for key in ('system', 'machine', 'cpus', 'image'))


def scene_seconds(scene, seconds):
    if scene.endswith(('tray-hidden', 'tray-search-hidden', 'tray-after-cycles-hidden',
                       'tui-closed', 'tui-reclosed', 'tui-after-cycles-closed')):
        return max(seconds, COOLDOWN_SECONDS)
    return seconds


def pagination_cases(capabilities):
    cases = {}
    if capabilities.get('group_pagination'):
        cases['proxies'] = ('proxy_groups_paginated', False)
    if capabilities.get('profile_pagination'):
        cases['profiles-grid'] = ('profiles_paginated', False)
        cases['profiles-list'] = ('profiles_paginated', True)
    return cases


def plan(capabilities):
    shards = {}
    for variant in (['plain', 'rules'] if capabilities['overrides'] else ['plain']):
        prefix = 'rules-' if variant == 'rules' else ''
        background = prefix + 'background-'
        gui = [prefix + name for name in VISIBLE]
        if capabilities['slint']:
            gui.extend(background + name for name in TRAY)
        shards[variant + '-gui'] = gui
        if capabilities['slint'] and capabilities['cli']:
            names = ['headless']
            if capabilities['tui']:
                pages = ['home', 'nodes', 'connections', 'network']
                if capabilities['settings']:
                    pages.extend(('settings', 'setting-editor'))
                if capabilities['catalog']:
                    pages.extend(('profiles', 'shortcuts'))
                pages.append('closed')
                if capabilities['catalog']:
                    pages.extend(('reopened', 'reclosed'))
                pages.extend(('after-cycles', 'after-cycles-closed'))
                names.extend('tui-' + page for page in pages)
            shards[variant + '-cli'] = [background + name for name in names]
    for page in pagination_cases(capabilities):
        shards['pagination-' + page] = [f'pagination-{page}-{mode}-{phase}'
                                         for mode in ('paged', 'all') for phase in PAGINATION_PHASES]
    return shards


def files(root):
    result = {}
    for path in sorted(root.rglob('*')):
        if path.is_symlink():
            raise ValueError('Unexpected symlink in memory artifact')
        if path.is_file():
            result[path.relative_to(root).as_posix()] = path
    return result


def freeze(directory, metadata, capabilities):
    payload = files(directory / 'payload')
    manifest = {'metadata': metadata, 'capabilities': capabilities, 'shards': plan(capabilities),
                'runner': runner(), 'payload': {name: {'sha256': sha256(path),
                                                       'executable': bool(path.stat().st_mode & 0o111)}
                                                 for name, path in payload.items()}}
    write_json(directory / 'manifest.json', manifest)
    digest = sha256(directory / 'manifest.json')
    if os.getenv('GITHUB_OUTPUT'):
        with Path(os.environ['GITHUB_OUTPUT']).open('a') as output:
            print('digest=' + digest, 'shards=' + json.dumps(list(manifest['shards'])), sep='\n', file=output)
    return digest


def manifest_for(directory, expected_digest):
    path = directory / 'manifest.json'
    if sha256(path) != expected_digest:
        raise ValueError('Prepared memory input manifest does not match the prepare job')
    manifest = json.loads(path.read_text())
    if manifest['shards'] != plan(manifest['capabilities']):
        raise ValueError('Unexpected scene plan for this source')
    return manifest


def check_payload(directory, manifest):
    payload = files(directory / 'payload')
    if payload.keys() != manifest['payload'].keys():
        raise ValueError('Prepared memory input inventory differs')
    for name, path in payload.items():
        entry = manifest['payload'][name]
        if sha256(path) != entry['sha256']:
            raise ValueError(f'Prepared input digest differs for {name}')
        if entry['executable']:
            path.chmod(0o755)
    if not compatible_runner(manifest['runner'], runner()):
        raise ValueError('Measurement runner differs in platform, architecture, CPU count or image family')


def combine(manifest, digest, directory, output, read_scene):
    expected = manifest['shards']
    metadata = manifest['metadata'].copy()
    folders = list(directory.iterdir())
    prefix = f'memory-scene-{digest}-'
    reports = {path.name.removeprefix(prefix): path for path in folders
               if path.is_dir() and path.name.startswith(prefix)}
    if len(expected) == 1 and (directory / 'receipt.json').is_file():
        reports = {next(iter(expected)): directory}
    elif len(reports) != len(folders) or set(reports) != set(expected):
        raise ValueError('Missing or unexpected memory shards')
    metadata.update(background_scenes=[], terminal_scenes=[], shards={})
    for shard, scenes in expected.items():
        folder = reports[shard]
        receipt = json.loads((folder / 'receipt.json').read_text())
        if (receipt['shard'] != shard or receipt['digest'] != digest
                or receipt['scenes'] != scenes
                or not compatible_runner(manifest['runner'], receipt['runner'])):
            raise ValueError(f'Inconsistent provenance or scene list for {shard}')
        available = files(folder)
        actual = {Path(name).stem for name in available if name.endswith('.jsonl')
                  and not name.endswith('-mapping-samples.jsonl')}
        if actual != set(scenes) or len(scenes) != len(set(scenes)):
            raise ValueError(f'Incomplete or duplicate memory scenes for {shard}')
        identity = None
        pagination_pairs = {}
        for scene in scenes:
            role = 'daemon' if shard.endswith('-cli') else 'gui'
            seconds = scene_seconds(scene, metadata['gui_sample_seconds'])
            rows = read_scene(folder / (scene + '.jsonl'), seconds, role)
            samples = json.loads((folder / (scene + '-mapping-peaks.json')).read_text())
            mapping_rows = [json.loads(line) for line in
                            (folder / (scene + '-mapping-samples.jsonl')).read_text().splitlines()]
            if (not mapping_rows or samples['samples'] != len(mapping_rows) or samples['unavailable_samples']
                    or any(row['unavailable_processes'] or not {role, 'core'} <= row['roles'].keys()
                           for row in mapping_rows)):
                raise ValueError(f'Incomplete process mappings in {scene}')
            if len(rows) < seconds * 2 or len(mapping_rows) < max(1, seconds // 2):
                raise ValueError(f'Incomplete sampling cadence in {scene}')
            if not {role, 'core'} <= {row['role'] for row in
                                     json.loads((folder / (scene + '-mappings.json')).read_text())}:
                raise ValueError(f'Incomplete final process mappings in {scene}')
            if 'background-' in scene:
                closed = scene.endswith(('headless', 'hidden', 'closed'))
                identity = owner_pair(rows, role, identity, closed)
                if 'tui-' in scene and not closed:
                    owner_pair(rows, 'tui')
                target = 'terminal_scenes' if 'tui-' in scene else 'background_scenes'
                metadata[target].append(scene)
            elif shard.startswith('pagination-'):
                mode = scene.removeprefix(shard + '-').split('-', 1)[0]
                pagination_pairs[mode] = owner_pair(rows, role, pagination_pairs.get(mode),
                                                    closed=scene.endswith('-hidden'))
            else:
                owner_pair(rows, role)
        for name, path in available.items():
            if name == 'receipt.json':
                continue
            target = output / name
            if target.exists():
                raise ValueError(f'Duplicate memory report file {name}')
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, target)
        metadata['shards'][shard] = receipt
    return metadata
