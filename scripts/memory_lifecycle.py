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


def checkpoints(samples, client, baseline):
    result = {}
    for seconds in (0, 10, 30, 60):
        target = seconds * 1000
        row = min(samples, key=lambda sample: abs(sample['elapsed_ms'] - target))
        if abs(row['elapsed_ms'] - target) > 1000:
            raise ValueError(f'Missing closed memory sample at {seconds} seconds')
        observed = readings(row, client)
        result[str(seconds)] = {'elapsed_ms': row['elapsed_ms'], 'memory': observed,
                                'gap_to_cli': difference(observed, baseline)}
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
        baseline_name = prefix + 'headless'
        baseline = readings(read_scene(output / (baseline_name + '.jsonl'))[-1], 'daemon')
        baseline_mapping = mapping_readings(output, baseline_name, 'daemon')
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
        result[prefix] = {'baseline': baseline, 'baseline_mapping': baseline_mapping, 'pairs': {},
                          'median_reopen_ms': median(proof['relaunch_to_visible_ms'])}
        for before, after, client in cases:
            visible = readings(read_scene(output / (prefix + before + '.jsonl'))[-1], client)
            after_rows = read_scene(output / (prefix + after + '.jsonl'))
            closed = readings(after_rows[-1], client)
            closed_mapping = mapping_readings(output, prefix + after, client)
            visible_mapping = mapping_readings(output, prefix + before, client)
            result[prefix]['pairs'][after] = {
                'before': before, 'after': after, 'visible': visible, 'closed': closed,
                'change_on_close': difference(closed, visible),
                'gap_to_cli': difference(closed, baseline),
                'checkpoints': checkpoints(after_rows, client, baseline),
                'closed_mapping': closed_mapping,
                'mapping_change_on_close': difference(closed_mapping, visible_mapping),
                'mapping_gap_to_cli': difference(closed_mapping, baseline_mapping),
            }
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
