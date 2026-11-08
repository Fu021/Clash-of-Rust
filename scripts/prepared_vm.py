"""Private-only prepared VM protocol, strict metadata and bounded transfers."""
import hashlib
import http.client
import json
import math
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import tempfile
import time
import urllib.parse

import action_cache
from build_support import sha256, write_json
from upstream_cache import configuration, valid_source

CAPABILITY = 'prepared-vm-cache-v1'
GROUP = 'vm/prepared'
MAX_JSON = 1024 * 1024
MAX_IMAGE = 8 * 1024**3
MAX_VIRTUAL = 32 * 1024**3
MAX_INTEGER = 9007199254740991
ARCHITECTURES = {'arm64': 'arm64', 'x64': 'amd64'}
FIELDS = 'schema group env_key descriptor image environment producer'.split()
ERRORS = (OSError, ValueError, RuntimeError, KeyError, TypeError,
          http.client.HTTPException, subprocess.SubprocessError)
SERVER_ERRORS = {
    'invalid_metadata', 'identity_conflict', 'unsupported_image', 'upload_busy',
    'upload_timeout', 'upload_expired', 'upload_interrupted', 'size_mismatch',
    'checksum_mismatch', 'capacity_unavailable', 'metadata_unavailable',
    'stored_object_changed', 'image_not_found', 'upload_not_found', 'endpoint_not_found',
}


class ProtocolError(ValueError):
    def __init__(self, code='invalid_metadata', status=None):
        super().__init__(code)
        self.code = code
        self.status = status


def require(condition, code='invalid_metadata'):
    if not condition:
        raise ProtocolError(code)


def fields(value, names):
    require(type(value) is dict and set(value) == set(names.split()))


def integer(value, minimum=1, maximum=MAX_INTEGER):
    require(type(value) is int and minimum <= value <= maximum)


def text(value, pattern, minimum=1, maximum=2048):
    require(type(value) is str and minimum <= len(value) <= maximum)
    require(re.fullmatch(pattern, value, flags=re.ASCII) is not None)


def digest(value):
    text(value, '[0-9a-f]{64}', 64, 64)


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True,
                      allow_nan=False).encode('ascii')


def checksum(value):
    return hashlib.sha256(canonical(value)).hexdigest()


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result)
        result[key] = value
    return result


def decode(payload):
    require(len(payload) <= MAX_JSON)
    try:
        return json.loads(payload, object_pairs_hook=unique_object,
                          parse_constant=lambda value: require(False))
    except (UnicodeError, json.JSONDecodeError, RecursionError) as error:
        raise ProtocolError() from error


def validate_descriptor(descriptor):
    fields(descriptor, 'schema kind distribution release arch kernel_series base_sha256 '
           'python_sha256 environment_lock_sha256 prepare_recipe_sha256')
    require(type(descriptor['schema']) is int and descriptor['schema'] == 1)
    for name, value in {'kind': 'ubuntu-compat', 'distribution': 'ubuntu',
                        'release': '20.04', 'kernel_series': '5.4'}.items():
        require(descriptor[name] == value)
    require(type(descriptor['arch']) is str)
    require(descriptor['arch'] in ARCHITECTURES, 'unsupported_image')
    for name in ('base_sha256', 'python_sha256', 'environment_lock_sha256', 'prepare_recipe_sha256'):
        digest(descriptor[name])
    require(descriptor['base_sha256'] != descriptor['python_sha256'])


def validate_packages(packages, arch='arm64'):
    require(arch in ARCHITECTURES, 'unsupported_image')
    require(type(packages) is list and 1 <= len(packages) <= 4096)
    identities = []
    for package in packages:
        fields(package, 'name version architecture')
        text(package['name'], '[a-z0-9][a-z0-9+.-]+', 2, 128)
        text(package['version'], '[0-9][A-Za-z0-9.+:~-]*', maximum=256)
        require(package['architecture'] in (ARCHITECTURES[arch], 'all'))
        identities.append((package['name'], package['architecture']))
    require(identities == sorted(set(identities)))


def validate(record):
    fields(record, ' '.join(FIELDS))
    require(type(record['schema']) is int and record['schema'] == 1)
    require(record['group'] == GROUP)
    text(record['env_key'], 'env1-[0-9a-f]{64}', 69, 69)
    descriptor = record['descriptor']
    validate_descriptor(descriptor)
    require(record['env_key'] == 'env1-' + checksum(descriptor), 'identity_conflict')
    image = record['image']
    fields(image, 'format size virtual_size sha256')
    require(image['format'] == 'qcow2')
    integer(image['size'])
    integer(image['virtual_size'], 512)
    require(image['size'] <= MAX_IMAGE and image['virtual_size'] <= MAX_VIRTUAL, 'unsupported_image')
    require(image['virtual_size'] % 512 == 0)
    digest(image['sha256'])
    environment = record['environment']
    fields(environment, 'kernel_release python_version packages packages_sha256 sources')
    text(environment['kernel_release'], r'5\.4\.[0-9][A-Za-z0-9.+_~-]*', maximum=128)
    text(environment['python_version'], r'[0-9]+\.[0-9]+\.[0-9]+', maximum=32)
    packages = environment['packages']
    validate_packages(packages, descriptor['arch'])
    digest(environment['packages_sha256'])
    require(environment['packages_sha256'] == checksum(packages))
    sources = environment['sources']
    require(type(sources) is list and 2 <= len(sources) <= 4096)
    identities = []
    for source in sources:
        fields(source, 'url size sha256')
        text(source['url'], r'[\x21-\x7e]+')
        integer(source['size'])
        digest(source['sha256'])
        require(valid_source(source['url'], source['sha256'])
                and '?' not in source['url'] and '#' not in source['url'])
        identities.append(source['sha256'])
    require(identities == sorted(set(identities)))
    require(all(descriptor[name] in identities for name in ('base_sha256', 'python_sha256')))
    producer = record['producer']
    fields(producer, 'repository commit run_id run_attempt')
    text(producer['repository'],
         '[A-Za-z0-9][A-Za-z0-9_.-]{0,99}/[A-Za-z0-9][A-Za-z0-9_.-]{0,99}', maximum=201)
    text(producer['commit'], '[0-9a-f]{40}', 40, 40)
    integer(producer['run_id'])
    integer(producer['run_attempt'])
    require(len(canonical(record)) <= MAX_JSON)
    return record


def producer():
    from build_inputs import current_commit
    return {'repository': os.environ.get('GITHUB_REPOSITORY', ''),
            'commit': current_commit(), 'run_id': int(os.environ['GITHUB_RUN_ID']),
            'run_attempt': int(os.environ.get('GITHUB_RUN_ATTEMPT', '1'))}


def identity(descriptor):
    return 'env1-' + checksum(descriptor)


def object_id(value):
    text(value, '[A-Za-z0-9_-]{1,128}', maximum=128)
    return value


def timestamp(value):
    require(type(value) in (int, float) and math.isfinite(value)
            and 0 < value <= MAX_INTEGER, 'invalid_receipt')


def qcow2_header(path, virtual_size):
    with Path(path).open('rb') as stream:
        header = stream.read(104)
        require(len(header) >= 72 and header[:4] == b'QFI\xfb', 'unsupported_image')
        version = struct.unpack_from('>I', header, 4)[0]
        backing_offset = struct.unpack_from('>Q', header, 8)[0]
        backing_size, cluster_bits = struct.unpack_from('>II', header, 16)
        size = struct.unpack_from('>Q', header, 24)[0]
        crypt_method = struct.unpack_from('>I', header, 32)[0]
        snapshots = struct.unpack_from('>I', header, 60)[0]
        snapshots_offset = struct.unpack_from('>Q', header, 64)[0]
        require(version in (2, 3) and 9 <= cluster_bits <= 21 and backing_offset == 0
                and backing_size == 0 and size == virtual_size and 512 <= size <= MAX_VIRTUAL
                and size % 512 == 0 and crypt_method == 0
                and snapshots == 0 and snapshots_offset == 0, 'unsupported_image')
        length = 72
        if version == 3:
            require(len(header) == 104, 'unsupported_image')
            incompatible = struct.unpack_from('>Q', header, 72)[0]
            autoclear = struct.unpack_from('>Q', header, 88)[0]
            length = struct.unpack_from('>I', header, 100)[0]
            require(incompatible == 0 and autoclear == 0 and 104 <= length <= 2**cluster_bits,
                    'unsupported_image')
        stream.seek(0)
        cluster = stream.read(2**cluster_bits)
        require(len(cluster) >= length, 'unsupported_image')
        if length > 104:
            require(not any(cluster[104:length]), 'unsupported_image')
        offset = length
        extensions = set()
        while offset + 8 <= len(cluster):
            kind, size = struct.unpack_from('>II', cluster, offset)
            offset += 8
            if kind == 0:
                require(size == 0, 'unsupported_image')
                return
            require(kind == 0x6803f857 and kind not in extensions and size <= len(cluster) - offset,
                    'unsupported_image')
            extensions.add(kind)
            offset += (size + 7) // 8 * 8
        raise ProtocolError('unsupported_image')


def receipt(value, descriptor):
    require(type(value) is dict and value.get('status') == 'ready', 'invalid_receipt')
    require(all(name in value for name in FIELDS), 'invalid_receipt')
    manifest = validate({name: value[name] for name in FIELDS})
    require(manifest['descriptor'] == descriptor, 'identity_conflict')
    require(type(value.get('size')) is int and value['size'] == manifest['image']['size']
            and value.get('sha256') == manifest['image']['sha256'], 'invalid_receipt')
    timestamp(value.get('created_at'))
    result = {**manifest, 'status': 'ready', 'object_id': object_id(value.get('object_id')),
              'size': value['size'], 'sha256': value['sha256'], 'created_at': value['created_at']}
    if 'upload_id' in value:
        result['upload_id'] = object_id(value['upload_id'])
    for name in ('terminal_at', 'receipt_expires_at'):
        if name in value:
            timestamp(value[name])
            result[name] = value[name]
    if 'terminal_at' in result and 'receipt_expires_at' in result:
        require(result['receipt_expires_at'] > result['terminal_at'], 'invalid_receipt')
    return result


def same_environment(left, right):
    return left == right


class DeadlineReader:
    def __init__(self, stream, config):
        self.stream = stream
        self.config = config

    def read(self, length):
        action_cache.restore_remaining(self.config)
        return self.stream.read(length)


class Client:
    def __init__(self, config, report):
        self.config = dict(config)
        self.report = Path(report)
        self.events = []

    def event(self, operation, result, started, **details):
        self.events.append({'operation': operation, 'result': result,
                            'seconds': round(time.monotonic() - started, 3), **details})
        print(f'Prepared VM: {operation}: {result}', flush=True)
        try:
            write_json(self.report, {'schema': 1, 'group': GROUP, 'events': self.events})
            summary = os.environ.get('GITHUB_STEP_SUMMARY')
            if summary:
                with open(summary, 'a') as output:
                    output.write(f'\nPrepared VM {operation}: **{result}**, '
                                 f'{self.events[-1]["seconds"]:.1f} s.\n')
        except OSError:
            print('Prepared VM: report unavailable', flush=True)

    def request(self, path, method='GET', payload=None, deadline=None, stream=None, size=None):
        config = {**self.config, 'restore_deadline': deadline or time.monotonic() + 20}
        headers = {'Accept': 'application/json'}
        body = None
        if payload is not None:
            body = canonical(payload)
            require(len(body) <= MAX_JSON)
            headers.update({'Content-Type': 'application/json', 'Content-Length': str(len(body))})
        if stream is not None:
            body = DeadlineReader(stream, config)
            headers.update({'Content-Type': 'application/octet-stream', 'Content-Length': str(size)})
        with action_cache.response(config, path, method=method, file=body, headers=headers) as response:
            payload = bytearray()
            while len(payload) <= MAX_JSON:
                action_cache.restore_remaining(config)
                block = response.read1(min(65536, MAX_JSON + 1 - len(payload)))
                if not block:
                    break
                payload.extend(block)
            require(len(payload) <= MAX_JSON)
            if response.status not in (200, 201, 202):
                try:
                    value = decode(payload)
                    code = value.get('error_code') if type(value) is dict else None
                except ValueError:
                    code = None
                raise ProtocolError(code if code in SERVER_ERRORS else 'http_failure', response.status)
            value = decode(payload)
            return response.status, value

    def available(self, arch='arm64'):
        started = time.monotonic()
        self.limits = {}
        try:
            if not self.config:
                self.event('capability', 'not_configured', started)
                return False
            status, health = self.request('/health')
            require(status == 200, 'invalid_receipt')
            enabled = (type(health) is dict and type(health.get('capabilities')) is list
                       and CAPABILITY in health['capabilities'])
            if enabled:
                limits = health.get('vm_limits')
                require('prepared_vm' not in health
                        or canonical(health['prepared_vm']) == canonical(limits), 'unsupported_limits')
                require(type(limits) is dict and limits.get('group') == GROUP
                        and type(limits.get('architectures')) is list
                        and all(type(value) is str for value in limits['architectures'])
                        and len(limits['architectures']) == len(set(limits['architectures']))
                        and arch in ARCHITECTURES
                        and arch in limits['architectures']
                        and type(limits.get('max_image_bytes')) is int
                        and 0 < limits['max_image_bytes'] <= MAX_IMAGE
                        and type(limits.get('max_virtual_bytes')) is int
                        and 24 * 1024**3 <= limits['max_virtual_bytes'] <= MAX_VIRTUAL
                        and limits.get('range_supported') is False
                        and limits.get('resume_supported') is False, 'unsupported_limits')
                require(all(name not in limits or limits[name] is False
                            for name in ('range', 'resume')), 'unsupported_limits')
                for name, maximum in (
                        ('max_metadata_bytes', MAX_JSON), ('max_packages', 4096), ('max_sources', 4096),
                        ('budget_bytes', MAX_INTEGER), ('max_environments', MAX_INTEGER),
                        ('upload_concurrency', 1), ('upload_timeout_seconds', 1800),
                        ('upload_lease_seconds', 2700), ('terminal_receipt_seconds', 604800)):
                    require(type(limits.get(name)) is int and 0 < limits[name] <= maximum,
                            'unsupported_limits')
                self.limits = limits
            self.event('capability', 'supported' if enabled else 'unavailable', started)
            return enabled
        except ERRORS as error:
            self.event('capability', getattr(error, 'code', 'request_failed'), started)
            return False

    def lookup(self, descriptor):
        query = urllib.parse.urlencode({'key': identity(descriptor)})
        try:
            status, value = self.request('/v1/vm-images?' + query)
            require(status == 200, 'invalid_receipt')
        except ProtocolError as error:
            if error.status == 404:
                return None
            raise
        return receipt(value, descriptor)

    def restore(self, descriptor, destination, verify):
        started = time.monotonic()
        destination = Path(destination)
        try:
            value = self.lookup(descriptor)
            if value is None:
                self.event('restore', 'miss', started)
                return None
            require(value['image']['size'] <= getattr(self, 'limits', {}).get('max_image_bytes', MAX_IMAGE),
                    'unsupported_image')
            require(value['image']['virtual_size']
                    <= getattr(self, 'limits', {}).get('max_virtual_bytes', MAX_VIRTUAL),
                    'unsupported_image')
            destination.parent.mkdir(parents=True, exist_ok=True)
            require(shutil.disk_usage(destination.parent).free >= value['image']['size'] + 1024**3,
                    'local_capacity_unavailable')
            deadline = time.monotonic() + 300
            for attempt in range(2):
                try:
                    config = {**self.config, 'restore_deadline': deadline}
                    query = urllib.parse.urlencode({'id': value['object_id']})
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    with tempfile.TemporaryDirectory(prefix='vm-download-', dir=destination.parent) as temporary:
                        staged = Path(temporary) / 'image.qcow2'
                        with action_cache.response(config, '/v1/vm-images/blob?' + query) as response:
                            require(response.status == 200, 'download_failed')
                            require(response.getheader('Content-Length') == str(value['image']['size']),
                                    'size_mismatch')
                            with staged.open('wb') as output:
                                action_cache.download_body(config, response, output, value['image']['size'])
                        require(sha256(staged) == value['image']['sha256'], 'checksum_mismatch')
                        verify(staged, value['image']['virtual_size'])
                        staged.chmod(0o400)
                        staged.replace(destination)
                    write_json(self.report.with_name('prepared-vm-receipt.json'), value)
                    self.event('restore', 'download_verified', started, bytes=value['image']['size'],
                               env_key=value['env_key'])
                    return value
                except ERRORS as error:
                    if attempt or isinstance(error, ProtocolError) and error.code == 'unsupported_image':
                        raise
        except ERRORS as error:
            self.event('restore', getattr(error, 'code', 'request_failed'), started)
            return None

    def upload(self, manifest, image):
        started = time.monotonic()
        upload_id = None
        try:
            validate(manifest)
            limits = getattr(self, 'limits', {})
            require(manifest['descriptor']['arch'] in limits.get('architectures', ARCHITECTURES)
                    and len(canonical(manifest)) <= limits.get('max_metadata_bytes', MAX_JSON)
                    and len(manifest['environment']['packages']) <= limits.get('max_packages', 4096)
                    and len(manifest['environment']['sources']) <= limits.get('max_sources', 4096),
                    'unsupported_limits')
            require(manifest['image']['size'] <= getattr(self, 'limits', {}).get('max_image_bytes', MAX_IMAGE),
                    'unsupported_image')
            require(manifest['image']['virtual_size']
                    <= getattr(self, 'limits', {}).get('max_virtual_bytes', MAX_VIRTUAL),
                    'unsupported_image')
            require(Path(image).stat().st_size == manifest['image']['size'], 'size_mismatch')
            require(sha256(image) == manifest['image']['sha256'], 'checksum_mismatch')
            status, value = self.request('/v1/vm-images/uploads', 'POST', manifest)
            if status == 200:
                saved = receipt(value, manifest['descriptor'])
            else:
                require(type(value) is dict and type(value.get('schema')) is int and value['schema'] == 1
                        and value.get('group') == GROUP and value.get('env_key') == manifest['env_key'],
                        'invalid_receipt')
                upload_id = object_id(value.get('upload_id'))
                if status == 202:
                    require(value.get('status') == 'busy', 'invalid_receipt')
                    self.event('upload', 'pending', started, upload_id=upload_id,
                               env_key=manifest['env_key'])
                    return None
                require(value.get('status') == 'uploading', 'invalid_receipt')
                timestamp(value.get('expires_at'))
                require(value['expires_at'] > time.time(), 'upload_expired')
                with Path(image).open('rb') as stream:
                    status, value = self.request('/v1/vm-images/uploads/' + upload_id + '/content', 'POST',
                                                stream=stream, size=manifest['image']['size'],
                                                deadline=min(time.monotonic() + limits.get(
                                                                 'upload_timeout_seconds', 1800),
                                                             time.monotonic() + value['expires_at'] - time.time()))
                require(status == 200, 'invalid_receipt')
                saved = receipt(value, manifest['descriptor'])
            require(same_environment(saved['environment'], manifest['environment']), 'identity_conflict')
            write_json(self.report.with_name('prepared-vm-receipt.json'), saved)
            self.event('upload', 'ready', started, env_key=saved['env_key'])
            return saved
        except ERRORS as error:
            if upload_id:
                try:
                    status, value = self.request('/v1/vm-images/uploads/' + upload_id)
                    require(status == 200, 'invalid_receipt')
                    if type(value) is dict and value.get('status') == 'failed':
                        require(value.get('group') == GROUP
                                and value.get('env_key') == manifest['env_key']
                                and value.get('upload_id') == upload_id, 'invalid_receipt')
                        code = value.get('error_code')
                        error = ProtocolError(code if code in SERVER_ERRORS else 'invalid_receipt')
                        raise error
                    if (type(value) is dict and value.get('status') in ('uploading', 'busy')
                            and value.get('group') == GROUP
                            and value.get('env_key') == manifest['env_key']
                            and value.get('upload_id') == upload_id):
                        self.event('upload', 'pending', started, upload_id=upload_id,
                                   env_key=manifest['env_key'])
                        return None
                    saved = receipt(value, manifest['descriptor'])
                    require(saved.get('upload_id', upload_id) == upload_id, 'invalid_receipt')
                    require(same_environment(saved['environment'], manifest['environment']), 'identity_conflict')
                    write_json(self.report.with_name('prepared-vm-receipt.json'), saved)
                    self.event('upload', 'ready_after_query', started, env_key=saved['env_key'])
                    return saved
                except ERRORS as recovery_error:
                    if isinstance(recovery_error, ProtocolError) and recovery_error.status == 404:
                        try:
                            saved = self.lookup(manifest['descriptor'])
                            if saved is not None:
                                require(same_environment(saved['environment'], manifest['environment']),
                                        'identity_conflict')
                                write_json(self.report.with_name('prepared-vm-receipt.json'), saved)
                                self.event('upload', 'ready_after_lookup', started, env_key=saved['env_key'])
                                return saved
                        except ERRORS:
                            pass
            details = {'upload_id': upload_id} if upload_id else {}
            if isinstance(error, ProtocolError) and error.status is not None:
                details['http_status'] = error.status
            self.event('upload', getattr(error, 'code', 'request_failed'), started, **details)
            return None


def client(report):
    try:
        config = configuration()
    except ERRORS:
        config = {}
    return Client(config, report)
