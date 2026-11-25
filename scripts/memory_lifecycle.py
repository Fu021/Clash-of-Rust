"""Compare resident memory of live clients and cores across frontend lifetimes."""
import json
from statistics import median


COOLDOWN_SECONDS = 61
METRICS = ('rss_bytes', 'pss_bytes', 'uss_bytes')


def owner_pair(samples, owner, expected=None, closed=False):
    for sample in samples:
        processes = sample['processes']
        if len({item['pid'] for item in processes}) != len(processes):
            raise ValueError('Duplicate process in memory sample')
        if closed and any(item['role'] in ('renderer', 'tui') for item in processes):
            raise ValueError('Frontend survived closing')
        pair = []
        for role in (owner, 'core'):
            identities = [(item['pid'], item['start_time']) for item in processes if item['role'] == role]
            if len(identities) != 1:
                raise ValueError(f'Expected one live {role} in every sample')
            pair.append(identities[0])
        if expected is None:
            expected = pair
        elif pair != expected:
            raise ValueError('Owner or core changed during the memory comparison')
    if not samples:
        raise ValueError('Empty memory scene')
    return expected


def readings(sample, client):
    roles = {'client': client, 'owner': 'host' if client == 'gui' else 'daemon',
             'core': 'core', 'total': 'total'}
    result = {label: {metric: sample[role][metric] for metric in METRICS}
              for label, role in roles.items()}
    if client == 'daemon' and any(process['role'] == 'tui' for process in sample['processes']):
        for metric in METRICS:
            parts = (sample['daemon'][metric], sample['tui'][metric])
            result['client'][metric] = None if None in parts else sum(parts)
    return result


def difference(measured, baseline):
    values = {}
    for role in measured:
        values[role] = {}
        for metric, current in measured[role].items():
            original = baseline[role][metric]
            if current is None or original is None:
                raise ValueError(f'Missing {role} {metric} for resident memory comparison')
            delta = current - original
            values[role][metric] = {'bytes': delta, 'percent': 100 * delta / original if original else None}
    return values


def nearest_sample(samples, seconds):
    target = seconds * 1000
    row = min(samples, key=lambda sample: abs(sample['elapsed_ms'] - target))
    if abs(row['elapsed_ms'] - target) > 1000:
        raise ValueError(f'Missing memory sample at {seconds} seconds')
    return row


def checkpoints(samples, client, cli_samples, tray_samples=None):
    result = {}
    for seconds in (0, 10, 30, 60):
        row = nearest_sample(samples, seconds)
        cli = nearest_sample(cli_samples, seconds)
        observed = readings(row, client)
        point = {'elapsed_ms': row['elapsed_ms'], 'memory': observed, 'cli_elapsed_ms': cli['elapsed_ms'],
                 'gap_to_cli': difference(observed, readings(cli, 'daemon'))}
        if tray_samples is not None:
            tray = nearest_sample(tray_samples, seconds)
            point['tray_elapsed_ms'] = tray['elapsed_ms']
            point['gap_to_idle_tray'] = difference(observed, readings(tray, 'gui'))
        result[str(seconds)] = point
    return result


def mapping_readings(output, name, client):
    roles = {row['role']: row['categories']
             for row in json.loads((output / (name + '-mappings.json')).read_text())}
    owner = 'gui' if client == 'gui' else 'daemon'
    front = 'renderer' if client == 'gui' else 'tui'
    measurements = {}
    for label, components in {'client': [owner, front], 'owner': [owner], 'core': ['core']}.items():
        measurements[label] = {
            'anonymous_heap_private_bytes': sum(roles.get(role, {}).get(category, {}).get('private_bytes', 0)
                                                for role in components for category in ('anonymous', 'heap')),
            'executable_pss_bytes': sum(roles.get(role, {}).get('executable', {}).get('pss_bytes', 0)
                                        for role in components),
        }
    return measurements


def comparisons(output, prefixes, read_scene):
    result = {}
    for prefix in prefixes:
        baseline_names = {'daemon': prefix + 'headless', 'gui': prefix + 'gui-headless'}
        baselines = {role: read_scene(output / (name + '.jsonl')) for role, name in baseline_names.items()}
        tray_name = prefix + 'tray-idle'
        tray_path = output / (tray_name + '.jsonl')
        tray_samples = read_scene(tray_path) if tray_path.is_file() else None
        cases = [('tray-open', 'tray-hidden', 'gui'), ('tray-search', 'tray-search-hidden', 'gui'),
                 ('tray-after-cycles', 'tray-after-cycles-hidden', 'gui')]
        if (output / (prefix + 'tui-closed.jsonl')).is_file():
            proof = json.loads((output / (prefix + 'headless-proof.json')).read_text())
            indexed = proof['tui_scenes']
            preceding = indexed[indexed.index(prefix + 'tui-closed') - 1].removeprefix(prefix)
            cases.append((preceding, 'tui-closed', 'daemon'))
            if prefix + 'tui-reclosed' in indexed:
                cases.append(('tui-reopened', 'tui-reclosed', 'daemon'))
            cases.append(('tui-after-cycles', 'tui-after-cycles-closed', 'daemon'))
        proof = json.loads((output / (prefix + 'proof.json')).read_text())
        result[prefix] = {'baseline': readings(baselines['daemon'][-1], 'daemon'),
                          'baseline_mapping': mapping_readings(output, baseline_names['daemon'], 'daemon'),
                          'pairs': {}, 'median_reopen_ms': median(proof['relaunch_to_visible_ms'])}
        for before, after, client in cases:
            visible = readings(read_scene(output / (prefix + before + '.jsonl'))[-1], client)
            after_rows = read_scene(output / (prefix + after + '.jsonl'))
            closed = readings(after_rows[-1], client)
            closed_mapping = mapping_readings(output, prefix + after, client)
            visible_mapping = mapping_readings(output, prefix + before, client)
            baseline_name = baseline_names[client]
            baseline_mapping = mapping_readings(output, baseline_name, 'daemon')
            stages = checkpoints(after_rows, client, baselines[client], tray_samples if client == 'gui' else None)
            last_cli = nearest_sample(baselines[client], after_rows[-1]['elapsed_ms'] / 1000)
            pair = {
                'before': before, 'after': after, 'visible': visible, 'closed': closed,
                'change_on_close': difference(closed, visible),
                'cli_baseline_scene': baseline_name,
                'gap_to_cli': difference(closed, readings(last_cli, 'daemon')),
                'checkpoints': stages,
                'closed_mapping': closed_mapping,
                'mapping_change_on_close': difference(closed_mapping, visible_mapping),
                'mapping_gap_to_cli': difference(closed_mapping, baseline_mapping),
            }
            if client == 'gui' and tray_samples is not None:
                pair['idle_tray_scene'] = tray_name
                pair['mapping_gap_to_idle_tray'] = difference(closed_mapping, mapping_readings(output, tray_name, 'gui'))
            result[prefix]['pairs'][after] = pair
    return result


def pagination_comparison(output, shard, phases, read_scene):
    result = {}
    for phase in phases:
        variants = {}
        for mode in ('paged', 'all'):
            name = f'{shard}-{mode}-{phase}'
            samples = [readings(sample, 'gui') for sample in read_scene(output / (name + '.jsonl'))]
            final = samples[-1]
            peak = {role: {metric: max(sample[role][metric] for sample in samples)
                           for metric in metrics} for role, metrics in final.items()}
            variants[mode] = {'final': final, 'peak': peak, 'mapping': mapping_readings(output, name, 'gui')}
        result[phase] = {**variants, 'all_minus_paged': {
            key: difference(variants['all'][key], variants['paged'][key]) for key in variants['all']
        }}
    return result
