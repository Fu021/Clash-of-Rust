"""Keep the latest main cache per target and remove branch caches."""
import argparse
import os
from pathlib import Path
import re
import time
import urllib.error

from workflow_artifacts import release_module

BUDGET = 20 * 1024**3


def caches(client):
    result = []
    page = 1
    while True:
        batch = client(f'/actions/caches?per_page=100&page={page}')['actions_caches']
        result.extend(batch)
        if len(batch) < 100:
            return result
        page += 1


def lineage(key):
    key = re.sub(r'-(?:workspace|vendor)-[0-9a-f]{64}', '', key)
    key = re.sub(r'-local-deps-v\d+', '', key)
    key = re.sub(r'^native-(?:main-v\d+|release-(?:ubuntu20-)?v\d+)-', 'native-', key)
    key = re.sub(r'^android-(?:main-v\d+|v\d+)-', 'android-', key)
    key = re.sub(r'^memory-(?:main-v\d+|ubuntu22-v\d+)-(release|dev)(?:-benchmark-Linux-x64)?',
                 r'memory-\1', key)
    if key.startswith('linux-build-linux-image-ubuntu20-'):
        return re.sub(r'-[0-9a-f]{64}$', '', key)
    if key.startswith('android-go-build-v1-'):
        return re.sub(r'-[0-9a-f]{64}$', '', key)
    if key.startswith(('linux-python-ubuntu20-', 'linux-cloud-ubuntu20-', 'linux-apt-ubuntu20-', 'linux-image-ubuntu20-')):
        return re.sub(r'^(linux-(?:python|cloud|apt|image)-ubuntu20-v\d+-(?:x64|arm64))-.+$',
                      r'\1', key)
    if key.startswith('runtime-v1-'):
        return re.sub(r'-[0-9a-f]{64}-(?:approved|\d+-\d+)$', '', key)
    if key.startswith(('android-go-', 'skia-source-')):
        return re.sub(r'-[^-]+-[0-9a-f]{64}$', '', key)
    if key.startswith('gradle-linux-'):
        return 'gradle-linux'
    if key.startswith('tools-nsis-'):
        return 'tools-nsis'
    if key.startswith('action-setup-tailscale/'):
        return 'tailscale/' + key.rsplit('/', 1)[-1]
    if key.startswith(('native-', 'android-', 'memory-release-', 'memory-dev-')):
        return re.sub(r'-[0-9a-f]{8,}-[0-9a-f]{8,}$', '', key)
    if key.startswith('macos-compiler-'):
        return re.sub(r'-[0-9a-f]{16}-[0-9a-f]{64}$', '', key)
    if key.startswith('resources-'):
        return re.sub(r'-(?:[0-9a-f]{64}-)?(?:[0-9a-f]{64}|\d{4}-\d{2}-\d{2})$', '', key)
    if key.startswith('source-cargo-'):
        return 'source-cargo'
    if key.startswith('source-inputs-'):
        return 'source-inputs'
    if key.startswith('source-go-'):
        return 'source-go'
    return key


def retired(key):
    if key.startswith('action-setup-tailscale/'):
        return re.fullmatch(r'action-setup-tailscale/[^/]+/macOS-(?:amd64|arm64)', key) is None
    if key.startswith('android-go-build-v1-'):
        return False
    return key.startswith((
        'branch-delta-', 'macos-compiler-', 'android-go-', 'runtime-v1-', 'resources-', 'source-',
        'gradle-', 'tools-nsis-', 'skia-source-', 'linux-python-ubuntu20-',
        'linux-cloud-ubuntu20-', 'linux-apt-ubuntu20-', 'linux-image-ubuntu20-',
    ))


def eviction_plan(entries, budget=BUDGET):
    if budget <= 0:
        raise ValueError('Cache budget must be positive')
    recent = sorted(entries, key=lambda c: (c['created_at'], c['id']), reverse=True)
    seen = set()
    remove = []
    for entry in recent:
        if entry['ref'] != 'refs/heads/main' or retired(entry['key']):
            remove.append(entry)
            continue
        family = (entry['ref'], lineage(entry['key']))
        if family in seen:
            remove.append(entry)
        else:
            seen.add(family)
    return remove


def prune(client, budget=BUDGET, read_only=False, dry_run=False):
    initial = caches(client)
    before = sum(c['size_in_bytes'] for c in initial)
    planned = eviction_plan(initial, budget)
    projected = before - sum(c['size_in_bytes'] for c in planned)
    if dry_run:
        for entry in planned:
            print('Would remove cache:', entry['id'], entry['key'], entry['ref'], flush=True)
    removed = 0
    current = initial
    for _ in range(4):
        plan = eviction_plan(current, budget)
        if not plan or read_only or dry_run:
            break
        for entry in plan:
            try:
                client(f'/actions/caches/{int(entry["id"])}', 'DELETE')
            except urllib.error.HTTPError as error:
                if error.code != 404:
                    raise
            removed += 1
            print('Removed cache:', entry['key'], entry['ref'], flush=True)
        # Matrix jobs have completed before this runs. Recheck in case another
        # workflow has just saved a cache; cleanup jobs themselves are serialized.
        time.sleep(1)
        current = caches(client)
    after = sum(c['size_in_bytes'] for c in current)
    if not read_only and not dry_run and after > budget:
        print(f'::warning::Latest main caches exceed the {budget / 1024**3:.0f} GiB '
              'working budget; GitHub may enforce its own storage limit.', flush=True)
    report = (f'Actions cache: {before / 1024**3:.2f} → {after / 1024**3:.2f} GiB; '
              f'budget {budget / 1024**3:.0f} GiB; removed {removed} entries'
              + (f'; dry run would remove {len(planned)} entries, leaving '
                 f'{projected / 1024**3:.2f} GiB' if dry_run else '')
              + (' (read-only fork report)' if read_only else ''))
    print(report, flush=True)
    if summary := os.environ.get('GITHUB_STEP_SUMMARY'):
        with Path(summary).open('a', encoding='utf-8') as output:
            output.write(report + '\n')
    return {'before': before, 'after': after, 'removed': removed,
            'planned': len(planned), 'projected': projected}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--read-only', action='store_true')
    mode.add_argument('--dry-run', action='store_true')
    args = parser.parse_args()
    repo = os.environ['GITHUB_REPOSITORY']
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repo):
        raise ValueError('Invalid repository name')
    release = release_module()
    client = release.GitHub(repo, os.environ['GH_TOKEN'])
    prune(client, read_only=args.read_only, dry_run=args.dry_run)


if __name__ == '__main__':
    main()
