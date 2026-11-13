"""Resolve CI inputs and verify independent release compatibility evidence."""
import argparse
import hashlib
import io
import json
import os
import re
import zipfile
from pathlib import Path
from urllib.parse import urlencode

from build_inputs import android_versions
from workflow_artifacts import commit, current_ci_artifacts, release_module

WORKFLOWS = {'android': 'android-compatibility.yml', 'ubuntu': 'ubuntu-compatibility.yml'}
REPORT_JOBS = {'android': 'Record Android compatibility', 'ubuntu': 'Record Ubuntu compatibility'}


def matrix():
    versions = android_versions()
    apis = set(map(int, versions['ime_system_images']))
    apis.update(int(versions[key].split(';')[1].removeprefix('android-'))
                for key in ('system_image', 'legacy_system_image'))
    names = {26: '8.0', 27: '8.1', 28: '9', 29: '10', 30: '11', 31: '12',
             32: '12L', 33: '13', 34: '14', 35: '15', 36: '16'}
    return {'include': [{'api': api, 'android': names[api]} for api in sorted(apis)]}


def required_jobs(suite):
    if suite == 'android':
        return {'Resolve verified Android inputs'} | {
            f'Android {entry["android"]} proxy and TUN' for entry in matrix()['include']}
    if suite == 'ubuntu':
        return {'Resolve verified Ubuntu inputs'} | {
            f'Ubuntu 20.04-{arch} installed proxy and TUN' for arch in ('x64', 'arm64')}
    raise ValueError('Unknown compatibility suite')


def input_names(suite):
    if suite == 'android':
        return ('build-android-x64', 'dependency-snapshot')
    if suite == 'ubuntu':
        return ('package-linux-x64', 'package-linux-arm64', 'linux-tun-test-x64',
                'linux-tun-test-arm64', 'dependency-snapshot')
    raise ValueError('Unknown compatibility suite')


def validate_run(run, repository, expected_commit):
    if (run.get('path', '').split('@', 1)[0] != '.github/workflows/ci.yml'
            or run.get('head_sha') != expected_commit
            or run.get('repository', {}).get('full_name') != repository
            or run.get('head_repository', {}).get('full_name') != repository
            or run.get('status') != 'completed' or run.get('conclusion') != 'success'
            or run.get('event') not in ('pull_request', 'push', 'workflow_dispatch')
            or type(run.get('id')) is not int or run['id'] <= 0):
        raise ValueError('Compatibility checks require successful same-repository CI for this commit')
    return run['id']


def resolve(client, repository, expected_commit, run_id=None, suite='android'):
    if not re.fullmatch('[a-f0-9]{40}', expected_commit):
        raise ValueError('Invalid compatibility source commit')
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
            raise ValueError('Finish CI for this commit before requesting compatibility checks')
        run_id = max(matches, key=lambda item: item['id'])['id']
    if type(run_id) is not int or run_id <= 0:
        raise ValueError('Invalid CI run ID')
    run = client(f'/actions/runs/{run_id}')
    if validate_run(run, repository, expected_commit) != run_id:
        raise ValueError('Compatibility source CI run ID mismatch')
    artifacts = []
    page = 1
    while True:
        batch = client(f'/actions/runs/{run_id}/artifacts?per_page=100&page={page}')['artifacts']
        artifacts.extend(batch)
        if len(batch) < 100:
            break
        page += 1
    artifacts = current_ci_artifacts(client, run, artifacts)
    inputs = []
    for name in input_names(suite):
        found = [item for item in artifacts if item.get('name') == name]
        if len(found) != 1 or found[0].get('expired') is not False:
            raise ValueError('Missing, duplicated or expired compatibility input: ' + name)
        source = found[0].get('workflow_run', {})
        if source.get('id') != run_id or source.get('head_sha') != expected_commit:
            raise ValueError('Compatibility artifact source mismatch: ' + name)
        if (type(found[0].get('id')) is not int or found[0]['id'] <= 0
                or not re.fullmatch(r'sha256:[a-f0-9]{64}', found[0].get('digest', ''))):
            raise ValueError('Compatibility artifact identity is invalid: ' + name)
        inputs.append({key: found[0][key] for key in ('id', 'name', 'digest')})
    return {'commit': expected_commit, 'ci_run_id': run_id,
            'matrix': matrix() if suite == 'android' else {'include': [
                {'os': 'ubuntu-latest', 'arch': 'x64', 'qemu': 'qemu-system-x86'},
                {'os': 'ubuntu-22.04-arm', 'arch': 'arm64',
                 'qemu': 'qemu-system-arm qemu-efi-aarch64'}]},
            'inputs': inputs}


def check_jobs(client, run, suite, include_report):
    expected = required_jobs(suite)
    if include_report:
        expected |= {REPORT_JOBS[suite]}
    jobs = release_module().workflow_jobs(client, run, expected)
    if {job['name'] for job in jobs} - expected - {REPORT_JOBS[suite]}:
        raise ValueError('Unexpected compatibility jobs')
    jobs = [job for job in jobs if job['name'] in expected]
    if ({job['name'] for job in jobs} != expected
            or not all(job['status'] == 'completed' and job['conclusion'] == 'success'
                       for job in jobs)):
        raise ValueError(suite + ' compatibility jobs are missing, skipped or unsuccessful')


def report_name(suite, attempt):
    if type(attempt) is not int or not 1 <= attempt <= 100:
        raise ValueError('Invalid compatibility attempt')
    return f'{suite}-compatibility-attempt-{attempt}'


def record(client, suite, ci_run_id, output):
    source = resolve(client, client.repo, commit(), ci_run_id, suite)
    run_id = int(os.environ['GITHUB_RUN_ID'])
    run = client(f'/actions/runs/{run_id}')
    if (run.get('id') != run_id
            or run.get('path', '').split('@', 1)[0] != '.github/workflows/' + WORKFLOWS[suite]
            or run.get('head_sha') != source['commit']
            or run.get('event') != 'workflow_dispatch'
            or run.get('repository', {}).get('full_name') != client.repo
            or run.get('head_repository', {}).get('full_name') != client.repo):
        raise ValueError('Compatibility report workflow source mismatch')
    attempt = run.get('run_attempt', 1)
    report_name(suite, attempt)
    if os.environ.get('GITHUB_RUN_ATTEMPT') and attempt != int(os.environ['GITHUB_RUN_ATTEMPT']):
        raise ValueError('Compatibility workflow attempt mismatch')
    check_jobs(client, run, suite, include_report=False)
    value = {'schema': 1, 'suite': suite, 'commit': source['commit'],
             'ci_run_id': ci_run_id, 'run_id': run_id, 'run_attempt': attempt,
             'checks': ['system_proxy', 'tun'], 'inputs': source['inputs']}
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(value, indent=2) + '\n')
    if summary := os.environ.get('GITHUB_STEP_SUMMARY'):
        with Path(summary).open('a') as stream:
            stream.write(f'\n{suite}: basic proxy/TUN compatibility passed for CI {ci_run_id}, '
                         f'commit `{source["commit"]}`. Screenshots do not replace these checks.\n')


def verified_suite(client, source_ci, suite, run_id=None):
    expected_commit = source_ci['head_sha']
    if run_id is None:
        query = urlencode({'head_sha': expected_commit, 'event': 'workflow_dispatch', 'per_page': 100})
        runs = client('/actions/workflows/' + WORKFLOWS[suite] + '/runs?' + query)['workflow_runs']
        matches = [run for run in runs if run.get('head_sha') == expected_commit]
        if not matches:
            raise ValueError(f'Run {suite} compatibility for CI {source_ci["id"]} before publishing')
        run_id = max(matches, key=lambda item: item['id'])['id']
    if type(run_id) is not int or run_id <= 0:
        raise ValueError('Invalid compatibility run ID')
    run = client(f'/actions/runs/{run_id}')
    if (run.get('id') != run_id
            or run.get('path', '').split('@', 1)[0] != '.github/workflows/' + WORKFLOWS[suite]
            or run.get('head_sha') != expected_commit
            or run.get('repository', {}).get('full_name') != client.repo
            or run.get('head_repository', {}).get('full_name') != client.repo
            or run.get('event') != 'workflow_dispatch'
            or run.get('status') != 'completed' or run.get('conclusion') != 'success'):
        raise ValueError(suite + ' compatibility has not passed for the selected CI source')
    check_jobs(client, run, suite, include_report=True)
    name = report_name(suite, run.get('run_attempt', 1))
    reports = []
    for page in range(1, 101):
        batch = client(f'/actions/runs/{run_id}/artifacts?per_page=100&page={page}')['artifacts']
        reports.extend(item for item in batch if item.get('name') == name)
        if len(batch) < 100:
            break
    else:
        raise ValueError('Too many compatibility receipt pages')
    if len(reports) != 1 or reports[0].get('expired') is not False:
        raise ValueError(suite + ' compatibility receipt missing, duplicated or expired')
    artifact = reports[0]
    if (artifact.get('workflow_run', {}).get('id') != run_id
            or artifact['workflow_run'].get('head_sha') != expected_commit
            or not 0 < artifact.get('size_in_bytes', 0) <= 256 * 1024):
        raise ValueError('Compatibility receipt artifact identity mismatch')
    payload = client(f'/actions/artifacts/{artifact["id"]}/zip', raw=True)
    if (len(payload) > 256 * 1024
            or artifact.get('digest') != 'sha256:' + hashlib.sha256(payload).hexdigest()):
        raise ValueError('Compatibility receipt archive checksum mismatch')
    with zipfile.ZipFile(io.BytesIO(payload)) as archive:
        members = archive.infolist()
        if (len(members) != 1 or members[0].filename != 'compatibility.json'
                or members[0].file_size > 128 * 1024 or members[0].is_dir()):
            raise ValueError('Unexpected compatibility receipt members')
        receipt = json.loads(archive.read(members[0]))
    source = resolve(client, client.repo, expected_commit, source_ci['id'], suite)
    expected = {'schema': 1, 'suite': suite, 'commit': expected_commit,
                'ci_run_id': source_ci['id'], 'run_id': run_id,
                'run_attempt': run.get('run_attempt', 1),
                'checks': ['system_proxy', 'tun'], 'inputs': source['inputs']}
    if (not isinstance(receipt, dict)
            or any(type(receipt.get(key)) is not int
                   for key in ('schema', 'ci_run_id', 'run_id', 'run_attempt'))
            or receipt != expected):
        raise ValueError('Compatibility receipt differs from the selected CI artifacts or checks')
    return run_id


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', nargs='?', choices=('resolve', 'record'), default='resolve')
    parser.add_argument('--suite', choices=tuple(WORKFLOWS), required=True)
    parser.add_argument('--run-id', type=int)
    parser.add_argument('--output', type=Path, default=Path('target/compatibility/compatibility.json'))
    args = parser.parse_args()
    module = release_module()
    repository = module.repository()
    client = module.GitHub(repository, module.credential(repository))
    if args.operation == 'record':
        if args.run_id is None:
            parser.error('record requires the source CI --run-id')
        record(client, args.suite, args.run_id, args.output)
        return
    plan = resolve(client, repository, commit(), args.run_id, args.suite)
    print(json.dumps(plan))
    if output := os.environ.get('GITHUB_OUTPUT'):
        with Path(output).open('a') as stream:
            for name, value in plan.items():
                stream.write(name + '=' + (json.dumps(value) if isinstance(value, (dict, list)) else str(value)) + '\n')
    if summary := os.environ.get('GITHUB_STEP_SUMMARY'):
        with Path(summary).open('a') as stream:
            stream.write(f'\n{args.suite} proxy/TUN compatibility. '
                         f'Source CI: {plan["ci_run_id"]}; commit: `{plan["commit"]}`.\n')


if __name__ == '__main__':
    main()
