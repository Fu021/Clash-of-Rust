"""Resolve verified CI inputs for Android previews and opt-in diagnostic checks."""
import argparse
import json
import os
import re
from pathlib import Path
from urllib.parse import urlencode

from build_inputs import android_versions
from workflow_artifacts import commit, release_module


def matrix():
    versions = android_versions()
    apis = set(map(int, versions['ime_system_images']))
    apis.update(int(versions[key].split(';')[1].removeprefix('android-'))
                for key in ('system_image', 'legacy_system_image'))
    names = {26: '8.0', 27: '8.1', 28: '9', 29: '10', 30: '11', 31: '12',
             32: '12L', 33: '13', 34: '14', 35: '15', 36: '16'}
    return {'include': [{'api': api, 'android': names[api]} for api in sorted(apis)]}


def validate_run(run, repository, expected_commit):
    if (run.get('path') != '.github/workflows/ci.yml'
            or run.get('head_sha') != expected_commit
            or run.get('repository', {}).get('full_name') != repository
            or run.get('head_repository', {}).get('full_name') != repository
            or run.get('status') != 'completed' or run.get('conclusion') != 'success'
            or run.get('event') not in ('pull_request', 'push', 'workflow_dispatch')
            or type(run.get('id')) is not int or run['id'] <= 0):
        raise ValueError('Preview requires successful same-repository CI for the checked-out commit')
    return run['id']


def resolve(client, repository, expected_commit, run_id=None):
    if not re.fullmatch('[a-f0-9]{40}', expected_commit):
        raise ValueError('Invalid preview source commit')
    if run_id is None:
        query = urlencode({'head_sha': expected_commit, 'status': 'success', 'per_page': 100})
        runs = client('/actions/workflows/ci.yml/runs?' + query)['workflow_runs']
        matches = []
        for run in runs:
            try:
                validate_run(run, repository, expected_commit)
            except ValueError:
                continue
            matches.append(run)
        if not matches:
            raise ValueError('No successful CI exists for this commit; finish CI before requesting previews')
        run_id = max(matches, key=lambda item: item['id'])['id']
    if type(run_id) is not int or run_id <= 0:
        raise ValueError('Invalid CI run ID')
    run = client(f'/actions/runs/{run_id}')
    validate_run(run, repository, expected_commit)
    artifacts = []
    page = 1
    while True:
        batch = client(f'/actions/runs/{run_id}/artifacts?per_page=100&page={page}')['artifacts']
        artifacts.extend(batch)
        if len(batch) < 100:
            break
        page += 1
    for name in ('build-android-x64', 'dependency-snapshot'):
        found = [item for item in artifacts if item.get('name') == name]
        if len(found) != 1 or found[0].get('expired') is not False:
            raise ValueError('Missing, duplicated or expired preview input: ' + name)
        source = found[0].get('workflow_run', {})
        if source.get('id') != run_id or source.get('head_sha') != expected_commit:
            raise ValueError('Preview artifact source mismatch: ' + name)
    return {'commit': expected_commit, 'ci_run_id': run_id, 'matrix': matrix()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run-id', type=int)
    parser.add_argument('--verify-diagnostics', action='store_true')
    args = parser.parse_args()
    module = release_module()
    repository = module.repository()
    client = module.GitHub(repository, module.credential(repository))
    plan = resolve(client, repository, commit(), args.run_id)
    print(json.dumps(plan))
    if output := os.environ.get('GITHUB_OUTPUT'):
        with Path(output).open('a') as stream:
            for name, value in plan.items():
                stream.write(name + '=' + (json.dumps(value) if isinstance(value, dict) else str(value)) + '\n')
    if summary := os.environ.get('GITHUB_STEP_SUMMARY'):
        with Path(summary).open('a') as stream:
            mode = ('Explicit Android measurement and diagnostic verification, including isolated VPN probes.'
                    if args.verify_diagnostics else 'Android UI screenshots. No VPN or traffic-routing checks.')
            stream.write(f'\n{mode} Source CI: {plan["ci_run_id"]}; commit: `{plan["commit"]}`.\n')


if __name__ == '__main__':
    main()
