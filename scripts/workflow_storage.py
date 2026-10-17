"""Keep the hot Actions cache below 9 GiB; full archives remain on the private host."""
import argparse
import os
from pathlib import Path
import re
import time
import urllib.error

from workflow_artifacts import release_module

BUDGET = 9 * 1024**3


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
    if key.startswith(('android-go-', 'skia-source-')):
        return re.sub(r'-[^-]+-[0-9a-f]{64}$', '', key)
    if key.startswith('gradle-linux-'):
        return 'gradle-linux'
    if key.startswith('tools-nsis-'):
        return 'tools-nsis'
    if key.startswith('action-setup-tailscale/'):
        return 'tailscale/' + key.rsplit('/', 1)[-1]
    # Rust caches differ by compiler/dependency hashes. Keep the newest revision
    # per native target and branch; obsolete lockfiles must not accumulate.
    if key.startswith(('native-release-', 'android-', 'memory-ubuntu22-')):
        return re.sub(r'-[0-9a-f]{8,}-[0-9a-f]{8,}$', '', key)
    if key.startswith('macos-compiler-'):
        return re.sub(r'-[0-9a-f]{16}-[0-9a-f]{64}$', '', key)
    if key.startswith('resources-'):
        return re.sub(r'-(?:[0-9a-f]{64}-)?(?:[0-9a-f]{64}|\d{4}-\d{2}-\d{2})$', '', key)
    if key.startswith('source-cargo-'):
        return 'source-cargo'
    if key.startswith('source-inputs-'):
        return 'source-inputs'
    # Gradle's shared content entries reference each other: do not deduplicate
    # those by prefix. Only the global least-recently-used budget applies.
    return key


def priority(entry):
    key = entry['key']
    # Retain small shared source/tools first, then slow desktop compilers.
    # The fast Linux jobs can overlap a private fallback with those compilations.
    if key.startswith(('resources-', 'source-', 'android-go-', 'gradle-linux-', 'skia-source-', 'tools-nsis-', 'action-setup-tailscale/')):
        return 4
    if key.startswith(('macos-compiler-', 'native-release-')) and ('macos' in key or 'windows' in key):
        return 3
    if key.startswith('android-'):
        return 2
    if key.startswith('memory-'):
        return 0
    return 1


def eviction_plan(entries, budget=BUDGET):
    if budget <= 0:
        raise ValueError('Cache budget must be positive')
    recent = sorted(entries, key=lambda c: (c['last_accessed_at'], c['created_at'], c['id']), reverse=True)
    seen = set()
    keep = []
    remove = []
    for entry in recent:
        family = (entry['ref'], lineage(entry['key']))
        if family in seen:
            remove.append(entry)
        else:
            seen.add(family)
            keep.append(entry)
    size = sum(c['size_in_bytes'] for c in keep)
    for entry in sorted(keep, key=lambda c: (priority(c), c['last_accessed_at'], c['created_at'], c['id'])):
        if size <= budget:
            break
        remove.append(entry)
        size -= entry['size_in_bytes']
    return remove


def prune(client, budget=BUDGET, read_only=False):
    initial = caches(client)
    before = sum(c['size_in_bytes'] for c in initial)
    removed = 0
    current = initial
    for _ in range(4):
        plan = eviction_plan(current, budget)
        if not plan or read_only:
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
    if not read_only and after > budget:
        raise RuntimeError(f'Actions cache still exceeds budget: {after} > {budget} bytes')
    report = (f'Actions cache: {before / 1024**3:.2f} → {after / 1024**3:.2f} GiB; '
              f'budget {budget / 1024**3:.0f} GiB; removed {removed} entries'
              + (' (read-only fork report)' if read_only else ''))
    print(report, flush=True)
    if summary := os.environ.get('GITHUB_STEP_SUMMARY'):
        with Path(summary).open('a', encoding='utf-8') as output:
            output.write(report + '\n')
    return {'before': before, 'after': after, 'removed': removed}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--read-only', action='store_true')
    args = parser.parse_args()
    repo = os.environ['GITHUB_REPOSITORY']
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repo):
        raise ValueError('Invalid repository name')
    release = release_module()
    client = release.GitHub(repo, os.environ['GH_TOKEN'])
    prune(client, read_only=args.read_only)


if __name__ == '__main__':
    main()
