"""Offline tests for resource validation and clean package staging."""
import ast
import contextlib
import hashlib
import json
import logging
import re
from pathlib import Path
import socket
import shlex
import tempfile
import threading
import tomllib
import struct
import shutil
import subprocess
import unittest
from unittest.mock import MagicMock, patch
import build_support as build
import android as android_build
import windows_ci as windows
import macos_ci as macos
import workflow_storage as storage
import action_cache as local_cache
import upstream_cache
import cache_shards
import linux_apt
import linux_ci
import prepared_vm
import prepared_vm_environment
import tun_dns_fixture
import tun_ipv6_lifecycle
import build_inputs
import ci_test_policy
import proxy_benchmark
import compatibility
import os
import copy
import importlib.util
import io
import sys
import urllib.error
import urllib.parse
import zipfile
import tarfile
from contextlib import contextmanager, nullcontext
from types import SimpleNamespace

_spec = importlib.util.spec_from_file_location('publish_release', build.ROOT/'scripts/publish-release.py')
release = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(release)

_memory_spec = importlib.util.spec_from_file_location('memory_benchmark', build.ROOT/'scripts/run-memory-benchmark.py')
memory_benchmark = importlib.util.module_from_spec(_memory_spec)
_memory_spec.loader.exec_module(memory_benchmark)

_release_package_spec = importlib.util.spec_from_file_location(
    'release_package', build.ROOT/'scripts/test-release-package.py')
release_package = importlib.util.module_from_spec(_release_package_spec)
_release_package_spec.loader.exec_module(release_package)

_cli_update_spec = importlib.util.spec_from_file_location(
    'cli_update', build.ROOT/'scripts/test-cli-update.py')
cli_update = importlib.util.module_from_spec(_cli_update_spec)
_cli_update_spec.loader.exec_module(cli_update)

_preview_spec = importlib.util.spec_from_file_location(
    'preview_slint', build.ROOT/'scripts/preview-slint.py')
preview_slint = importlib.util.module_from_spec(_preview_spec)
_preview_spec.loader.exec_module(preview_slint)

_linux_tun_spec = importlib.util.spec_from_file_location(
    'linux_tun_test', build.ROOT/'scripts/test-linux-tun.py')
linux_tun_test = importlib.util.module_from_spec(_linux_tun_spec)
_linux_tun_spec.loader.exec_module(linux_tun_test)

_cosmic_startup_guest_spec = importlib.util.spec_from_file_location(
    'cosmic_startup_guest', build.ROOT/'scripts/cosmic_startup_guest.py')
cosmic_startup_guest = importlib.util.module_from_spec(_cosmic_startup_guest_spec)
_cosmic_startup_guest_spec.loader.exec_module(cosmic_startup_guest)

_cosmic_startup_host_spec = importlib.util.spec_from_file_location(
    'cosmic_startup_host', build.ROOT/'scripts/investigate-cosmic-startup.py')
cosmic_startup_host = importlib.util.module_from_spec(_cosmic_startup_host_spec)
_cosmic_startup_host_spec.loader.exec_module(cosmic_startup_host)


class TunDnsFixtureTests(unittest.TestCase):
    def query(self, name, kind):
        labels = b''.join(bytes([len(label)]) + label.encode('ascii') for label in name.split('.'))
        return struct.pack('!HHHHHH', 1, 0x0100, 1, 0, 0, 0) + labels + b'\0' + struct.pack('!HH', kind, 1)

    def test_update_addresses_stay_outside_tun_traffic_targets(self):
        import ipaddress
        for kind, offset in ((1, 0), (28, 1)):
            with self.subTest(kind=kind):
                query = self.query('updates.fixture.test', kind)
                answer = tun_dns_fixture.dns_response(query, False)
                length = 16 if kind == 28 else 4
                self.assertEqual(str(ipaddress.ip_address(answer[-length:])),
                                 tun_dns_fixture.UPDATES[offset])
                query = self.query('web.fixture.test', kind)
                answer = tun_dns_fixture.dns_response(query, False)
                self.assertEqual(str(ipaddress.ip_address(answer[-length:])),
                                 tun_dns_fixture.WEBSITE[offset])
                if kind == 1:
                    self.assertNotIn(ipaddress.ip_address(tun_dns_fixture.UPDATES[offset]),
                                     ipaddress.ip_network('203.0.113.0/24'))

    def test_update_lookup_is_not_allowed_through_private_bootstrap(self):
        with patch.dict(tun_dns_fixture.METRICS, tun_dns_fixture.METRICS.copy()):
            for kind in (1, 28):
                answer = tun_dns_fixture.dns_response(self.query('updates.fixture.test', kind), True)
                self.assertEqual(struct.unpack_from('!H', answer, 6)[0], 0)

    def test_managed_direct_probe_delegates_dns_and_preserves_http_target(self):
        for bound_type, bound in ((1, b'\x7f\x00\x00\x01'),
                                  (4, b'\0'*16),
                                  (3, b'\x09localhost')):
            with self.subTest(bound_type=bound_type):
                connection = MagicMock()
                connection.recv.side_effect = [
                    b'\x05', b'\x00', bytes([5, 0, 0, bound_type]),
                    *([bound[:1], bound[1:]+b'\0\0'] if bound_type == 3 else [bound+b'\0\0']),
                ]
                connection.makefile.return_value = io.BytesIO(
                    b'HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok')
                with patch.object(linux_tun_test.socket, 'create_connection') as connect:
                    connect.return_value.__enter__.return_value = connection
                    self.assertEqual(linux_tun_test.socks_http(
                        '127.0.0.1', 7898, 'http://updates.fixture.test:8080/subscription.yaml?test=1'), b'ok')
                self.assertEqual(connection.sendall.call_args_list[0].args[0], b'\x05\x01\x00')
                self.assertEqual(connection.sendall.call_args_list[1].args[0],
                                 b'\x05\x01\x00\x03\x14updates.fixture.test\x1f\x90')
                self.assertEqual(connection.sendall.call_args_list[2].args[0],
                                 b'GET /subscription.yaml?test=1 HTTP/1.1\r\n'
                                 b'Host: updates.fixture.test:8080\r\nConnection: close\r\n\r\n')

    def test_managed_direct_probe_rejects_refused_and_truncated_tunnels(self):
        for fragments, expected in (([b'\x05\xff'], RuntimeError),
                                    ([b'\x05\x00', b'\x05\x05\x00\x01'], RuntimeError),
                                    ([b'\x05\x00', b''], EOFError)):
            with self.subTest(fragments=fragments):
                connection = MagicMock()
                connection.recv.side_effect = fragments
                with patch.object(linux_tun_test.socket, 'create_connection') as connect:
                    connect.return_value.__enter__.return_value = connection
                    with self.assertRaises(expected):
                        linux_tun_test.socks_http('127.0.0.1', 7898, 'http://updates.fixture.test/')
                connection.makefile.assert_not_called()


class TunIpv6LifecycleTests(unittest.TestCase):
    def test_protected_observed_ipv6_routes_require_the_corresponding_warning(self):
        for kind in ('tun', 'other'):
            for warning in (False, True):
                with self.subTest(kind=kind, warning=warning):
                    current = {'uid': 0, 'ipv6': {'state': 'observed', 'kind': kind}}
                    routes = {'warnings': ['tun_ipv6_route_bypass'] if warning else []}
                    with patch.object(tun_ipv6_lifecycle.subprocess, 'run') as probe:
                        if warning == (kind == 'other'):
                            tun_ipv6_lifecycle.check_protected_ipv6_route(routes, current)
                        else:
                            with self.assertRaises(AssertionError):
                                tun_ipv6_lifecycle.check_protected_ipv6_route(routes, current)
                        probe.assert_not_called()

    def test_protected_unknown_ipv6_requires_an_independent_unreachable_route(self):
        current = {'uid': 1000, 'ipv6': {'state': 'unknown', 'reason': 'tcp_route_unavailable'}}
        result = subprocess.CompletedProcess([], 2, '', 'RTNETLINK answers: Network is unreachable\n')
        with patch.object(tun_ipv6_lifecycle.subprocess, 'run', return_value=result) as probe:
            tun_ipv6_lifecycle.check_protected_ipv6_route({'warnings': []}, current)
        self.assertEqual(probe.call_args.args[0], [
            'ip', '-j', '-6', 'route', 'get', '2606:4700:4700::1111',
            'uid', '1000', 'ipproto', 'tcp', 'dport', '443',
        ])
        self.assertEqual(probe.call_args.kwargs['env']['LC_ALL'], 'C')
        self.assertEqual(current['ipv6'], {'state': 'unknown', 'reason': 'tcp_route_unavailable'})

    def test_unknown_ipv6_cannot_pass_on_permission_errors_or_a_changed_route(self):
        current = {'uid': 0, 'ipv6': {'state': 'unknown', 'reason': 'tcp_route_unavailable'}}
        for code, output, error in (
                (0, '[{"dev":"uplink"}]', ''),
                (0, '[{"dev":"CorRootTest"}]', ''),
                (1, '', 'ip: command not found'),
                (2, '', 'RTNETLINK answers: Permission denied'),
                (2, '[{"dev":"uplink"}]', 'Network is unreachable')):
            with self.subTest(code=code, output=output, error=error), \
                    patch.object(tun_ipv6_lifecycle.subprocess, 'run',
                                 return_value=subprocess.CompletedProcess([], code, output, error)):
                with self.assertRaisesRegex(AssertionError, 'unconfirmed'):
                    tun_ipv6_lifecycle.check_protected_ipv6_route({'warnings': []}, current)

    def test_invalid_ipv6_samples_and_unexpected_warnings_remain_failures(self):
        samples = (
            {'state': 'observed'},
            {'state': 'unknown', 'reason': 'route_query_timeout'},
            {'state': 'unknown', 'reason': 'ip_command_unavailable'},
        )
        for sample in samples:
            with self.subTest(sample=sample), \
                    patch.object(tun_ipv6_lifecycle.subprocess, 'run') as probe:
                with self.assertRaises(AssertionError):
                    tun_ipv6_lifecycle.check_protected_ipv6_route(
                        {'warnings': []}, {'uid': 0, 'ipv6': sample})
                probe.assert_not_called()
        result = subprocess.CompletedProcess([], 2, '', 'RTNETLINK answers: Network is unreachable\n')
        with patch.object(tun_ipv6_lifecycle.subprocess, 'run', return_value=result), \
                self.assertRaises(AssertionError):
            tun_ipv6_lifecycle.check_protected_ipv6_route(
                {'warnings': ['tun_ipv6_route_bypass']},
                {'uid': 0, 'ipv6': {'state': 'unknown', 'reason': 'tcp_route_unavailable'}})


class PreparedVmTests(unittest.TestCase):
    def fixture(self):
        return json.loads((build.ROOT/'docs/workflows/fixtures/prepared-vm-v1.json').read_text())

    def health(self):
        return json.loads((build.ROOT/'docs/workflows/fixtures/prepared-vm-health-v1.json').read_text())

    def manifest(self, payload=b'prepared-system-disk'):
        record = self.fixture()['base_request']
        record['image']['size'] = len(payload)
        record['image']['sha256'] = hashlib.sha256(payload).hexdigest()
        return record

    def receipt(self, record):
        return {**copy.deepcopy(record), 'status': 'ready', 'object_id': 'object_one',
                'size': record['image']['size'], 'sha256': record['image']['sha256'],
                'created_at': 1794143576.125}

    @contextmanager
    def replies(self, responses):
        calls = []

        @contextmanager
        def response(config, path, **kwargs):
            calls.append((config, path, kwargs))
            status, body = responses.pop(0)
            payload = body if isinstance(body, bytes) else json.dumps(body).encode()
            stream = io.BytesIO(payload)
            reply = MagicMock(status=status)
            reply.read = stream.read
            reply.read1 = stream.read1
            reply.getheader.side_effect = lambda name: str(len(payload)) if name == 'Content-Length' else None
            yield reply

        with patch.object(local_cache, 'response', side_effect=response):
            yield calls

    def test_shared_contract_vectors(self):
        fixture = self.fixture()
        self.assertEqual(fixture['revision'], '2026-11-08-x64')
        self.assertEqual(len(fixture['cases']), 46)
        for case in fixture['cases']:
            with self.subTest(case=case['id']):
                record = copy.deepcopy(fixture['base_request'])
                for operation in case['operations']:
                    container = record
                    for component in operation['path'][:-1]:
                        container = container[component]
                    name = operation['path'][-1]
                    if operation['op'] == 'set':
                        container[name] = copy.deepcopy(operation['value'])
                    else:
                        del container[name]
                payload = json.dumps(record)
                wire = case.get('wire', {})
                if name := wire.get('duplicate_top_key'):
                    payload = '{' + json.dumps(name) + ':' + json.dumps(record[name]) + ',' + payload[1:]
                payload += wire.get('append', '')
                if case['expected']['valid']:
                    value = prepared_vm.validate(prepared_vm.decode(payload.encode()))
                    self.assertEqual(value['env_key'], case['expected']['env_key'])
                    self.assertEqual(value['environment']['packages_sha256'],
                                     case['expected']['packages_sha256'])
                else:
                    with self.assertRaises(prepared_vm.ProtocolError) as failure:
                        prepared_vm.validate(prepared_vm.decode(payload.encode()))
                    self.assertEqual(failure.exception.code, case['expected']['error'])

    def test_parser_rejects_nested_duplicates_nonfinite_invalid_utf8_and_oversize(self):
        for payload in (b'{"nested":{"key":1,"key":2}}', b'{"key":NaN}', b'{"key":Infinity}',
                        b'\xff', b' ' * (prepared_vm.MAX_JSON + 1)):
            with self.subTest(payload=payload[:30]), self.assertRaises(prepared_vm.ProtocolError):
                prepared_vm.decode(payload)

    def test_disabled_capability_never_probes_a_vm_endpoint(self):
        with tempfile.TemporaryDirectory() as temporary:
            client = prepared_vm.Client({'url': 'http://127.0.0.1', 'token': 'private'},
                                        Path(temporary)/'report.json')
            with self.replies([(200, {'capabilities': []})]) as calls:
                self.assertFalse(client.available())
            self.assertEqual([call[1] for call in calls], ['/health'])
            self.assertNotIn('private', (Path(temporary)/'report.json').read_text())

    def test_capability_requires_exact_supported_limits(self):
        limits = self.health()['vm_limits']
        with tempfile.TemporaryDirectory() as temporary:
            client = prepared_vm.Client({'url': 'http://127.0.0.1', 'token': 'private'},
                                        Path(temporary)/'report.json')
            invalid = [{**limits, name: value} for name, value in (
                ('group', 'upstream/files'), ('architectures', ['arm64', 'arm64']),
                ('architectures', ['arm64', 1]), ('architectures', 'arm64'),
                ('max_image_bytes', True), ('max_image_bytes', 0),
                ('max_image_bytes', prepared_vm.MAX_IMAGE + 1),
                ('max_virtual_bytes', 24 * 1024**3 - 1),
                ('max_virtual_bytes', prepared_vm.MAX_VIRTUAL + 1),
                ('range_supported', True), ('range_supported', 0),
                ('resume_supported', True), ('resume_supported', 0))]
            invalid.extend({name: value for name, value in limits.items() if name != missing}
                           for missing in ('group', 'architectures', 'max_image_bytes',
                                           'max_virtual_bytes', 'range_supported', 'resume_supported'))
            for advertised, expected in [(limits, True), *((value, False) for value in invalid)]:
                with self.subTest(limits=advertised), self.replies([
                    (200, {**self.health(), 'vm_limits': advertised, 'prepared_vm': advertised})
                ]):
                    self.assertEqual(client.available(), expected)
                    if not expected:
                        self.assertEqual(client.limits, {})

    def test_legacy_health_names_do_not_enable_vm_uploads(self):
        health = self.health()
        limits = health.pop('vm_limits')
        for advertised in ({**health, 'prepared_vm': limits},
                           {**health, 'vm_limits': {
                               name: value for name, value in limits.items() if name != 'range_supported'
                           } | {'range': False}}):
            with tempfile.TemporaryDirectory() as temporary:
                client = prepared_vm.Client({'url': 'http://127.0.0.1', 'token': 'private'},
                                            Path(temporary)/'report.json')
                with self.subTest(health=advertised), self.replies([(200, advertised)]) as calls:
                    self.assertFalse(client.available())
                self.assertEqual([call[1] for call in calls], ['/health'])

    def test_lost_capability_clears_previous_limits(self):
        with tempfile.TemporaryDirectory() as temporary:
            client = prepared_vm.Client({'url': 'http://127.0.0.1', 'token': 'private'},
                                        Path(temporary)/'report.json')
            with self.replies([(200, self.health()), (200, {'capabilities': []})]):
                self.assertTrue(client.available())
                self.assertFalse(client.available())
            self.assertEqual(client.limits, {})

    def test_conflicting_health_aliases_never_enable_vm_operations(self):
        for key, value in (('max_environments', True), ('architectures', ['arm64']),
                           ('range', True), ('resume', True)):
            advertised = self.health()
            advertised['prepared_vm'][key] = value
            with self.subTest(key=key), tempfile.TemporaryDirectory() as temporary:
                client = prepared_vm.Client({'url': 'http://127.0.0.1', 'token': 'private'},
                                            Path(temporary)/'report.json')
                with self.replies([(200, advertised)]) as calls:
                    self.assertFalse(client.available())
                self.assertEqual([call[1] for call in calls], ['/health'])
                self.assertEqual(client.limits, {})

    def test_ready_receipt_checks_identity_copies_and_fractional_times(self):
        record = self.manifest()
        valid = self.receipt(record)
        valid.update(upload_id='upload_one', terminal_at=1794143576.25,
                     receipt_expires_at=1794748376.25)
        self.assertEqual(prepared_vm.receipt(valid, record['descriptor'])['upload_id'], 'upload_one')
        for name, value in (('size', True), ('size', record['image']['size'] + 1),
                            ('sha256', 'f' * 64), ('created_at', '1794143576'),
                            ('terminal_at', True), ('receipt_expires_at', 1)):
            with self.subTest(name=name), self.assertRaises(prepared_vm.ProtocolError):
                prepared_vm.receipt({**valid, name: value}, record['descriptor'])
        for value in (True, None, 0, -1, '123', float('nan'), float('inf')):
            with self.subTest(timestamp=value), self.assertRaises(prepared_vm.ProtocolError):
                prepared_vm.timestamp(value)

    def test_capability_selects_only_an_advertised_native_architecture(self):
        limits = {**self.health()['vm_limits'], 'architectures': ['arm64', 'x64']}
        with tempfile.TemporaryDirectory() as temporary:
            client = prepared_vm.Client({}, Path(temporary)/'report.json')
            client.config = {'url': 'http://127.0.0.1', 'token': 'private'}
            for arch in prepared_vm.ARCHITECTURES:
                for advertised, expected in ((limits, True),
                                             ({**limits, 'architectures': ['arm64']}, arch == 'arm64')):
                    with self.subTest(arch=arch, limits=advertised), self.replies([
                        (200, {**self.health(), 'vm_limits': advertised, 'prepared_vm': advertised})
                    ]) as calls:
                        self.assertEqual(client.available(arch), expected)
                    self.assertEqual([call[1] for call in calls], ['/health'])

    def test_apt_sources_use_native_official_suites(self):
        for arch, native, base in (
                ('arm64', 'arm64', 'https://ports.ubuntu.com/ubuntu-ports'),
                ('x64', 'amd64', 'https://archive.ubuntu.com/ubuntu')):
            with self.subTest(arch=arch):
                sources = prepared_vm_environment.sources(arch).splitlines()
                self.assertEqual(len(sources), 3)
                for source, suite in zip(sources, ('focal', 'focal-updates', 'focal-security')):
                    origin = 'https://security.ubuntu.com/ubuntu' if arch == 'x64' and suite == 'focal-security' else base
                    self.assertTrue(source.startswith('deb [arch=' + native + ' signed-by='))
                    self.assertIn(f'] {origin} {suite} main universe restricted multiverse', source)

    def test_vm_ssh_retries_transient_timeouts_before_the_boot_deadline(self):
        clock = [0.0]
        attempts = []
        process = MagicMock()
        process.poll.return_value = None

        def execute(command, **kwargs):
            attempts.append(kwargs['timeout'])
            clock[0] += 1
            if len(attempts) == 1:
                raise subprocess.TimeoutExpired(command, kwargs['timeout'])
            return subprocess.CompletedProcess(command, 1 if len(attempts) == 2 else 0)

        def sleep(seconds):
            clock[0] += seconds

        with patch.object(linux_ci.time, 'monotonic', side_effect=lambda: clock[0]), \
                patch.object(linux_ci.time, 'sleep', side_effect=sleep), \
                patch.object(linux_ci.subprocess, 'run', side_effect=execute):
            linux_ci.wait_vm_ssh(process, ['ssh', 'guest'], timeout=30)
        self.assertEqual(attempts, [15, 15, 15])

    def test_vm_ssh_never_outlives_the_total_deadline_or_a_dead_guest(self):
        clock = [0.0]
        process = MagicMock()
        process.poll.return_value = None

        def execute(command, **kwargs):
            clock[0] += kwargs['timeout']
            raise subprocess.TimeoutExpired(command, kwargs['timeout'])

        def sleep(seconds):
            clock[0] += seconds

        with patch.object(linux_ci.time, 'monotonic', side_effect=lambda: clock[0]), \
                patch.object(linux_ci.time, 'sleep', side_effect=sleep), \
                patch.object(linux_ci.subprocess, 'run', side_effect=execute) as commands:
            with self.assertRaisesRegex(TimeoutError, 'SSH-ready'):
                linux_ci.wait_vm_ssh(process, ['ssh', 'guest'], timeout=18)
            self.assertEqual(clock[0], 18)
            self.assertEqual(commands.call_count, 1)
            process.poll.return_value = 1
            with self.assertRaisesRegex(RuntimeError, 'exited during boot'):
                linux_ci.wait_vm_ssh(process, ['ssh', 'guest'])
            self.assertEqual(commands.call_count, 1)

    def test_guest_report_preserves_only_complete_generated_archives(self):
        for result, body, state in ((0, b'', 'not_generated'),
                                    (0, b'complete report', 'saved'),
                                    (1, b'partial report', 'transfer_failed'),
                                    ('timeout', b'partial report', 'transfer_failed')):
            with self.subTest(result=result, state=state), tempfile.TemporaryDirectory() as temporary:
                report = Path(temporary)
                guest = object.__new__(linux_ci.VmGuest)
                guest.ssh = ['ssh', 'guest']
                guest.directory = report / 'warm'

                def transfer(command, **kwargs):
                    kwargs['stdout'].write(body)
                    if result == 'timeout':
                        raise subprocess.TimeoutExpired(command, kwargs['timeout'])
                    return subprocess.CompletedProcess(command, result, stderr=b'fixture failure')

                with patch.object(linux_ci, 'REPORT', report), \
                        patch.object(linux_ci, 'guest_receipts') as receipts, \
                        patch.object(guest, 'output', return_value='boot analysis\n'), \
                        patch.object(linux_ci.subprocess, 'run', side_effect=transfer):
                    guest.preserve()
                self.assertEqual(json.loads((report/'focal-guest-report-status.json').read_text()),
                                 {'schema': 1, 'state': state})
                archive = report/'focal-guest-report.tar.gz'
                self.assertEqual(archive.exists(), state == 'saved')
                if state == 'saved':
                    self.assertEqual(archive.read_bytes(), body)
                receipts.assert_called_once_with(guest.ssh)
                self.assertEqual((report / 'warm-cloud-init.txt').read_text(), 'boot analysis\n')
                self.assertFalse(list(report.glob('guest-report-*')))

    def test_private_restore_is_atomic_and_preserves_existing_file_on_bad_bytes(self):
        record = self.manifest()
        for body, result in ((b'prepared-system-disk', True), (b'damaged-system-disk!', False)):
            with self.subTest(result=result), tempfile.TemporaryDirectory() as temporary:
                target = Path(temporary)/'image.qcow2'
                target.write_bytes(b'existing')
                client = prepared_vm.Client({}, Path(temporary)/'report.json')
                replies = [(200, self.receipt(record)), (200, body)]
                if not result:
                    replies.append((200, body))
                with self.replies(replies) as calls:
                    restored = client.restore(record['descriptor'], target, lambda path, size: None)
                self.assertEqual(restored is not None, result)
                self.assertEqual(target.read_bytes(), body if result else b'existing')
                self.assertTrue(all('Range' not in call[2].get('headers', {}) for call in calls))
                self.assertFalse(list(Path(temporary).glob('vm-download-*')))

    def test_http_failure_bodies_are_not_exported_and_404_is_a_miss(self):
        with tempfile.TemporaryDirectory() as temporary:
            client = prepared_vm.Client({}, Path(temporary)/'report.json')
            with self.replies([(404, b'not-json private-service-address')]):
                self.assertIsNone(client.lookup(self.manifest()['descriptor']))
            with self.replies([(507, {'error_code': 'capacity_unavailable', 'detail': 'private'})]):
                with self.assertRaises(prepared_vm.ProtocolError) as failure:
                    client.request('/v1/vm-images/uploads', 'POST', self.manifest())
            self.assertEqual(failure.exception.status, 507)
            self.assertEqual(str(failure.exception), 'capacity_unavailable')

    def test_upload_streams_a_file_and_recovers_a_lost_content_response(self):
        record = self.manifest()
        for lost in (False, True):
            with self.subTest(lost=lost), tempfile.TemporaryDirectory() as temporary:
                image = Path(temporary)/'image.qcow2'
                image.write_bytes(b'prepared-system-disk')
                client = prepared_vm.Client({}, Path(temporary)/'report.json')
                reservation = {'schema': 1, 'group': prepared_vm.GROUP, 'env_key': record['env_key'],
                               'status': 'uploading', 'upload_id': 'upload_one', 'expires_at': 4102444800.125}
                responses = [(201, reservation),
                             (502, b'private error') if lost else (200, self.receipt(record))]
                if lost:
                    responses.append((200, self.receipt(record)))
                with self.replies(responses) as calls:
                    self.assertIsNotNone(client.upload(record, image))
                self.assertEqual(calls[1][2]['headers']['Content-Length'], str(image.stat().st_size))
                self.assertIsInstance(calls[1][2]['file'], prepared_vm.DeadlineReader)
                self.assertEqual(calls[1][1], '/v1/vm-images/uploads/upload_one/content')
                if lost:
                    self.assertEqual(calls[2][1], '/v1/vm-images/uploads/upload_one')
                self.assertNotIn('private', (Path(temporary)/'report.json').read_text())

    def test_existing_immutable_image_keeps_original_producer_and_different_file_identity(self):
        record = self.manifest()
        existing = self.receipt(record)
        existing['producer']['run_id'] = 9
        existing['image']['sha256'] = 'f' * 64
        existing['sha256'] = 'f' * 64
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary)/'image.qcow2'
            image.write_bytes(b'prepared-system-disk')
            client = prepared_vm.Client({}, Path(temporary)/'report.json')
            with self.replies([(200, existing)]) as calls:
                result = client.upload(record, image)
            self.assertEqual(result['producer']['run_id'], 9)
            self.assertEqual(result['image']['sha256'], 'f' * 64)
            self.assertEqual(len(calls), 1)

    def test_existing_environment_cannot_change_sources_under_the_same_key(self):
        record = self.manifest()
        existing = self.receipt(record)
        existing['environment']['sources'][0]['url'] = 'https://cloud-images.ubuntu.com/synthetic-alias.img'
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary)/'image.qcow2'
            image.write_bytes(b'prepared-system-disk')
            client = prepared_vm.Client({}, Path(temporary)/'report.json')
            with self.replies([(200, existing)]):
                self.assertIsNone(client.upload(record, image))
            self.assertEqual(client.events[-1]['result'], 'identity_conflict')

    def test_expired_upload_receipt_recovers_ready_image_by_exact_environment(self):
        record = self.manifest()
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary)/'image.qcow2'
            image.write_bytes(b'prepared-system-disk')
            client = prepared_vm.Client({}, Path(temporary)/'report.json')
            reservation = {'schema': 1, 'group': prepared_vm.GROUP, 'env_key': record['env_key'],
                           'status': 'uploading', 'upload_id': 'upload_one', 'expires_at': 4102444800.25}
            with self.replies([(201, reservation), (502, {}), (404, {'error_code': 'upload_not_found'}),
                               (200, self.receipt(record))]) as calls:
                saved = client.upload(record, image)
            self.assertEqual(saved['env_key'], record['env_key'])
            self.assertEqual(calls[-1][1], '/v1/vm-images?key=' + record['env_key'])
            self.assertEqual(client.events[-1]['result'], 'ready_after_lookup')

    def test_interrupted_upload_keeps_terminal_error_without_retransmitting(self):
        record = self.manifest()
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary)/'image.qcow2'
            image.write_bytes(b'prepared-system-disk')
            client = prepared_vm.Client({}, Path(temporary)/'report.json')
            reservation = {'schema': 1, 'group': prepared_vm.GROUP, 'env_key': record['env_key'],
                           'status': 'uploading', 'upload_id': 'upload_one', 'expires_at': 4102444800}
            failed = {**reservation, 'status': 'failed', 'error_code': 'upload_interrupted'}
            with self.replies([(201, reservation), (502, {}), (200, failed)]) as calls:
                self.assertIsNone(client.upload(record, image))
            self.assertEqual(client.events[-1]['result'], 'upload_interrupted')
            self.assertEqual(len(calls), 3)

    def test_busy_upload_is_pending_and_does_not_send_content(self):
        record = self.manifest()
        busy = {'schema': 1, 'group': prepared_vm.GROUP, 'env_key': record['env_key'],
                'status': 'busy', 'upload_id': 'upload_one'}
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary)/'image.qcow2'
            image.write_bytes(b'prepared-system-disk')
            client = prepared_vm.Client({}, Path(temporary)/'report.json')
            with self.replies([(202, busy)]) as calls:
                self.assertIsNone(client.upload(record, image))
            self.assertEqual(len(calls), 1)
            self.assertEqual(client.events[-1]['result'], 'pending')

    def test_expired_upload_and_capacity_failure_never_send_file_or_delete(self):
        record = self.manifest()
        for status, value in ((507, {'error_code': 'capacity_unavailable'}),
                              (201, {'schema': 1, 'group': prepared_vm.GROUP,
                                     'env_key': record['env_key'], 'status': 'uploading',
                                     'upload_id': 'upload_one', 'expires_at': 1})):
            with self.subTest(status=status), tempfile.TemporaryDirectory() as temporary:
                image = Path(temporary)/'image.qcow2'
                image.write_bytes(b'prepared-system-disk')
                client = prepared_vm.Client({}, Path(temporary)/'report.json')
                responses = [(status, value)]
                if status == 201:
                    responses.extend([(404, {}), (404, {})])
                with self.replies(responses) as calls:
                    self.assertIsNone(client.upload(record, image))
                self.assertTrue(all(call[2].get('stream') is None for call in calls))
                self.assertTrue(all(call[2].get('method', 'GET') != 'DELETE' for call in calls))

    def test_stream_reader_enforces_wall_clock(self):
        with self.assertRaises(local_cache.RestoreBudgetExceeded):
            prepared_vm.DeadlineReader(io.BytesIO(b'file'), {'restore_deadline': 0}).read(4)

    def test_metadata_reader_rechecks_wall_clock_between_chunks(self):
        with tempfile.TemporaryDirectory() as temporary:
            client = prepared_vm.Client({}, Path(temporary)/'report.json')
            with self.replies([(200, {'status': 'ready'})]), \
                    patch.object(local_cache, 'restore_remaining',
                                 side_effect=[1, local_cache.RestoreBudgetExceeded()]):
                with self.assertRaises(local_cache.RestoreBudgetExceeded):
                    client.request('/health')

    def test_limits_reject_oversize_virtual_disk_before_transfer(self):
        record = self.manifest()
        record['image']['virtual_size'] = 32 * 1024**3
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary)/'image.qcow2'
            image.write_bytes(b'prepared-system-disk')
            client = prepared_vm.Client({}, Path(temporary)/'report.json')
            client.limits = {'max_image_bytes': prepared_vm.MAX_IMAGE,
                             'max_virtual_bytes': 24 * 1024**3}
            with self.replies([(200, self.receipt(record))]) as calls:
                self.assertIsNone(client.restore(record['descriptor'], image, MagicMock()))
            self.assertEqual(len(calls), 1)
            with self.replies([]) as calls:
                self.assertIsNone(client.upload(record, image))
            self.assertEqual(calls, [])

    def test_environment_resolution_is_opt_in_and_requires_linux_inputs(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {}, clear=True), \
                patch.object(prepared_vm_environment, 'run') as execute:
            self.assertEqual(prepared_vm_environment.freeze(Path(temporary), {})['status'], 'disabled')
            execute.assert_not_called()
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {
                'GITHUB_ACTIONS': 'true', 'COR_PREPARED_VM_INPUTS': 'true'}, clear=True), \
                patch.object(prepared_vm, 'client', side_effect=AssertionError('No private access during freeze')), \
                patch.object(prepared_vm_environment, 'run') as execute:
            self.assertEqual(prepared_vm_environment.freeze(Path(temporary), {})['status'], 'disabled')
            execute.assert_not_called()

    def test_environment_key_ignores_application_and_run_identity(self):
        descriptor = self.manifest()['descriptor']
        expected = prepared_vm.identity(descriptor)
        with patch.dict(os.environ, {'GITHUB_RUN_ID': '999', 'GITHUB_SHA': 'f' * 40}):
            self.assertEqual(prepared_vm.identity(descriptor), expected)
        changed = {**descriptor, 'environment_lock_sha256': 'f' * 64}
        self.assertNotEqual(prepared_vm.identity(changed), expected)

    @contextmanager
    def vm_context(self, temporary, arch='arm64'):
        record = self.manifest()
        record['descriptor']['arch'] = arch
        for package in record['environment']['packages']:
            if package['architecture'] != 'all':
                package['architecture'] = prepared_vm.ARCHITECTURES[arch]
        record['environment']['packages_sha256'] = prepared_vm.checksum(record['environment']['packages'])
        record['env_key'] = prepared_vm.identity(record['descriptor'])
        frozen = {'descriptor': record['descriptor'],
                  'lock': {'packages': record['environment']['packages'], 'downloads': {}}}
        linux = {'architectures': {arch: {'cloud': {
            'url': record['environment']['sources'][0]['url'], 'sha256': 'a' * 64},
            'python': {'url': record['environment']['sources'][1]['url'], 'sha256': 'b' * 64}}}}
        client = MagicMock()
        client.available.return_value = True
        client.restore.return_value = self.receipt(record)
        guest = MagicMock(kernel='5.4.0-216-generic',
                          architecture='aarch64' if arch == 'arm64' else 'x86_64',
                          kvm=arch == 'x64', boot_seconds=0.1,
                          stage_seconds=0.1, inspect_seconds=0.1)
        guest.inspect.return_value = record['environment']
        (Path(temporary) / 'source.tar.gz').touch()
        with patch.dict(os.environ, {'RUNNER_TEMP': temporary}, clear=False), \
                patch.object(linux_ci, 'require_actions'), \
                patch.object(linux_ci, 'REPORT', Path(temporary)/'report'), \
                patch.object(linux_ci, 'inputs', return_value=linux), \
                patch.object(linux_ci, 'host_arch', return_value=arch), \
                patch.object(build_inputs, 'manifest', return_value={'prepared_vms': {
                    name: frozen if name == arch else {'schema': 1, 'status': 'disabled'}
                    for name in prepared_vm.ARCHITECTURES}}), \
                patch.object(prepared_vm, 'client', return_value=client), \
                patch.object(prepared_vm_environment, 'verify_frozen', return_value=True), \
                patch.object(linux_ci, 'vm_source_archive', return_value=Path(temporary)/'source.tar.gz'), \
                patch.object(linux_ci, 'VmGuest', return_value=guest), \
                patch.object(linux_ci, 'download') as download, \
                patch.object(linux_ci, 'prepare_vm_guest') as prepare, \
                patch.object(linux_ci, 'fetch_vm_package',
                             return_value=Path(temporary)/'package.tar.gz'), \
                patch.object(linux_ci, 'verify_vm_package') as application:
            yield client, guest, download, prepare, application

    def test_warm_vm_skips_all_cold_downloads_but_runs_application_tests(self):
        for arch in prepared_vm.ARCHITECTURES:
            with self.subTest(arch=arch), tempfile.TemporaryDirectory() as temporary, \
                    self.vm_context(temporary, arch) as context:
                client, guest, download, prepare, application = context
                linux_ci.vm_test(123)
                client.available.assert_called_once_with(arch)
                download.assert_not_called()
                prepare.assert_not_called()
                client.upload.assert_not_called()
                self.assertEqual(application.call_args.kwargs['prepared'], True)
                self.assertEqual(application.call_args.args[2], 123)
                guest.preserve.assert_called_once()
                guest.close.assert_called_once()

    def test_vm_starts_candidate_download_before_private_lookup_and_boot(self):
        from concurrent.futures import Future
        with tempfile.TemporaryDirectory() as temporary, self.vm_context(temporary) as context:
            client, _, _, _, application = context
            downloaded = threading.Event()
            def fetch(*args):
                downloaded.set()
                return Path(temporary) / 'package.tar.gz'
            def restore(*args):
                self.assertTrue(downloaded.wait(5), 'Package download did not overlap VM lookup')
                return self.receipt(self.manifest())
            client.restore.side_effect = restore
            with patch.object(linux_ci, 'fetch_vm_package', side_effect=fetch):
                linux_ci.vm_test(123)
            self.assertIsInstance(application.call_args.kwargs['package_future'], Future)

    def test_vm_candidate_download_failure_prevents_guest_tests(self):
        from concurrent.futures import Future
        failed = Future()
        failed.set_exception(ValueError('Package receipt mismatch'))
        guest = MagicMock()
        timings = {}
        with self.assertRaisesRegex(ValueError, 'receipt mismatch'):
            linux_ci.verify_vm_package(guest, Path('/unused'), 123,
                                       package_future=failed, timings=timings)
        guest.remote.assert_not_called()
        self.assertIn('package_remaining_wait', timings)

    def test_vm_archive_omits_git_history_builds_and_other_architecture_indexes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            snapshot = root / 'snapshot'
            snapshot.mkdir()
            (snapshot / 'inputs.json').write_text('{}')
            for arch in ('x64', 'arm64'):
                indexes = snapshot / 'vm-environment' / arch
                indexes.mkdir(parents=True)
                (indexes / 'index').write_text(arch)
            with patch.dict(os.environ, {'BUILD_INPUTS_DIR': str(snapshot)}), \
                    patch.object(linux_ci, 'host_arch', return_value='arm64'):
                archive = linux_ci.vm_source_archive(root)
            with tarfile.open(archive) as packed:
                names = packed.getnames()
                self.assertFalse(any(name.startswith(('.git/', 'crates/', 'dist/', 'target/'))
                                     for name in names))
                self.assertNotIn('dependency-snapshot/vm-environment/x64/index', names)
                self.assertIn('dependency-snapshot/vm-environment/arm64/index', names)
                self.assertIn('scripts/test-linux-tun.py', names)
                self.assertIn('scripts/tun_dns_fixture.py', names)
                self.assertIn('scripts/tun_ipv6_lifecycle.py', names)
                source = packed.extractfile('source-commit.txt').read().decode().strip()
                extracted = root / 'guest'
                packed.extractall(extracted, filter='data')
            environment = dict(os.environ, COR_LINUX_GUEST='true', GITHUB_ACTIONS='true')
            actual = subprocess.check_output([
                sys.executable, '-B', '-c',
                "import sys; sys.path.insert(0, 'scripts'); "
                "from workflow_artifacts import commit; print(commit())"],
                cwd=extracted, env=environment, text=True).strip()
            self.assertEqual(actual, source)

    def test_missing_service_capability_keeps_frozen_inputs_on_the_cold_path(self):
        with tempfile.TemporaryDirectory() as temporary, self.vm_context(temporary) as context:
            client, guest, download, prepare, application = context
            client.available.return_value = False
            linux_ci.vm_test(123)
            client.restore.assert_not_called()
            client.upload.assert_not_called()
            self.assertEqual(download.call_count, 2)
            prepare.assert_called_once()
            self.assertIsNone(prepare.call_args.args[2])
            self.assertFalse(application.call_args.kwargs['prepared'])

    def test_application_failure_on_warm_vm_is_not_hidden_by_cold_retry(self):
        with tempfile.TemporaryDirectory() as temporary, self.vm_context(temporary) as context:
            client, guest, download, prepare, application = context
            application.side_effect = subprocess.CalledProcessError(1, ['application-test'])
            with self.assertRaises(subprocess.CalledProcessError):
                linux_ci.vm_test(123)
            download.assert_not_called()
            prepare.assert_not_called()
            guest.close.assert_called_once()
            report = json.loads((Path(temporary)/'report/focal-vm.json').read_text())
            self.assertFalse(report['success'])
            self.assertEqual(report['cache_path'], 'warm')

    def test_invalid_warm_environment_falls_back_without_republishing_same_identity(self):
        with tempfile.TemporaryDirectory() as temporary, self.vm_context(temporary) as context:
            client, guest, download, prepare, application = context
            guest.inspect.side_effect = prepared_vm.ProtocolError('environment_mismatch')
            source = Path(temporary)/'original'
            source.write_bytes(b'original')
            download.return_value = source
            linux_ci.vm_test(123)
            self.assertEqual(download.call_count, 2)
            self.assertEqual(prepare.call_count, 1)
            self.assertIsNone(prepare.call_args.args[2])
            self.assertFalse(application.call_args.kwargs['prepared'])
            events = [call.args[1] for call in client.event.call_args_list]
            self.assertIn('cold_fallback', events)
            client.upload.assert_not_called()

    def test_cold_preparation_failure_keeps_failure_report(self):
        with tempfile.TemporaryDirectory() as temporary, self.vm_context(temporary) as context:
            client, guest, download, prepare, application = context
            client.restore.return_value = None
            client.available.return_value = False
            prepare.side_effect = RuntimeError('isolated preparation failure')
            with self.assertRaises(RuntimeError):
                linux_ci.vm_test(123)
            application.assert_not_called()
            client.upload.assert_not_called()
            report = json.loads((Path(temporary)/'report/focal-vm.json').read_text())
            self.assertFalse(report['success'])
            self.assertIn('upload_tail_wait', report['timings_seconds'])

    def test_restored_sources_must_cover_the_exact_frozen_downloads(self):
        record = self.manifest()
        download = {'url': 'https://ports.ubuntu.com/ubuntu-ports/pool/test.deb',
                    'size': 100, 'sha256': 'c' * 64}
        frozen = {'descriptor': record['descriptor'], 'lock': {'downloads': {'test.deb': download}}}
        with self.assertRaises(prepared_vm.ProtocolError):
            prepared_vm_environment.check_sources(record['environment'], frozen)
        record['environment']['sources'].append(download)
        prepared_vm_environment.check_sources(record['environment'], frozen)
        record['environment']['sources'][-1] = {**download, 'size': 101}
        with self.assertRaises(prepared_vm.ProtocolError):
            prepared_vm_environment.check_sources(record['environment'], frozen)

    def test_new_template_is_shut_down_and_second_boot_checked_before_upload(self):
        with tempfile.TemporaryDirectory() as temporary, self.vm_context(temporary) as context:
            client, guest, download, prepare, application = context
            client.restore.return_value = None
            source = Path(temporary)/'original'
            source.write_bytes(b'original')
            download.return_value = source
            guest.disk = source

            def qemu(command, **kwargs):
                if command[:2] == ['qemu-img', 'convert']:
                    self.assertTrue(guest.shutdown.called)
                    Path(command[-1]).write_bytes(b'prepared-system-disk')

            with patch.object(linux_ci, 'run', side_effect=qemu), \
                    patch.object(linux_ci, 'verify_vm_image'), \
                    patch.object(prepared_vm, 'producer', return_value=self.manifest()['producer']):
                linux_ci.vm_test(123)
            guest.shutdown.assert_called_once()
            self.assertEqual(guest.inspect.call_count, 2)
            client.upload.assert_called_once()
            uploaded = client.upload.call_args.args[0]
            self.assertEqual(uploaded['image']['sha256'], hashlib.sha256(b'prepared-system-disk').hexdigest())
            prepared_vm.validate(uploaded)
            self.assertTrue(application.call_args.kwargs['prepared'])

    def test_signed_apt_resolution_freezes_versions_without_touching_host_apt(self):
        for arch in prepared_vm.ARCHITECTURES:
            with self.subTest(arch=arch):
                self.check_signed_apt_resolution(arch)

    def check_signed_apt_resolution(self, arch):
        changes = {'version': '1', 'index': 0}
        native = prepared_vm.ARCHITECTURES[arch]
        base = ('https://ports.ubuntu.com/ubuntu-ports' if arch == 'arm64'
                else 'https://archive.ubuntu.com/ubuntu')

        def execute(command, **kwargs):
            apt = Path(kwargs['env']['APT_CONFIG']).parent
            self.assertIn('APT::Architecture=' + native, command)
            if command[-1] == 'update':
                changes['index'] += 1
                for suite in ('focal', 'focal-updates', 'focal-security'):
                    (apt/'lists'/('ports.ubuntu.com_' + suite + '_InRelease')).write_text(
                        'isolated signed-index stand-in ' + str(changes['index']))
                self.assertNotIn('/etc/apt/sources.list', command)
                self.assertTrue((apt/'isolated.conf').read_text().startswith('Dir::Etc::parts "-"'))
                return subprocess.CompletedProcess(command, 0)
            packages = command[command.index('--no-install-recommends') + 1:]
            lines = []
            for package in packages:
                filename = package + '_' + changes['version'] + '_' + native + '.deb'
                checksum = hashlib.sha256(filename.encode()).hexdigest()
                lines.append(f"'{base}/pool/{filename}' {filename} 10 SHA256:{checksum}")
            return subprocess.CompletedProcess(command, 0, '\n'.join(lines))

        linux = {'architectures': {arch: {'cloud': {'sha256': 'a' * 64},
                                            'python': {'sha256': 'b' * 64}}}}
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {
                'GITHUB_ACTIONS': 'true', 'COR_PREPARED_VM_INPUTS': 'true'}, clear=True), \
                patch.object(prepared_vm, 'client', side_effect=AssertionError('No private access during freeze')), \
                patch.object(prepared_vm_environment, 'run', side_effect=execute):
            root = Path(temporary)
            first = prepared_vm_environment.freeze(root/'first', linux, arch)
            second = prepared_vm_environment.freeze(root/'second', linux, arch)
            self.assertEqual(first['status'], 'frozen')
            self.assertEqual(first['descriptor'], second['descriptor'])
            self.assertNotEqual(first['files'], second['files'])
            self.assertTrue(prepared_vm_environment.verify_frozen(root/'first', first, linux))
            for change in ('invalid_name', 'missing_download'):
                malformed = copy.deepcopy(first)
                if change == 'invalid_name':
                    malformed['lock']['packages'][0]['name'] = '--option'
                else:
                    malformed['lock']['downloads'].clear()
                malformed['descriptor']['environment_lock_sha256'] = prepared_vm.checksum(malformed['lock'])
                with self.subTest(change=change), self.assertRaises(prepared_vm.ProtocolError):
                    prepared_vm_environment.verify_frozen(root/'first', malformed, linux)
            changes['version'] = '2'
            changed = prepared_vm_environment.freeze(root/'third', linux, arch)
            self.assertNotEqual(first['descriptor']['environment_lock_sha256'],
                                changed['descriptor']['environment_lock_sha256'])
            damaged = root/'first'/next(iter(first['files']))
            damaged.write_text('tampered')
            with self.assertRaises(prepared_vm.ProtocolError):
                prepared_vm_environment.verify_frozen(root/'first', first, linux)

    def test_package_plan_decodes_epochs_and_rejects_duplicate_architecture(self):
        result = prepared_vm_environment.packages_from_plan({
            'example_1%3a2.0-1_arm64.deb': {}, 'second_3_all.deb': {}})
        self.assertEqual(result[0]['version'], '1:2.0-1')
        with self.assertRaises(prepared_vm.ProtocolError):
            prepared_vm_environment.packages_from_plan({
                'example_1_arm64.deb': {}, 'example_2_arm64.deb': {}})

    def test_snapshot_keeps_native_environment_locks_separate(self):
        environments = {arch: {'schema': 1, 'status': 'disabled'} for arch in prepared_vm.ARCHITECTURES}
        self.assertEqual(prepared_vm_environment.frozen_environments({'prepared_vms': environments}),
                         environments)
        legacy = {'schema': 1, 'status': 'disabled'}
        self.assertEqual(prepared_vm_environment.frozen_environments({'prepared_vm': legacy}),
                         {'arm64': legacy})
        with self.assertRaises(prepared_vm.ProtocolError):
            prepared_vm_environment.frozen_environments({'prepared_vms': {'arm64': legacy}})
        with self.assertRaises(prepared_vm.ProtocolError):
            prepared_vm_environment.packages_from_plan({'example_1_arm64.deb': {}}, 'x64')
        result = prepared_vm_environment.packages_from_plan({'example_1_amd64.deb': {}}, 'x64')
        self.assertEqual(result[0]['architecture'], 'amd64')

    def test_verified_guest_checks_its_native_frozen_environment(self):
        environments = {
            arch: {'status': 'frozen', 'lock': {'arch': arch}}
            for arch in prepared_vm.ARCHITECTURES
        }
        linux = {'architectures': {}}
        snapshot = {'prepared_vms': environments, 'upstream': {'linux': linux}}
        actual = {'packages': []}
        for arch in prepared_vm.ARCHITECTURES:
            with self.subTest(arch=arch), patch.dict(os.environ, {
                    'GITHUB_ACTIONS': 'true', 'COR_PREPARED_VM_VERIFIED': 'true',
                    'BUILD_INPUTS_DIR': 'dependency-snapshot'}), \
                    patch.object(build_inputs, 'manifest', return_value=snapshot), \
                    patch.object(release_package, 'host_arch', return_value=arch), \
                    patch.object(prepared_vm_environment, 'verify_frozen') as verify, \
                    patch.object(prepared_vm_environment, 'inspect_environment', return_value=actual), \
                    patch.object(prepared_vm_environment, 'check_environment') as check, \
                    patch.object(release_package, 'install_packages') as install:
                release_package.prepare_compatibility_environment()
                verify.assert_called_once_with(Path('dependency-snapshot'), environments[arch], linux)
                check.assert_called_once_with(actual, None, environments[arch]['lock'])
                install.assert_not_called()
                verify.side_effect = prepared_vm.ProtocolError('environment_mismatch')
                with self.assertRaises(prepared_vm.ProtocolError):
                    release_package.prepare_compatibility_environment()
                self.assertEqual(check.call_count, 1)
                verify.side_effect = None
                environments[arch]['status'] = 'disabled'
                with self.assertRaisesRegex(ValueError, 'frozen CI environment'):
                    release_package.prepare_compatibility_environment()
                environments[arch]['status'] = 'frozen'

    def test_warm_guest_must_match_full_package_manifest_and_frozen_versions(self):
        actual = self.manifest()['environment']
        lock = {'packages': actual['packages']}
        prepared_vm_environment.check_environment(actual, actual, lock)
        changed = copy.deepcopy(actual)
        changed['packages'][0]['version'] = '999'
        changed['packages_sha256'] = prepared_vm.checksum(changed['packages'])
        with self.assertRaises(prepared_vm.ProtocolError):
            prepared_vm_environment.check_environment(changed, actual, lock)
        with self.assertRaises(prepared_vm.ProtocolError):
            prepared_vm_environment.check_environment(changed, None, lock)

    def test_guest_template_operations_cannot_run_on_the_host(self):
        with patch.dict(os.environ, {}, clear=True), patch.object(prepared_vm_environment, 'run') as execute:
            for function in (prepared_vm_environment.clean_template, prepared_vm_environment.inspect_environment):
                with self.assertRaises(RuntimeError):
                    function()
            execute.assert_not_called()

    def test_template_cleanup_finishes_before_its_working_directory_disappears(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / 'cor-source'
            source.mkdir()
            original_exists = Path.exists

            def exists(path):
                return path != Path('/opt/clash-of-rust') and original_exists(path)

            def execute(command, **kwargs):
                if not source.exists():
                    raise FileNotFoundError('The source working directory has been deleted')
                if '/home/ubuntu/cor-source' in command[-1]:
                    source.rmdir()

            with patch.object(prepared_vm_environment, 'guest_only'), \
                    patch.object(Path, 'exists', exists), \
                    patch.object(prepared_vm_environment, 'run', side_effect=execute) as commands:
                prepared_vm_environment.clean_template()
            self.assertFalse(source.exists())
            self.assertIn('fstrim /', commands.call_args.args[0][-1])

    def test_image_checks_reject_external_dependencies_dirty_images_and_leaks(self):
        information = {'format': 'qcow2', 'virtual-size': 24 * 1024**3}
        invalid = [
            {**information, 'backing-filename': 'external'},
            {**information, 'snapshots': [{'id': 'snapshot'}]},
            {**information, 'encrypted': True},
            {**information, 'dirty-flag': True},
            {**information, 'format-specific': {'data': {'data-file': 'external'}}},
            {**information, 'format-specific': {'data': {'dirty-flag': True}}},
        ]
        for value in invalid:
            with self.subTest(value=value), patch.object(prepared_vm, 'qcow2_header'), \
                    patch.object(linux_ci.subprocess, 'check_output', return_value=json.dumps(value)):
                with self.assertRaises(prepared_vm.ProtocolError):
                    linux_ci.verify_vm_image(Path('/synthetic/image.qcow2'), 24 * 1024**3)
        for checked in ({}, {'format': 'qcow2', 'check-errors': 0, 'leaks': 1},
                        {'format': 'qcow2', 'check-errors': 1}):
            with self.subTest(checked=checked), patch.object(prepared_vm, 'qcow2_header'), \
                    patch.object(linux_ci.subprocess, 'check_output', side_effect=[
                        json.dumps(information), json.dumps(checked)]):
                with self.assertRaises(prepared_vm.ProtocolError):
                    linux_ci.verify_vm_image(Path('/fake/image.qcow2'), 24 * 1024**3)
        with patch.object(prepared_vm, 'qcow2_header'), patch.object(linux_ci.subprocess, 'check_output', side_effect=[
                json.dumps(information), json.dumps({'format': 'qcow2', 'check-errors': 0})]):
            linux_ci.verify_vm_image(Path('/fake/image.qcow2'), 24 * 1024**3)

    def test_qcow_header_rejects_external_storage_before_invoking_qemu(self):
        header = bytearray(512)
        struct.pack_into('>4sI', header, 0, b'QFI\xfb', 3)
        struct.pack_into('>I', header, 20, 9)
        struct.pack_into('>Q', header, 24, 24 * 1024**3)
        struct.pack_into('>I', header, 100, 104)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)/'image.qcow2'
            path.write_bytes(header)
            prepared_vm.qcow2_header(path, 24 * 1024**3)
            for offset, value, format in ((8, 512, '>Q'), (32, 1, '>I'), (60, 1, '>I'),
                                           (72, 4, '>Q'), (88, 1, '>Q'),
                                           (104, 0x44415441, '>I')):
                changed = bytearray(header)
                struct.pack_into(format, changed, offset, value)
                path.write_bytes(changed)
                with self.subTest(offset=offset), patch.object(linux_ci.subprocess, 'check_output') as execute:
                    with self.assertRaises(prepared_vm.ProtocolError):
                        linux_ci.verify_vm_image(path, 24 * 1024**3)
                    execute.assert_not_called()


class BuildTests(unittest.TestCase):
    def test_windows_update_gate_handles_spaces_without_quoting_the_option_name(self):
        command = windows._update_handoff_command(
            r'C:\Downloads\COR setup.exe', r'C:\Users\Test User\cor update', 123,
            r'C:\Program Files\Clash of Rust')
        self.assertIn('/CORWAITDIR="C:\\Users\\Test User\\cor update"', command)
        self.assertNotIn('"/CORWAITDIR=', command)
        self.assertTrue(command.endswith('/D=C:\\Program Files\\Clash of Rust'))
        with self.assertRaises(ValueError):
            windows._update_handoff_command('setup.exe', 'bad"path', 123, 'installed')

    def test_installed_update_driver_runs_outside_the_runner_checkout(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            driver = cli_update.stage_test_driver(directory)
            result = subprocess.run([sys.executable, '-B', driver, '--help'], cwd=directory,
                                    text=True, capture_output=True, check=True)
            self.assertIn('--phase', result.stdout)
            self.assertEqual(driver.stat().st_mode & 0o777, 0o644)

    def test_terminal_update_result_requires_helper_exit_and_a_persisted_terminal_state(self):
        for code, state, accepted in ((0, 'succeeded', True), (0, 'failed', True),
                                      (0, 'preparing', False), (0, 'installing', False),
                                      (1, 'succeeded', False)):
            with self.subTest(code=code, state=state), \
                    patch.object(cli_update.subprocess, 'run',
                                 return_value=subprocess.CompletedProcess([], code, '', 'helper error')), \
                    patch.object(cli_update, 'cli', return_value={'state': state}):
                if accepted:
                    self.assertEqual(cli_update.run_helper(Path('/fixture/update'), Path('/fixture/data')),
                                     {'state': state})
                else:
                    with self.assertRaisesRegex(AssertionError, 'terminal result'):
                        cli_update.run_helper(Path('/fixture/update'), Path('/fixture/data'))

    def test_installed_terminal_update_is_not_run_in_a_development_environment(self):
        with patch.dict(os.environ, {'GITHUB_ACTIONS': 'false'}), \
                patch.object(sys, 'argv', ['test-cli-update.py', '--package', '/candidate.deb']), \
                patch.object(cli_update.subprocess, 'run') as commands:
            with self.assertRaisesRegex(RuntimeError, 'disposable Actions'):
                cli_update.main()
            commands.assert_not_called()

    def test_ci_policy_keeps_real_network_smoke_and_selects_changed_specialties(self):
        basic = 'basicSystemProxyAndTun'
        routing = 'applicationRoutingPersistsAndKeepsOtherAppsOnline'
        lifecycle = 'localProxyAndRealTunCarryTrafficAndDisconnectCleanly'
        tile = 'quickTileRestoresColdCoreWithoutOpeningTheActivity'
        diagnostic = 'measurementDiagnosticsDistinguishFailuresAndExport'
        source = (build.ROOT/'apps/android/app/src/androidTest/java/org/clashofrust/android/ProxyTunTest.java').read_text()
        for name in (basic, routing, lifecycle, tile, diagnostic):
            self.assertRegex(source, r'@Test\s+public\s+void\s+'+name+r'\s*\(')
        with patch.dict(os.environ, {'COR_CI_EXTENDED_TESTS': 'false',
                                     'COR_CI_ANDROID_ROUTING_TESTS': 'false',
                                     'COR_CI_ANDROID_DIAGNOSTICS_TESTS': 'false',
                                     'COR_CI_LINUX_USER_TESTS': 'false',
                                     'COR_CI_UPDATE_TESTS': 'false',
                                     'COR_CI_DESKTOP_UI_TESTS': 'false'}):
            self.assertEqual(ci_test_policy.android_cases(), (basic,))
            self.assertFalse(ci_test_policy.linux_user_tests())
            self.assertFalse(ci_test_policy.desktop_ui_tests())
            self.assertFalse(ci_test_policy.update_tests())
            self.assertEqual(ci_test_policy.select([]), {
                'extended_tests': False, 'android_routing_tests': False, 'android_diagnostics_tests': False,
                'linux_user_tests': False,
                'desktop_ui_tests': False, 'proxy_benchmark': False, 'update_tests': False})
            self.assertTrue(ci_test_policy.select([
                'apps/android/app/src/main/java/org/clashofrust/android/ApplicationRouting.java'
            ])['android_routing_tests'])
            self.assertTrue(ci_test_policy.select([
                'apps/android/native/src/application_routing_mode.rs'
            ])['android_routing_tests'])
            self.assertTrue(ci_test_policy.select([
                'crates/client-runtime/src/platform/host_network/linux.rs'
            ])['linux_user_tests'])
            for path in ('crates/client-runtime/src/platform/host_network/ipv6.rs',
                         'crates/client-runtime/src/platform/linux/startup.rs',
                         'crates/client-runtime/src/platform/network/mod.rs',
                         'crates/client-runtime/src/control/network.rs',
                         'crates/client-runtime/src/engine/recovery.rs',
                         'scripts/tun_ipv6_lifecycle.py',
                         'scripts/ci_test_policy.py',
                         'apps/desktop/tests/core_integration.rs',
                         'apps/desktop/tests/support/protected_dns.rs'):
                self.assertTrue(ci_test_policy.select([path])['linux_user_tests'], path)
            self.assertTrue(ci_test_policy.select([
                'crates/client-ui/ui/pages/proxies.slint'
            ])['desktop_ui_tests'])
            self.assertTrue(ci_test_policy.select([
                'vendor/winit/src/platform_impl/linux/x11/resize_sync.rs'
            ])['desktop_ui_tests'])
            for path in ('crates/client-core/src/config/mod.rs',
                         'crates/client-core/src/protection/mod.rs',
                         'crates/client-runtime/src/control/mod.rs',
                         'crates/mobile-ffi/src/lib.rs',
                         'apps/android/native/src/android.rs',
                         'crates/client-core/src/application_rules/groups.rs',
                         'apps/android/core/application_rules.go',
                         'scripts/run-ci.py',
                         'scripts/ci_test_policy.py'):
                self.assertTrue(ci_test_policy.select([path])['desktop_ui_tests'], path)
            for path in ('crates/client-core/src/config/mod.rs',
                         'crates/client-core/src/protection/mod.rs',
                         'crates/mobile-ffi/src/lib.rs',
                         'crates/client-core/src/application_rules/groups.rs',
                         'apps/android/app/src/main/java/org/clashofrust/android/IncidentNotifications.java'):
                self.assertTrue(ci_test_policy.select([path])['android_routing_tests'], path)
            for path in ('crates/client-runtime/src/update/install/terminal.rs',
                         'apps/cli/src/commands/maintenance.rs', 'scripts/test-cli-update.py'):
                self.assertTrue(ci_test_policy.select([path])['update_tests'])
        with patch.dict(os.environ, {'COR_CI_EXTENDED_TESTS': 'false',
                                     'COR_CI_ANDROID_ROUTING_TESTS': 'true',
                                     'COR_CI_ANDROID_DIAGNOSTICS_TESTS': 'false'}):
            self.assertEqual(ci_test_policy.android_cases(), (basic, routing))
        with patch.dict(os.environ, {'COR_CI_EXTENDED_TESTS': 'false',
                                     'COR_CI_ANDROID_ROUTING_TESTS': 'false',
                                     'COR_CI_ANDROID_DIAGNOSTICS_TESTS': 'true'}):
            self.assertEqual(ci_test_policy.android_cases(), (basic, diagnostic))
            for path in ('crates/client-core/src/controller/measurement.rs',
                         'apps/android/app/src/main/java/org/clashofrust/android/NetworkOverview.java',
                         'apps/android/native/src/android.rs',
                         'apps/android/app/src/main/java/org/clashofrust/android/IncidentNotifications.java'):
                self.assertTrue(ci_test_policy.select([path])['android_diagnostics_tests'])
        with patch.dict(os.environ, {'COR_CI_EXTENDED_TESTS': 'true'}):
            self.assertEqual(ci_test_policy.android_cases(), (lifecycle, routing, tile, diagnostic))
            self.assertTrue(ci_test_policy.linux_user_tests())
            self.assertTrue(ci_test_policy.desktop_ui_tests())
            self.assertTrue(ci_test_policy.update_tests())

    def test_ci_mobile_ui_filters_name_existing_tests(self):
        source = (build.ROOT/'scripts/run-ci.py').read_text()
        loops = []
        for node in ast.walk(ast.parse(source)):
            if not isinstance(node, ast.For) or not isinstance(node.iter, ast.Tuple):
                continue
            for call in ast.walk(node):
                if not isinstance(call, ast.Call) or not isinstance(call.func, ast.Name):
                    continue
                if call.func.id != 'run' or not call.args or not isinstance(call.args[0], ast.List):
                    continue
                if any(isinstance(item, ast.Constant) and item.value == 'mobile_lists'
                       for item in call.args[0].elts):
                    loops.append(node)
        self.assertEqual(len(loops), 1)
        cases = [ast.literal_eval(item) for item in loops[0].iter.elts]
        self.assertTrue(cases)
        self.assertEqual(len(cases), len(set(cases)))
        tests = re.findall(r'#\[test\]\s*fn\s+([A-Za-z_][A-Za-z_0-9]*)',
                           (build.ROOT/'crates/client-ui/tests/mobile_lists.rs').read_text())
        for case in cases:
            with self.subTest(case=case):
                self.assertEqual(tests.count(case), 1)

    def test_linux_network_selection_survives_sudo(self):
        specification = importlib.util.spec_from_file_location(
            'proxy_modes', build.ROOT/'scripts/test-proxy-modes.py')
        proxy_modes = importlib.util.module_from_spec(specification)
        specification.loader.exec_module(proxy_modes)
        for selected in (False, True):
            with self.subTest(selected=selected), \
                    patch.dict(os.environ, {'GITHUB_ACTIONS': 'true',
                                            'COR_CI_EXTENDED_TESTS': 'false',
                                            'COR_CI_LINUX_USER_TESTS': str(selected).lower()}), \
                    patch.object(proxy_modes, 'host_system', return_value='linux'), \
                    patch.object(proxy_modes.Path, 'resolve', autospec=True,
                                 side_effect=lambda path, **kwargs: path), \
                    patch.object(proxy_modes, 'run') as invoke:
                proxy_modes.check(Path('/test/core_integration'), Path('/opt/clash-of-rust'))
            command = invoke.call_args.args[0]
            self.assertEqual('--with-user-tests' in command, selected)

    def test_proxy_benchmark_is_selected_for_transport_changes_and_survives_sudo(self):
        self.assertTrue(ci_test_policy.select([
            'apps/desktop/tests/support/proxy_benchmark.rs'])['proxy_benchmark'])
        self.assertTrue(ci_test_policy.select([
            'crates/client-core/src/subscription/routes.rs'])['proxy_benchmark'])
        self.assertTrue(ci_test_policy.select([
            'crates/client-core/src/controller/mod.rs'])['proxy_benchmark'])
        self.assertFalse(ci_test_policy.select(['README.md'])['proxy_benchmark'])
        specification = importlib.util.spec_from_file_location(
            'proxy_modes_benchmark', build.ROOT/'scripts/test-proxy-modes.py')
        module = importlib.util.module_from_spec(specification)
        specification.loader.exec_module(module)
        with patch.dict(os.environ, {'GITHUB_ACTIONS': 'true',
                                    'COR_CI_PROXY_BENCHMARK': 'true',
                                    'COR_CI_LINUX_USER_TESTS': 'false',
                                    'COR_CI_EXTENDED_TESTS': 'false'}), \
                patch.object(module, 'host_system', return_value='linux'), \
                patch.object(module.Path, 'resolve', autospec=True,
                             side_effect=lambda path, **kwargs: path), \
                patch.object(module, 'run') as invoke:
            module.check(Path('/test/core_integration'), Path('/opt/clash-of-rust'))
        self.assertIn('--proxy-benchmark', invoke.call_args.args[0])

    def test_proxy_benchmark_fixture_serves_identical_verified_http_and_https_payloads(self):
        import ssl
        import urllib.request
        with patch.object(proxy_benchmark, 'BODY_SIZE', len(proxy_benchmark.BLOCK)), \
                patch.object(proxy_benchmark, 'save_report') as save:
            with proxy_benchmark.fixture() as environment:
                manifest = json.loads(Path(environment['CLASH_PROXY_BENCHMARK_FIXTURE']).read_text())
                context = ssl.create_default_context(cafile=manifest['certificate'])
                context.verify_flags |= ssl.VERIFY_X509_STRICT
                opener = urllib.request.build_opener(
                    urllib.request.ProxyHandler({}), urllib.request.HTTPSHandler(context=context))
                for url in manifest['urls']:
                    with opener.open(url, timeout=5) as response:
                        body = response.read()
                        self.assertEqual(len(body), manifest['size'])
                        self.assertEqual(hashlib.sha256(body).hexdigest(), manifest['sha256'])
                        self.assertEqual(response.headers['Content-Length'], str(len(body)))
                request = urllib.request.Request(manifest['latency_url'], method='HEAD')
                with opener.open(request, timeout=5) as response:
                    self.assertEqual(response.status, 204)
                    self.assertEqual(response.read(), b'')
                Path(manifest['report']).write_text('{"complete": true}')
            save.assert_called_once()
            self.assertFalse(Path(manifest['certificate']).exists())

    def test_proxy_benchmark_rejects_missing_or_partial_reports_without_masking_test_failure(self):
        for report in (None, {'complete': False}):
            with self.subTest(report=report), \
                    patch.object(proxy_benchmark, 'save_report') as save:
                with self.assertRaisesRegex(RuntimeError, 'did not'):
                    with proxy_benchmark.fixture() as environment:
                        manifest = json.loads(Path(environment['CLASH_PROXY_BENCHMARK_FIXTURE']).read_text())
                        if report is not None:
                            Path(manifest['report']).write_text(json.dumps(report))
                self.assertEqual(save.call_count, int(report is not None))
        with patch.object(proxy_benchmark, 'save_report'):
            with self.assertRaisesRegex(ValueError, 'original failure'):
                with proxy_benchmark.fixture():
                    raise ValueError('original failure')

    def test_proxy_benchmark_preserves_partial_measurements_and_labels_their_scope(self):
        report = {
            'system': 'windows', 'arch': 'x64', 'host': {'logical_cpus': 4},
            'complete': False, 'error': 'TLS handshake timed out',
            'samples': [], 'summaries': [{
                'protocol': 'https', 'route': 'native-http', 'sink': 'memory',
                'median_mib_per_second': 12.5, 'median_first_byte_ms': 4.25,
            }],
            'latency_summaries': [{
                'route': 'controller-loopback-socks', 'median_elapsed_ms': 51.5,
                'median_reported_delay_ms': 25,
            }, {
                'route': 'direct', 'median_elapsed_ms': 25.5,
                'median_reported_delay_ms': None,
            }],
        }
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root/'report.json'
            source.write_text(json.dumps(report))
            with patch.object(proxy_benchmark, 'ROOT', root), \
                    patch.dict(os.environ, {'GITHUB_STEP_SUMMARY': str(root/'summary.md')}), \
                    patch('builtins.print'):
                proxy_benchmark.save_report(source)
            preserved = json.loads((root/'target/proxy-benchmark-report/loopback.json').read_text())
            self.assertFalse(preserved['complete'])
            self.assertEqual(preserved['error'], report['error'])
            text = (root/'summary.md').read_text()
            self.assertIn('| https | native-http | memory | 12.50 | 4.25 |', text)
            self.assertIn('| controller-loopback-socks | 51.50 | 25.00 |', text)
            self.assertIn('| direct | 25.50 | — |', text)
            self.assertIn('no public DNS, remote node or GitHub CDN', text)

    def test_previews_reject_a_viewer_without_color_font_support(self):
        for version, supported in [('slint-viewer 1.18.1', False),
                                   ('slint-viewer 1.19.0', True)]:
            with self.subTest(version=version), patch.object(
                    preview_slint.subprocess, 'run',
                    return_value=subprocess.CompletedProcess([], 0, version)):
                if supported:
                    self.assertEqual(preview_slint.rendering_viewer('viewer')['version'], version)
                else:
                    with self.assertRaisesRegex(RuntimeError, 'Accurate emoji previews'):
                        preview_slint.rendering_viewer('viewer')

    def test_previews_use_bundled_font_fallback_instead_of_the_hosts_preferences(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(
                os.environ, {'SLINT_FONT_PATH': '/unrelated/font',
                             'SLINT_DEFAULT_FONT': '/unrelated/primary'}):
            environment = preview_slint.preview_environment(Path(temporary))
            self.assertEqual(environment['SLINT_FONT_PATH'],
                             str(build.ROOT/'resources/fonts/NotoColorEmoji-Regular.ttf'))
            self.assertEqual(environment['SLINT_DEFAULT_FONT'],
                             str(build.ROOT/'resources/fonts/SourceHanSansCN-Regular.otf'))

    def test_desktop_scope_uses_the_selected_checkout_members(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root/'Cargo.toml').write_text('[workspace]\nmembers=["apps/desktop"]\n')
            self.assertEqual(build.desktop_cargo_scope(root), ['--workspace'])
            native = root/'apps/android/native'
            native.mkdir(parents=True)
            (native/'Cargo.toml').write_text('[package]\nname="legacy-android-host"\n')
            (root/'Cargo.toml').write_text('[workspace]\nmembers=["apps/desktop","apps/android/native"]\n')
            self.assertEqual(build.desktop_cargo_scope(root), ['--workspace', '--exclude', 'legacy-android-host'])

    def test_desktop_pipeline_excludes_mobile_without_dropping_shared_sources(self):
        specification = importlib.util.spec_from_file_location('native_ci', build.ROOT/'scripts/run-ci.py')
        native_ci = importlib.util.module_from_spec(specification)
        specification.loader.exec_module(native_ci)
        commands = []

        def capture(command, **kwargs):
            commands.append(command)
        def capture_test(command, **kwargs):
            commands.append(command)
            raise RuntimeError('source checks completed')

        with patch.dict(os.environ, {'COR_CI_EXTENDED_TESTS': 'false'}), \
                patch.object(sys, 'argv', ['run-ci.py', '--system', 'linux', '--arch', 'x64']), \
                patch.object(native_ci, 'host_system', return_value='linux'), \
                patch.object(native_ci, 'host_arch', return_value='x64'), \
                patch.object(local_cache, 'workspace_sources'), \
                patch.object(native_ci, 'run', side_effect=capture), \
                patch.object(native_ci, 'cargo_messages', side_effect=capture_test):
            with self.assertRaisesRegex(RuntimeError, 'source checks completed'):
                native_ci.main()
        for command in commands[1:3]:
            excluded = [command[index + 1] for index, value in enumerate(command) if value == '--exclude']
            self.assertEqual(set(excluded), {'clash-mobile-runtime', 'clash-android'})
            self.assertIn('--workspace', command)
        self.assertEqual([command[1] for command in commands], ['fmt', 'clippy', 'build', 'test'])
        self.assertEqual(commands[-1][2:6], ['-p', 'clash-of-rust', '--test', 'core_integration'])
        self.assertIn('--no-run', commands[-1])

    def test_update_changes_run_shared_schedule_config_cli_and_gui_regressions(self):
        specification = importlib.util.spec_from_file_location('native_update_ci', build.ROOT/'scripts/run-ci.py')
        native_ci = importlib.util.module_from_spec(specification)
        specification.loader.exec_module(native_ci)
        commands = []
        with patch.dict(os.environ, {'COR_CI_EXTENDED_TESTS': 'false',
                                     'COR_CI_DESKTOP_UI_TESTS': 'false',
                                     'COR_CI_UPDATE_TESTS': 'true'}), \
                patch.object(sys, 'argv', ['run-ci.py', '--system', 'linux', '--arch', 'x64']), \
                patch.object(native_ci, 'host_system', return_value='linux'), \
                patch.object(native_ci, 'host_arch', return_value='x64'), \
                patch.object(local_cache, 'workspace_sources'), \
                patch.object(native_ci, 'run', side_effect=lambda command, **kwargs: commands.append(command)), \
                patch.object(native_ci, 'cargo_messages', side_effect=RuntimeError('source checks completed')):
            with self.assertRaisesRegex(RuntimeError, 'source checks completed'):
                native_ci.main()
        checks = [command for command in commands if len(command) > 1 and command[1] == 'test']
        for package, case in (('clash-client-core', 'update'),
                              ('clash-client-runtime', 'update_check_interval'),
                              ('clash-cli', 'update_'),
                              ('clash-of-rust', 'update_')):
            self.assertTrue(any(command[command.index('-p')+1] == package and case in command
                                for command in checks), (package, case, checks))
        for path in ('crates/client-core/src/config/mod.rs',
                     'crates/client-core/src/update/schedule.rs'):
            self.assertTrue(ci_test_policy.select([path])['update_tests'])

    def test_desktop_ui_changes_run_targeted_checks_in_linux_container(self):
        specification = importlib.util.spec_from_file_location('native_ui_ci', build.ROOT/'scripts/run-ci.py')
        native_ci = importlib.util.module_from_spec(specification)
        specification.loader.exec_module(native_ci)
        commands = []
        with patch.dict(os.environ, {'COR_CI_EXTENDED_TESTS': 'false',
                                     'COR_CI_DESKTOP_UI_TESTS': 'true'}), \
                patch.object(sys, 'argv', ['run-ci.py', '--system', 'linux', '--arch', 'x64']), \
                patch.object(native_ci, 'host_system', return_value='linux'), \
                patch.object(native_ci, 'host_arch', return_value='x64'), \
                patch.object(local_cache, 'workspace_sources'), \
                patch.object(native_ci, 'run', side_effect=lambda command, **kwargs: commands.append(command)), \
                patch.object(native_ci, 'cargo_messages', side_effect=RuntimeError('source checks completed')):
            with self.assertRaisesRegex(RuntimeError, 'source checks completed'):
                native_ci.main()
        ui_checks = [command for command in commands if len(command) > 1 and command[1] == 'test']
        self.assertTrue(any('application::ui_tests::resizing_a_node_grid_preserves_odd_tail_nodes_and_their_own_actions'
                            in command for command in ui_checks))
        self.assertTrue(any('application::ui_tests::shortcut_dialog_edits_the_draft_and_keeps_disabled_choices_after_document_release'
                            in command for command in ui_checks))
        self.assertTrue(any('application::ui_tests::appearance_remains_interactive_while_geo_owns_the_engine_and_late_reply_cannot_undo_it'
                            in command for command in ui_checks))
        self.assertTrue(any('notice_countdown_restarts_and_expires_after_five_seconds'
                            in command for command in ui_checks))
        self.assertTrue(any('applications_scroll_across_phone_and_tablet_viewports'
                            in command for command in ui_checks))
        self.assertTrue(any('mobile_accent_picker_remains_usable_while_protected'
                            in command for command in ui_checks))
        self.assertTrue(any('application_controls_distinguish_scope_filter_save_and_protection'
                            in command for command in ui_checks))
        self.assertTrue(any('mobile_appearance_settings_reopen_with_geometry_and_protection'
                            in command for command in ui_checks))
        self.assertTrue(any(command[command.index('-p')+1] == 'clash-client-runtime'
                            and 'control::tests::' in command for command in ui_checks))
        self.assertTrue(any(command[command.index('-p')+1] == 'clash-client-runtime'
                            and 'control::dispatch::tests::' in command for command in ui_checks))
        self.assertTrue(any('mobile_rule_rows_keep_actions_aligned_for_all_sources_and_sizes'
                            in command for command in ui_checks))
        focused = [command for command in ui_checks if
                   command[command.index('-p')+1] == 'clash-of-rust'
                   or 'mobile_lists' in command]
        for case in (
                'application::ui_tests::protection_locks_rule_controls_and_proxy_mode_without_a_warning_or_draft_change',
                'application::ui_tests::protection_rejects_confirmations_that_would_save_or_delete',
                'application::ui_tests::a_locked_rules_page_still_accepts_the_global_exit_confirmation',
                'application::commands::synchronization_tests::protection_allows_observation_and_disable_but_rejects_mutations'):
            self.assertTrue(any(case in command for command in focused), case)
        self.assertTrue(any('application_policy_form_respects_platform_protection_and_scrolls'
                            in command for command in focused))
        self.assertTrue(all('--exact' in command for command in focused))
        for package, target in (('clash-client-ui', 'ui_models'),
                                ('clash-desktop-ui', '--lib'),
                                ('clash-client-core', '--lib')):
            self.assertTrue(any(command[command.index('-p')+1] == package and target in command
                                for command in ui_checks), (package, target))
        with patch.dict(os.environ, {'COR_CI_DESKTOP_UI_TESTS': 'true'}), \
                patch.object(linux_ci, 'build_image', return_value='test-image'), \
                patch.object(linux_ci, 'run') as run:
            linux_ci.container(['true'])
        arguments = run.call_args.args[0]
        index = arguments.index('COR_CI_DESKTOP_UI_TESTS')
        self.assertEqual(arguments[index - 1], '--env')

    def test_linux_tun_changes_run_agent_and_restart_checks_without_full_suite(self):
        specification = importlib.util.spec_from_file_location('native_tun_ci', build.ROOT/'scripts/run-ci.py')
        native_ci = importlib.util.module_from_spec(specification)
        specification.loader.exec_module(native_ci)
        commands = []
        with patch.dict(os.environ, {'COR_CI_EXTENDED_TESTS': 'false',
                                     'COR_CI_LINUX_USER_TESTS': 'true',
                                     'COR_CI_DESKTOP_UI_TESTS': 'false',
                                     'COR_CI_UPDATE_TESTS': 'false'}), \
                patch.object(sys, 'argv', ['run-ci.py', '--system', 'linux', '--arch', 'x64']), \
                patch.object(native_ci, 'host_system', return_value='linux'), \
                patch.object(native_ci, 'host_arch', return_value='x64'), \
                patch.object(local_cache, 'workspace_sources'), \
                patch.object(native_ci, 'run', side_effect=lambda command, **kwargs: commands.append(command)), \
                patch.object(native_ci, 'cargo_messages', side_effect=RuntimeError('source checks completed')):
            with self.assertRaisesRegex(RuntimeError, 'source checks completed'):
                native_ci.main()
        checks = [command for command in commands if len(command) > 1 and command[1] == 'test']
        self.assertEqual(len(checks), 5)
        for package, case in (('clash-client-runtime', 'platform::linux_tun::'),
                              ('clash-client-runtime', 'platform::linux_startup::'),
                              ('clash-client-runtime', 'platform::host_network::'),
                              ('clash-client-runtime', 'control::tests::'),
                              ('clash-cli', 'backend::tests::')):
            self.assertTrue(any(command[command.index('-p')+1] == package and case in command
                                for command in checks), (package, case))
        self.assertTrue(ci_test_policy.select([
            'crates/client-runtime/src/platform/linux/tun/terminal_agent.rs'
        ])['linux_user_tests'])

    def test_android_build_runs_mobile_unit_tests_before_preparing_target_tools(self):
        with patch.dict(os.environ, {'COR_CI_EXTENDED_TESTS': 'true'}), \
                patch.object(local_cache, 'workspace_sources'), \
                patch.object(android_build, 'cargo_build') as clippy, \
                patch.object(android_build, 'run', side_effect=RuntimeError('mobile tests executed')) as tests:
            with self.assertRaisesRegex(RuntimeError, 'mobile tests executed'):
                android_build.build('arm64')
        for command in (clippy.call_args.args[0], tests.call_args.args[0]):
            packages = [command[index + 1] for index, value in enumerate(command) if value == '-p']
            self.assertEqual(packages, ['clash-mobile-runtime', 'clash-android'])
            self.assertNotIn('--workspace', command)
            self.assertIn('--locked', command)
        self.assertIn('--lib', tests.call_args.args[0])

    def test_linux_ci_prepares_resources_on_host_before_entering_container(self):
        with patch.object(sys, 'argv', ['linux_ci.py', 'ci']), \
                patch.object(linux_ci, 'require_actions'), \
                patch.object(linux_ci, 'host_arch', return_value='x64'), \
                patch.object(linux_ci, 'run') as run, \
                patch.object(linux_ci, 'container') as container:
            calls = []
            run.side_effect = lambda command: calls.append(('host', command))
            container.side_effect = lambda command: calls.append(('container', command))
            linux_ci.main()
        self.assertEqual([call[0] for call in calls], ['host', 'container'])
        self.assertEqual(calls[0][1][1:], [
            build.ROOT/'scripts/prepare-resources.py', '--system', 'linux', '--arch', 'x64'])
        self.assertEqual(calls[1][1], [
            'python3', 'scripts/run-ci.py', '--system', 'linux', '--arch', 'x64'])

    def test_isolated_tls_fixture_passes_strict_certificate_validation(self):
        if os.name != 'posix' or not shutil.which('openssl'):
            self.skipTest('POSIX and OpenSSL are required for the isolated certificate fixture')
        specification = importlib.util.spec_from_file_location('linux_tun_fixture', build.ROOT/'scripts/test-linux-tun.py')
        fixture = importlib.util.module_from_spec(specification)
        specification.loader.exec_module(fixture)
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            root = fixture.create_uplink_certificates(directory, ('127.0.0.1', '::1'))
            for hostname in ('127.0.0.1', '::1', 'resolver.fixture.test'):
                identity = '-verify_hostname' if hostname.endswith('.test') else '-verify_ip'
                result = subprocess.run(['openssl', 'verify', '-x509_strict', '-CAfile', str(root),
                                         '-purpose', 'sslserver', identity, hostname, str(directory/'server.crt')],
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_composite_actions_only_use_supported_step_properties(self):
        supported = {'name', 'id', 'if', 'run', 'env', 'shell', 'working-directory',
                     'uses', 'with', 'continue-on-error'}
        for path in (build.ROOT/'.github/actions').glob('*/action.yml'):
            content = path.read_text()
            if not re.search(r'^  using: composite$', content, re.MULTILINE):
                continue
            steps = content.split('\n  steps:\n', 1)[1]
            for entry in re.finditer(r'^(?:    - |      )([a-z][a-z0-9-]*):', steps, re.MULTILINE):
                with self.subTest(action=path.parent.name, property=entry[1]):
                    self.assertIn(entry[1], supported)

    def test_windows_crash_readiness_allows_native_suite_startup_budget(self):
        ready = MagicMock()
        ready.is_file.side_effect = [False, False, True]
        kernel = MagicMock()
        kernel.WaitForSingleObject.return_value = 258
        with patch.object(windows.time, 'monotonic', side_effect=[0, 46, 120]), \
             patch.object(windows.time, 'sleep') as pause:
            windows.wait_unelevated_ready(kernel, 12, ready)
        self.assertEqual(pause.call_count, 2)
        self.assertEqual(kernel.WaitForSingleObject.call_count, 2)

    def test_windows_crash_readiness_fails_when_owner_exits_or_cannot_be_inspected(self):
        ready = MagicMock()
        ready.is_file.return_value = False
        for state, message in ((0, 'exited before'), (0xFFFFFFFF, 'wait status')):
            kernel = MagicMock()
            kernel.WaitForSingleObject.return_value = state
            with self.subTest(state=state), patch.object(windows.time, 'sleep') as pause:
                with self.assertRaisesRegex(RuntimeError, message):
                    windows.wait_unelevated_ready(kernel, 12, ready)
                pause.assert_not_called()

    def test_windows_crash_readiness_retains_bounded_timeout(self):
        ready = MagicMock()
        ready.is_file.return_value = False
        kernel = MagicMock()
        kernel.WaitForSingleObject.return_value = 258
        with patch.object(windows.time, 'monotonic', side_effect=[0, 240]), \
             patch.object(windows.time, 'sleep') as pause:
            with self.assertRaisesRegex(TimeoutError, 'did not become ready'):
                windows.wait_unelevated_ready(kernel, 12, ready)
            pause.assert_not_called()

    def test_macos_window_waits_for_native_mapping_and_records_rejected_candidates(self):
        pending = {'windows': [], 'candidates': [{'checks': {'onscreen': False}}]}
        mapped = {'windows': [{'kCGWindowOwnerPID': 42}], 'candidates': []}
        process = MagicMock()
        process.poll.return_value = None
        inspect = MagicMock(side_effect=[(pending, {42: 'clash-desktop-ui'}),
                                         (mapped, {42: 'clash-desktop-ui'})])
        with tempfile.TemporaryDirectory() as temporary, \
             patch.object(macos, 'REPORT', Path(temporary)), \
             patch.object(macos.time, 'sleep') as pause:
            result = macos.wait_for_window(inspect, process, 'installed')
            self.assertEqual(result, (mapped, {42: 'clash-desktop-ui'}))
            self.assertEqual(json.loads((Path(temporary)/'installed-window-observations.json').read_text()),
                             [pending, mapped])
        pause.assert_called_once_with(.5)

    def test_macos_window_wait_reports_candidates_at_bounded_deadline(self):
        pending = {'windows': [], 'candidates': [{'checks': {'minimum_width': False}}]}
        process = MagicMock()
        process.poll.return_value = None
        with tempfile.TemporaryDirectory() as temporary, \
             patch.object(macos, 'REPORT', Path(temporary)), \
             patch.object(macos.time, 'monotonic', side_effect=[0, 30]), \
             patch.object(macos.time, 'sleep') as pause:
            with self.assertRaisesRegex(RuntimeError, r'30 seconds.*minimum_width'):
                macos.wait_for_window(lambda label: (pending, {}), process, 'installed')
        pause.assert_not_called()

    def test_macos_window_wait_rejects_an_exited_owner(self):
        process = MagicMock()
        process.poll.return_value = -11
        process.returncode = -11
        inspect = MagicMock()
        with self.assertRaisesRegex(RuntimeError, r'GUI exited.*-11'):
            macos.wait_for_window(inspect, process, 'installed')
        inspect.assert_not_called()

    def test_macos_cleanup_keeps_primary_failure_and_still_rejects_cleanup_only_failure(self):
        for primary in (RuntimeError('window missing'), None):
            process = MagicMock()
            process.terminate.side_effect = RuntimeError('cleanup failed')
            with self.subTest(primary=primary), tempfile.TemporaryDirectory() as temporary, \
                 patch.object(macos, 'REPORT', Path(temporary)):
                if primary is None:
                    with self.assertRaisesRegex(RuntimeError, 'cleanup failed'):
                        macos.cleanup_gui(process, None, 'installed')
                else:
                    macos.cleanup_gui(process, None, 'installed', primary)
                report = json.loads((Path(temporary)/'installed-cleanup-error.json').read_text())
                self.assertEqual(report, {'error': 'cleanup failed',
                                         'primary_error': str(primary) if primary else None})

    def test_linux_guest_package_preparation_skips_documentation_indexes(self):
        with patch.dict(os.environ, {'COR_LINUX_GUEST': 'true'}), \
             patch.object(release_package, 'run') as execute:
            release_package.prepare_guest_package_environment()
        calls = [call.args[0] for call in execute.call_args_list]
        self.assertEqual(calls, [
            ['sudo', 'debconf-set-selections'],
            ['sudo', 'tee', '/etc/dpkg/dpkg.cfg.d/99-cor-ci-no-docs'],
            ['sudo', 'rm', '-f', '/var/lib/man-db/auto-update'],
        ])
        self.assertIn('man-db man-db/auto-update boolean false\n',
                      execute.call_args_list[0].kwargs['input'])
        self.assertIn('path-exclude=/usr/share/man/*',
                      execute.call_args_list[1].kwargs['input'])

    def test_linux_guest_package_preparation_is_guest_only(self):
        with patch.dict(os.environ, {}, clear=True), \
             patch.object(release_package, 'run') as execute:
            release_package.prepare_guest_package_environment()
        execute.assert_not_called()

    def test_linux_package_install_delegates_timeout_to_root(self):
        with patch.object(linux_apt, 'run') as execute:
            release_package.run_privileged_with_timeout(['apt-get', 'install'], 900)
        execute.assert_called_once_with([
            'sudo', 'env', 'DEBIAN_FRONTEND=noninteractive', 'timeout',
            '--signal=TERM', '--kill-after=30s', '900s', 'apt-get', 'install',
        ])

    def test_linux_download_cache_identity_tracks_arch_and_upstream_content(self):
        value = {'image': 'ubuntu@sha256:' + 'c'*64, 'architectures': {
            arch: {'python': {'sha256': 'a'*64}, 'cloud': {'sha256': 'b'*64}}
            for arch in ('x64', 'arm64')}}
        keys = linux_ci.cache_keys(value, 'x64')
        self.assertIn('a'*64, keys['python'])
        self.assertIn('b'*64, keys['cloud'])
        self.assertNotEqual(keys, linux_ci.cache_keys(value, 'arm64'))
        value['architectures']['x64']['python']['sha256'] = 'not-a-checksum\ninjected=key'
        with self.assertRaises(ValueError):
            linux_ci.cache_keys(value, 'x64')

    def test_linux_build_bootstraps_https_with_runner_trust_and_removes_it_from_context(self):
        value = {'image': 'ubuntu@sha256:' + 'c'*64, 'architectures': {
            'x64': {'python': {'sha256': 'a'*64}}}}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root/'python.tar.gz'
            archive.write_bytes(b'portable Python')
            certificates = root/'ca.crt'
            certificates.write_bytes(b'runner trust bundle')
            contexts = []

            def build_image(command, **kwargs):
                context = Path(command[-1])
                contexts.append(context)
                self.assertEqual((context/'bootstrap-ca.crt').read_bytes(), certificates.read_bytes())
                self.assertEqual((context/'python.tar.gz').read_bytes(), archive.read_bytes())
                self.assertEqual((context/'Dockerfile').read_bytes(),
                                 (build.ROOT/'resources/linux/ubuntu20.Dockerfile').read_bytes())

            with patch.object(linux_ci, 'require_actions'), \
                    patch.object(linux_ci, 'inputs', return_value=value), \
                    patch.object(linux_ci, 'host_arch', return_value='x64'), \
                    patch.object(linux_ci, 'download', return_value=archive), \
                    patch.object(linux_ci, 'tools_directory', return_value=root), \
                    patch.object(linux_ci.subprocess, 'run') as inspect, \
                    patch.object(linux_ci, 'system_ca_bundle') as trust, \
                    patch.object(linux_ci, 'run', side_effect=build_image) as execute:
                inspect.return_value.returncode = 0
                linux_ci.build_image()
                trust.assert_not_called()
                execute.assert_not_called()
                inspect.return_value.returncode = 1
                trust.side_effect = RuntimeError('Ubuntu build requires the runner system CA bundle')
                with self.assertRaisesRegex(RuntimeError, 'system CA bundle'):
                    linux_ci.build_image()
                execute.assert_not_called()
                trust.side_effect = None
                trust.return_value = certificates
                linux_ci.build_image()
                execute.assert_called_once()
            self.assertEqual(len(contexts), 1)
            self.assertFalse(contexts[0].exists())

    def test_linux_build_uses_runner_ca_when_standalone_python_has_no_default(self):
        with patch.object(linux_ci.ssl, 'get_default_verify_paths') as trust, \
                patch.object(Path, 'is_file', autospec=True) as exists:
            trust.return_value.cafile = None
            exists.side_effect = lambda path: str(path) == '/etc/ssl/certs/ca-certificates.crt'
            self.assertEqual(linux_ci.system_ca_bundle(), Path('/etc/ssl/certs/ca-certificates.crt'))
            trust.return_value.cafile = '/custom/runner-ca.crt'
            exists.side_effect = lambda path: str(path) == '/custom/runner-ca.crt'
            self.assertEqual(linux_ci.system_ca_bundle(), Path('/custom/runner-ca.crt'))
            exists.return_value = False
            exists.side_effect = None
            with self.assertRaisesRegex(RuntimeError, 'system CA bundle'):
                linux_ci.system_ca_bundle()

    def test_compiler_container_restore_verifies_bytes_environment_and_loaded_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            image = root/'image.tar'
            image.write_bytes(b'compiler environment without applications or VM disks')
            receipt = {
                'schema': 1, 'identity': 'a'*64, 'id': 'sha256:'+'b'*64,
                'size': image.stat().st_size, 'sha256': build.sha256(image),
            }
            inspect = ['docker', 'image', 'inspect', '--format={{.Id}}', 'fixture']
            for replacement in (None, [], dict(receipt, identity='c'*64),
                                dict(receipt, sha256='0'*64), dict(receipt, size=1),
                                dict(receipt, id=['invalid'])):
                with self.subTest(replacement=replacement):
                    (root/'image.json').write_text(json.dumps(replacement))
                    with patch.object(linux_ci, 'run') as execute:
                        self.assertFalse(linux_ci.restore_build_image(root, 'a'*64, inspect))
                    execute.assert_not_called()
            (root/'image.json').write_text(json.dumps(receipt))
            with patch.object(linux_ci, 'run') as execute, \
                    patch.object(linux_ci.subprocess, 'check_output', return_value=receipt['id']+'\n'):
                self.assertTrue(linux_ci.restore_build_image(root, 'a'*64, inspect))
            execute.assert_called_once_with(['docker', 'image', 'load', '--input', image], timeout=120)
            with patch.object(linux_ci, 'run'), \
                    patch.object(linux_ci.subprocess, 'check_output', return_value='sha256:'+'c'*64):
                self.assertFalse(linux_ci.restore_build_image(root, 'a'*64, inspect))
            image.write_bytes(b'corrupt')
            with patch.object(linux_ci, 'run') as execute:
                self.assertFalse(linux_ci.restore_build_image(root, 'a'*64, inspect))
            execute.assert_not_called()

    def test_linux_container_mounts_checked_workspace_receipt_for_both_restore_and_save(self):
        with tempfile.TemporaryDirectory() as temporary:
            snapshot = str(Path(temporary)/'sources/source.json')
            with patch.dict(os.environ, {'WORKSPACE_SOURCE_CACHE': snapshot}), \
                    patch.object(linux_ci, 'build_image', return_value='fixture'), \
                    patch.object(linux_ci, 'run') as execute:
                linux_ci.container(['true'])
            arguments = execute.call_args.args[0]
            self.assertIn('WORKSPACE_SOURCE_CACHE', arguments)
            directory = Path(snapshot).parent
            self.assertIn(f'{directory}:{directory}', arguments)
            self.assertTrue(directory.is_dir())

    def test_linux_download_revalidates_cached_bytes_and_uses_frozen_digest(self):
        data = b'verified upstream archive'
        record = {'url': 'https://example.invalid/archive',
                  'sha256': hashlib.sha256(data).hexdigest()}
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)/'archive'
            path.write_bytes(data)
            with patch.object(build, 'opener') as http:
                self.assertEqual(linux_ci.download(record, path), path)
                http.assert_not_called()
            path.write_bytes(b'x'*len(data))
            with patch.object(build, 'opener') as http:
                http.return_value.open.return_value = io.BytesIO(data)
                linux_ci.download(record, path)
            self.assertEqual(path.read_bytes(), data)
            path.unlink()
            with patch.object(build, 'opener') as http, \
                    patch.object(build.time, 'sleep'), patch.object(upstream_cache, 'restore', return_value=False):
                http.return_value.open.return_value = io.BytesIO(b'bad')
                http.return_value.open.side_effect = lambda *args, **kwargs: io.BytesIO(b'bad')
                with self.assertRaisesRegex(ValueError, 'SHA256 mismatch'):
                    linux_ci.download(record, path)
            self.assertFalse(path.exists())
            self.assertFalse(path.with_suffix('.partial').exists())

    def test_linux_gui_supports_gnome_336_and_newer_compositors(self):
        import linux_gui

        old = linux_gui.gnome_command('--wayland --nested', 3)
        self.assertIn('--nested', old)
        self.assertNotIn('--wayland-display=cor-wayland', old)
        self.assertNotIn('--unsafe-mode', old)
        modern = linux_gui.gnome_command('--wayland --wayland-display --headless', 46)
        self.assertIn('--wayland-display=cor-wayland', modern)
        self.assertIn('--virtual-monitor=1280x900', modern)
        self.assertIn('--unsafe-mode', modern)

    def test_dbus_probe_errors_remain_in_the_report_and_still_raise(self):
        import linux_gui
        failure = subprocess.CalledProcessError(1, ['gdbus'], stderr='service is starting')
        with tempfile.TemporaryDirectory() as temporary, \
                patch.dict(os.environ, {'COR_GUI_REPORT': temporary}), \
                patch.object(linux_gui, 'output', side_effect=failure):
            with self.assertRaises(subprocess.CalledProcessError):
                linux_gui.dbus('org.gnome.Shell', '/org/gnome/Shell', 'org.gnome.Shell.Eval', 'true')
            self.assertIn('service is starting', (Path(temporary)/'dbus-probes.log').read_text())

    def test_only_macos_tailscale_bootstrap_binaries_survive_retirement(self):
        self.assertFalse(storage.retired('action-setup-tailscale/1.104.5/macOS-amd64'))
        self.assertFalse(storage.retired('action-setup-tailscale/1.104.5/macOS-arm64'))
        self.assertTrue(storage.retired('action-setup-tailscale/1.104.5/Linux-amd64'))
        self.assertTrue(storage.retired('action-setup-tailscale/1.104.5/Windows-arm64'))
        self.assertEqual(storage.lineage('action-setup-tailscale/1.104.5/macOS-arm64'),
                         storage.lineage('action-setup-tailscale/1.104.1/macOS-arm64'))

    def test_guest_original_receipts_replace_slow_archive_exports_and_reject_bad_sources(self):
        record = {'url': 'https://ports.ubuntu.com/ubuntu-ports/pool/main/a/a.deb',
                  'sha256': 'a'*64, 'size': 123}
        for valid in (True, False):
            value = record if valid else dict(record, url='https://untrusted.invalid/a.deb')
            with self.subTest(valid=valid), tempfile.TemporaryDirectory() as temporary, \
                    patch.dict(os.environ, {'RUNNER_TEMP': temporary}), \
                    patch.object(linux_ci, 'REPORT', Path(temporary)/'report'), \
                    patch.object(linux_ci.subprocess, 'check_output',
                                 return_value=json.dumps([value]).encode()):
                linux_ci.guest_receipts(['ssh', 'isolated-guest'])
                receipts = list((Path(temporary)/'cor-upstream-downloads').glob('*.json'))
                self.assertEqual(len(receipts), int(valid))
                if valid:
                    self.assertEqual(json.loads(receipts[0].read_text()), record)

    def test_guest_python_loads_the_real_package_entry_from_the_checkout_root(self):
        calls = []
        linux_ci.guest_python(
            lambda command, **kwargs: calls.append((command, kwargs)), '',
            "import runpy; "
            "module=runpy.run_path('scripts/test-release-package.py'); "
            "assert callable(module['prepare_compatibility_environment']); "
            "assert 'git' in module['compatibility_dependencies']()",
            timeout=2100)
        command, options = calls.pop()
        self.assertEqual(options, {'timeout': 2100})
        arguments = shlex.split(command)
        self.assertEqual(arguments[0], '/opt/cor-test-python/bin/python3')
        result = subprocess.run(
            [sys.executable, '-I', *arguments[1:]], cwd=build.ROOT,
            capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_guest_debs_use_host_fallback_and_verify_bytes_before_transferring(self):
        payload = b'verified-signed-deb'
        record = {'url': 'http://ports.ubuntu.com/ubuntu-ports/pool/main/a/a.deb',
                  'sha256': hashlib.sha256(payload).hexdigest(), 'size': len(payload)}
        for scenario in ('valid', 'changed-bytes', 'bad-source', 'bad-name', 'unavailable'):
            value = dict(record, url='https://untrusted.invalid/a.deb') if scenario == 'bad-source' else record
            name = '../a.deb' if scenario == 'bad-name' else 'a.deb'
            received = []

            def download(url, destination, digest):
                self.assertTrue(url.startswith('https://ports.ubuntu.com/'))
                self.assertEqual(digest, record['sha256'])
                if scenario == 'unavailable':
                    raise OSError('Both official and private downloads failed')
                destination.write_bytes(payload if scenario != 'changed-bytes' else b'changed')

            def remote(command, **kwargs):
                if stream := kwargs.get('stdin'):
                    with tarfile.open(fileobj=io.BytesIO(stream.read())) as archive:
                        received.append(archive.extractfile('a.deb').read())

            with self.subTest(scenario=scenario), tempfile.TemporaryDirectory() as temporary, \
                    patch.object(linux_ci.subprocess, 'check_output',
                                 return_value=json.dumps({name: value}).encode()), \
                    patch.object(linux_ci, 'download_original', side_effect=download):
                if scenario == 'valid':
                    linux_ci.guest_deb_downloads(['ssh'], remote, 'environment ', Path(temporary)/'debs')
                    self.assertEqual(received, [payload])
                else:
                    expected = OSError if scenario == 'unavailable' else ValueError
                    with self.assertRaises(expected):
                        linux_ci.guest_deb_downloads(['ssh'], remote, 'environment ', Path(temporary)/'debs')
                    self.assertFalse(received)

    def test_vm_prefetches_package_dependencies_without_preinstalling_them(self):
        package_font = 'fonts-wqy-microhei'
        self.assertIn(package_font, release_package.compatibility_download_dependencies())
        self.assertNotIn(package_font, release_package.compatibility_dependencies())

    def test_basic_hosted_network_checks_do_not_install_an_unused_desktop_session(self):
        for extended, guest in (('false', ''), ('true', ''), ('false', 'true')):
            with self.subTest(extended=extended, guest=guest), patch.dict(os.environ, {
                    'COR_CI_EXTENDED_TESTS': extended, 'COR_LINUX_GUEST': guest}):
                packages = release_package.compatibility_dependencies()
                self.assertTrue({'git', 'dbus-x11', 'iproute2', 'util-linux', 'libcap2-bin'}
                                .issubset(packages))
                if extended == 'true' or guest:
                    self.assertEqual(packages, list(prepared_vm_environment.TEST_PACKAGES))
                else:
                    self.assertTrue({'gnome-shell', 'openbox', 'xwayland', 'policykit-1-gnome'}
                                    .isdisjoint(packages))
                    self.assertIn('gsettings-desktop-schemas',
                                  release_package.compatibility_download_dependencies())

    @unittest.skipUnless(hasattr(socket, 'AF_UNIX'), 'Unix sockets are unavailable')
    def test_linux_gui_finds_only_wayland_sockets(self):
        import linux_gui

        with tempfile.TemporaryDirectory() as temporary:
            runtime = Path(temporary)
            (runtime/'wayland-0.lock').write_text('')
            with socket.socket(socket.AF_UNIX) as default, socket.socket(socket.AF_UNIX) as named:
                default.bind(str(runtime/'wayland-0'))
                named.bind(str(runtime/'cor-wayland'))
                self.assertEqual(linux_gui.wayland_sockets(runtime), {'wayland-0', 'cor-wayland'})

    def test_linux_gui_waits_for_configured_window_size(self):
        import linux_gui

        small = [{'pid': 12, 'width': 0, 'height': 0}]
        usable = [{'pid': 12, 'width': 950, 'height': 700}]
        probe = MagicMock(side_effect=[[], small, usable])
        with patch.object(linux_gui.time, 'sleep'):
            self.assertEqual(linux_gui.wait_for_usable_window('wayland', probe), usable)
        self.assertEqual(probe.call_count, 3)

    def test_linux_gui_rejects_a_permanently_small_window_with_diagnostics(self):
        import linux_gui

        small = [{'pid': 12, 'width': 640, 'height': 300}]
        with (patch.object(linux_gui.time, 'sleep'),
              patch.object(linux_gui.time, 'monotonic', side_effect=[0, 0, 1])):
            with self.assertRaisesRegex(RuntimeError, r'wayland.*800x450.*640.*300'):
                linux_gui.wait_for_usable_window('wayland', lambda: small, timeout=1)

    def test_linux_gui_matches_native_wayland_windows_on_old_and_new_gnome(self):
        import linux_gui

        legacy = {'pid': -1, 'sequence': 1, 'wayland': True}
        legacy_zero = {'pid': 0, 'sequence': 5, 'wayland': True}
        modern = {'pid': 42, 'sequence': 2, 'wayland': True}
        foreign = {'pid': 99, 'sequence': 3, 'wayland': True}
        xwayland = {'pid': -1, 'sequence': 4, 'wayland': False}
        self.assertEqual(linux_gui.select_wayland_windows([legacy], [42]), [legacy])
        self.assertEqual(linux_gui.select_wayland_windows([legacy_zero], [42]), [legacy_zero])
        self.assertEqual(linux_gui.select_wayland_windows([modern, foreign], [42]), [modern])
        self.assertEqual(linux_gui.select_wayland_windows([foreign], [42]), [])
        self.assertEqual(linux_gui.select_wayland_windows([xwayland], [42]), [])
        self.assertEqual(linux_gui.select_wayland_windows([legacy], []), [])
        self.assertEqual(linux_gui.select_wayland_windows([legacy], [42, 43]), [])
        self.assertEqual(linux_gui.select_wayland_windows([legacy, xwayland], [42]), [])

    def test_linux_gui_retries_only_missing_authorization_agent(self):
        import linux_gui

        request = MagicMock()
        agent = MagicMock()
        agent.poll.return_value = None
        request.poll.return_value = 1
        self.assertEqual(
            linux_gui.authorization_dialog(request, agent, io.StringIO(
                'Error executing command as another user: No authentication agent found.\n')),
            ('retry', None))
        with self.assertRaisesRegex(RuntimeError, 'denied'):
            linux_gui.authorization_dialog(request, agent, io.StringIO('authorization denied'))
        agent.poll.return_value = 1
        with self.assertRaisesRegex(RuntimeError, 'agent exited'):
            linux_gui.authorization_dialog(request, agent, io.StringIO(''))

    def test_linux_gui_finds_authorized_core_without_reading_exe_link(self):
        import linux_gui

        with tempfile.TemporaryDirectory() as temporary:
            proc = Path(temporary)
            for pid, name, uid, parent in (
                    (101, 'mihomo', 1001, 42),
                    (102, 'mihomo', 1001, 99),
                    (103, 'mihomo', 1002, 42),
                    (104, 'unrelated', 1001, 42)):
                directory = proc/str(pid)
                directory.mkdir()
                (directory/'status').write_text(
                    f'Name:\t{name}\nPPid:\t{parent}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n')
            (proc/'105').mkdir()
            (proc/'105'/'status').write_text('Name:\tmihomo\n')

            # The two matching processes have no readable exe link or useful
            # directory ownership; both must be counted for duplicate checks.
            self.assertEqual(linux_gui.core_processes(proc, 1001), {101: 42, 102: 99})

    def test_linux_gui_session_is_staged_outside_a_private_workspace(self):
        import linux_gui

        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            directory.chmod(0o755)
            staged = linux_gui.stage_session(directory)
            self.assertEqual(staged.stat().st_mode & 0o777, 0o644)
            self.assertEqual(staged.parent.stat().st_mode & 0o777, 0o755)
            resolved_root = subprocess.check_output(
                [sys.executable, '-B', '-c', 'import build_support; print(build_support.ROOT)'],
                cwd=staged.parent, text=True).strip()
            self.assertEqual(Path(resolved_root), directory)

            report = directory/'report'
            report.mkdir()
            (report/'gui.json').write_text('{"status":"failed"}')
            (report/'desktop.log').write_text('diagnostic')
            (report/'local.yaml').write_text('private fixture')
            (report/'x11.log').symlink_to(report/'local.yaml')
            with patch.object(linux_gui, 'REPORT', directory/'saved'):
                linux_gui.preserve_report(report)
            saved = directory/'saved/desktop'
            self.assertEqual((saved/'gui.json').read_text(), '{"status":"failed"}')
            self.assertEqual((saved/'desktop.log').read_text(), 'diagnostic')
            self.assertFalse((saved/'local.yaml').exists())
            self.assertFalse((saved/'x11.log').exists())

    def test_linux_gui_session_keeps_pam_identity_but_not_runner_credentials(self):
        import linux_gui

        environment = {'XDG_SESSION_ID': 'c12', 'XDG_RUNTIME_DIR': '/run/user/1234',
                       'HOME': '/home/runner', 'XDG_CONFIG_HOME': '/home/runner/.config',
                       'COR_GUI_REPORT': '/tmp/disposable/report',
                       'COR_GUI_AUTH_PASSWORD_FILE': '/tmp/disposable/password',
                       'GITHUB_ACTIONS': 'true', 'GITHUB_TOKEN': 'secret',
                       'AWS_SECRET_ACCESS_KEY': 'secret'}
        with patch.dict(os.environ, environment, clear=True), patch.object(os, 'execve') as launch:
            linux_gui.launch_session()
        executable, command, retained = launch.call_args.args
        self.assertEqual(executable, '/usr/bin/dbus-run-session')
        self.assertIn('--session', command)
        self.assertEqual(retained['XDG_SESSION_ID'], 'c12')
        self.assertEqual(retained['XDG_RUNTIME_DIR'], '/run/user/1234')
        self.assertEqual(retained['HOME'], str(Path('/tmp/disposable/home')))
        self.assertEqual(retained['XDG_CONFIG_HOME'], str(Path('/tmp/disposable/home/.config')))
        self.assertEqual(retained['COR_GUI_AUTH_PASSWORD_FILE'], '/tmp/disposable/password')
        self.assertNotIn('GITHUB_TOKEN', retained)
        self.assertNotIn('AWS_SECRET_ACCESS_KEY', retained)

    def test_memory_mapping_categories_do_not_keep_addresses_or_paths(self):
        text = """1000-2000 rw-p 0 00:00 0
Rss: 8 kB
Pss: 6 kB
Private_Clean: 1 kB
Private_Dirty: 3 kB
2000-3000 rw-p 0 00:00 0 [heap]
Rss: 4 kB
Pss: 4 kB
Private_Dirty: 4 kB
3000-4000 r-xp 0 08:01 1 /private/path/program
Rss: 12 kB
Pss: 9 kB
Private_Clean: 2 kB
"""
        maps = memory_benchmark.memory_maps(text)
        self.assertEqual(maps['anonymous'], {'rss_bytes':8192, 'pss_bytes':6144, 'private_bytes':4096})
        self.assertEqual(maps['heap']['private_bytes'], 4096)
        self.assertEqual(maps['file-backed']['rss_bytes'], 12288)
        self.assertNotIn('/private/path', json.dumps(maps))
        classified = memory_benchmark.memory_maps(text, '/private/path/program')
        self.assertEqual(classified['executable']['rss_bytes'],12288)
        self.assertNotIn('/private/path',json.dumps(classified))

    def test_mapping_peaks_preserve_missing_samples_and_use_simultaneous_private_totals(self):
        snapshots = [
            {'roles': {'gui': {'heap': {'private_bytes': 100},
                               'anonymous': {'private_bytes': 10}}}, 'unavailable_processes': 0},
            {'roles': {}, 'unavailable_processes': 2},
            {'roles': {'gui': {'heap': {'private_bytes': 20},
                               'anonymous': {'private_bytes': 70}}}, 'unavailable_processes': 0},
        ]
        self.assertEqual(memory_benchmark.mapping_peaks(snapshots)['gui']['heap']['private_bytes'], 100)
        self.assertEqual(memory_benchmark.mapping_peaks(snapshots)['gui']['anonymous']['private_bytes'], 70)
        self.assertEqual(memory_benchmark.mapping_private_peaks(snapshots), {'gui': 110})
        self.assertEqual(memory_benchmark.mapping_private_peaks([snapshots[1]]), {})

    def test_mapping_snapshot_matches_epoch_identity_and_rejects_reused_processes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root/'stat').write_text('btime 1700000000\n')
            process = root/'42'
            process.mkdir()
            fields = ['S'] + ['0']*18 + ['12345']
            stat = '42 (name with ) parentheses) '+' '.join(fields)
            (process/'stat').write_text(stat)
            (process/'smaps').write_text('1000-2000 rw-p 0 00:00 0\nRss: 8 kB\nPrivate_Dirty: 4 kB\n')
            (process/'exe').symlink_to('/private/example/program')
            observed = {'pid': 42, 'role': 'gui', 'start_time': 1700000123}
            with patch.object(memory_benchmark.os, 'sysconf', return_value=100):
                result = memory_benchmark.mapping_snapshot([observed], root)
                self.assertEqual(result['roles']['gui']['anonymous']['private_bytes'], 4096)
                self.assertEqual(result['unavailable_processes'], 0)
                self.assertEqual(result['unavailable_reasons'], {})
                self.assertNotIn('/private', json.dumps(result))
                mismatch = dict(observed, start_time=12345)
                result = memory_benchmark.mapping_snapshot([mismatch], root)
                self.assertEqual(result['roles'], {})
                self.assertEqual(result['unavailable_reasons'], {'process_identity_changed': 1})
                original_read = Path.read_text
                def replaced(path, *args, **kwargs):
                    content = original_read(path, *args, **kwargs)
                    if path == process/'smaps':
                        (process/'stat').write_text(stat.replace('12345', '12346'))
                    return content
                with patch.object(Path, 'read_text', replaced):
                    result = memory_benchmark.mapping_snapshot([observed], root)
                self.assertEqual(result['roles'], {})
                self.assertEqual(result['unavailable_reasons'], {'process_identity_changed': 1})

    def test_mapping_snapshot_records_missing_denied_and_invalid_observations(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            observed = {'pid': 42, 'role': 'gui', 'start_time': 1700000123}
            result = memory_benchmark.mapping_snapshot([observed], root)
            self.assertEqual(result['unavailable_reasons'], {'clock_unavailable': 1})
            (root/'stat').write_text('btime 1700000000\n')
            with patch.object(memory_benchmark.os, 'sysconf', return_value=100):
                result = memory_benchmark.mapping_snapshot([observed], root)
                self.assertEqual(result['unavailable_reasons'], {'process_gone': 1})
                process = root/'42'
                process.mkdir()
                (process/'stat').write_text('invalid')
                result = memory_benchmark.mapping_snapshot([observed], root)
                self.assertEqual(result['unavailable_reasons'], {'invalid_proc_data': 1})
                original_read = Path.read_text
                def denied(path, *args, **kwargs):
                    if path == process/'stat':
                        raise PermissionError('/private/proc/path')
                    return original_read(path, *args, **kwargs)
                with patch.object(Path, 'read_text', denied):
                    result = memory_benchmark.mapping_snapshot([observed], root)
                self.assertEqual(result['roles'], {})
                self.assertEqual(result['unavailable_processes'], 1)
                self.assertEqual(result['unavailable_reasons'], {'permission_denied': 1})
                self.assertNotIn('/private', json.dumps(result))

    def test_renderer_bundle_is_an_agent_before_launch_and_signed_inside_out(self):
        import macos_package as mac
        import plistlib
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload = root/'payload'
            (payload/'resources').mkdir(parents=True)
            (payload/'resources/core.json').write_text('{}')
            for name in ('clash-of-rust','clash-desktop-ui','clash-tun-launcher','cor','LICENSE','resources/mihomo'):
                (payload/name).write_bytes(name.encode())
            commands=[]
            def tool(command, **kwargs):
                commands.append([str(v) for v in command])
                if command[0] == '/usr/bin/iconutil':
                    Path(command[-1]).write_bytes(b'icon')
            with patch.object(mac,'validate_binary_arch'),patch.object(mac,'run',side_effect=tool),patch.dict(os.environ,{'MACOS_SIGNING_IDENTITY':'-'}):
                app = mac.build_app(payload,root/mac.APP_NAME,'0.6.3-dev.1','arm64')
            helper=app/mac.UI_APP
            info=plistlib.loads((helper/'Contents/Info.plist').read_bytes())
            owner=plistlib.loads((app/'Contents/Info.plist').read_bytes())
            self.assertIs(info['LSUIElement'],True)
            self.assertNotIn('LSUIElement',owner)
            self.assertNotEqual(info['CFBundleIdentifier'],owner['CFBundleIdentifier'])
            self.assertEqual((helper/'Contents/MacOS'/info['CFBundleExecutable']).read_bytes(),b'clash-desktop-ui')
            self.assertFalse((app/'Contents/MacOS/clash-desktop-ui').exists())
            signed=[command[-1] for command in commands if command[:2]==['/usr/bin/codesign','--force']]
            self.assertLess(signed.index(str(helper/'Contents/MacOS/clash-desktop-ui')),signed.index(str(helper)))
            self.assertLess(signed.index(str(helper)),signed.index(str(app)))

    def test_memory_scene_rejects_an_early_exit_missing_core_or_gui_in_headless_sample(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)/'headless.jsonl'

            def sample(roles, elapsed=9750):
                path.write_text(json.dumps({'elapsed_ms': elapsed,
                    'processes': [{'role': role} for role in roles]})+'\n')

            for roles in [('daemon',), ('gui', 'core'), ('daemon', 'gui', 'core')]:
                sample(roles)
                with self.assertRaisesRegex(ValueError, 'identity'):
                    memory_benchmark.read_scene(path, seconds=10, role='daemon')
            sample(('daemon', 'core'), elapsed=100)
            with self.assertRaisesRegex(ValueError, 'exited'):
                memory_benchmark.read_scene(path, seconds=10, role='daemon')
            path.write_text('{"type":"metadata"}\n')
            with self.assertRaisesRegex(ValueError, 'No process'):
                memory_benchmark.read_scene(path, seconds=10, role='daemon')
            sample(('daemon', 'core'))
            self.assertEqual(len(memory_benchmark.read_scene(path, seconds=10, role='daemon')), 1)

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        override = patch.dict(os.environ, {
            'BUILD_INPUTS_DIR': directory.name,
            'RUNNER_TEMP': directory.name,
            'COR_UPSTREAM_CACHE_CONFIG': '',
            **{name: str(Path(directory.name) / name.lower()) for name in (
                'GITHUB_ENV', 'GITHUB_OUTPUT', 'GITHUB_STATE', 'GITHUB_STEP_SUMMARY')},
        })
        override.start()
        self.addCleanup(override.stop)

    def test_local_rust_cache_invalidates_dependencies_compiler_and_build_flags(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root/'Cargo.lock').write_text('slint = 1.18.1')
            (root/'Cargo.toml').write_text('[workspace]')
            (root/'crates/ui').mkdir(parents=True)
            ui = root/'crates/ui/view.slint'
            ui.write_text('old UI')
            key, prefix = local_cache.rust_keys(root, b'compiler one', {})
            ui.write_text('new UI')
            changed, compatible = local_cache.rust_keys(root, b'compiler one', {})
            self.assertNotEqual(key, changed)
            self.assertEqual(prefix, compatible)
            icon = root/'crates/ui/icon.svg'
            icon.write_text('<svg>old</svg>')
            before, generation = local_cache.rust_keys(root, b'compiler one', {})
            icon.write_text('<svg>new</svg>')
            after, compatible = local_cache.rust_keys(root, b'compiler one', {})
            self.assertNotEqual(before, after)
            self.assertEqual(generation, compatible)
            (root/'.github/workflows').mkdir(parents=True)
            workflow = root/'.github/workflows/ci.yml'
            workflow.write_text('ndk: 28')
            before = local_cache.rust_keys(root, b'compiler one', {})[0]
            workflow.write_text('ndk: 29')
            self.assertNotEqual(before, local_cache.rust_keys(root, b'compiler one', {})[0])
            vendor = root/'vendor/backend'
            (vendor/'java').mkdir(parents=True)
            (vendor/'Cargo.toml').write_text('[package]\nname="backend"\nversion="1.0.0"\n')
            java = vendor/'java/Helper.java'
            java.write_text('old helper')
            before, generation = local_cache.rust_keys(root, b'compiler one', {})
            java.write_text('new helper')
            after, compatible = local_cache.rust_keys(root, b'compiler one', {})
            self.assertNotEqual(before, after)
            self.assertEqual(generation, compatible)
            (vendor/'Cargo.toml').write_text('[package]\nname="backend"\nversion="1.0.0"\n[features]\nnew=[]\n')
            self.assertNotEqual(generation, local_cache.rust_keys(root, b'compiler one', {})[1])
            (root/'Cargo.lock').write_text('slint = 1.19.0')
            new_prefix = local_cache.rust_keys(root, b'compiler one', {})[1]
            self.assertNotEqual(prefix, new_prefix)
            # The compiler-compatible fallback may retain unrelated libraries;
            # Cargo still resolves/builds against the new lockfile.
            self.assertTrue(prefix.startswith(local_cache.rust_environment_key(b'compiler one', {})))
            self.assertTrue(new_prefix.startswith(local_cache.rust_environment_key(b'compiler one', {})))
            self.assertNotEqual(new_prefix, local_cache.rust_keys(root, b'compiler two', {})[1])
            self.assertNotEqual(new_prefix, local_cache.rust_keys(root, b'compiler one', {'RUSTFLAGS':'-C target-cpu=native'})[1])

    def test_local_cache_archives_preserve_data_and_reject_escaping_links(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root/'source'; source.mkdir()
            (source/'module').write_bytes(b'valid cached dependency')
            archive = root/'cache.tar.gz'
            local_cache.pack([source], archive)
            restored = root/'restored'
            local_cache.unpack(archive, [restored])
            self.assertEqual((restored/'module').read_bytes(), b'valid cached dependency')
            escape = 'escape-' + root.name
            for name, kind in [('0/../../'+escape, tarfile.REGTYPE), ('0/link', tarfile.SYMTYPE)]:
                with tarfile.open(archive, 'w:gz') as output:
                    info=tarfile.TarInfo(name); info.type=kind; info.linkname='../../'+escape
                    output.addfile(info)
                with self.assertRaises(ValueError): local_cache.unpack(archive, [restored])
            self.assertFalse((root.parent/escape).exists())

    def test_rust_cache_version_bump_preserves_dependencies_but_feature_changes_do_not(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            def version(value, feature='old'):
                (root/'Cargo.toml').write_text('[workspace.package]\nversion="'+value+'"\n'
                    '[workspace.dependencies]\nslint={version="1",features=["'+feature+'"]}\n')
                (root/'Cargo.lock').write_text('[[package]]\nname="clash"\nversion="'+value+'"\n'
                    '[[package]]\nname="slint"\nversion="1.0.0"\nsource="registry+test"\n')
            version('0.6.0')
            first=local_cache.rust_keys(root,b'compiler',{})[1]
            version('0.6.1')
            self.assertEqual(first,local_cache.rust_keys(root,b'compiler',{})[1])
            version('0.6.1','new')
            self.assertNotEqual(first,local_cache.rust_keys(root,b'compiler',{})[1])

    def test_local_vendor_artifacts_survive_cache_cleanup_without_saving_other_workspace_objects(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target, cache = root/'target', root/'external'
            packages = {'vendor/renderer': 'renderer-patch', 'crates/core': 'clash-core'}
            objects = {
                'release/deps/librenderer_patch-first.rlib': b'compiled library',
                'release/deps/renderer_patch-first.d': b'dependency paths',
                'release/.fingerprint/renderer-patch-first/lib-renderer_patch.json': b'fingerprint',
                'aarch64-linux-android/release/build/renderer-patch/second/output': b'generated bindings',
            }
            for relative, data in {**objects, 'release/deps/libclash_core-first.rlib': b'application'}.items():
                path = target/relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
            local_cache.workspace_vendor_objects(root, cache, packages, save=True)
            shutil.rmtree(target)
            local_cache.workspace_vendor_objects(root, cache, packages)
            for relative, data in objects.items():
                self.assertEqual((target/relative).read_bytes(), data)
            self.assertFalse((target/'release/deps/libclash_core-first.rlib').exists())

    def test_incomplete_or_escaping_vendor_receipts_restore_no_partial_compiler_objects(self):
        for corruption in ('changed', 'missing', 'escape'):
            with self.subTest(corruption=corruption), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                target, cache = root/'target', root/'external'
                packages = {'vendor/renderer': 'renderer-patch'}
                for name in ('first', 'second'):
                    path = target/f'release/deps/librenderer_patch-{name}.rlib'
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(name.encode())
                local_cache.workspace_vendor_objects(root, cache, packages, save=True)
                shutil.rmtree(target)
                path = cache/'vendor-objects/release/deps/librenderer_patch-second.rlib'
                if corruption == 'changed':
                    path.write_bytes(b'wrong')
                elif corruption == 'missing':
                    path.unlink()
                else:
                    receipt = cache/'vendor-objects.json'
                    value = json.loads(receipt.read_text())
                    value['files']['../../outside'] = {'size': 1, 'sha256': 'a'*64}
                    receipt.write_text(json.dumps(value))
                local_cache.workspace_vendor_objects(root, cache, packages)
                self.assertFalse(target.exists())
                self.assertFalse((root.parent/'outside').exists())

    def test_changed_vendor_source_invalidates_restored_fingerprints_before_cargo_runs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            vendor = root/'vendor/renderer'
            vendor.mkdir(parents=True)
            (vendor/'Cargo.toml').write_text('[package]\nname="renderer-patch"\nversion="1.0.0"\n')
            code = vendor/'lib.rs'
            code.write_text('old source')
            fingerprint = root/'target/release/.fingerprint/renderer-patch-first/lib-renderer_patch.json'
            fingerprint.parent.mkdir(parents=True)
            fingerprint.write_text('old fingerprint')
            tracked = b'vendor/renderer/Cargo.toml\0vendor/renderer/lib.rs\0'
            with patch.object(local_cache.subprocess, 'check_output', return_value=tracked), \
                    patch.dict(os.environ, {'WORKSPACE_SOURCE_CACHE': str(root/'external/source.json')}):
                local_cache.workspace_sources(root, save=True)
                shutil.rmtree(root/'target')
                code.write_text('changed source')
                local_cache.workspace_sources(root)
            self.assertFalse(fingerprint.exists())

    def test_malformed_workspace_receipt_preserves_sources_and_existing_compiler_objects(self):
        for value in ([], None, 'invalid', 1):
            with self.subTest(value=value), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                source = root/'crates/core/lib.rs'
                source.parent.mkdir(parents=True)
                source.write_bytes(b'current source')
                stamp = source.stat().st_mtime_ns
                artifact = root/'target/release/deps/libcore-current.rlib'
                artifact.parent.mkdir(parents=True)
                artifact.write_bytes(b'existing compiler object')
                receipt = root/'external/source.json'
                receipt.parent.mkdir()
                receipt.write_text(json.dumps(value))
                with patch.object(local_cache.subprocess, 'check_output', return_value=b'crates/core/lib.rs\0'), \
                        patch.dict(os.environ, {'WORKSPACE_SOURCE_CACHE': str(receipt)}):
                    local_cache.workspace_sources(root)
                self.assertEqual(source.read_bytes(), b'current source')
                self.assertEqual(source.stat().st_mtime_ns, stamp)
                self.assertEqual(artifact.read_bytes(), b'existing compiler object')

    def test_workspace_cache_verifies_contents_before_restoring_times_and_invalidates_changed_crates(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            for name in ('ui','core'):
                folder=root/'crates'/name;folder.mkdir(parents=True)
                (folder/'Cargo.toml').write_text('[package]\nname="clash-'+name+'"\nversion="1.0.0"\n')
                (folder/'lib.rs').write_bytes(b'original source')
            java=root/'apps/android/app/MainActivity.java';java.parent.mkdir(parents=True)
            java.write_bytes(b'original Java host')
            vendor=root/'vendor/slint-android';vendor.mkdir(parents=True)
            (vendor/'Cargo.toml').write_text('[package]\nname="slint-android"\nversion="1.0.0"\n')
            backend=vendor/'lib.rs';backend.write_bytes(b'original backend')
            paths=sorted(p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file())
            tracked='\0'.join(paths).encode()
            ui=root/'crates/ui/lib.rs';core=root/'crates/core/lib.rs'
            previous=core.stat().st_mtime_ns
            with patch.object(local_cache.subprocess,'check_output',return_value=tracked), \
                    patch.dict(os.environ,{'WORKSPACE_SOURCE_CACHE':str(root/'external/source.json')}):
                local_cache.workspace_sources(root,save=True)
                for name in ('ui','core'):
                    target=root/'target/release/.fingerprint'/('clash-'+name+'-hash')
                    target.mkdir(parents=True);(target/'lib-info').write_bytes(b'fingerprint')
                backend_fingerprint=root/'target/release/.fingerprint/slint-android-hash'
                backend_fingerprint.mkdir(parents=True)
                v2=root/'target/release/.fingerprint/clash-ui/another-hash'
                v2.mkdir(parents=True);(v2/'lib-info').write_bytes(b'fingerprint')
                # Checkout made unchanged sources newer. Conversely, changed
                # contents can belong to an older branch timestamp.
                os.utime(core,ns=(previous+1_000_000,previous+1_000_000))
                ui.write_bytes(b'changed source');os.utime(ui,ns=(1,1))
                java.write_bytes(b'changed Android host; unrelated to desktop Rust libraries')
                backend.write_bytes(b'changed backend');os.utime(backend,ns=(1,1))
                local_cache.workspace_sources(root)
            self.assertEqual(core.stat().st_mtime_ns,previous)
            self.assertEqual(ui.read_bytes(),b'changed source')
            self.assertFalse((root/'target/release/.fingerprint/clash-ui-hash').exists())
            self.assertFalse(v2.exists())
            self.assertFalse(backend_fingerprint.exists())
            self.assertTrue((root/'target/release/.fingerprint/clash-core-hash/lib-info').exists())

    def test_workspace_cache_invalidates_shared_slint_consumers_outside_the_ui_crate(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);ui=root/'crates/ui';mobile=root/'apps/android/native'
            ui.mkdir(parents=True);mobile.mkdir(parents=True)
            (ui/'Cargo.toml').write_text('[package]\nname="clash-ui"\nversion="1.0.0"\n')
            (mobile/'Cargo.toml').write_text('[package]\nname="clash-android"\nversion="1.0.0"\n')
            view=ui/'mobile.slint';view.write_bytes(b'old component')
            paths='\0'.join(p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file()).encode()
            with patch.object(local_cache.subprocess,'check_output',return_value=paths), \
                    patch.dict(os.environ,{'WORKSPACE_SOURCE_CACHE':str(root/'external/source.json')}):
                local_cache.workspace_sources(root,save=True)
                profile=root/'target/aarch64-linux-android/release'
                output=profile/'build/clash-android-hash/output';output.parent.mkdir(parents=True)
                output.write_text('cargo:rerun-if-changed='+str(view)+'\n')
                fingerprint=profile/'.fingerprint/clash-android-hash';fingerprint.mkdir(parents=True)
                (fingerprint/'lib-info').write_bytes(b'cached fingerprint')
                view.write_bytes(b'new component');os.utime(view,ns=(1,1))
                local_cache.workspace_sources(root)
                self.assertFalse(fingerprint.exists())

    def test_rust_shards_only_upload_changed_buckets_and_verify_before_extracting(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            source=root/'target/release/deps';source.mkdir(parents=True)
            for number in range(80): (source/f'dep{number}.rlib').write_bytes(f'dependency-{number}'.encode())
            config={'group':'rust/linux-x64','key':'generation-source-one','prefix':['generation-','compiler-'],
                    'resolved_paths':[str(root/'target')],'kind':'rust'}
            store=local_cache.Store(root/'server',budget=10*1024**2,min_free=0)
            uploads=[]
            def request(_config,path,method='GET',file=None,headers=None):
                url=urllib.parse.urlsplit(path);query=urllib.parse.parse_qs(url.query)
                if url.path == '/health':
                    reply = io.BytesIO(json.dumps({
                        'schema': local_cache.SCHEMA, 'status': 'ready',
                        'capabilities': ['validated-cache-commit-v1', 'rust-index-references-v1',
                                         'complete-cache-retention-v1'],
                    }).encode())
                    reply.status = 200
                    return nullcontext(reply)
                group=query['group'][0]
                if url.path=='/v1/lookup':
                    value=store.lookup(group,query['key'][0],query.get('prefix',[''])[0].splitlines())
                    reply=io.BytesIO(json.dumps(value).encode());reply.status=200 if value else 404
                elif method=='PUT':
                    payload=file.read();uploads.append((group,len(payload)))
                    saved=store.put(group,query['key'][0],len(payload),headers['X-Content-SHA256'],io.BytesIO(payload),
                                    int(headers['X-Cache-Sequence']))
                    reply=io.BytesIO();reply.status=201 if saved else 409
                else:
                    blob=store.path(group,query['id'][0])
                    reply=io.BytesIO(blob.read_bytes() if blob.exists() else b'');reply.status=200 if blob.exists() else 404
                return nullcontext(reply)
            with patch.object(local_cache,'response',side_effect=request),patch.object(local_cache,'cache_report'), \
                    patch.dict(os.environ,{'GITHUB_RUN_ID':'1','GITHUB_RUN_ATTEMPT':'1'}):
                cache_shards.save(config,[root/'target'])
                self.assertGreater(len(uploads),2)
                initial_bytes=sum(size for _,size in uploads)
                target=root/'restored'
                restored={**config,'resolved_paths':[str(target)]}
                self.assertTrue(cache_shards.restore(restored,False))
                self.assertEqual((target/'release/deps/dep5.rlib').read_bytes(),b'dependency-5')
                # Updating one dependency sends one bucket and the small index,
                # rather than another complete target archive.
                (source/'dep5.rlib').write_bytes(b'updated-dependency')
                uploads.clear()
                config.update(key='generation-source-two',shard_index=restored['shard_index'])
                with patch.dict(os.environ,{'GITHUB_RUN_ID':'2'}): cache_shards.save(config,[root/'target'])
                self.assertEqual(len(uploads),2)
                self.assertLess(sum(size for _,size in uploads),initial_bytes)
                restored.update(key=config['key'])
                self.assertTrue(cache_shards.restore(restored,False))
                self.assertEqual((target/'release/deps/dep5.rlib').read_bytes(),b'updated-dependency')
                # LRU eviction can remove any shard while the index and a
                # build-script success marker still exist. Cargo does not
                # validate arbitrary generated OUT_DIR files on its own.
                marker=target/'release/.fingerprint/serde_core-hash/run-build-script-build-script-build'
                marker.parent.mkdir(parents=True)
                marker.write_bytes(b'cached success')
                shard=next(s for s in restored['shard_index']['shards'] if s)
                blob=store.path(shard['group'],shard['id'])
                blob.unlink()
                self.assertTrue(cache_shards.restore(restored,False))
                self.assertFalse(restored['hit'])
                self.assertTrue(restored['mirror_required'])
                self.assertFalse(marker.parent.exists())
                github_config = {**restored, 'hit': True, 'restored': True}
                with patch.object(local_cache, 'unpack') as unpack:
                    self.assertTrue(cache_shards.restore(github_config, True))
                    unpack.assert_not_called()
                self.assertTrue(github_config['mirror_required'])
                self.assertTrue(github_config['hit'])
                uploads.clear()
                github_config.update(save=True, source=str(root), mirror_required=False)
                with patch.object(local_cache, 'cache_paths', return_value=[root/'target']), \
                        patch.dict(os.environ, {'GITHUB_RUN_ID': '3'}):
                    local_cache.save(github_config)
                self.assertCountEqual([group for group, _ in uploads],
                                      [shard['group'], config['group'] + '/index'])
                self.assertTrue(cache_shards.restore(github_config, True))
                self.assertFalse(github_config['mirror_required'])
                self.assertTrue(cache_shards.index_available(github_config, github_config['shard_index']))
                shard=next(s for s in restored['shard_index']['shards'] if s)
                store.path(shard['group'],shard['id']).write_bytes(b'corrupt')
                with self.assertRaisesRegex(ValueError,'Incomplete|checksum'):
                    cache_shards.restore(restored,False)

    def test_shard_save_uses_the_configured_budget_and_keeps_the_index_when_incomplete(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary)/'dependency.rlib'
            source.write_bytes(b'dependency')
            info = tarfile.TarInfo('0/release/deps/dependency.rlib')
            info.size = source.stat().st_size
            groups = [[(source, info, local_cache.digest(source))] for _ in range(3)]
            groups.extend([[] for _ in range(cache_shards.COUNT - len(groups))])
            for budget, committed in ((900, True), (120, False)):
                clock = [100]
                def lookup(config, group, key):
                    clock[0] += 250
                    return {'group': group, 'key': key}
                with self.subTest(budget=budget), \
                        patch.object(cache_shards.time, 'monotonic', side_effect=lambda: clock[0]), \
                        patch.object(local_cache, 'phase', return_value=nullcontext()), \
                        patch.object(local_cache, 'uploads_supported', return_value=True), \
                        patch.object(cache_shards, 'partition', return_value=groups), \
                        patch.object(cache_shards, 'lookup', side_effect=lookup), \
                        patch.object(cache_shards, 'upload', return_value={'saved': True}) as upload, \
                        patch.object(local_cache, 'cache_report') as report:
                    cache_shards.save({'group': 'rust/linux-x64', 'key': 'current',
                                       'prefix': ['compiler-'], 'save_seconds': budget}, [source.parent])
                    self.assertEqual(upload.call_count, int(committed))
                    if committed:
                        self.assertEqual(upload.call_args.args[1], 'rust/linux-x64/index')
                    self.assertIn('committed' if committed else 'previous index retained',
                                  report.call_args.args[2])

    def test_restore_budget_keeps_verified_shards_and_invalidates_partial_build_scripts(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);source=root/'source';target=root/'target'
            library=source/'release/deps/verified.rlib'
            library.parent.mkdir(parents=True);library.write_bytes(b'complete dependency')
            archive=root/'fixture.tar.gz';local_cache.pack([source],archive)
            marker=target/'release/.fingerprint/generated-hash/run-build-script-cached'
            marker.parent.mkdir(parents=True);marker.write_bytes(b'old success')
            group='rust/macos-x64'
            shards=[{'schema':local_cache.SCHEMA,'group':f'{group}/p{i:02d}',
                     'key':'generation','id':f'{i+1:064x}','sha256':'b'*64,'size':archive.stat().st_size}
                    if i<2 else None for i in range(cache_shards.COUNT)]
            record={'key':'generation','size':1000}
            config={'group':group,'key':'generation','prefix':['generation'],
                    'resolved_paths':[str(target)],'kind':'rust'}
            def download(_config,item,destination):
                if item is record:
                    destination.write_text(json.dumps({'format':'rust-shards-v1','shards':shards}));return
                if item['group'].endswith('p00'):
                    shutil.copyfile(archive,destination);return
                self.assertEqual((target/'release/deps/verified.rlib').read_bytes(),b'complete dependency')
                config['restore_deadline']=local_cache.time.monotonic()-1
                raise local_cache.RestoreBudgetExceeded('test deadline')
            with patch.object(cache_shards,'lookup',return_value=record), \
                    patch.object(cache_shards,'download',side_effect=download), \
                    patch.object(local_cache,'cache_report'):
                self.assertTrue(cache_shards.restore(config,False))
            self.assertEqual((target/'release/deps/verified.rlib').read_bytes(),b'complete dependency')
            self.assertFalse(marker.parent.exists())
            self.assertTrue(config['restored'])
            self.assertTrue(config['mirror_required'])
            self.assertFalse(config['hit'])

    def test_continuously_trickling_cache_data_cannot_extend_restore_budget(self):
        clock=[0.0]
        def tick():
            clock[0]+=0.25;return clock[0]
        reply=io.BytesIO(b'x'*100)
        # read1 returns immediately available data; a blocking read(size) would
        # permit a peer to extend the restore indefinitely by trickling bytes.
        class Trickle:
            def read1(self,_size):return reply.read(1)
        output=io.BytesIO()
        with patch.object(local_cache.time,'monotonic',side_effect=tick):
            with self.assertRaises(local_cache.RestoreBudgetExceeded):
                local_cache.download_body({'restore_deadline':2},Trickle(),output,100)
        self.assertGreater(len(output.getvalue()),0)
        self.assertLess(len(output.getvalue()),100)
        config={'restore_seconds':300}
        with patch.object(local_cache,'restore_snapshot',side_effect=local_cache.RestoreBudgetExceeded):
            with self.assertRaises(local_cache.RestoreBudgetExceeded):local_cache.restore(config)
        self.assertNotIn('restore_deadline',config,'post-job saves must have an independent deadline')


    def test_blob_requests_use_remaining_restore_budget_without_short_metadata_limit(self):
        config = {'url': 'http://100.64.0.1:8080', 'token': 'test', 'restore_deadline': 1000}
        for path, budget, expected in (('/v1/blob?id=test', 180, 180),
                                        ('/v1/blob?id=test', 800, 600),
                                        ('/v1/lookup', 180, 30)):
            with self.subTest(path=path, budget=budget), \
                    patch.object(local_cache, 'restore_remaining', return_value=budget), \
                    patch.object(local_cache.http.client, 'HTTPConnection') as connection:
                local_cache.request(config, path)
                self.assertEqual(connection.call_args.kwargs['timeout'], expected)


    def test_partial_rust_cache_invalidates_only_native_and_cross_build_script_runs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            profiles=[root/'target/release',root/'target/aarch64-linux-android/release',
                      root/'target/debug',root/'target/aarch64-linux-android/debug']
            markers=[]
            for profile in profiles:
                for layout in ['serde_core-hash', 'serde_core/another-hash']:
                    marker=profile/'.fingerprint'/layout/'run-build-script-build-script-build.json'
                    marker.parent.mkdir(parents=True)
                    marker.write_text('{"cached":"success"}')
                    markers.append(marker)
                for name in ['.fingerprint/library-hash/lib-library',
                             'build/serde_core-hash/build-script-build',
                             'deps/library.rlib']:
                    path=profile/name;path.parent.mkdir(parents=True,exist_ok=True)
                    path.write_bytes(b'reusable compilation')
            cache_shards.invalidate_build_scripts([root/'target'])
            for marker in markers:
                self.assertFalse(marker.parent.exists())
            for profile in profiles:
                for name in ['.fingerprint/library-hash/lib-library',
                             'build/serde_core-hash/build-script-build',
                             'deps/library.rlib']:
                    self.assertEqual((profile/name).read_bytes(),b'reusable compilation')

    def test_rust_shards_reject_indexes_pointing_outside_their_cache_group(self):
        record={'schema':local_cache.SCHEMA,'group':'rust/linux-x64/p00','key':'compiler-object',
                'id':'a'*64,'sha256':'b'*64,'size':1024}
        cache_shards.validate(record,'rust/linux-x64/p00')
        with self.assertRaises(ValueError): cache_shards.validate(record,'rust/windows-x64/p00')
        for field,value in (('id','../escape'),('size',-1),('sha256','bad')):
            with self.subTest(field=field),self.assertRaises(ValueError):
                cache_shards.validate({**record,field:value},record['group'])

    def test_rust_shard_index_requires_matching_remote_metadata(self):
        shard = {'schema': local_cache.SCHEMA, 'group': 'rust/linux-x64/p00',
                 'key': 'generation', 'id': 'a'*64, 'sha256': 'b'*64, 'size': 32}
        index = {'shards': [shard, *([None] * (cache_shards.COUNT - 1))]}
        for current in (None, {**shard, 'id': 'c'*64},
                        {**shard, 'sha256': 'd'*64}, {**shard, 'size': 31}, shard):
            with self.subTest(current=current), patch.object(cache_shards, 'lookup', return_value=current):
                self.assertEqual(cache_shards.index_available({}, index), current == shard)

    def test_cache_writes_require_explicit_safe_commit_capabilities(self):
        for kind in ('archives', 'rust'):
            for capabilities in (None, [], ['validated-cache-commit-v1'],
                                 ['rust-index-references-v1'],
                                 ['validated-cache-commit-v1', 'rust-index-references-v1'],
                                 ['validated-cache-commit-v1', 'complete-cache-retention-v1'],
                                 ['validated-cache-commit-v1', 'rust-index-references-v1',
                                  'complete-cache-retention-v1'], [{}]):
                with self.subTest(kind=kind, capabilities=capabilities):
                    reply = io.BytesIO(json.dumps({
                        'schema': local_cache.SCHEMA, 'status': 'ready',
                        'capabilities': capabilities,
                    }).encode())
                    reply.status = 200
                    expected = (isinstance(capabilities, list)
                                and 'validated-cache-commit-v1' in capabilities
                                and 'complete-cache-retention-v1' in capabilities
                                and (kind != 'rust' or 'rust-index-references-v1' in capabilities))
                    with patch.object(local_cache, 'response', return_value=nullcontext(reply)):
                        self.assertEqual(local_cache.uploads_supported({'kind': kind}), expected)
        reply = io.BytesIO()
        reply.status = 404
        with patch.object(local_cache, 'response', return_value=nullcontext(reply)):
            self.assertFalse(local_cache.uploads_supported({'kind': 'rust'}))

    def test_unsafe_cache_server_never_receives_archives_or_shards(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for kind in ('archives', 'rust'):
                config = {'kind': kind, 'group': 'dependencies/source' if kind == 'archives' else 'rust/linux-x64',
                          'key': 'generation', 'save': True, 'available': True}
                with self.subTest(kind=kind), \
                        patch.object(local_cache, 'cache_paths', return_value=[root]), \
                        patch.object(local_cache, 'uploads_supported', return_value=False), \
                        patch.object(local_cache, 'cache_report') as report, \
                        patch.object(local_cache, 'pack') as pack, \
                        patch.object(cache_shards, 'partition') as partition, \
                        patch.object(local_cache, 'response') as request:
                    local_cache.save(config)
                    pack.assert_not_called()
                    partition.assert_not_called()
                    request.assert_not_called()
                    self.assertIn('server upgrade required', report.call_args.args[2])

    def test_shared_android_sources_require_the_current_lockfiles_and_archive_digest(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);home=root/'home';out=root/'shared'
            (home/'go/pkg/mod/cache/download/example/@v').mkdir(parents=True)
            module=home/'go/pkg/mod/cache/download/example/@v/v1.0.0.zip';module.write_bytes(b'verified source zip')
            (home/'go/bin').mkdir(parents=True)
            for tool in ('gomobile','gobind'): (home/'go/bin'/tool).write_bytes(b'verified tool')
            (root/'apps/android/core').mkdir(parents=True)
            mod=root/'apps/android/core/go.mod';mod.write_bytes(b'module test')
            (root/'apps/android/core/go.sum').write_bytes(b'checksums')
            with patch.object(build_inputs,'ROOT',root),patch.object(build_inputs.Path,'home',return_value=home), \
                    patch.object(build_inputs,'current_commit',return_value='a'*40), \
                    patch.object(android_build,'ensure_binding_tool'),patch.object(build_inputs.subprocess,'run'), \
                    patch.dict(os.environ, {'GITHUB_ACTIONS': 'true'}):
                build_inputs.shared_go(out)
                reports = list(upstream_cache.directory().glob('*.json'))
                self.assertEqual(len(reports), 1)
                self.assertTrue(upstream_cache.directory().is_relative_to(
                    Path(os.environ['BUILD_INPUTS_DIR'])))
                self.assertEqual(json.loads(reports[0].read_text()), {
                    'url': 'https://proxy.golang.org/example/@v/v1.0.0.zip',
                    'size': len(b'verified source zip'),
                    'sha256': build.sha256(module),
                })
                module.unlink()
                build_inputs.shared_go(out,restore=True)
                self.assertEqual(module.read_bytes(),b'verified source zip')
                mod.write_bytes(b'changed dependency')
                with self.assertRaises(ValueError): build_inputs.shared_go(out,restore=True)
                mod.write_bytes(b'module test')
                archive=out/'go-source.tar.gz';archive.write_bytes(archive.read_bytes()+b'tampered')
                with self.assertRaises(ValueError): build_inputs.shared_go(out,restore=True)

    def test_shared_android_sources_reject_untidy_lockfiles_before_packing_or_building_tools(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(build_inputs.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, 'go')) as execute, \
                patch.object(android_build, 'ensure_binding_tool') as binding, \
                patch.object(local_cache, 'pack') as pack:
            directory = Path(temporary)
            with self.assertRaises(subprocess.CalledProcessError):
                build_inputs.shared_go(directory)
            self.assertEqual(execute.call_args.args[0][-3:], ['mod', 'tidy', '-diff'])
            binding.assert_not_called()
            pack.assert_not_called()
            self.assertFalse((directory/'receipt.json').exists())

    def test_cache_timing_report_never_copies_credentials_or_private_action_state(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            config={'group':'rust/linux-x64','key':'verified','token':'never-publish-this-token',
                    'url':'http://100.64.3.1:19081','started':10,
                    'timings':{'download':1.25,'token':'never-publish-this-token'}}
            with patch.dict(os.environ,{'GITHUB_WORKSPACE':str(root),'GITHUB_STEP_SUMMARY':str(root/'summary.txt')}), \
                    patch.object(local_cache.time,'monotonic',return_value=12), \
                    patch('sys.stdout',new_callable=io.StringIO) as logged:
                local_cache.cache_report(config,'restore','exact',1024)
            self.assertNotIn(config['token'],logged.getvalue())
            self.assertNotIn(config['url'],logged.getvalue())
            self.assertIn('restore: exact',logged.getvalue())
            report=next((root/'target/cache-report').glob('*.json'))
            self.assertEqual(json.loads(report.read_text())['timings_seconds'],{'download':1.25,'total':2})
            for path in (report,root/'summary.txt'):
                self.assertNotIn(config['token'],path.read_text())
                self.assertNotIn(config['url'],path.read_text())

    def test_interrupted_extraction_keeps_existing_files_and_compiler_cleanup_preserves_reports(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);source=root/'source';source.mkdir()
            (source/'object.rlib').write_bytes(b'new complete object')
            archive=root/'cache.tar.gz';local_cache.pack([source],archive)
            restored=root/'restored';restored.mkdir();object_file=restored/'object.rlib'
            object_file.write_bytes(b'old valid object')
            with patch.object(local_cache.shutil,'copyfileobj',side_effect=OSError('disk write interrupted')):
                with self.assertRaises(OSError):local_cache.unpack(archive,[restored])
            self.assertEqual(object_file.read_bytes(),b'old valid object')
            self.assertEqual(list(restored.glob('.cache-extract-*')),[])
            target=root/'target'
            for name in ('release/deps/bad.rlib','aarch64-linux-android/release/deps/bad.rlib',
                         'debug/deps/bad.rlib','aarch64-linux-android/debug/deps/bad.rlib',
                         'dependency-snapshot/inputs.json','android-report/logcat.txt'):
                path=target/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(b'preserve only reports/inputs')
            unpacked=root/'cargo/registry/src/crate/lib.rs'
            unpacked.parent.mkdir(parents=True);unpacked.write_bytes(b'incomplete restored source')
            with patch.dict(os.environ,{'GITHUB_ACTIONS':'true','CARGO_HOME':str(root/'cargo'),
                                       'WORKSPACE_SOURCE_CACHE':str(root/'external/source.json')}):
                local_cache.discard_restore({'kind':'rust','source':str(root)})
            self.assertFalse((target/'release').exists())
            self.assertFalse((target/'aarch64-linux-android/release').exists())
            self.assertFalse((target/'debug').exists())
            self.assertFalse((target/'aarch64-linux-android/debug').exists())
            self.assertTrue((target/'dependency-snapshot/inputs.json').exists())
            self.assertTrue((target/'android-report/logcat.txt').exists())
            self.assertFalse(unpacked.exists())

    def test_github_hit_only_checks_private_metadata_and_reuses_current_mirror(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            (root/'module.zip').write_bytes(b'GitHub restored source')
            config={'group':'dependencies/source','key':'current','base_key':'current',
                    'kind':'archives','paths':str(root),'prefix':['compatible-'],
                    'url':'http://127.0.0.1:19081','token':'test-only','save':True,'github_hit':'true'}
            generation='v'+str(local_cache.SCHEMA)+'-'+hashlib.sha256(str(root).encode()).hexdigest()[:16]+'-'
            record={'schema':local_cache.SCHEMA,'group':config['group'],'key':generation+'current','id':'a'*64}
            reply=io.BytesIO(json.dumps(record).encode()); reply.status=200
            with patch.object(local_cache,'response',return_value=nullcontext(reply)) as request, \
                    patch.object(local_cache,'cache_report'),patch.object(local_cache,'pack') as pack:
                local_cache.restore(config)
                self.assertTrue(config['restored'])
                self.assertFalse(config['mirror_required'])
                request.assert_called_once()
                self.assertTrue(request.call_args.args[1].startswith('/v1/lookup?'))
                local_cache.save(config)
                pack.assert_not_called()

    def test_github_hit_with_no_private_copy_registers_a_required_mirror(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            (root/'module.zip').write_bytes(b'GitHub restored source')
            config={'group':'dependencies/source','key':'current','base_key':'current',
                    'kind':'archives','paths':str(root),'prefix':[],
                    'url':'http://127.0.0.1:19081','token':'test-only','save':True,'github_hit':'true'}
            reply=io.BytesIO(); reply.status=404
            with patch.object(local_cache,'response',return_value=nullcontext(reply)) as request, \
                    patch.object(local_cache,'cache_report'):
                local_cache.restore(config)
                self.assertTrue(config['restored'])
                self.assertTrue(config['available'])
                self.assertTrue(config['mirror_required'])
                request.assert_called_once()

            uploaded=[]
            def upload(_config,path,**kwargs):
                self.assertEqual(kwargs['method'],'PUT')
                self.assertTrue(path.startswith('/v1/blob?'))
                uploaded.append(kwargs['file'].read())
                result=io.BytesIO(); result.status=201
                return nullcontext(result)
            with patch.object(local_cache,'response',side_effect=upload),patch.object(local_cache,'cache_report'), \
                    patch.object(local_cache, 'uploads_supported', return_value=True):
                local_cache.save(config)
                local_cache.save(config)
            self.assertEqual(len(uploaded),1)
            self.assertTrue(uploaded[0])

    def test_rust_compatible_github_hit_avoids_a_second_download_but_empty_miss_does_not(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            config={'kind':'rust','source':str(root),'github_hit':'false'}
            self.assertFalse(local_cache.github_restored(config))
            deps=root/'target/release/deps'; deps.mkdir(parents=True)
            (deps/'not-an-object.d').write_text('build metadata')
            (deps/'empty.rlib').touch()
            self.assertFalse(local_cache.github_restored(config))
            (deps/'dependency.rlib').write_bytes(b'restored dependency')
            self.assertTrue(local_cache.github_restored(config))
            config['github_hit']=''
            self.assertFalse(local_cache.github_restored(config))

    def test_rust_private_mirror_reuses_sources_but_invalidates_changed_dependencies_and_compiler(self):
        config={'kind':'rust','key':'compiler-dependencies-new-source',
                'prefix':['compiler-dependencies-','compiler-']}
        self.assertTrue(local_cache.private_current(config,{'key':'compiler-dependencies-old-source'}))
        self.assertFalse(local_cache.private_current(config,{'key':'compiler-old-dependencies-old-source'}))
        self.assertFalse(local_cache.private_current(config,{'key':'old-compiler-dependencies-old-source'}))
        self.assertFalse(local_cache.private_current(config,None))

    def test_source_only_changes_use_shards_but_exact_private_snapshots_are_not_uploaded(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary); (root/'target').mkdir()
            config={'kind':'rust','source':str(root),'group':'rust/linux-x64','key':'new-source',
                    'available':True,'save':True,'hit':False,'mirror_required':False,
                    'shard_index': {'shards': [None] * cache_shards.COUNT}}
            with patch.object(local_cache,'pack') as pack,patch.object(local_cache,'cache_report'), \
                    patch.object(cache_shards,'save') as sharded:
                local_cache.save(config)
                pack.assert_not_called()
                sharded.assert_called_once()
                config['private_exact']=True
                local_cache.save(config)
                sharded.assert_called_once()

    def test_immutable_downloads_do_not_reupload_build_script_mutations(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);(root/'source').write_bytes(b'pinned source')
            config={'kind':'archives','paths':str(root),'group':'dependencies/skia-armv7','key':'crate-checksum',
                    'available':True,'save':True,'hit':True,'immutable':True}
            with patch.object(local_cache,'pack') as pack,patch.object(local_cache,'fingerprint') as fingerprint, \
                    patch.object(local_cache,'cache_report'):
                local_cache.save(config)
                pack.assert_not_called()
                fingerprint.assert_not_called()

    def test_github_miss_restores_a_verified_private_archive(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary); source=root/'source'; source.mkdir()
            (source/'module.zip').write_bytes(b'private dependency')
            archive=root/'cache.tar.gz'; local_cache.pack([source],archive)
            destination=root/'restored'
            config={'group':'dependencies/source','key':'current','base_key':'current',
                    'kind':'archives','paths':str(destination),'prefix':[],
                    'url':'http://127.0.0.1:19081','token':'test-only','save':True,'github_hit':''}
            generation='v'+str(local_cache.SCHEMA)+'-'+hashlib.sha256(str(destination).encode()).hexdigest()[:16]+'-'
            record={'schema':local_cache.SCHEMA,'group':config['group'],'key':generation+'current',
                    'id':'a'*64,'size':archive.stat().st_size,'sha256':local_cache.digest(archive)}
            def reply(_config,path):
                result=io.BytesIO(json.dumps(record).encode() if path.startswith('/v1/lookup?') else archive.read_bytes())
                result.status=200
                return nullcontext(result)
            with patch.object(local_cache,'response',side_effect=reply) as request,patch.object(local_cache,'cache_report'):
                local_cache.restore(config)
                self.assertTrue(config['restored'])
                self.assertEqual((destination/'module.zip').read_bytes(),b'private dependency')
                self.assertEqual(request.call_count,2)

    def test_local_cache_uploads_are_atomic_checked_and_immutable(self):
        with tempfile.TemporaryDirectory() as temporary:
            store = local_cache.Store(temporary, budget=1000, min_free=0)
            payload = b'verified-cache'
            checksum = hashlib.sha256(payload).hexdigest()
            with self.assertRaises(ValueError): store.put('rust/linux-x64', 'generation-one', len(payload), checksum, io.BytesIO(b'short'))
            with self.assertRaises(ValueError): store.put('rust/linux-x64', 'generation-one', len(payload), '0'*64, io.BytesIO(payload))
            self.assertIsNone(store.lookup('rust/linux-x64', 'generation-one', []))
            self.assertEqual(list(store.incoming.iterdir()), [])
            store.put('rust/linux-x64', 'generation-one', len(payload), checksum, io.BytesIO(payload))
            record = store.lookup('rust/linux-x64', 'generation-one', [])
            with patch.object(local_cache.time,'time',return_value=9_999_999_999):
                self.assertEqual(store.fetch(record['group'],record['id']).read_bytes(),payload)
            with store.database() as database:
                self.assertEqual(database.execute('SELECT accessed FROM entries').fetchone()[0],9_999_999_999)
            other = b'another-cache'
            store.put('rust/linux-x64', 'generation-one', len(other), hashlib.sha256(other).hexdigest(), io.BytesIO(other))
            self.assertEqual(store.path(record['group'], record['id']).read_bytes(), payload)
            self.assertEqual(store.lookup('rust/linux-x64', 'generation-two', ['generation-'])['key'], 'generation-one')
            with self.assertRaises(ValueError): store.path('../../escape', record['id'])

    def test_cache_readme_rejects_bad_and_stale_uploads_without_replacing_content(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            store = local_cache.Store(root, budget=1000, min_free=0)

            def put(data, sequence, checksum=None, length=None):
                return store.put_readme(length if length is not None else len(data),
                                        checksum or hashlib.sha256(data).hexdigest(),
                                        sequence, io.BytesIO(data))

            current = b'Action-Sequence: 2001\nSource-Commit: current\n\nCurrent README\n'
            self.assertEqual(put(current, 2001), 'updated')
            self.assertEqual((root/'state/README.txt').read_bytes(), current)
            self.assertFalse((root/'README.txt').exists())
            self.assertEqual(put(current, 2001), 'unchanged')
            older = b'Action-Sequence: 1001\n\nOld README\n'
            self.assertEqual(put(older, 1001), 'superseded')
            self.assertEqual(put(b'Action-Sequence: 2001\n\nDifferent\n', 2001), 'superseded')
            with self.assertRaises(ValueError):
                put(current, 2001, checksum='0'*64)
            with self.assertRaises(ValueError):
                put(current, 3001, length=len(current)+1)
            with self.assertRaises(ValueError):
                put(b'x'*(local_cache.README_LIMIT+1), 3001)
            with self.assertRaises(ValueError):
                put(b'Action-Sequence: 3001\n\xff', 3001)
            self.assertEqual((root/'state/README.txt').read_bytes(), current)
            self.assertEqual(list((root/'state').glob('.readme-*')), [])

    def test_cache_readme_rejects_symlink_in_writable_state(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            store = local_cache.Store(root/'server', budget=1000, min_free=0)
            protected = root/'protected.txt'
            protected.write_text('keep', encoding='utf-8')
            (root/'server/state/README.txt').symlink_to(protected)
            body = b'Action-Sequence: 1001\n\nNew README\n'
            with self.assertRaises(ValueError):
                store.put_readme(len(body), hashlib.sha256(body).hexdigest(), 1001, io.BytesIO(body))
            self.assertEqual(protected.read_text(encoding='utf-8'), 'keep')

    def test_local_cache_pruning_keeps_recent_generations_and_respects_budget(self):
        with tempfile.TemporaryDirectory() as temporary:
            store = local_cache.Store(temporary, budget=100, min_free=0, versions=2)
            with patch.object(local_cache.time, 'time', return_value=100):
                for key in ('old', 'current', 'new'):
                    store.put('compiler/macos-arm64', key, 20, hashlib.sha256(b'a'*20).hexdigest(), io.BytesIO(b'a'*20))
            self.assertEqual(store.status()['groups'][0]['count'], 2)
            with self.assertRaises(OSError):
                store.put('rust/linux-x64', 'too-large', 101, hashlib.sha256(b'x'*101).hexdigest(), io.BytesIO(b'x'*101))
            self.assertEqual(store.status()['groups'][0]['count'], 2)

    def test_local_cache_plain_http_is_limited_to_private_tailnet_transport(self):
        for address in ('http://100.64.3.1:19081', 'http://127.0.0.1:19081', 'https://cache.example.com'):
            local_cache.endpoint({'url':address})
        for address in ('http://1.1.1.1', 'http://10.0.0.1', 'https://user:password@example.com', 'file:///tmp/cache'):
            with self.assertRaises(ValueError): local_cache.endpoint({'url':address})

    def test_cache_replacement_rejects_stale_jobs_and_keeps_old_data_on_failed_upload(self):
        with tempfile.TemporaryDirectory() as temporary:
            store = local_cache.Store(temporary, budget=1000, min_free=0)
            def put(key, data, sequence):
                return store.put('dependencies/source', key, len(data), hashlib.sha256(data).hexdigest(), io.BytesIO(data), sequence)
            self.assertTrue(put('snapshot-one', b'old', 100))
            with self.assertRaises(ValueError):
                store.put('dependencies/source', 'snapshot-two', 10, '0'*64, io.BytesIO(b'short'), 200)
            self.assertIsNotNone(store.lookup('dependencies/source', 'snapshot-one', []))
            self.assertTrue(put('snapshot-two', b'new', 200))
            self.assertFalse(put('snapshot-one', b'stale', 100))
            self.assertIsNone(store.lookup('dependencies/source', 'snapshot-one', []))
            self.assertTrue(put('snapshot-two', b'newer content', 300))
            record = store.lookup('dependencies/source', 'snapshot-two', [])
            self.assertEqual(store.path(record['group'], record['id']).read_bytes(), b'newer content')
            self.assertEqual(len(store.status()['entries']), 1)

    def test_dependency_snapshot_archives_keep_lockfiles(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root/'snapshot'; source.mkdir()
            (source/'Cargo.lock').write_bytes(b'locked source versions')
            archive = root/'cache.tar.gz'
            local_cache.pack([source], archive)
            local_cache.unpack(archive, [root/'restored'])
            self.assertEqual((root/'restored/Cargo.lock').read_bytes(), b'locked source versions')

    def test_unchanged_archives_ignore_temporary_locks_and_need_no_reupload(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root/'module.zip').write_bytes(b'cached source')
            fingerprint = local_cache.fingerprint([root])
            (root/'download.lock').write_bytes(b'temporary lock')
            self.assertEqual(local_cache.fingerprint([root]), fingerprint)
            config = {'group':'dependencies/source', 'key':'verified',
                      'kind':'archives', 'available':True, 'save':True, 'hit':False,
                      'fingerprint':fingerprint}
            with patch.object(local_cache, 'cache_paths', return_value=[root]), \
                 patch.object(local_cache, 'cache_report') as report, patch.object(local_cache, 'pack') as pack:
                local_cache.save(config)
                pack.assert_not_called()
                self.assertIn('unchanged', report.call_args.args[2])

    def test_rust_cache_keeps_build_scripts_without_archiving_application_or_test_binaries(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root/'target'
            names = ('release/build/tool/build-script-build.exe', 'release/deps/dep.rlib',
                     'release/clash-of-rust.exe', 'release/clash-of-rust', 'release/deps/unit-test',
                     'debug/deps/dep.rlib', 'debug/build/tool/build-script-build',
                     'debug/deps/unit-test', 'debug/clash-of-rust', 'debug/clash-desktop-ui', 'release/cor',
                     'aarch64-linux-android/debug/deps/dep.rlib',
                     'dependency-snapshot/inputs.json', 'cache-network.json', 'android-report/logcat.txt')
            for name in names:
                path = source/name; path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(b'binary')
            archive = root/'cache.tar.gz'
            local_cache.pack([source], archive, rust=True)
            local_cache.unpack(archive, [root/'restored'])
            files = sorted(p.relative_to(root/'restored').as_posix() for p in (root/'restored').rglob('*') if p.is_file())
            self.assertEqual(files, ['aarch64-linux-android/debug/deps/dep.rlib',
                                     'debug/build/tool/build-script-build', 'debug/deps/dep.rlib',
                                     'release/build/tool/build-script-build.exe', 'release/deps/dep.rlib'])

    def test_dependency_refresh_rejects_normal_ci_before_running_commands(self):
        for variables in ({}, {'GITHUB_ACTIONS': 'true', 'GITHUB_EVENT_NAME': 'push'}):
            with patch.dict(os.environ, variables, clear=True), \
                    patch.object(build_inputs.subprocess, 'run') as command:
                with self.assertRaisesRegex(ValueError, 'explicitly dispatched'):
                    build_inputs.refresh(Path('/unused'))
                command.assert_not_called()

    def test_approved_tool_check_rejects_search_path_drift_before_compiling(self):
        selected = build_inputs.tool_versions()
        for python_version, go_version, java_version, expected in (
            (selected['python'], 'go' + selected['go'], selected['java'].split('+')[0], None),
            ('3.14.0', 'go' + selected['go'], selected['java'].split('+')[0], 'Python'),
            (selected['python'], 'go1.26.10', selected['java'].split('+')[0], 'Go'),
            (selected['python'], 'go' + selected['go'], '17.0.21', 'Java'),
        ):
            with self.subTest(python=python_version, go=go_version, java=java_version), \
                    patch.object(build_inputs.sys, 'version', python_version), \
                    patch.object(build_inputs.subprocess, 'run', side_effect=[
                        subprocess.CompletedProcess([], 0, go_version + '\n', ''),
                        subprocess.CompletedProcess([], 0, '', '    java.version = ' + java_version + '\n'),
                    ]):
                if expected:
                    with self.assertRaisesRegex(ValueError, 'Active ' + expected):
                        build_inputs.check_tools()
                else:
                    build_inputs.check_tools()

    def test_android_package_checks_python_and_java_without_requiring_unused_go(self):
        selected = build_inputs.tool_versions()
        for java_version, accepted in ((selected['java'].split('+')[0], True), ('17.0.21', False)):
            with self.subTest(java=java_version), \
                    patch.object(build_inputs.sys, 'version', selected['python']), \
                    patch.object(build_inputs.subprocess, 'run', return_value=subprocess.CompletedProcess(
                        [], 0, '', '    java.version = ' + java_version + '\n')) as execute:
                if accepted:
                    build_inputs.check_tools(('java',))
                else:
                    with self.assertRaisesRegex(ValueError, 'Active Java'):
                        build_inputs.check_tools(('java',))
                execute.assert_called_once()
                self.assertEqual(execute.call_args.args[0][0], 'java')

    def test_desktop_toolchain_report_never_invokes_ambient_android_tools(self):
        for system in ('windows', 'macos', 'linux'):
            def execute(command, **kwargs):
                self.assertIn(command[0], ('rustc', 'cargo'))
                return subprocess.CompletedProcess(command, 0, command[0] + ' version', '')

            with self.subTest(system=system), \
                    patch.object(build_inputs.shutil, 'which', side_effect=lambda name: name) as locate, \
                    patch.object(build_inputs.subprocess, 'run', side_effect=execute) as run_tool, \
                    patch.dict(os.environ, {'ANDROID_NDK_ROOT': '/unused-android-toolchain'}):
                report = build_inputs.toolchain_report(system)
            self.assertEqual([call.args[0] for call in locate.call_args_list], ['rustc', 'cargo'])
            self.assertEqual(run_tool.call_count, 2)
            self.assertEqual(set(report), {'python', 'rustc', 'cargo'})

    def test_android_toolchain_report_records_required_tools_and_ndk(self):
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / 'source.properties').write_text('Pkg.Revision = fixture')
            with patch.object(build_inputs.shutil, 'which', side_effect=lambda name: name), \
                    patch.object(build_inputs.subprocess, 'run', side_effect=lambda command, **kwargs:
                                 subprocess.CompletedProcess(command, 0, command[0] + ' version', '')) as execute, \
                    patch.dict(os.environ, {'ANDROID_NDK_ROOT': directory}):
                report = build_inputs.toolchain_report('android')
            self.assertEqual([call.args[0][0] for call in execute.call_args_list],
                             ['rustc', 'cargo', 'go', 'java', 'gradle'])
            self.assertEqual(set(report), {'python', 'rustc', 'cargo', 'go', 'java', 'gradle', 'android_ndk'})
            self.assertEqual(report['android_ndk'], 'Pkg.Revision = fixture')

    def test_toolchain_report_rejects_an_unknown_system_before_running_tools(self):
        with patch.object(build_inputs.subprocess, 'run') as execute, \
                self.assertRaisesRegex(ValueError, 'Unknown build system'):
            build_inputs.toolchain_report('unknown')
        execute.assert_not_called()

    def test_gradle_tool_check_rejects_another_version_on_path(self):
        selected = build_inputs.tool_versions()
        for version, accepted in ((selected['gradle'], True), ('9.6.0', False)):
            with self.subTest(version=version), \
                    patch.object(build_inputs.sys, 'version', selected['python']), \
                    patch.object(build_inputs.subprocess, 'run', return_value=subprocess.CompletedProcess(
                        [], 0, '\nGradle ' + version + '\n', '')):
                if accepted:
                    build_inputs.check_tools(('gradle',))
                else:
                    with self.assertRaisesRegex(ValueError, 'Active Gradle'):
                        build_inputs.check_tools(('gradle',))

    def test_gradle_official_failure_restores_the_verified_private_original(self):
        version = build_inputs.tool_versions()['gradle']
        prefix = 'gradle-' + version
        content = io.BytesIO()
        with zipfile.ZipFile(content, 'w') as archive:
            for name in ('bin/gradle', 'bin/gradle.bat', f'lib/gradle-launcher-{version}.jar'):
                archive.writestr(prefix + '/' + name, 'fixture ' + name)
        payload = content.getvalue()
        selected = {'version': version,
                    'url': f'https://services.gradle.org/distributions/gradle-{version}-bin.zip',
                    'sha256': hashlib.sha256(payload).hexdigest()}

        def restore(url, target, checksum):
            target.write_bytes(payload)
            return True

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path_output = root / 'path'
            with patch.object(build_inputs, 'component', return_value=selected), \
                    patch.dict(os.environ, {'GITHUB_PATH': str(path_output)}), \
                    patch.object(build, 'opener') as http, \
                    patch.object(build.time, 'sleep'), \
                    patch.object(upstream_cache, 'restore', side_effect=restore) as mirror:
                http.return_value.open.side_effect = OSError('HTTP 504')
                result = build_inputs.prepare_gradle(root / 'tools')
            self.assertEqual(http.return_value.open.call_count, 3)
            mirror.assert_called_once_with(selected['url'], root / 'tools' / (prefix + '-bin.zip'),
                                           selected['sha256'])
            self.assertEqual(path_output.read_text().splitlines(), [str(result / 'bin')])
            self.assertTrue(os.access(result / 'bin/gradle', os.X_OK))
            self.assertEqual((result / 'bin/gradle.bat').read_text(), 'fixture bin/gradle.bat')
            self.assertFalse(any((root / 'tools').glob('.gradle-*')))

    def test_gradle_rejects_unapproved_identity_before_downloading(self):
        selected = build_inputs.approved()['upstream']['gradle']
        for changed in ({'version': '9.6.0'}, {'sha256': ''},
                        {'url': 'https://example.com/gradle.zip'}):
            with self.subTest(changed=changed), \
                    patch.object(build_inputs, 'component', return_value={**selected, **changed}), \
                    patch.object(build_inputs, 'download') as download:
                with self.assertRaisesRegex(ValueError, 'approved identity'):
                    build_inputs.prepare_gradle(Path('/unused'))
                download.assert_not_called()

    def test_gradle_rejects_corrupt_or_unsafe_archives_without_exposing_tools(self):
        version = build_inputs.tool_versions()['gradle']
        prefix = 'gradle-' + version
        for scenario in ('checksum', 'parent', 'absolute', 'symlink', 'missing-launcher'):
            content = io.BytesIO()
            with zipfile.ZipFile(content, 'w') as archive:
                archive.writestr(prefix + '/bin/gradle', 'fixture launcher')
                if scenario == 'parent':
                    archive.writestr(prefix + '/../outside', 'invalid')
                elif scenario == 'absolute':
                    archive.writestr('/' + prefix + '/outside', 'invalid')
                elif scenario == 'symlink':
                    member = zipfile.ZipInfo(prefix + '/link')
                    member.create_system = 3
                    member.external_attr = 0o120777 << 16
                    archive.writestr(member, '../../outside')
            payload = content.getvalue()
            selected = {'version': version,
                        'url': f'https://services.gradle.org/distributions/gradle-{version}-bin.zip',
                        'sha256': '0' * 64 if scenario == 'checksum' else hashlib.sha256(payload).hexdigest()}

            def save(url, target, **kwargs):
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(payload)

            with self.subTest(scenario=scenario), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                with patch.object(build_inputs, 'component', return_value=selected), \
                        patch.object(build_inputs, 'download', side_effect=save), \
                        patch.dict(os.environ, {'GITHUB_PATH': str(root / 'path')}):
                    with self.assertRaises(ValueError):
                        build_inputs.prepare_gradle(root / 'tools')
                self.assertFalse((root / 'path').exists())
                self.assertFalse((root / 'outside').exists())
                self.assertFalse((root / 'tools' / prefix).exists())
                self.assertFalse(any((root / 'tools').glob('.gradle-*')))

    def test_dependency_refresh_does_not_export_partial_lockfiles_after_resolution_failure(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.dict(os.environ, {'GITHUB_ACTIONS': 'true', 'GITHUB_EVENT_NAME': 'workflow_dispatch'}), \
                patch.object(build_inputs.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, 'cargo')):
            output = Path(temporary) / 'updates'
            with self.assertRaises(subprocess.CalledProcessError):
                build_inputs.refresh(output)
            self.assertFalse(output.exists())

    def test_dependency_refresh_sets_approved_go_version_before_resolving_modules(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.dict(os.environ, {'GITHUB_ACTIONS': 'true', 'GITHUB_EVENT_NAME': 'workflow_dispatch'}), \
                patch.object(build_inputs.subprocess, 'run') as execute, \
                patch.object(build_inputs, 'constraints', return_value={}), \
                patch.object(build_inputs, 'current_commit', return_value='current'):
            build_inputs.refresh(Path(temporary))
        self.assertEqual([call.args[0] for call in execute.call_args_list], [
            ['cargo', 'update'], ['go', 'mod', 'edit', '-go=' + build_inputs.tool_versions()['go']],
            ['go', 'get', '-u', './...'], ['go', 'mod', 'tidy'],
        ])

    def test_resolve_uses_repository_dependencies_and_approved_tools_without_updating(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            files = {name: b'approved source' for name in build_inputs.FILES}
            files['Cargo.lock'] = b'[[package]]\nname="example"\nversion="1.0.0"\n'
            for name, content in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(content)
            snapshot = root / 'snapshot'
            snapshot.mkdir()
            (snapshot / 'inputs.json').write_text('{"upstream":{"gradle":{"version":"99.0.0"}}}')
            with patch.object(build_inputs, 'ROOT', root), \
                    patch.object(build_inputs, 'original', side_effect=files.__getitem__), \
                    patch.object(build_inputs, 'constraints', return_value={}), \
                    patch.object(build_inputs, 'current_commit', return_value='current'), \
                    patch.object(build_inputs, 'approved', return_value={'upstream': {'gradle': {'version': '9.8.2'}}}), \
                    patch.object(build_inputs, 'read_json', side_effect=AssertionError('unexpected network update')), \
                    patch.object(build_inputs.subprocess, 'run', side_effect=AssertionError('unexpected resolver')), \
                    patch.object(build_inputs, 'report'):
                build_inputs.resolve(snapshot)
                value = json.loads((snapshot / 'inputs.json').read_text())
                self.assertEqual(value['upstream']['gradle']['version'], '9.8.2')
                self.assertEqual(value['files'], {name: hashlib.sha256(content).hexdigest() for name, content in files.items()})
                self.assertTrue(all((root / name).read_bytes() == content for name, content in files.items()))
                (root / 'Cargo.lock').write_text('unapproved update')
                with self.assertRaisesRegex(ValueError, 'approved repository'):
                    build_inputs.resolve(snapshot)

    def test_geo_snapshot_reuses_verified_bytes_without_network_access(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload = b'frozen official database'
            checksum = hashlib.sha256(payload).hexdigest()
            upstream = {'geo': {'assets': [
                {'name': name, 'size': len(payload), 'digest': 'sha256:' + checksum,
                 'browser_download_url': 'https://github.com/example/geo/' + name}
                for name in build_inputs.GEO]}}
            with patch.object(build_inputs, 'download_asset') as download:
                def save(asset, path):
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(payload)
                download.side_effect = save
                resources = build_inputs.freeze_geo(root, upstream)
            value = {'upstream': upstream, 'resources': resources}
            with patch.object(build_inputs, 'download_asset', side_effect=AssertionError('unexpected download')):
                self.assertEqual(set(build_inputs.verify_geo_snapshot(value, root)), set(build_inputs.GEO))
                damaged = root / 'resources/GeoIP.dat'
                damaged.write_bytes(b'x' * len(payload))
                with self.assertRaisesRegex(ValueError, 'checksum or size'):
                    build_inputs.verify_geo_snapshot(value, root)
                damaged.unlink()
                with self.assertRaisesRegex(ValueError, 'checksum or size'):
                    build_inputs.verify_geo_snapshot(value, root)
                with self.assertRaisesRegex(ValueError, 'inventory'):
                    build_inputs.verify_geo_snapshot({**value, 'resources': {}}, root)

    def test_geo_snapshot_rejects_incomplete_assets_and_wrong_download_size(self):
        payload = b'frozen official database'
        checksum = hashlib.sha256(payload).hexdigest()
        assets = [{'name': name, 'size': len(payload), 'digest': 'sha256:' + checksum}
                  for name in build_inputs.GEO]
        with self.assertRaisesRegex(ValueError, 'incomplete'):
            build_inputs.geo_assets({'geo': {'assets': assets[:-1]}})
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(build_inputs, 'download_asset') as download:
            def save(asset, path):
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(payload)
            download.side_effect = save
            assets[0]['size'] += 1
            with self.assertRaisesRegex(ValueError, 'checksum or size'):
                build_inputs.freeze_geo(Path(temporary), {'geo': {'assets': assets}})

    def test_apply_verifies_geo_before_replacing_source_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            snapshot = root / 'snapshot'
            payload = b'verified geo'
            checksum = hashlib.sha256(payload).hexdigest()
            upstream = {'geo': {'assets': [
                {'name': name, 'size': len(payload), 'digest': 'sha256:' + checksum}
                for name in build_inputs.GEO]}}
            resources = {}
            for name in build_inputs.GEO.values():
                path = snapshot / 'resources' / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(payload)
                resources['resources/' + name] = checksum
            sources = {name: b'approved source' for name in build_inputs.FILES}
            for name, content in sources.items():
                path = snapshot / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(content)
                target = root / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(b'unchanged local source')
            files = {name: hashlib.sha256(content).hexdigest() for name, content in sources.items()}
            value = {'schema': 1, 'commit': 'current', 'constraints': {}, 'files': files,
                     'baseline': files, 'upstream': upstream, 'resources': resources}
            build.write_json(snapshot / 'inputs.json', value)
            damaged = snapshot / 'resources/GeoIP.dat'
            damaged.write_bytes(b'x' * len(payload))
            with patch.object(build_inputs, 'ROOT', root), \
                    patch.object(build_inputs, 'current_commit', return_value='current'), \
                    patch.object(build_inputs, 'constraints', return_value={}), \
                    patch.object(build_inputs, 'original', side_effect=sources.__getitem__):
                with self.assertRaisesRegex(ValueError, 'checksum or size'):
                    build_inputs.apply(snapshot)
                self.assertEqual((root / 'Cargo.lock').read_bytes(), b'unchanged local source')
                damaged.write_bytes(payload)
                build_inputs.apply(snapshot)
                self.assertEqual((root / 'Cargo.lock').read_bytes(), sources['Cargo.lock'])
                for name in build_inputs.GEO.values():
                    self.assertEqual((root / 'bundle/resources' / name).read_bytes(), payload)

    def test_version_bootstrap_does_not_require_python_311_modules(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'versions'
            script = build.ROOT / 'scripts/build_inputs.py'
            program = '''
import builtins, runpy, sys
from pathlib import Path
original_import = builtins.__import__
def bootstrap_import(name, *args, **kwargs):
    if name == 'tomllib':
        raise ModuleNotFoundError('unavailable before Python setup')
    return original_import(name, *args, **kwargs)
builtins.__import__ = bootstrap_import
script = Path(sys.argv[1])
sys.path.insert(0, str(script.parent))
sys.argv = [str(script), 'versions']
runpy.run_path(str(script), run_name='__main__')
'''
            environment = {**os.environ, 'GITHUB_OUTPUT': str(output),
                           'GITHUB_ENV': str(Path(temporary) / 'environment')}
            subprocess.run([sys.executable, '-c', program, str(script)], env=environment,
                           check=True, capture_output=True, text=True)
            self.assertIn('python=', output.read_text())

    def test_python_baseline_and_audit_are_specific_to_target_platforms(self):
        import dependency_audit

        for system, arch, expected in (('linux', 'arm64', '3.15.2'), ('windows', 'x64', '3.15.2'),
                                       ('windows', 'arm64', '3.15.2'), ('macos', 'arm64', '3.15.2')):
            with self.subTest(system=system, arch=arch), \
                    patch.object(build_inputs, 'host_system', return_value=system), \
                    patch.object(build_inputs, 'host_arch', return_value=arch):
                self.assertEqual(build_inputs.tool_versions()['python'], expected)
        releases = [
            {'version': '3.11.9', 'stable': True, 'files': [{'platform': 'win32', 'arch': 'x64'}]},
            {'version': '3.11.18', 'stable': True, 'files': [
                {'platform': 'linux', 'arch': 'x64'}, {'platform': 'linux', 'arch': 'arm64'}]},
            {'version': '3.15.2', 'stable': True, 'files': [
                {'platform': 'linux', 'arch': 'x64'}, {'platform': 'linux', 'arch': 'arm64'}]},
        ]
        with patch.object(dependency_audit, 'metadata', return_value=releases):
            self.assertEqual(dependency_audit.tool_candidate('python_windows', '3.11.9')[0], '3.11.9')
            self.assertEqual(dependency_audit.tool_candidate('python', '3.11.18')[0], '3.15.2')
        with patch.dict(os.environ, {'COR_USE_BOOTSTRAP_TAILNET': 'true'}):
            self.assertEqual(build_inputs.tool_versions()['tailscale'], build_inputs.approved()['tools']['tailscale_bootstrap'])

    def test_java_audit_follows_the_latest_supported_lts(self):
        import dependency_audit

        with patch.object(dependency_audit, 'metadata', side_effect=[
                {'available_lts_releases': [8, 11, 17, 21, 25]},
                [{'version': {'semver': '25.0.5+8.0.LTS', 'build': 8}}],
        ]) as fetch:
            self.assertEqual(dependency_audit.tool_candidate('java', '17.0.21+9')[0], '25.0.5+8')
        self.assertIn('/latest/25/', fetch.call_args.args[0])

    def test_android_image_audit_checks_revisions_and_ignores_preview_and_other_variants(self):
        import dependency_audit

        content = b'''<sdk>
<remotePackage path="system-images;android-35;google_apis;x86_64"><revision><major>99</major></revision></remotePackage>
<remotePackage path="system-images;android-36;google_apis;x86_64"><revision><major>8</major></revision></remotePackage>
<remotePackage path="system-images;android-37;google_apis;x86_64"><revision><major>1</major></revision><channelRef ref="channel-1"/></remotePackage>
<remotePackage path="system-images;android-38;google_apis;arm64-v8a"><revision><major>1</major></revision></remotePackage>
<remotePackage path="system-images;android-36-ext19;google_apis;x86_64"><revision><major>9</major></revision></remotePackage>
</sdk>'''
        with patch.object(dependency_audit, 'metadata', return_value=content):
            self.assertEqual(dependency_audit.android_candidate('system-image', '36-r7')[0], '36-r8')
        with patch.object(dependency_audit, 'metadata', return_value=content.replace(b'channel-1', b'channel-0')):
            self.assertEqual(dependency_audit.android_candidate('system-image', '36-r7')[0], '37-r1')
        legacy=content.replace(b'android-35',b'android-31')
        with patch.object(dependency_audit, 'metadata', return_value=legacy):
            self.assertEqual(dependency_audit.android_candidate('system-image-legacy','31-r14')[0],'31-r99')

    def test_emulator_storage_accounts_for_the_effective_userdata_floor(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            avd = root/'device.avd'
            avd.mkdir()
            config = avd/'config.ini'
            config.write_text('hw.device.name=pixel_6\n'
                              'disk.dataPartition.size=10G\n'
                              '  disk.dataPartition.size = 12G\n'
                              'hw.ramSize=4096\n')
            free = 10 * 1024**3
            with patch.object(android_build.shutil, 'disk_usage',
                              return_value=MagicMock(free=free)):
                android_build.configure_emulator_storage(avd, root/'report')
                android_build.configure_emulator_storage(avd, root/'report')
            self.assertEqual(config.read_text(), 'hw.device.name=pixel_6\nhw.ramSize=4096\n'
                             'disk.dataPartition.size=6144M\n')
            report = json.loads((root/'report/storage.json').read_text())
            self.assertEqual(report['free_bytes'], free)
            self.assertEqual(report['userdata_mib'], 6144)
            self.assertGreater(report['required_free_bytes'], 7 * 1024**3)
            self.assertLess(report['required_free_bytes'], free)
            self.assertEqual(report['removed_host_tools'], [])

    def test_emulator_storage_reports_insufficient_space_before_changing_the_device(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config = root/'config.ini'
            config.write_text('disk.dataPartition.size=10G\n')
            with patch.dict(os.environ, {'GITHUB_ACTIONS': ''}), \
                 patch.object(android_build, 'run') as invoke, \
                 patch.object(android_build.shutil, 'disk_usage',
                              return_value=MagicMock(free=3 * 1024**3)):
                with self.assertRaisesRegex(RuntimeError, 'available: 3.00 GiB'):
                    android_build.configure_emulator_storage(root, root/'report')
            self.assertEqual(config.read_text(), 'disk.dataPartition.size=10G\n')
            report = json.loads((root/'report/storage.json').read_text())
            self.assertEqual(report['free_bytes'], 3 * 1024**3)
            invoke.assert_not_called()

    def test_emulator_storage_reclaims_only_unused_tools_on_hosted_linux(self):
        for environment, system in (('github-hosted', 'Linux'),
                                    ('self-hosted', 'Linux'),
                                    ('github-hosted', 'Windows')):
            with self.subTest(environment=environment, system=system), \
                 tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                config = root/'config.ini'
                config.write_text('disk.dataPartition.size=10G\n')
                free_before = 5255 * 1024**2
                hosted_linux = environment == 'github-hosted' and system == 'Linux'
                with patch.dict(os.environ, {'GITHUB_ACTIONS': 'true',
                                             'RUNNER_ENVIRONMENT': environment,
                                             'RUNNER_OS': system}), \
                     patch.object(android_build, 'run') as invoke, \
                     patch.object(android_build.shutil, 'disk_usage', side_effect=[
                         MagicMock(free=free_before), MagicMock(free=15 * 1024**3),
                     ]):
                    if hosted_linux:
                        android_build.configure_emulator_storage(root, root/'report')
                        invoke.assert_called_once_with(
                            ['sudo', '-n', 'rm', '-rf', '--', '/usr/share/dotnet'], timeout=120)
                    else:
                        with self.assertRaisesRegex(RuntimeError, 'available: 5.13 GiB'):
                            android_build.configure_emulator_storage(root, root/'report')
                        invoke.assert_not_called()
                        self.assertEqual(config.read_text(), 'disk.dataPartition.size=10G\n')
                report = json.loads((root/'report/storage.json').read_text())
                self.assertEqual(report['free_bytes_before'], free_before)
                self.assertEqual(report['free_bytes'],
                                 15 * 1024**3 if hosted_linux else free_before)
                self.assertEqual(report['removed_host_tools'],
                                 ['/usr/share/dotnet'] if hosted_linux else [])

    def test_emulator_storage_rechecks_capacity_after_reclaiming_host_tools(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config = root/'config.ini'
            config.write_text('disk.dataPartition.size=10G\n')
            with patch.dict(os.environ, {'GITHUB_ACTIONS': 'true',
                                         'RUNNER_ENVIRONMENT': 'github-hosted',
                                         'RUNNER_OS': 'Linux'}), \
                 patch.object(android_build, 'run'), \
                 patch.object(android_build.shutil, 'disk_usage', side_effect=[
                     MagicMock(free=5 * 1024**3), MagicMock(free=6 * 1024**3),
                 ]):
                with self.assertRaisesRegex(RuntimeError, 'available: 6.00 GiB'):
                    android_build.configure_emulator_storage(root, root/'report')
            self.assertEqual(config.read_text(), 'disk.dataPartition.size=10G\n')
            report = json.loads((root/'report/storage.json').read_text())
            self.assertEqual(report['free_bytes'], 6 * 1024**3)

    def test_preview_installs_apk_inspection_tools_before_starting_the_emulator(self):
        versions = build_inputs.android_versions()
        image, revision, _ = android_build.emulator_image(api=31)
        regular = android_build.emulator_sdk_packages(image, revision)
        preview = android_build.emulator_sdk_packages(image, revision, preview=True)
        self.assertEqual(preview, regular + ['build-tools;' + versions['build_tools']])
        self.assertIn('platform-tools@' + versions['platform_tools'], preview)

    def test_emulator_rejects_revisions_that_differ_from_the_cache_identity(self):
        versions = build_inputs.android_versions()
        with tempfile.TemporaryDirectory() as temporary:
            sdk = Path(temporary)
            roots = (sdk/'emulator', sdk/versions['system_image'].replace(';', '/'))
            for root, revision in zip(roots, (versions['emulator'], versions['system_image_revision'])):
                root.mkdir(parents=True)
                (root/'source.properties').write_text('Pkg.Revision = ' + revision + '\n')
            android_build.verify_emulator_versions(sdk)
            for root in roots:
                properties = root/'source.properties'
                original = properties.read_text()
                for revision in ('999', 'preview', original.split('=')[1].strip() + '0'):
                    properties.write_text('Pkg.Revision = ' + revision + '\n')
                    with self.assertRaisesRegex(ValueError, 'revision differs'):
                        android_build.verify_emulator_versions(sdk)
                properties.write_text(original)
            legacy=sdk/versions['legacy_system_image'].replace(';','/')
            legacy.mkdir(parents=True)
            source_revision=versions['legacy_system_image_source_revision']
            self.assertNotEqual(source_revision,versions['legacy_system_image_revision'])
            (legacy/'source.properties').write_text('Pkg.Revision = '+source_revision+'\n')
            android_build.verify_emulator_versions(sdk,legacy=True)
            for invalid in ('999',versions['legacy_system_image_revision']):
                (legacy/'source.properties').write_text('Pkg.Revision = '+invalid+'\n')
                with self.assertRaisesRegex(ValueError,'revision differs'):
                    android_build.verify_emulator_versions(sdk,legacy=True)

    def test_historical_android_probe_images_keep_archive_and_embedded_identity_separate(self):
        for api in (26,27,28,29,30,31,32,33,34,35,36):
            with self.subTest(api=api), tempfile.TemporaryDirectory() as temporary:
                sdk=Path(temporary)
                image,archive_revision,source_revision=android_build.emulator_image(api=api)
                tag = 'default' if api == 27 else 'google_apis'
                self.assertEqual(image,f'system-images;android-{api};{tag};x86_64')
                self.assertTrue(archive_revision.isdigit())
                emulator=sdk/'emulator'
                emulator.mkdir()
                (emulator/'source.properties').write_text('Pkg.Revision='+build_inputs.android_versions()['emulator']+'\n')
                installed=sdk/image.replace(';','/')
                installed.mkdir(parents=True)
                (installed/'source.properties').write_text('Pkg.Revision='+source_revision+'\n')
                android_build.verify_emulator_versions(sdk,api=api)
                if archive_revision!=source_revision:
                    (installed/'source.properties').write_text('Pkg.Revision='+archive_revision+'\n')
                    with self.assertRaisesRegex(ValueError,'revision differs'):
                        android_build.verify_emulator_versions(sdk,api=api)
        with self.assertRaisesRegex(ValueError,'No approved'):
            android_build.emulator_image(api=25)

    def test_ime_export_retries_permission_failure_only_on_a_debuggable_emulator(self):
        for name in ('ime-report', 'application-preview'):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                self.verify_device_report_export(Path(temporary), name)

    def verify_device_report_export(self, report, name):
        commands=[]
        uid='2000'
        def invoke(command, **kwargs):
            nonlocal uid
            commands.append(command[1:])
            if command[1]=='root':
                uid='0'
            if command[1]=='unroot':
                uid='2000'
            if command[1]=='pull':
                if commands.count(command[1:])==1:
                    return subprocess.CompletedProcess(command,1,'','Permission denied')
                (report/name/'name.png').write_bytes(b'screenshot')
            value=uid+'\n' if command[1:]==['shell','id','-u'] else '1\n' if command[1]=='shell' else ''
            return subprocess.CompletedProcess(command,0,value,'')
        with patch.object(android_build.subprocess,'run',side_effect=invoke):
            android_build.pull_device_report('/sdcard/'+name,report,required=True,name=name)
        self.assertIn(['root'],commands)
        self.assertIn(['unroot'],commands)
        self.assertIn(['wait-for-device'],commands)
        self.assertEqual(sum(command[0]=='pull' for command in commands),2)
        self.assertEqual(commands[-1],['shell','id','-u'])
        self.assertEqual(uid,'2000')
        pull_log = name.removesuffix('-report') + '-pull.txt'
        self.assertIn('Permission denied',(report/pull_log).read_text())

    def test_android_report_export_restores_identity_after_root_disconnects_and_copy_failures(self):
        for copy_succeeds, restore_succeeds in ((True,True),(False,True),(True,False)):
            with self.subTest(copy=copy_succeeds,restore=restore_succeeds), \
                    tempfile.TemporaryDirectory() as temporary:
                report=Path(temporary)
                commands=[]
                uid='2000'
                def invoke(command, **kwargs):
                    nonlocal uid
                    arguments=command[1:]
                    commands.append(arguments)
                    if arguments==['root']:
                        uid='0'
                        return subprocess.CompletedProcess(command,255,'','closed')
                    if arguments==['unroot'] and restore_succeeds:
                        uid='2000'
                    if arguments==['shell','id','-u']:
                        return subprocess.CompletedProcess(command,0,uid+'\n','')
                    if arguments[0]=='shell':
                        return subprocess.CompletedProcess(command,0,'1\n','')
                    if arguments[0]=='pull':
                        if uid!='0' or not copy_succeeds:
                            return subprocess.CompletedProcess(command,1,'','Permission denied')
                        (report/'ime-report/name.png').write_bytes(b'capture')
                    return subprocess.CompletedProcess(command,0,'','')
                with patch.object(android_build.subprocess,'run',side_effect=invoke), \
                        patch.object(android_build.time,'sleep'):
                    if copy_succeeds and restore_succeeds:
                        android_build.pull_device_report('/sdcard/report',report,required=True)
                    else:
                        expected='original ADB identity' if not restore_succeeds else 'export failed'
                        with self.assertRaisesRegex(RuntimeError,expected):
                            android_build.pull_device_report('/sdcard/report',report,required=True)
                self.assertIn(['unroot'],commands)
                self.assertEqual(uid,'2000' if restore_succeeds else '0')

    def test_ime_export_does_not_root_a_physical_or_non_debuggable_device(self):
        for values in (('0','1'),('1','0')):
            with self.subTest(properties=values), tempfile.TemporaryDirectory() as temporary:
                report=Path(temporary)
                responses=[subprocess.CompletedProcess([],1,'','Permission denied'),
                           *(subprocess.CompletedProcess([],0,value+'\n','') for value in values)]
                with patch.object(android_build.subprocess,'run',side_effect=responses) as invoke:
                    with self.assertRaisesRegex(RuntimeError,'screenshot export failed'):
                        android_build.pull_device_report('/sdcard/ime-report',report,required=True)
                self.assertNotIn([android_build.ADB,'root'],[call.args[0] for call in invoke.call_args_list])

    def test_ime_export_rejects_successful_empty_reports(self):
        with tempfile.TemporaryDirectory() as temporary:
            report=Path(temporary)
            with patch.object(android_build.subprocess,'run',
                              return_value=subprocess.CompletedProcess([],0,'0 files pulled','')) as invoke:
                with self.assertRaisesRegex(RuntimeError,'screenshot export failed'):
                    android_build.pull_device_report('/sdcard/ime-report',report,required=True)
            self.assertEqual(invoke.call_count,1)

    def test_ime_export_can_require_native_control_state_instead_of_subscription_screenshots(self):
        with tempfile.TemporaryDirectory() as temporary:
            report=Path(temporary)
            def invoke(command, **kwargs):
                (report/'ime-report/state.json').write_text('{"editor":"android.widget.EditText"}')
                return subprocess.CompletedProcess(command,0,'1 file pulled','')
            with patch.object(android_build.subprocess,'run',side_effect=invoke):
                android_build.pull_device_report('/sdcard/native-control',report,
                                              required=True,required_pattern='state.json')
            self.assertTrue((report/'ime-report/state.json').is_file())

    def test_diagnostic_export_requires_a_receipt_without_requiring_screenshots(self):
        for complete in (False, True):
            with self.subTest(complete=complete), tempfile.TemporaryDirectory() as temporary:
                report = Path(temporary)
                def invoke(command, **kwargs):
                    destination = report / 'measurement-diagnostics'
                    (destination / 'without-vpn.json').write_text('{}')
                    if complete:
                        (destination / 'result.json').write_text('{"passed":true}')
                    return subprocess.CompletedProcess(command, 0, 'files pulled', '')
                with patch.object(android_build.subprocess, 'run', side_effect=invoke):
                    if complete:
                        android_build.pull_device_report('/sdcard/measurement-diagnostics', report,
                            required=True, required_pattern='result.json', name='measurement-diagnostics')
                    else:
                        with self.assertRaisesRegex(RuntimeError, 'evidence export failed'):
                            android_build.pull_device_report('/sdcard/measurement-diagnostics', report,
                                required=True, required_pattern='result.json', name='measurement-diagnostics')

    def test_android_tun_origin_keeps_the_probe_live_until_controller_verification(self):
        for path, acknowledgement, tunnel in (
                ('/probe', None, False), ('/hold', b'!', False), ('/hold', b'?', False),
                ('/diagnostic-probe?token=ci-only-url-secret', None, True)):
            with self.subTest(path=path, acknowledgement=acknowledgement, tunnel=tunnel):
                server, client = socket.socketpair()
                client.settimeout(1)
                errors = []
                def serve():
                    try:
                        with server:
                            android_build.network_origin_handler()(server, ('127.0.0.1', 0), None)
                    except Exception as error:
                        errors.append(error)
                worker = threading.Thread(target=serve, daemon=True)
                worker.start()
                try:
                    if tunnel:
                        client.sendall(
                            b'CONNECT 198.51.100.1:18081 HTTP/1.1\r\n'
                            b'Host: 198.51.100.1:18081\r\n\r\n'
                        )
                        response = bytearray()
                        while not response.endswith(b'\r\n\r\n'):
                            fragment = client.recv(1)
                            self.assertTrue(fragment, 'Origin rejected the diagnostic proxy tunnel')
                            response.extend(fragment)
                        self.assertTrue(response.startswith(b'HTTP/1.1 200 '))
                    client.sendall(
                        f'GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\nContent-Length: 1\r\n\r\n'.encode()
                    )
                    headers = bytearray()
                    while not headers.endswith(b'\r\n\r\n'):
                        fragment = client.recv(1)
                        self.assertTrue(fragment, 'Origin closed before sending HTTP headers')
                        headers.extend(fragment)
                    self.assertTrue(headers.startswith(b'HTTP/1.1 200 '))
                    expected = b'cor-android-network-ok'
                    body = bytearray()
                    while len(body) < len(expected):
                        fragment = client.recv(len(expected) - len(body))
                        self.assertTrue(fragment, 'Origin truncated the body')
                        body.extend(fragment)
                    self.assertEqual(body, expected)
                    if acknowledgement is not None:
                        self.assertIn(f'Content-Length: {len(expected) + 1}\r\n'.encode(), headers)
                        client.settimeout(0.05)
                        with self.assertRaises(socket.timeout):
                            client.recv(1)
                        client.settimeout(1)
                        client.sendall(acknowledgement)
                        self.assertEqual(client.recv(1), b'\n' if acknowledgement == b'!' else b'')
                    self.assertEqual(client.recv(1), b'')
                finally:
                    client.close()
                    worker.join(2)
                self.assertFalse(worker.is_alive())
                self.assertEqual(errors, [])

    def test_android_network_runs_exclude_keyboard_without_hiding_failures(self):
        network_cases=('localProxyAndRealTunCarryTrafficAndDisconnectCleanly',
                       'applicationRoutingPersistsAndKeepsOtherAppsOnline',
                       'quickTileRestoresColdCoreWithoutOpeningTheActivity',
                       'measurementDiagnosticsDistinguishFailuresAndExport')
        for failed_case, log_error, captured_log in (
                (None,None,'network test complete'),
                (network_cases[0],None,'network test failed'),
                (network_cases[1],None,'application routing test failed'),
                (network_cases[2],None,'tile test failed'),
                (network_cases[3],None,'diagnostic export test failed'),
                (None,RuntimeError('Android test log end marker is missing'),'partial log'),
                (None,None,'Mobile runtime stopped: core failure')):
            with self.subTest(failed_case=failed_case,log_error=log_error,log=captured_log), \
                    patch.dict(os.environ, {'COR_CI_EXTENDED_TESTS': 'true'}), \
                    tempfile.TemporaryDirectory() as temporary:
                root=Path(temporary)
                cases=[]
                def invoke(command, **kwargs):
                    if 'instrument' in command:
                        case=command[command.index('class')+1].split('#')[1]
                        cases.append(case)
                        output='FAILURES!!!' if case==failed_case else 'OK (1 test)'
                        return subprocess.CompletedProcess(command,0,output,'')
                    return subprocess.CompletedProcess(command,0,'','')
                with patch.object(android_build,'ROOT',root), \
                        patch.object(android_build,'run'), \
                        patch.object(android_build.subprocess,'check_output',return_value='36\n') as properties, \
                        patch.object(android_build.subprocess,'run',side_effect=invoke) as commands, \
                        patch('http.server.ThreadingHTTPServer') as server, \
                        patch('threading.Thread'), \
                        patch.object(android_build,'start_test_log'), \
                        patch.object(android_build,'finish_test_log',return_value=(captured_log,log_error)), \
                        patch.object(android_build,'pull_device_report') as pull, \
                        patch.object(android_build,'validate_measurement_diagnostics') as validate, \
                        patch.object(android_build,'input_method_crashes') as ime_crashes:
                    if failed_case or log_error or 'Mobile runtime stopped:' in captured_log:
                        with self.assertRaises(RuntimeError):
                            android_build.test('x64',root/'app.apk',root/'test.apk')
                    else:
                        android_build.test('x64',root/'app.apk',root/'test.apk')
                    self.assertEqual(cases,list(network_cases))
                    self.assertTrue(all('getprop' in call.args[0] for call in properties.call_args_list))
                    self.assertFalse(any('input_method' in call.args[0] for call in commands.call_args_list))
                    pull.assert_called_once_with(
                        '/sdcard/Android/data/org.clashofrust.android/files/measurement-diagnostics',
                        root/'target/android-report/x64', required=True,
                        required_pattern='result.json', name='measurement-diagnostics')
                    validate.assert_called_once()
                    ime_crashes.assert_not_called()
                    server.return_value.shutdown.assert_called_once()
                    server.return_value.server_close.assert_called_once()
                report=root/'target/android-report/x64'
                self.assertFalse((report/'input-method-status.json').exists())
                self.assertTrue((report/'instrumentation.txt').is_file())

    def test_android_measurement_reports_reject_stale_misclassified_or_incompatible_evidence(self):
        cases = ['without-vpn', 'with-vpn', 'controller-authentication', 'recovered']
        def reports(api):
            result = {}
            for index, name in enumerate(cases):
                measurement = {'started_at': f'2026-11-11T00:00:0{index}Z',
                               'completed': name != 'controller-authentication',
                               'attempted': 2, 'succeeded': 1, 'node_failures': 1, 'request_failures': 0}
                if name == 'controller-authentication':
                    measurement.update(node_failures=0, failure={
                        'code': 'controller_authentication', 'http_status': 401})
                network = {}
                if api >= 29:
                    network['mtu'] = 1500
                if api >= 28:
                    network['private_dns_active'] = False
                history = [dict(measurement) for _ in range(4 if name == 'recovered' else 1)]
                result[name] = {'schema': 1, 'command': 'diagnose', 'data': {'context': {
                    'measurements': history, 'platform': {'Ok': {'sdk': api,
                        'vpn': {'connected': name == 'with-vpn'},
                        'network': {'physical_networks': [network]}}}}}}
            return result
        for api in (26, 28, 29, 36):
            for mutation in ('valid', 'false_node_failure', 'stale', 'mtu', 'credential', 'unbounded'):
                with self.subTest(api=api, mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                    directory = Path(temporary)
                    values = reports(api)
                    if mutation == 'false_node_failure':
                        values['controller-authentication']['data']['context']['measurements'][-1]['node_failures'] = 2
                    elif mutation == 'stale':
                        values['with-vpn']['data']['context']['measurements'][-1]['started_at'] = '2026-11-11T00:00:00Z'
                    elif mutation == 'mtu':
                        network = values['without-vpn']['data']['context']['platform']['Ok']['network']['physical_networks'][0]
                        if api >= 29:
                            network.pop('mtu')
                        else:
                            network['mtu'] = 1500
                    elif mutation == 'credential':
                        values['recovered']['unexpected'] = 'ci-controller-rejected'
                    elif mutation == 'unbounded':
                        history = values['recovered']['data']['context']['measurements']
                        history.append(copy.deepcopy(history[-1]))
                    (directory / 'result.json').write_text(json.dumps({
                        'schema': 1, 'sdk': api, 'cases': cases, 'passed': True}))
                    for name, report in values.items():
                        (directory / (name + '.json')).write_text(json.dumps(report))
                    if mutation == 'valid':
                        android_build.validate_measurement_diagnostics(directory, api)
                    else:
                        with self.assertRaises(ValueError):
                            android_build.validate_measurement_diagnostics(directory, api)

    def test_android_routing_policy_requires_both_real_network_cases(self):
        cases = ('basicSystemProxyAndTun', 'applicationRoutingPersistsAndKeepsOtherAppsOnline')
        for failed in (None, *cases):
            with self.subTest(failed=failed), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                observed = []
                def invoke(command, **kwargs):
                    if 'instrument' in command:
                        case = command[command.index('class') + 1].split('#')[1]
                        observed.append(case)
                        result = 'FAILURES!!!' if case == failed else 'OK (1 test)'
                        return subprocess.CompletedProcess(command, 0, result, '')
                    return subprocess.CompletedProcess(command, 0, '', '')
                with patch.dict(os.environ, {
                        'COR_CI_EXTENDED_TESTS': 'false',
                        'COR_CI_ANDROID_ROUTING_TESTS': 'true',
                    }), \
                        patch.object(android_build, 'ROOT', root), \
                        patch.object(android_build, 'run'), \
                        patch.object(android_build.subprocess, 'check_output', return_value='31\n'), \
                        patch.object(android_build.subprocess, 'run', side_effect=invoke), \
                        patch('http.server.ThreadingHTTPServer') as origin, \
                        patch('threading.Thread'), \
                        patch.object(android_build, 'start_test_log'), \
                        patch.object(android_build, 'finish_test_log', return_value=('', None)):
                    if failed:
                        with self.assertRaisesRegex(RuntimeError, 'Android integration test failed'):
                            android_build.test('x64', root/'app.apk', root/'test.apk')
                    else:
                        android_build.test('x64', root/'app.apk', root/'test.apk')
                    self.assertEqual(observed, list(cases))
                    origin.return_value.shutdown.assert_called_once()
                report = root/'target/android-report/x64'
                self.assertTrue((report/'instrumentation.txt').is_file())
                self.assertTrue((report/'runtime-status.json').is_file())

    def test_device_properties_retry_only_transient_adb_errors_and_keep_terminal_failure(self):
        command = [android_build.ADB, 'shell', 'getprop', 'ro.product.manufacturer']
        transient = subprocess.CalledProcessError(255, command, stderr='device offline')
        permanent = subprocess.CalledProcessError(1, command, stderr='permission denied')
        for failures in ([transient, 'Google\n'], [subprocess.TimeoutExpired(command, 15), 'Google\n'],
                         [transient] * 3, [permanent]):
            with self.subTest(failures=failures), \
                    patch.object(android_build.subprocess, 'check_output', side_effect=failures) as read, \
                    patch.object(android_build.time, 'sleep') as delay:
                if isinstance(failures[-1], str):
                    self.assertEqual(android_build.device_property('ro.product.manufacturer'), 'Google')
                else:
                    with self.assertRaises(type(failures[-1])):
                        android_build.device_property('ro.product.manufacturer')
                self.assertEqual(read.call_count, len(failures))
                self.assertEqual(delay.call_count, len(failures) - 1)

    def test_android_ime_runs_remain_explicit_and_require_screenshot_evidence(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            cases=[]
            def invoke(command, **kwargs):
                if 'instrument' in command:
                    cases.append(command[command.index('class')+1].split('#')[1])
                return subprocess.CompletedProcess(command,0,'OK (1 test)','')
            def property_value(command, **kwargs):
                return 'com.example.ime/.Keyboard\n' if 'default_input_method' in command else '36\n'
            with patch.object(android_build,'ROOT',root), \
                    patch.object(android_build,'run'), \
                    patch.object(android_build.subprocess,'check_output',side_effect=property_value), \
                    patch.object(android_build.subprocess,'run',side_effect=invoke), \
                    patch('http.server.ThreadingHTTPServer') as server, \
                    patch.object(android_build,'start_test_log'), \
                    patch.object(android_build,'finish_test_log',return_value=('',None)), \
                    patch.object(android_build,'pull_device_report') as pull:
                with self.assertRaisesRegex(RuntimeError,'IME evidence is incomplete'):
                    android_build.test('x64',root/'app.apk',root/'test.apk',ime_only=True)
                self.assertEqual(cases,['subscriptionFieldsShowImeAcceptTextAndReopenAfterBack'])
                server.assert_not_called()
                pull.assert_called_once()
                self.assertTrue(pull.call_args.kwargs['required'])
            self.assertTrue((root/'target/android-report/x64/ime-api-36/regression/input-method-status.json').is_file())

    def test_application_preview_matrix_uses_every_approved_image_and_readable_names(self):
        from build_inputs import android_versions
        versions = android_versions()
        apis = set(map(int, versions['ime_system_images']))
        apis.update(int(versions[name].split(';')[1].removeprefix('android-'))
                    for name in ('system_image', 'legacy_system_image'))
        matrix = compatibility.matrix()['include']
        self.assertEqual([item['api'] for item in matrix], sorted(apis))
        self.assertEqual(next(item['android'] for item in matrix if item['api'] == 27), '8.1')
        self.assertEqual(next(item['android'] for item in matrix if item['api'] == 31), '12')
        self.assertEqual(next(item['android'] for item in matrix if item['api'] == 32), '12L')

    def test_preview_source_rejects_failed_foreign_or_different_commit_ci(self):
        source = {'id': 123, 'path': '.github/workflows/ci.yml', 'head_sha': 'a' * 40,
                  'repository': {'full_name': 'example/project'},
                  'head_repository': {'full_name': 'example/project'},
                  'status': 'completed', 'conclusion': 'success', 'event': 'pull_request'}
        self.assertEqual(compatibility.validate_run(source, 'example/project', 'a' * 40), 123)
        for changes in ({'status': 'in_progress'}, {'conclusion': 'failure'}, {'head_sha': 'b' * 40},
                        {'path': '.github/workflows/android-compatibility.yml'}, {'event': 'workflow_run'},
                        {'head_repository': {'full_name': 'external/fork'}},
                        {'repository': {'full_name': 'other/project'}}, {'id': True}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                compatibility.validate_run(source | changes, 'example/project', 'a' * 40)

    def test_preview_source_requires_unique_live_same_run_inputs(self):
        source = {'id': 123, 'path': '.github/workflows/ci.yml', 'head_sha': 'a' * 40,
                  'repository': {'full_name': 'example/project'},
                  'head_repository': {'full_name': 'example/project'},
                  'status': 'completed', 'conclusion': 'success', 'event': 'pull_request'}
        artifacts = [{'id': number, 'name': name, 'expired': False, 'digest': 'sha256:' + 'c' * 64,
                      'workflow_run': {'id': 123, 'head_sha': 'a' * 40}}
                     for number, name in enumerate(('build-android-x64', 'dependency-snapshot'), 1)]
        def client(items):
            return lambda path: {'artifacts': items} if '/artifacts?' in path else source
        self.assertEqual(compatibility.resolve(client(artifacts), 'example/project', 'a' * 40, 123)
                         ['ci_run_id'], 123)
        for items in (artifacts[:1], artifacts + [artifacts[0]],
                      [artifacts[0] | {'expired': True}, artifacts[1]],
                      [artifacts[0] | {'workflow_run': {'id': 124, 'head_sha': 'a' * 40}}, artifacts[1]],
                      [artifacts[0] | {'workflow_run': {'id': 123, 'head_sha': 'b' * 40}}, artifacts[1]]):
            with self.subTest(items=items), self.assertRaises(ValueError):
                compatibility.resolve(client(items), 'example/project', 'a' * 40, 123)

    def test_android_preview_requires_all_pages_real_applications_and_matching_version(self):
        base = {'live': True, 'vpn': False, 'vpn_pending': False, 'detail': 0,
                'applications_loaded': True, 'applications_loading': False,
                'applications_error': '', 'applications_total': 15, 'applications_visible': 7,
                'applications_system': False, 'applications_mode': 1, 'applications_selected': 15,
                'applications_icons': 7, 'applications_icon_failures': 0,
                'applications_visibility_limited': False, 'applications_query_permission': True,
                'applications_saved_summary': '全部应用',
                'nodes_list': False, 'profiles_list': False, 'scale': 1.0}
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            cases = []
            def add(layout, name, page, **state):
                width, height = ((800, 1280) if layout == 'tablet-portrait' else
                                 (1280, 800) if layout == 'tablet-landscape' else
                                 (852, 393) if layout == 'landscape' else (393, 852))
                case_name = layout + '-' + name
                image = b'\x89PNG\r\n\x1a\n' + struct.pack('>I', 13) + b'IHDR' + struct.pack('>II', width, height)
                (directory / (case_name + '.png')).write_bytes(image)
                (directory / (case_name + '-screen.png')).write_bytes(image)
                cases.append({'name': case_name, 'image': case_name + '.png',
                              'screen_image': case_name + '-screen.png',
                              'capture_method': 'pixelcopy-window', 'black_sample_ratio': 0,
                              'pixel_width': width, 'pixel_height': height,
                              'state': base | {'width': width, 'height': height, 'page': page} | state})
            for layout in ('phone', 'landscape', 'tablet-portrait', 'tablet-landscape'):
                for name, page, state in (
                        ('home', 0, {}), ('proxies-grid', 1, {}), ('proxies-list', 1, {'nodes_list': True}),
                        ('subscriptions-grid', 2, {}), ('subscriptions-list', 2, {'profiles_list': True}),
                        ('settings-top', 9, {}), ('settings-middle', 9, {}), ('settings-bottom', 9, {}),
                        ('applications-all', 9, {'detail': 18, 'applications_mode': 0,
                                                 'applications_visible': 0, 'applications_icons': 0}),
                        ('applications-default', 9, {'detail': 18}),
                        ('applications-scrolled', 9, {'detail': 18, 'applications_scroll_y': -120})):
                    add(layout, name, page, **state)
            for name, page, state in (
                    ('subscription-editor', 2, {'detail': 3}), ('tools', 10, {}),
                    ('connections', 3, {}), ('rules', 4, {}), ('rule-shortcuts', 4, {'shortcuts_open': True}),
                    ('logs', 5, {}), ('network', 6, {}), ('ip', 7, {}), ('ai', 8, {}),
                    ('applications-system', 9, {'detail': 18, 'applications_system': True}),
                    ('applications-only', 9, {'detail': 18, 'applications_mode': 1}),
                    ('applications-exclude', 9, {'detail': 18, 'applications_mode': 2,
                                                'applications_selected': 0}),
                    ('applications-unsaved', 9, {'detail': 12})):
                add('phone', name, page, **state)
            manifest = {'schema': 3, 'scope': 'android-ui-preview', 'android_api': 31,
                        'android_release': '12', 'cases': cases}
            def write():
                (directory / 'manifest.json').write_text(json.dumps(manifest))
            write()
            android_build.validate_application_preview(directory, 31)
            for name, changes, message in (
                    ('phone-applications-all', {'applications_visible': 7}, 'application list'),
                    ('phone-applications-all', {'applications_icons': 1}, 'application icons'),
                    ('phone-applications-default', {'applications_mode': 0}, 'routing mode'),
                    ('phone-applications-exclude', {'applications_selected': 1}, 'invert'),
                    ('phone-applications-only', {'applications_selected': True}, 'selection'),
                    ('phone-applications-only', {'applications_selected': 16}, 'selection')):
                with self.subTest(name=name, changes=changes):
                    changed = next(case for case in cases if case['name'] == name)
                    original = changed['state']
                    changed['state'] = original | changes
                    write()
                    with self.assertRaisesRegex(ValueError, message):
                        android_build.validate_application_preview(directory, 31)
                    changed['state'] = original
            write()
            application = next(case for case in cases if case['name'] == 'phone-applications-default')
            for change in ({'applications_icons': 0}, {'applications_icons': True},
                           {'applications_icons': 8}, {'applications_icon_failures': -1},
                           {'applications_visibility_limited': True},
                           {'applications_query_permission': False},
                           {'applications_saved_summary': ''}):
                with self.subTest(application=change):
                    original = application['state']
                    application['state'] = original | change
                    write()
                    with self.assertRaisesRegex(ValueError, 'application (icons|visibility)'):
                        android_build.validate_application_preview(directory, 31)
                    application['state'] = original
            manifest['schema'] = 2
            write()
            with self.assertRaisesRegex(ValueError, 'identity'):
                android_build.validate_application_preview(directory, 31)
            manifest['schema'] = 3
            tablet = next(case for case in cases if case['name'] == 'tablet-portrait-home')
            for scale in (None, 0, -1, 2.75, True, float('nan'), float('inf')):
                with self.subTest(scale=scale):
                    original = tablet['state']
                    tablet['state'] = original | {'scale': scale}
                    write()
                    with self.assertRaisesRegex(ValueError, 'display scale'):
                        android_build.validate_application_preview(directory, 31)
                    tablet['state'] = original
            write()
            for change in ({'capture_method': None}, {'capture_method': 'uiautomation'},
                           {'state': tablet['state'] | {'width': 1000}},
                           {'black_sample_ratio': None}, {'black_sample_ratio': -1},
                           {'black_sample_ratio': .21}, {'black_sample_ratio': True},
                           {'black_sample_ratio': float('nan')}, {'black_sample_ratio': float('inf')}):
                with self.subTest(change=change):
                    original = dict(tablet)
                    tablet.update(change)
                    write()
                    with self.assertRaisesRegex(ValueError, 'window capture'):
                        android_build.validate_application_preview(directory, 31)
                    tablet.clear()
                    tablet.update(original)
            write()
            application = next(case for case in cases if case['name'] == 'phone-applications-default')
            for changes in ({'detail': 9}, {'live': False}, {'applications_loading': True},
                            {'applications_loaded': False}, {'applications_error': 'access denied'},
                            {'applications_total': 0}, {'applications_visible': 1},
                            {'applications_visible': 16}, {'applications_visible': True},
                            {'vpn': True}, {'vpn_pending': True}):
                with self.subTest(changes=changes):
                    original = application['state']
                    application['state'] = original | changes
                    write()
                    with self.assertRaises(ValueError):
                        android_build.validate_application_preview(directory, 31)
                    application['state'] = original
            scrolled = next(case for case in cases if case['name'] == 'phone-applications-scrolled')
            for offset in (None, 0, 1, True, float('nan')):
                with self.subTest(offset=offset):
                    original = scrolled['state']
                    scrolled['state'] = original | {'applications_scroll_y': offset}
                    write()
                    with self.assertRaisesRegex(ValueError, 'did not scroll'):
                        android_build.validate_application_preview(directory, 31)
                    scrolled['state'] = original
            write()
            with self.assertRaisesRegex(ValueError, 'version'):
                android_build.validate_application_preview(directory, 30)
            missing = cases.pop()
            write()
            with self.assertRaisesRegex(ValueError, 'incomplete'):
                android_build.validate_application_preview(directory, 31)
            cases.append(missing)
            write()
            (directory / 'phone-applications-default.png').write_bytes(b'not an image')
            with self.assertRaisesRegex(ValueError, 'screenshot'):
                android_build.validate_application_preview(directory, 31)

    def test_application_preview_rejects_the_wrong_device_before_installing_apks(self):
        with patch.object(android_build.subprocess, 'check_output', return_value='36\n'), \
                patch.object(android_build, 'test') as run_test:
            with self.assertRaisesRegex(ValueError, 'selected Android version'):
                android_build.application_preview(Path('/missing-build'), 31)
            run_test.assert_not_called()

    def test_application_preview_keeps_the_live_adb_installation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sdk = root/'sdk'
            tool = sdk/'build-tools'/build_inputs.android_versions()['build_tools']/'aapt2'
            tool.parent.mkdir(parents=True)
            tool.touch()
            source = root/'build'
            source.mkdir()
            (source/'build.tar.gz').write_bytes(b'archive')
            (source/'build.tar.gz.sha256').write_text('a' * 64)
            with patch.dict(os.environ, {'ANDROID_HOME': str(sdk)}), \
                    patch.object(android_build, 'ROOT', root), \
                    patch.object(android_build.subprocess, 'check_output', return_value='31\n'), \
                    patch.object(android_build, 'sha256', return_value='a' * 64), \
                    patch('workflow_artifacts.extract_build'), \
                    patch.object(android_build, 'prepare_sdk') as prepare, \
                    patch.object(android_build, 'verify_build'), \
                    patch.object(android_build, 'commit', return_value='b' * 40), \
                    patch.object(android_build, 'test') as run_test:
                android_build.application_preview(source, 31)
            prepare.assert_not_called()
            self.assertTrue(run_test.call_args.kwargs['applications_only'])

    def test_android_compatibility_reuses_ci_build_and_skips_screenshot_and_extended_tests(self):
        for diagnostics, screenshots, routing, expected in (
                (False, False, False, [{'basic_only': True}]),
                (False, True, False, [{'basic_only': True}, {'applications_only': True}]),
                (True, False, False, [{'applications_only': False, 'diagnostics_only': True}]),
                (True, True, False, [{'applications_only': False, 'diagnostics_only': True},
                                     {'applications_only': True}]),
                (False, False, True, [{'basic_only': True}, {'routing_only': True}]),
                (False, True, True, [{'basic_only': True}, {'routing_only': True},
                                     {'applications_only': True}]),
                (True, False, True, [{'applications_only': False, 'diagnostics_only': True},
                                     {'routing_only': True}]),
                (True, True, True, [{'applications_only': False, 'diagnostics_only': True},
                                    {'routing_only': True}, {'applications_only': True}])):
            with self.subTest(diagnostics=diagnostics, screenshots=screenshots, routing=routing), \
                    tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                source = root / 'build'
                source.mkdir()
                (source / 'build.tar.gz').write_bytes(b'archive')
                (source / 'build.tar.gz.sha256').write_text('a' * 64)
                (root / 'aapt2').touch()
                with patch.object(android_build, 'ROOT', root), \
                        patch.object(android_build, 'sdk_tools', return_value=root), \
                        patch.object(android_build, 'device_property', return_value='31'), \
                        patch.object(android_build, 'sha256', return_value='a' * 64), \
                        patch('workflow_artifacts.extract_build'), \
                        patch.object(android_build, 'verify_build'), \
                        patch.object(android_build, 'commit', return_value='b' * 40), \
                        patch.object(android_build, 'test') as run_test:
                    android_build.application_preview(source, 31, diagnostics, True, screenshots, routing)
                self.assertEqual(
                    [{key: value for key, value in call.kwargs.items()
                      if key not in ('apk', 'test_apk')} for call in run_test.call_args_list],
                    expected)

    def test_application_preview_never_starts_network_tests_and_requires_exported_evidence(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            commands = []
            def execute(command, **kwargs):
                commands.append(list(map(str, command)))
                return subprocess.CompletedProcess(command, 0, 'OK (1 test)', '')
            with patch.object(android_build, 'ROOT', root), \
                    patch.object(android_build, 'run', side_effect=execute), \
                    patch.object(android_build.subprocess, 'run', side_effect=execute), \
                    patch.object(android_build.subprocess, 'check_output', return_value='31\n'), \
                    patch('http.server.ThreadingHTTPServer') as origin, \
                    patch.object(android_build, 'start_test_log'), \
                    patch.object(android_build, 'finish_test_log', return_value=('', None)), \
                    patch.object(android_build, 'pull_device_report') as pull, \
                    patch.object(android_build, 'validate_application_preview',
                                 side_effect=ValueError('missing screenshot')) as verify:
                with self.assertRaisesRegex(RuntimeError, 'missing screenshot'):
                    android_build.test('x64', root/'app.apk', root/'test.apk', applications_only=True)
                origin.assert_not_called()
                self.assertFalse(any('ACTIVATE_VPN' in command for command in commands))
                instruments = [command for command in commands if 'instrument' in command]
                self.assertEqual(len(instruments), 1)
                self.assertIn('org.clashofrust.android.ProxyTunTest#androidUiPreviewCapturesPagesAndApplications',
                              instruments[0])
                verify.assert_called_once_with(
                    root/'target/android-report/x64/applications-api-31/application-preview', 31)
                pull.assert_called_once_with(
                    '/sdcard/Android/data/org.clashofrust.android/files/application-preview',
                    root/'target/android-report/x64/applications-api-31',
                    required=True, name='application-preview')

    def test_android_application_rules_preserve_consent_and_prior_network_evidence(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = root/'target/android-report/x64'
            report.mkdir(parents=True)
            (report/'instrumentation.txt').write_text('basic proxy and TUN evidence')
            commands = []
            def execute(command, **kwargs):
                commands.append(list(map(str, command)))
                return subprocess.CompletedProcess(command, 0, 'OK (1 test)', '')
            with patch.object(android_build, 'ROOT', root), \
                    patch.object(android_build, 'run', side_effect=execute), \
                    patch.object(android_build.subprocess, 'run', side_effect=execute), \
                    patch.object(android_build, 'device_property', return_value='31'), \
                    patch('http.server.ThreadingHTTPServer'), \
                    patch.object(android_build, 'start_test_log'), \
                    patch.object(android_build, 'finish_test_log', return_value=('', None)):
                android_build.test('x64', root/'app.apk', root/'test.apk', routing_only=True)
            self.assertEqual((report/'instrumentation.txt').read_text(), 'basic proxy and TUN evidence')
            self.assertTrue((report/'application-rules/runtime-status.json').is_file())
            self.assertFalse(any('ACTIVATE_VPN' in command for command in commands))
            instruments = [command for command in commands if 'instrument' in command]
            self.assertEqual(len(instruments), 1)
            self.assertIn('org.clashofrust.android.ProxyTunTest#applicationRoutingPersistsAndKeepsOtherAppsOnline',
                          instruments[0])

    def test_android_apk_validation_records_badging_without_test_mode_state(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            image = bytearray(120)
            image[:6] = b'\x7fELF\x02\x01'
            struct.pack_into('<H', image, 18, 62)
            struct.pack_into('<Q', image, 32, 64)
            struct.pack_into('<HH', image, 54, 56, 1)
            struct.pack_into('<I', image, 64, 1)
            struct.pack_into('<Q', image, 112, 16384)
            apk = root/'app.apk'
            with zipfile.ZipFile(apk, 'w') as archive:
                for name in ('libclash_mobile.so', 'libgojni.so'):
                    archive.writestr('lib/x86_64/'+name, image)
            metadata = ("package: name='org.clashofrust.android' versionCode='123' versionName='1.2.3'\n"
                        "sdkVersion:'26'\ntargetSdkVersion:'36'\n")
            with patch.object(android_build, 'ROOT', root), \
                    patch.object(android_build, 'sdk_tools', return_value=root), \
                    patch.object(android_build, 'package_version', return_value='1.2.3'), \
                    patch.object(android_build.subprocess, 'check_output', return_value=metadata):
                result = android_build.validate_apk(apk, 'x64')
            self.assertEqual(result['version_code'], 123)
            self.assertEqual(result['native_bytes'], 240)
            self.assertEqual((root/'target/android-report/x64/apk-badging.txt').read_text(), metadata)

    def test_android_runtime_report_separates_adjacent_app_and_system_ui_crashes(self):
        log=('E AndroidRuntime: FATAL EXCEPTION: main\n'
             'E AndroidRuntime: Process: com.android.systemui, PID: 42\n'
             'E AndroidRuntime: java.lang.NullPointerException: navigation bar\n'
             'E AndroidRuntime: FATAL EXCEPTION: main\n'
             'E AndroidRuntime: Process: org.clashofrust.android, PID: 43\n'
             'E AndroidRuntime: java.lang.IllegalStateException: application\n')
        report=android_build.runtime_crash_status(log)
        self.assertTrue(report['log_scoped'])
        self.assertEqual(report['application_crashes'],
                         [{'process':'org.clashofrust.android',
                           'exception':'java.lang.IllegalStateException: application'}])
        self.assertEqual(report['system_ui_crashes'],
                         [{'process':'com.android.systemui',
                           'exception':'java.lang.NullPointerException: navigation bar'}])
        self.assertFalse(android_build.runtime_crash_status(log,False)['log_scoped'])
        self.assertEqual(android_build.runtime_crash_status('ordinary log')['application_crashes'],[])

    def test_android_runtime_report_includes_native_panics_without_a_java_exception(self):
        log = (
            '11-10 08:36:18.681 3904 3969 E GoLog: thread panicked at layout.rs:567\n'
            '11-10 08:36:18.692 3904 3953 E RustPanic: index out of bounds: the len is 4 but the index is 4\n'
        )
        report = android_build.runtime_crash_status(log)
        self.assertEqual(report['application_crashes'],
                         [{'process': 'org.clashofrust.android',
                           'exception': 'RustPanic: index out of bounds: the len is 4 but the index is 4'}])
        self.assertEqual(report['system_ui_crashes'], [])
        self.assertEqual(android_build.runtime_crash_status('I RustPanic: tag example')['application_crashes'], [])

    def test_android_ime_crash_report_uses_only_the_current_run_and_selected_input_method(self):
        crash=('E AndroidRuntime: FATAL EXCEPTION: main\n'
               'E AndroidRuntime: Process: com.example.ime, PID: 42\n'
               'E AndroidRuntime: java.lang.NullPointerException: unavailable keyboard\n')
        log=crash+'I CorAndroidTest: start-current-test\n'+crash
        current=android_build.scoped_test_log(log,'start-current-test')
        self.assertEqual(android_build.input_method_crashes(current,'com.example.ime'),
                         [{'process':'com.example.ime',
                           'exception':'java.lang.NullPointerException: unavailable keyboard'}])
        self.assertEqual(android_build.input_method_crashes(current,'org.clashofrust.android'),[])
        self.assertEqual(android_build.input_method_crashes(
            android_build.scoped_test_log(crash+'I CorAndroidTest: start-current-test\n','start-current-test'),
            'com.example.ime'),[])
        with self.assertRaisesRegex(RuntimeError,'start marker is missing'):
            android_build.scoped_test_log(log,'unrecorded-marker')

    def test_android_ime_crash_report_ignores_other_packages_and_ordinary_log_errors(self):
        log=('E AndroidRuntime: FATAL EXCEPTION: main\n'
             'E AndroidRuntime: Process: com.example.other, PID: 42\n'
             'E AndroidRuntime: java.lang.IllegalArgumentException: bad input\n'
             'E AndroidRuntime: Process: com.example.ime.extra, PID: 43\n'
             'E InputMethodManager: java.lang.NullPointerException: remote connection\n')
        self.assertEqual(android_build.input_method_crashes(log,'com.example.ime'),[])

    def test_android_ime_log_accepts_invalid_utf8_without_hiding_crashes_or_losing_scope(self):
        log=(b'I system: old invalid message \xc0\n'
             b'I CorAndroidTest: current-ime-test\n'
             b'I system: new invalid message \xc0\n'
             b'E AndroidRuntime: FATAL EXCEPTION: main\n'
             b'E AndroidRuntime: Process: com.example.ime, PID: 42\n'
             b'E AndroidRuntime: java.lang.NullPointerException: bad keyboard \xc0\n')
        current=android_build.scoped_test_log(log,'current-ime-test')
        self.assertNotIn('old invalid message',current)
        self.assertIn('new invalid message \ufffd',current)
        self.assertEqual(android_build.input_method_crashes(current,'com.example.ime'),
                         [{'process':'com.example.ime',
                           'exception':'java.lang.NullPointerException: bad keyboard \ufffd'}])
        with self.assertRaisesRegex(RuntimeError,'start marker is missing'):
            android_build.scoped_test_log(log,'absent-ime-test')

    def test_android_log_stream_preserves_bytes_and_checks_crashes_between_both_markers(self):
        with tempfile.TemporaryDirectory() as temporary:
            report=Path(temporary)
            process=MagicMock()
            process.poll.return_value=None
            def start(command, **kwargs):
                self.assertNotIn('-d',command)
                self.assertNotIn('-T',command)
                kwargs['stdout'].write(b'I system: previous log\n')
                kwargs['stdout'].flush()
                return process
            def emit(command, **kwargs):
                marker=command[-1]
                with (report/'logcat.raw').open('ab') as output:
                    output.write(f'I CorAndroidTest: {marker}\n'.encode())
                    if not marker.endswith('-end'):
                        output.write(b'I system: invalid byte \xc0\n'
                                     b'E AndroidRuntime: FATAL EXCEPTION: main\n'
                                     b'E AndroidRuntime: Process: com.example.ime, PID: 42\n'
                                     b'E AndroidRuntime: java.lang.NullPointerException: keyboard\n')
                return subprocess.CompletedProcess(command,0)
            with patch.object(android_build.subprocess,'Popen',side_effect=start), \
                    patch.object(android_build.subprocess,'run',side_effect=emit):
                capture=android_build.start_test_log(report,'current-test')
                captured,error=android_build.finish_test_log(capture,report,'current-test')
            self.assertIsNone(error)
            self.assertNotIn('previous log',captured)
            self.assertNotIn('current-test-end',captured)
            self.assertIn('\ufffd',captured)
            self.assertIn(b'\xc0',(report/'logcat.raw').read_bytes())
            self.assertEqual(len(android_build.input_method_crashes(captured,'com.example.ime')),1)
            process.terminate.assert_called_once()

    def test_android_log_stream_failure_preserves_raw_evidence_and_requires_complete_scope(self):
        for ended,timeout in ((True,False),(False,True)):
            with self.subTest(ended=ended), tempfile.TemporaryDirectory() as temporary:
                report=Path(temporary)
                raw=b'I CorAndroidTest: current-test\nI system: partial evidence\n'
                (report/'logcat.raw').write_bytes(raw)
                process=MagicMock()
                process.poll.return_value=1 if ended else None
                with patch.object(android_build.subprocess,'run'), \
                        patch.object(android_build.time,'monotonic',side_effect=[0,31]), \
                        patch.object(android_build.time,'sleep') as pause:
                    captured,error=android_build.finish_test_log(process,report,'current-test')
                self.assertIsNotNone(error)
                self.assertIn('partial evidence',captured)
                self.assertEqual((report/'logcat.raw').read_bytes(),raw)
                pause.assert_not_called()
                if timeout:
                    process.terminate.assert_called_once()
                else:
                    process.terminate.assert_not_called()
        with self.assertRaisesRegex(RuntimeError,'end marker is missing'):
            android_build.scoped_test_log(b'I CorAndroidTest: current-test\n','current-test','current-test-end')

    def test_android_log_stream_cleans_up_when_start_marker_cannot_be_written(self):
        with tempfile.TemporaryDirectory() as temporary:
            report=Path(temporary)
            process=MagicMock()
            process.poll.return_value=None
            process.wait.side_effect=[subprocess.TimeoutExpired([],5),0]
            with patch.object(android_build.subprocess,'Popen',return_value=process), \
                    patch.object(android_build.subprocess,'run',side_effect=subprocess.TimeoutExpired([],15)):
                with self.assertRaises(subprocess.TimeoutExpired):
                    android_build.start_test_log(report,'current-test')
            process.terminate.assert_called_once()
            process.kill.assert_called_once()

    def test_android_log_start_reconnects_only_for_transport_failures(self):
        for status, retries in ((255,True),(1,False)):
            with self.subTest(status=status), tempfile.TemporaryDirectory() as temporary:
                report=Path(temporary)
                first, second=MagicMock(),MagicMock()
                first.poll.return_value=None
                second.poll.return_value=None
                failure=subprocess.CalledProcessError(status,[],stderr='')
                with patch.object(android_build.subprocess,'Popen',side_effect=[first,second]) as spawn, \
                        patch.object(android_build.subprocess,'run',
                            side_effect=[failure,subprocess.CompletedProcess([],0,'','')]), \
                        patch.object(android_build,'wait_test_log_marker') as boundary, \
                        patch.object(android_build.time,'sleep'):
                    if retries:
                        self.assertIs(android_build.start_test_log(report,'boundary'),second)
                        boundary.assert_called_once_with(second,report,'boundary')
                    else:
                        with self.assertRaises(subprocess.CalledProcessError):
                            android_build.start_test_log(report,'boundary')
                        boundary.assert_not_called()
                self.assertEqual(spawn.call_count,2 if retries else 1)
                first.terminate.assert_called_once()

    def test_android_log_stream_requires_a_captured_start_marker_before_running_tests(self):
        with tempfile.TemporaryDirectory() as temporary:
            report=Path(temporary)
            process=MagicMock()
            process.poll.return_value=None
            with patch.object(android_build.subprocess,'Popen',return_value=process), \
                    patch.object(android_build.subprocess,'run'), \
                    patch.object(android_build.time,'monotonic',side_effect=[0,31]):
                with self.assertRaisesRegex(RuntimeError,'did not capture its marker'):
                    android_build.start_test_log(report,'current-test')
            process.terminate.assert_called_once()

    def test_android_input_method_preparation_never_mutates_a_physical_or_other_api_device(self):
        for values in (('0','1','31'),('1','0','31'),('1','1','36')):
            with self.subTest(properties=values), tempfile.TemporaryDirectory() as temporary:
                with patch.object(android_build.subprocess,'check_output',side_effect=values), \
                        patch.object(android_build.subprocess,'run') as invoke:
                    if values[2]=='31':
                        with self.assertRaisesRegex(RuntimeError,'debuggable API 31 emulator'):
                            android_build.prepare_emulator_input_method(Path(temporary))
                    else:
                        android_build.prepare_emulator_input_method(Path(temporary))
                invoke.assert_not_called()

    def test_android_input_method_preparation_keeps_a_different_keyboard_unchanged(self):
        with tempfile.TemporaryDirectory() as temporary:
            with patch.object(android_build.subprocess,'check_output',
                              side_effect=['1','1','31','com.example.ime/.Keyboard']), \
                    patch.object(android_build.subprocess,'run') as invoke:
                android_build.prepare_emulator_input_method(Path(temporary))
            invoke.assert_not_called()

    def test_android_input_method_preparation_preserves_identity_and_rejects_incomplete_setup(self):
        service='com.google.android.inputmethod.latin/com.android.inputmethod.latin.LatinIME'
        for reset_ok in (True,False):
            with self.subTest(reset_ok=reset_ok), tempfile.TemporaryDirectory() as temporary:
                report=Path(temporary)
                def invoke(command, **kwargs):
                    output=(b'approved Gboard version' if command[1:3]==['shell','dumpsys']
                            else 'Success\n' if reset_ok else 'Failed\n')
                    return subprocess.CompletedProcess(command,0,output,
                                                       b'' if isinstance(output,bytes) else '')
                with patch.object(android_build.subprocess,'check_output',
                                  side_effect=['1','1','31',service,service]), \
                        patch.object(android_build.subprocess,'run',side_effect=invoke) as execute:
                    if reset_ok:
                        android_build.prepare_emulator_input_method(report)
                    else:
                        with self.assertRaisesRegex(RuntimeError,'Could not prepare'):
                            android_build.prepare_emulator_input_method(report)
                record=json.loads((report/'input-method-setup.json').read_text())
                self.assertEqual(record['complete'],reset_ok)
                self.assertTrue(record['before_application_tests'])
                self.assertEqual(record['input_method'],service)
                self.assertEqual((report/'input-method-package-before-setup.txt').read_bytes(),
                                 b'approved Gboard version')
                mutations=[call.args[0][1:] for call in execute.call_args_list][1:]
                self.assertEqual(mutations[0],['shell','pm','clear','com.google.android.inputmethod.latin'])
                self.assertEqual(len(mutations),3 if reset_ok else 1)

    def test_android_native_ime_control_never_resets_a_physical_or_other_api_device(self):
        for values in (('0','1','31'),('1','0','31'),('1','1','36')):
            with self.subTest(properties=values), tempfile.TemporaryDirectory() as temporary:
                with patch.object(android_build.subprocess,'check_output',side_effect=values), \
                        patch.object(android_build.subprocess,'run') as invoke:
                    with self.assertRaisesRegex(RuntimeError,'debuggable API 31 emulator'):
                        android_build.platform_ime_control(Path(temporary),'com.example.ime/.Keyboard')
                invoke.assert_not_called()

    def test_android_native_ime_controls_preserve_crashes_without_replacing_production_results(self):
        crash=('E AndroidRuntime: FATAL EXCEPTION: main\n'
               'E AndroidRuntime: Process: com.example.ime, PID: 42\n'
               'E AndroidRuntime: java.lang.NullPointerException: native editor\n')
        with tempfile.TemporaryDirectory() as temporary:
            report=Path(temporary)
            def invoke(command, **kwargs):
                output='Success\n' if command[1:4]==['shell','pm','clear'] else b'evidence'
                return subprocess.CompletedProcess(command,0,output,'' if isinstance(output,str) else b'')
            with patch.object(android_build.subprocess,'check_output',side_effect=['1','1','31']), \
                    patch.object(android_build.subprocess,'run',side_effect=invoke) as execute, \
                    patch.object(android_build,'start_test_log'), \
                    patch.object(android_build,'finish_test_log',return_value=(crash,None)), \
                    patch.object(android_build,'pull_device_report'), \
                    patch.object(android_build.time,'monotonic',side_effect=[0,16,0,16]):
                android_build.platform_ime_control(report,'com.example.ime/.Keyboard')
            reset_commands=[call.args[0] for call in execute.call_args_list
                            if call.args[0][1:4]==['shell','pm','clear']]
            self.assertEqual(reset_commands,[[android_build.ADB,'shell','pm','clear','com.example.ime']]*2)
            for requests in (1,2):
                result=json.loads((report/'platform-control'/f'show-{requests}'/'result.json').read_text())
                self.assertEqual(len(result['crashes']),1)
                self.assertTrue(result['diagnostic_only'])
                self.assertFalse(result['replaces_production_result'])

    def test_android_preparation_uses_the_approved_sdk_and_skips_unused_package_tools(self):
        versions = build_inputs.android_versions()
        for package_only in (False, True):
            with self.subTest(package_only=package_only), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                sdk = root / 'sdk'
                output = root / 'environment'
                with patch.dict(os.environ, {'ANDROID_HOME': str(sdk), 'RUNNER_TEMP': str(root),
                                             'GITHUB_ENV': str(output)}), \
                        patch.object(android_build, 'install_sdk_packages') as install:
                    android_build.prepare_sdk(package_only=package_only)
                commands = [list(map(str, call.args[0])) for call in install.call_args_list]
                self.assertEqual(commands[0], [str(sdk / 'cmdline-tools/latest/bin/sdkmanager'),
                                               'cmdline-tools;' + versions['cmdline_tools']])
                self.assertEqual(commands[1][:7], [
                    str(sdk / 'cmdline-tools' / versions['cmdline_tools'] / 'bin/android'),
                    '--sdk='+str(sdk), 'sdk', 'install', '--force',
                    'platform-tools@'+versions['platform_tools'], 'build-tools/' + versions['build_tools'],
                ])
                if package_only:
                    self.assertEqual(len(commands[1]), 7)
                    self.assertFalse(output.exists())
                else:
                    self.assertEqual(commands[1][7:], ['platforms/android-' + versions['platform'],
                                                       'ndk/' + versions['ndk']])
                    self.assertEqual(output.read_text(), 'ANDROID_NDK_ROOT=' + str(sdk / 'ndk' / versions['ndk']) + '\n')

    def test_android_gradle_uses_the_approved_ndk_for_each_architecture(self):
        for arch, (abi, _, _, _) in android_build.ABIS.items():
            for debug in ('0', '1'):
                with self.subTest(arch=arch, debug=debug), \
                        patch.dict(os.environ, {'GITHUB_RUN_NUMBER': '42', 'RUNNER_DEBUG': debug}), \
                        patch.object(android_build, 'android_versions', return_value={'ndk': '30.0.12345678'}):
                    command = android_build.gradle('assembleRelease', arch=arch)
                self.assertIn('-PappNdkVersion=30.0.12345678', command)
                self.assertIn('-PappAbi=' + abi, command)
                self.assertEqual('--info' in command, debug == '1')
                self.assertEqual('-Dorg.gradle.deprecation.trace=true' in command, debug == '1')
                self.assertEqual(command[-1], 'assembleRelease')

    def test_android_sdk_bootstrap_prefers_the_native_cli_and_reuses_the_pinned_install(self):
        versions = build_inputs.android_versions()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sdk = root/'sdk'
            bootstrap = sdk/'cmdline-tools/latest/bin'
            bootstrap.mkdir(parents=True)
            (bootstrap/'android').touch()
            with patch.dict(os.environ, {'ANDROID_HOME': str(sdk), 'RUNNER_TEMP': str(root)}), \
                    patch.object(android_build, 'install_sdk_packages') as install:
                android_build.prepare_sdk(package_only=True)
                commands = [list(map(str, call.args[0])) for call in install.call_args_list]
                self.assertEqual(commands[0], [
                    str(bootstrap/'android'), '--sdk='+str(sdk), 'sdk', 'install', '--force',
                    'cmdline-tools/'+versions['cmdline_tools']])
                pinned = sdk/'cmdline-tools'/versions['cmdline_tools']/'bin'
                pinned.mkdir(parents=True)
                (pinned/'android').touch()
                install.reset_mock()
                android_build.prepare_sdk(package_only=True)
                install.assert_called_once()
                self.assertEqual(install.call_args.args[0][0], pinned/'android')

    def test_packaging_reuses_ci_tool_metadata_without_querying_new_versions(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            path = directory / 'inputs.json'
            path.write_text(json.dumps({'commit': 'current', 'upstream': {'nsis': {'sha256': 'a'*64}},
                                        'warnings': []}))
            before = path.read_bytes()
            with patch.object(build_inputs, 'current_commit', return_value='current'), \
                    patch.object(build_inputs, 'read_json', side_effect=AssertionError('unexpected tool upgrade')), \
                    patch.object(build_inputs, 'report'):
                build_inputs.packaging(directory)
            self.assertEqual(path.read_bytes(), before)

    def test_go_audit_checks_exact_modules_and_does_not_execute_updates(self):
        import dependency_audit

        queries=[]
        def fetch(url, raw=False):
            queries.append(url)
            if '/offline/' in url:
                raise urllib.error.URLError('offline')
            if raw:
                return b'module example.org/Module\n'
            return {'Version': 'v1.3.0' if '!module' in url else 'v0.0.0-20260112195520-a5071408f32f'}
        entries = [('go', 'go4.org/netipx', 'v0.0.0-20260823151212-3075585bcbeb', dependency_audit.go_candidate),
                   ('go', 'example.org/Module', 'v1.2.0', dependency_audit.go_candidate),
                   ('go', 'example.org/offline', 'v2.0.0', dependency_audit.go_candidate)]
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(dependency_audit, 'inventory', return_value=entries), \
                patch.object(dependency_audit, 'metadata', side_effect=fetch), \
                patch.object(build_inputs.subprocess, 'run', side_effect=AssertionError('unexpected update')):
            dependency_audit.audit(Path(temporary))
            records = json.loads((Path(temporary) / 'dependency-audit.json').read_text())['results']
        self.assertEqual([entry['status'] for entry in records], ['current', 'review', 'unavailable'])
        self.assertIn('https://proxy.golang.org/go4.org/netipx/@latest', queries)
        self.assertFalse(any(url.endswith('/go4.org/@latest') for url in queries))
        self.assertGreater(build_inputs.go_version('v1.0.0'),build_inputs.go_version('v1.0.0-rc.1'))
        self.assertGreater(build_inputs.go_version('v1.0.0-rc.10'),build_inputs.go_version('v1.0.0-rc.2'))

    def test_macos_python_mirror_uses_the_installed_framework_instead_of_its_external_aliases(self):
        import runtime_cache

        with patch.object(runtime_cache, 'host_system', return_value='macos'), \
                patch.object(runtime_cache, 'host_arch', return_value='arm64'), \
                patch.object(runtime_cache, 'tool_versions', return_value={'python': '3.11.9', 'tailscale': '1.104.5'}), \
                patch.dict(os.environ, {'RUNNER_TOOL_CACHE': '/runner/toolcache'}):
            self.assertEqual(runtime_cache.roots('bootstrap'), [
                Path('/Library/Frameworks/Python.framework/Versions/3.11'),
                Path('/runner/toolcache/tailscale/1.104.5')])

    def test_runtime_snapshot_preserves_internal_links_and_rejects_parent_escapes(self):
        import runtime_cache

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            tool = root / 'tool'
            tool.mkdir()
            (tool / 'compiler').write_bytes(b'compiler')
            try:
                (tool / 'alias').symlink_to('compiler')
            except OSError:
                self.skipTest('Symbolic links are unavailable')
            cache = root / 'cache'
            runtime_cache.capture('rust', cache, [tool])
            tool.rename(root / 'original')
            runtime_cache.hydrate('rust', cache, [tool])
            self.assertTrue((tool / 'alias').is_symlink())
            self.assertEqual((tool / 'alias').read_bytes(), b'compiler')
            (tool / 'escape').symlink_to('../outside')
            with self.assertRaisesRegex(ValueError, 'escapes'):
                runtime_cache.capture('rust', cache, [tool])

    def test_runtime_restore_rejects_an_inner_link_outside_payload_before_publish(self):
        import runtime_cache

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cache = root / 'cache'
            cache.mkdir()
            destination = root / 'tool'
            archive = cache / '0.tar'
            with tarfile.open(archive, 'w') as output:
                link = tarfile.TarInfo('payload/escape')
                link.type = tarfile.SYMTYPE
                link.linkname = '../outside'
                output.addfile(link)
            (cache / 'manifest.json').write_text(json.dumps({'schema': 1, 'kind': 'rust', 'roots': [
                {'index': 0, 'root': str(destination), 'size': archive.stat().st_size,
                 'sha256': build.sha256(archive)}]}))
            with self.assertRaisesRegex(ValueError, 'escapes'):
                runtime_cache.hydrate('rust', cache, [destination])
            self.assertFalse(destination.exists())

    def test_java_cache_restores_the_official_archive_to_a_complete_tool_directory(self):
        import runtime_cache

        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            archive = directory / 'jdk.tar.gz'
            with tarfile.open(archive, 'w:gz') as output:
                executable = tarfile.TarInfo('jdk-17/bin/java')
                executable.size = len(b'approved runtime')
                executable.mode = 0o755
                output.addfile(executable, io.BytesIO(b'approved runtime'))
            destination = directory / 'toolcache/version'
            record = {'url': 'https://github.com/adoptium/example.tar.gz', 'sha256': build.sha256(archive)}
            with patch.object(runtime_cache, 'java_archive', return_value=record), \
                    patch.object(runtime_cache, 'host_arch', return_value='x64'):
                runtime_cache.hydrate('java', directory, [destination])
            self.assertEqual((destination / 'x64/bin/java').read_bytes(), b'approved runtime')
            self.assertTrue((destination / 'x64.complete').is_file())
            config = {'kind': 'archives', 'runtime': 'java', 'paths': str(directory)}
            self.assertEqual(local_cache.cache_paths(config), [archive])
            with patch.object(runtime_cache, 'java_archive', return_value=record), \
                    patch.object(runtime_cache, 'download') as download:
                runtime_cache.capture('java', directory, [destination])
            download.assert_called_once_with(record['url'], archive, digest=record['sha256'])

    def test_java_cache_rejects_invalid_archive_checksums_before_publishing(self):
        import runtime_cache

        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / 'jdk.tar.gz').write_bytes(b'invalid archive')
            destination = directory / 'toolcache/version'
            with patch.object(runtime_cache, 'java_archive', return_value={'sha256': '0' * 64}):
                with self.assertRaisesRegex(ValueError, 'checksum'):
                    runtime_cache.hydrate('java', directory, [destination])
            self.assertFalse(destination.exists())

    def test_cache_upload_preserves_early_http_rejections(self):
        config = {'url': 'http://100.64.0.1:9080', 'token': ''}
        for status in (409, 413, 507):
            for failure in (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                with self.subTest(status=status, failure=failure.__name__):
                    connection = MagicMock()
                    connection.request.side_effect = failure('body upload interrupted')
                    connection.getresponse.return_value.status = status
                    with patch.object(local_cache.http.client, 'HTTPConnection', return_value=connection):
                        with local_cache.response(config, '/v1/blob', method='PUT', file=io.BytesIO(b'archive')) as reply:
                            self.assertEqual(reply.status, status)
                    connection.close.assert_called_once()
                    connection.getresponse.return_value.close.assert_called_once()

    def test_cache_upload_closes_connection_when_no_rejection_can_be_read(self):
        config = {'url': 'http://100.64.0.1:9080', 'token': ''}
        for failure_type in (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            with self.subTest(failure=failure_type.__name__):
                connection = MagicMock()
                failure = failure_type('body upload interrupted')
                connection.request.side_effect = failure
                connection.getresponse.side_effect = local_cache.http.client.RemoteDisconnected('no response')
                with patch.object(local_cache.http.client, 'HTTPConnection', return_value=connection):
                    with self.assertRaises(failure_type) as raised:
                        local_cache.request(config, '/v1/blob', method='PUT', file=io.BytesIO(b'archive'))
                self.assertIs(raised.exception, failure)
                connection.close.assert_called_once()

    def test_upstream_manifest_survives_removal_of_the_original_file_and_sends_only_urls(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.dict(os.environ, {'RUNNER_TEMP': temporary, 'GITHUB_ACTIONS': 'true'}):
            source = Path(temporary) / 'python.tar.gz'
            source.write_bytes(b'verified upstream')
            build.record_download('https://github.com/example/tool/releases/download/v1/tool.tar.gz',
                                  source, build.sha256(source))
            checksum = build.sha256(source)
            config = {'url': 'http://100.64.0.1:9080', 'token': 'never-publish-this'}
            sequence = int(os.environ.get('GITHUB_RUN_ID', '0')) * 1000 + min(int(os.environ.get('GITHUB_RUN_ATTEMPT', '0')), 999)
            reply = io.BytesIO(json.dumps({'schema': 1, 'group': upstream_cache.GROUP, 'key': 'sha256-' + checksum,
                                           'sequence': sequence, 'status': 'queued'}).encode())
            reply.status = 202
            source.unlink()
            with patch.object(upstream_cache, 'configuration', return_value=config), \
                    patch.object(local_cache, 'server_capabilities', return_value={
                        'validated-cache-commit-v1', 'complete-cache-retention-v1', 'upstream-fetch-v1'}), \
                    patch.object(upstream_cache, 'lookup', return_value=None), \
                    patch.object(local_cache, 'response', return_value=nullcontext(reply)) as request:
                report = upstream_cache.submit(Path(temporary) / 'report.json')
            payload = json.loads(request.call_args.kwargs['file'])
            self.assertEqual(request.call_args.args[1], '/v1/import')
            self.assertEqual(request.call_args.kwargs['method'], 'POST')
            self.assertEqual(payload['files'][0]['path'], '0')
            self.assertEqual(payload['files'][0]['sha256'], checksum)
            self.assertNotIn(temporary, json.dumps(payload))
            self.assertNotIn('never-publish-this', json.dumps(report))
            self.assertEqual(report['files'][0]['status'], 'queued')
            with patch.object(upstream_cache, 'configuration', return_value=config), \
                    patch.object(local_cache, 'server_capabilities', return_value=set()), \
                    patch.object(local_cache, 'response') as request:
                report = upstream_cache.submit(Path(temporary) / 'report.json')
                self.assertEqual(report['files'][0]['status'], 'queued')
                self.assertEqual(report['protocol'], 'disabled')
                request.assert_not_called()

    def test_release_metadata_network_fallback_respects_explicit_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)/'core.json'
            cached = {'tag_name':'v1.2.3','assets':[]}
            path.write_text(json.dumps({'release':cached}))
            with patch.object(build_inputs, 'component', return_value=None), \
                 patch.object(build_inputs, 'approved', return_value={'upstream': {}}), \
                 patch.object(build_inputs, 'read_json', side_effect=urllib.error.URLError('network unavailable')):
                self.assertEqual(build_inputs.resource_release('mihomo', path), cached)
                with self.assertRaisesRegex(RuntimeError, 'no verified'):
                    build_inputs.resource_release('mihomo', path, version='v9.0.0')

    def test_network_fallback_can_migrate_legacy_resource_receipts(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)/'core.json'
            value = {'version':'v1.2.3','source':'https://github.com/MetaCubeX/mihomo/releases/download/v1.2.3/mihomo-linux-arm64-v1.2.3.gz',
                     'archive_sha256':'a'*64,'exe_sha256':'b'*64}
            path.write_text(json.dumps(value))
            with patch.object(build_inputs, 'component', return_value=None), \
                 patch.object(build_inputs, 'approved', return_value={'upstream': {}}), \
                 patch.object(build_inputs, 'read_json', side_effect=urllib.error.URLError('offline')):
                release = build_inputs.resource_release('mihomo', path)
                self.assertEqual(release['tag_name'], 'v1.2.3')
                self.assertEqual(release['assets'][0]['digest'], 'sha256:'+'a'*64)
                value['archive_sha256'] = 'invalid'; path.write_text(json.dumps(value))
                with self.assertRaisesRegex(RuntimeError, 'no verified'):
                    build_inputs.resource_release('mihomo', path)
                records = [{'name':local,'sha256':'c'*64,'size':1,'source':'https://example.test/'+remote}
                           for remote, local in build_inputs.GEO.items()]
                path.write_text(json.dumps({'version':'latest | 2026-10-16','files':records}))
                release = build_inputs.resource_release('geo', path)
                self.assertEqual({asset['name'] for asset in release['assets']}, set(build_inputs.GEO))
                path.write_text(json.dumps({'version':'latest','files':records[:1]}))
                with self.assertRaisesRegex(RuntimeError, 'no verified'):
                    build_inputs.resource_release('geo', path)

    def test_cache_login_restores_mac_dns_including_automatic_search_domains(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            original = {'dnsservers':['9.9.9.9','2620:fe::fe'], 'searchdomains':[]}
            state = copy.deepcopy(original)
            def get(command, **kwargs):
                values = state[command[1][4:]]
                return '\n'.join(values) if values else "There aren't any Search Domains set on Ethernet."
            def set_value(command, **kwargs):
                values = command[5:]
                state[command[3][4:]] = [] if values == ['Empty'] else values
            with patch.object(local_cache.sys, 'platform', 'darwin'), \
                 patch.object(local_cache, 'Path', side_effect=lambda path: root/path), \
                 patch.object(local_cache.subprocess, 'check_output', side_effect=get), \
                 patch.object(local_cache.subprocess, 'run', side_effect=set_value):
                local_cache.preserve_network()
                state.update(dnsservers=['100.100.100.100'], searchdomains=['tailnet.example'])
                local_cache.preserve_network(restore=True)
            self.assertEqual(state, original)

    def test_nsis_version_query_failure_uses_only_verified_cached_compiler(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            compiler = root/'tools/nsis/current/makensis.exe'
            compiler.parent.mkdir(parents=True)
            compiler.write_bytes(b'verified compiler')
            (root/'tools/nsis/tool.json').write_text(json.dumps({'compiler_sha256':build.sha256(compiler)}))
            with patch.object(build, 'ROOT', root), patch.object(build_inputs, 'component', return_value=None), \
                 patch.object(build_inputs, 'approved', side_effect=urllib.error.URLError('unavailable')):
                self.assertEqual(build.nsis(), compiler)
                compiler.write_bytes(b'corrupt')
                with self.assertRaises(urllib.error.URLError): build.nsis()

    def test_nsis_completed_install_survives_a_locked_temporary_download(self):
        for persistent in (False, True):
            with self.subTest(persistent=persistent), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                installer = root/'tools/nsis-installer.exe'
                compiler = root/'tools/nsis/current/makensis.exe'
                selected = {'version':'test', 'url':'https://example.test/nsis.exe', 'sha256':'1'*64}
                original_unlink = Path.unlink
                attempts = []
                def unlink(path, *args, **kwargs):
                    if path == installer:
                        attempts.append(path)
                        if persistent or len(attempts) == 1:
                            raise PermissionError('installer still open')
                    return original_unlink(path, *args, **kwargs)
                def install(*args, **kwargs):
                    candidate = root/'tools/nsis-stage/makensis.exe'
                    candidate.parent.mkdir(parents=True)
                    candidate.write_bytes(b'installed compiler')
                def download(*args, **kwargs):
                    installer.parent.mkdir(parents=True, exist_ok=True)
                    installer.write_bytes(b'installer')
                with patch.object(build, 'ROOT', root), patch.object(build_inputs, 'component', return_value=selected), \
                     patch.object(build, 'download', side_effect=download), \
                     patch.object(build.subprocess, 'run', side_effect=install), \
                     patch.object(Path, 'unlink', unlink), patch.object(build.time, 'sleep'):
                    self.assertEqual(build.nsis(), compiler)
                    self.assertEqual(build.nsis(), compiler)
                self.assertEqual(len(attempts), 3 if persistent else 2)
                self.assertEqual(compiler.read_bytes(), b'installed compiler')
                receipt = json.loads((root/'tools/nsis/tool.json').read_text())
                self.assertEqual(receipt['compiler_sha256'], build.sha256(compiler))

    def test_dependency_cli_and_readers_share_the_configured_snapshot_directory(self):
        directory = Path(os.environ['BUILD_INPUTS_DIR'])
        (directory/'inputs.json').write_text(json.dumps({'marker':'selected snapshot'}))
        self.assertEqual(build_inputs.manifest(), {'marker':'selected snapshot'})
        with patch.object(build_inputs, 'apply') as apply, \
             patch('sys.argv', ['build_inputs.py', 'apply']):
            build_inputs.main()
            apply.assert_called_once_with(directory)

    def test_export_rejects_missing_stale_or_modified_dependency_snapshots(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            snapshot = Path(os.environ['BUILD_INPUTS_DIR'])/'inputs.json'
            with patch.object(build_inputs, 'ROOT', root), \
                 patch.object(build_inputs, 'current_commit', return_value='current'):
                with self.assertRaisesRegex(ValueError, 'requires the current'):
                    build_inputs.verified_manifest()
                files = {}
                for name in build_inputs.FILES:
                    path = root/name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text('resolved '+name)
                    files[name] = build.sha256(path)
                value = {'schema':1, 'commit':'old', 'files':files}
                snapshot.write_text(json.dumps(value))
                with self.assertRaisesRegex(ValueError, 'requires the current'):
                    build_inputs.verified_manifest()
                value['commit'] = 'current'
                snapshot.write_text(json.dumps(value))
                self.assertEqual(build_inputs.verified_manifest(), value)
                (root/'Cargo.lock').write_text('changed after resolving')
                with self.assertRaisesRegex(ValueError, 'changed after resolution: Cargo.lock'):
                    build_inputs.verified_manifest()

    def test_android_dependency_retry_is_bounded_and_keeps_compiler_errors(self):
        def process(status, output):
            child = MagicMock()
            child.__enter__.return_value = child
            child.stdout = output.splitlines(keepends=True)
            child.wait.return_value = status
            return child
        command = ['cargo', 'clippy', '--locked']
        unavailable = 'fatal: requested URL returned error: 503\n'
        with patch.object(android_build.subprocess, 'Popen', side_effect=[process(101, unavailable), process(0, 'Finished\n')]) as spawn, \
             patch.object(android_build.time, 'sleep') as delay:
            android_build.cargo_build(command)
            self.assertEqual(spawn.call_count, 2)
            delay.assert_called_once_with(15)
        with patch.object(android_build.subprocess, 'Popen', side_effect=[process(101, unavailable) for _ in range(3)]) as spawn, \
             patch.object(android_build.time, 'sleep'):
            with self.assertRaises(subprocess.CalledProcessError): android_build.cargo_build(command)
            self.assertEqual(spawn.call_count, 3)
        with patch.object(android_build.subprocess, 'Popen', return_value=process(101, 'error[E0308]: mismatched types\n')) as spawn, \
             patch.object(android_build.time, 'sleep') as delay:
            with self.assertRaises(subprocess.CalledProcessError): android_build.cargo_build(command)
            spawn.assert_called_once()
            delay.assert_not_called()

    def test_android_sdk_retries_closed_downloads_without_retrying_package_errors(self):
        for detail, retries in (
                ('Failed to download package archive for system-images/android-35: closed',True),
                ('Failed to download package archive for emulator: unexpected EOF',True),
                ('Failed to download package archive for emulator: connection reset by peer',True),
                ('Package closed is not available',False)):
            with self.subTest(detail=detail), tempfile.TemporaryDirectory() as temporary:
                with patch.object(android_build,'streamed_command',
                                  side_effect=[(1,detail),(0,'Installed')]) as execute, \
                        patch.object(android_build.time,'sleep') as pause:
                    if retries:
                        android_build.install_sdk_packages(['android','sdk','install'],Path(temporary))
                        self.assertEqual(execute.call_count,2)
                        pause.assert_called_once_with(15)
                    else:
                        with self.assertRaises(subprocess.CalledProcessError):
                            android_build.install_sdk_packages(['android','sdk','install'],Path(temporary))
                        execute.assert_called_once()
                        pause.assert_not_called()

    def test_android_sdk_download_retry_is_bounded_and_uses_fresh_user_caches(self):
        def process(status, output):
            child = MagicMock()
            child.__enter__.return_value = child
            child.stdout = output.splitlines(keepends=True)
            child.wait.return_value = status
            return child

        command = ['android', '--sdk=/sdk', 'sdk', 'install', 'system-images/android-35/google_apis/x86_64@7']
        corrupt = 'Error reading Zip content from a SeekableByteChannel\n'
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with patch.object(android_build.subprocess, 'Popen',
                              side_effect=[process(1, corrupt), process(0, 'Installed\n')]) as spawn, \
                 patch.object(android_build.time, 'sleep') as delay:
                android_build.install_sdk_packages(command, root)
                self.assertEqual(spawn.call_count, 2)
                self.assertNotEqual(spawn.call_args_list[0].kwargs['env']['ANDROID_USER_HOME'],
                                    spawn.call_args_list[1].kwargs['env']['ANDROID_USER_HOME'])
                delay.assert_called_once_with(15)
            with patch.object(android_build.subprocess, 'Popen',
                              side_effect=[process(1, corrupt) for _ in range(3)]) as spawn, \
                 patch.object(android_build.time, 'sleep'):
                with self.assertRaises(subprocess.CalledProcessError):
                    android_build.install_sdk_packages(command, root)
                self.assertEqual(spawn.call_count, 3)
            with patch.object(android_build.subprocess, 'Popen',
                              return_value=process(1, 'Package does not exist\n')) as spawn, \
                 patch.object(android_build.time, 'sleep') as delay:
                with self.assertRaises(subprocess.CalledProcessError):
                    android_build.install_sdk_packages(command, root)
                spawn.assert_called_once()
                delay.assert_not_called()

    def test_cache_budget_preserves_latest_main_and_removes_retired_caches(self):
        def entry(number, key, size, ref='refs/heads/main'):
            return {'id': number, 'key': key, 'size_in_bytes': size, 'ref': ref,
                    'last_accessed_at': f'2026-10-16T10:00:{number:02d}Z',
                    'created_at': f'2026-10-16T09:00:{number:02d}Z'}
        old = entry(1, 'native-release-linux-check-Linux-x64-aaaaaaaa-bbbbbbbb', 30)
        new = entry(2, 'native-release-linux-check-Linux-x64-aaaaaaaa-cccccccc', 30)
        go = entry(3, 'source-go-version-modules', 40)
        gradle = entry(4, 'gradle-content-unique-blob', 40)
        self.assertEqual(storage.eviction_plan([old, new, go, gradle], 1), [gradle, go, old])
        other_branch = entry(5, old['key'], 30, 'refs/pull/18/merge')
        self.assertEqual(storage.eviction_plan([new, other_branch], 100), [other_branch])
        for older,newer in (
                ('linux-python-ubuntu20-v1-x64-'+'a'*64, 'linux-python-ubuntu20-v1-x64-'+'b'*64),
                ('linux-cloud-ubuntu20-v1-arm64-'+'a'*64, 'linux-cloud-ubuntu20-v1-arm64-'+'b'*64),
                ('linux-apt-ubuntu20-v1-x64-100-1', 'linux-apt-ubuntu20-v1-x64-101-1'),
                ('android-go-Linux-X64-1.26.0-'+'a'*64,'android-go-Linux-X64-1.27.0-'+'b'*64),
                ('skia-source-Linux-X64-0.153.3-'+'a'*64,'skia-source-Linux-X64-0.154.0-'+'b'*64),
                ('resources-macos-x64-'+'a'*64+'-2026-10-16','resources-macos-x64-'+'b'*64+'-'+'c'*64),
                ('tools-nsis-'+'a'*64,'tools-nsis-'+'b'*64),
                ('action-setup-tailscale/1.94.2/Linux-amd64','action-setup-tailscale/1.104.1/Linux-amd64')):
            self.assertEqual(storage.lineage(older),storage.lineage(newer))
        with self.assertRaises(ValueError): storage.eviction_plan([], 0)

    def test_cache_main_migration_keeps_one_generation_per_target(self):
        pairs = []
        for target in ('windows-latest', 'windows-11-arm', 'macos-latest', 'macos-15-intel',
                       'ubuntu-22.04', 'ubuntu-22.04-arm'):
            legacy = 'native-release-ubuntu20-v1-' if target.startswith('ubuntu') else 'native-release-v2-'
            for qualifier in ('workspace', 'vendor'):
                suffix = f'-{qualifier}-'+'a'*64+'-local-deps-v1-check-Host-arch-aaaaaaaa-bbbbbbbb'
                pairs.append((legacy+target+suffix, 'native-main-v1-'+target+suffix))
        for arch in ('arm64', 'armv7', 'x64'):
            suffix = '-workspace-'+'a'*64+'-android-Linux-x64-aaaaaaaa-bbbbbbbb'
            pairs.append(('android-v2-'+arch+suffix, 'android-main-v1-'+arch+suffix))
        for profile in ('release', 'dev'):
            pairs.append(('memory-ubuntu22-v2-'+profile+'-aaaaaaaa-bbbbbbbb',
                          'memory-main-v1-'+profile+'-benchmark-Linux-x64-cccccccc-dddddddd'))
        for previous_key, latest_key in pairs:
            with self.subTest(key=latest_key):
                previous = {'id': 1, 'key': previous_key, 'ref': 'refs/heads/main',
                            'size_in_bytes': 10, 'created_at': '2026-11-10',
                            'last_accessed_at': '2026-11-12'}
                latest = dict(previous, id=2, key=latest_key, created_at='2026-11-11')
                self.assertEqual(storage.lineage(previous_key), storage.lineage(latest_key))
                self.assertEqual(storage.eviction_plan([previous, latest], 100), [previous])
                changed = re.sub(r'[0-9a-f]{8,}', lambda match: 'f'*len(match[0]), latest_key)
                newest = dict(latest, id=3, key=changed, created_at='2026-11-12')
                self.assertEqual(storage.eviction_plan([previous, latest, newest], 100),
                                 [latest, previous])

    def test_cache_main_keeps_profiles_architectures_and_unknown_keys_separate(self):
        keys = [
            'native-main-v1-windows-latest-check-Windows_NT-x64-aaaaaaaa-bbbbbbbb',
            'native-main-v1-windows-11-arm-check-Windows_NT-arm64-aaaaaaaa-bbbbbbbb',
            'memory-main-v1-release-benchmark-Linux-x64-aaaaaaaa-bbbbbbbb',
            'memory-main-v1-dev-benchmark-Linux-x64-aaaaaaaa-bbbbbbbb',
            'action-setup-tailscale/1.104.5/macOS-arm64',
            'action-setup-tailscale/1.104.5/macOS-amd64',
            'future-tool-one',
            'future-tool-two',
        ]
        entries = [{'id': index, 'key': key, 'ref': 'refs/heads/main',
                    'size_in_bytes': 10, 'created_at': '2026-11-11',
                    'last_accessed_at': '2026-11-11'}
                   for index, key in enumerate(keys)]
        self.assertEqual(storage.eviction_plan(entries, 1), [])
        older_duplicate = dict(entries[-1], id=100, created_at='2026-11-10')
        self.assertEqual(storage.eviction_plan([older_duplicate, *entries], 100),
                         [older_duplicate])
        newer_duplicate = dict(entries[-1], id=100, version='changed-archive-paths')
        self.assertEqual(storage.eviction_plan([*entries, newer_duplicate], 100),
                         [entries[-1]])

    def test_cache_removes_branch_deltas_regardless_of_ref_or_access_time(self):
        older = {
            'id': 1, 'key': 'branch-delta-v1-native-macos-'+'a'*64+'-'+'1'*40,
            'ref': 'refs/pull/34/merge', 'size_in_bytes': 30,
            'created_at': '2026-11-09T00:00:00Z', 'last_accessed_at': '2026-11-10T12:00:00Z',
        }
        newer = dict(older, id=2,
                     key='branch-delta-v1-native-macos-'+'b'*64+'-'+'2'*40,
                     created_at='2026-11-10T00:00:00Z', last_accessed_at='2026-11-10T00:00:00Z')
        self.assertEqual(storage.eviction_plan([older, newer], 100), [newer, older])
        other_ref = dict(newer, ref='refs/pull/35/merge')
        self.assertEqual(storage.eviction_plan([older, other_ref], 100), [other_ref, older])
        main_delta = dict(older, ref='refs/heads/main')
        self.assertEqual(storage.eviction_plan([main_delta, newer], 100), [newer, main_delta])

    def test_cache_branch_refs_never_keep_objects_or_tool_copies(self):
        def entry(number, key, ref):
            return {'id': number, 'key': key, 'ref': ref, 'size_in_bytes': 10,
                    'created_at': f'2026-11-10T00:00:0{number}Z',
                    'last_accessed_at': f'2026-11-10T00:00:0{number}Z'}

        workspace = 'native-release-v2-windows-arm64-build-workspace-'
        previous = entry(1, workspace + 'a'*64 + '-aaaaaaaa-bbbbbbbb', 'refs/heads/main')
        latest = entry(2, workspace + 'b'*64 + '-cccccccc-dddddddd', 'refs/pull/34/merge')
        tools_previous = dict(entry(3, 'action-setup-tailscale/1.104.5/macOS-arm64', 'refs/heads/main'), version='same')
        tools_latest = dict(entry(4, 'action-setup-tailscale/1.104.5/macOS-arm64', 'refs/pull/34/merge'), version='same')
        tools_other_arch = entry(5, 'action-setup-tailscale/1.104.5/macOS-amd64', 'refs/heads/main')
        self.assertEqual(storage.eviction_plan(
            [previous, latest, tools_previous, tools_latest, tools_other_arch], 100),
            [tools_latest, latest])
        different_version = dict(tools_latest, version='different')
        self.assertEqual(storage.eviction_plan([tools_previous, different_version], 100),
                         [different_version])
        for key in ('unknown-tool', 'branch-delta-v1-native-linux-short',
                    'branch-delta-v1-unrecognized-family-'+'a'*64+'-'+'1'*40):
            with self.subTest(key=key):
                branch = entry(6, key, 'refs/pull/34/merge')
                self.assertEqual(storage.eviction_plan([branch], 100), [branch])

    def test_cache_budget_keeps_main_and_removes_all_branch_profiles(self):
        def entry(number, key, ref):
            return {'id': number, 'key': key, 'ref': ref, 'size_in_bytes': 10,
                    'created_at': f'2026-11-10T00:00:0{number}Z',
                    'last_accessed_at': f'2026-11-10T00:00:0{number}Z'}

        main = entry(1, 'unknown-main-tool', 'refs/heads/main')
        pull = entry(2, 'unknown-pull-tool', 'refs/pull/34/merge')
        self.assertEqual(storage.eviction_plan([main, pull], 10), [pull])
        release = entry(3, 'branch-delta-v1-memory-release-'+'a'*64+'-'+'1'*40, 'refs/pull/34/merge')
        dev = entry(4, 'branch-delta-v1-memory-dev-'+'a'*64+'-'+'1'*40, 'refs/pull/34/merge')
        self.assertEqual(storage.eviction_plan([release, dev], 20), [dev, release])

    def test_full_compiler_environments_are_not_kept_on_branch_refs(self):
        def entry(number, key):
            return {'id': number, 'key': key, 'ref': 'refs/pull/34/merge', 'size_in_bytes': 10,
                    'created_at': f'2026-11-10T00:00:0{number}Z',
                    'last_accessed_at': f'2026-11-10T00:00:0{number}Z'}
        for prefix in ('linux-build-linux-image-ubuntu20-v1-', 'android-go-build-v1-'):
            previous = entry(1, prefix+'arm64-'+'a'*64)
            latest = entry(2, prefix+'arm64-'+'b'*64)
            other = entry(3, prefix+'x64-'+'c'*64)
            self.assertFalse(storage.retired(latest['key']))
            self.assertEqual(storage.eviction_plan([previous, latest, other], 100), [other, latest, previous])
        previous = entry(1, 'native-release-v2-windows-11-arm-check-Windows_NT-arm64-aaaaaaaa-bbbbbbbb')
        latest = entry(2, 'native-release-v2-windows-11-arm-vendor-'+'a'*64
                       +'-local-deps-v1-check-Windows_NT-arm64-aaaaaaaa-bbbbbbbb')
        self.assertEqual(storage.eviction_plan([previous, latest], 100), [latest, previous])

    def test_cache_cleanup_rechecks_size_and_fork_report_never_deletes(self):
        entries = [{'id': 1, 'key': 'tool-one', 'ref': 'refs/pull/1/merge', 'size_in_bytes': 30,
                    'last_accessed_at': '2026-10-15', 'created_at': '2026-10-15'},
                   {'id': 2, 'key': 'tool-two', 'ref': 'refs/heads/main', 'size_in_bytes': 50,
                    'last_accessed_at': '2026-10-16', 'created_at': '2026-10-16'}]
        writes = []
        def client(path, method='GET'):
            if method == 'DELETE':
                writes.append(path)
                entries[:] = [e for e in entries if not path.endswith('/' + str(e['id']))]
                return
            return {'actions_caches': copy.deepcopy(entries)}
        with patch.dict(build.os.environ, {}, clear=True), patch.object(storage.time, 'sleep'):
            report = storage.prune(client, 50, read_only=True)
            self.assertEqual(writes, [])
            self.assertEqual(report['after'], 80)
            report = storage.prune(client, 50, dry_run=True)
            self.assertEqual(writes, [])
            self.assertEqual(report['after'], 80)
            self.assertEqual(report['planned'], 1)
            self.assertEqual(report['projected'], 50)
            report = storage.prune(client, 50)
            self.assertEqual(writes, ['/actions/caches/1'])
            self.assertEqual(report['after'], 50)
            self.assertEqual(report['removed'], 1)
            with patch('sys.stdout', new_callable=io.StringIO) as output:
                report = storage.prune(client, 10)
            self.assertEqual(report['after'], 50)
            self.assertEqual(report['removed'], 0)
            self.assertIn('Latest main caches exceed', output.getvalue())

    def test_cache_inventory_reads_every_page(self):
        calls = []
        def client(path):
            calls.append(path)
            return {'actions_caches': list(range(100)) if path.endswith('&page=1') else [100]}
        self.assertEqual(len(storage.caches(client)), 101)
        self.assertEqual(calls, ['/actions/caches?per_page=100&page=1', '/actions/caches?per_page=100&page=2'])

    def test_android_binding_cache_requires_the_pinned_tool_metadata(self):
        import re
        version = re.search(r'golang.org/x/mobile\s+(v\S+)', (android_build.ANDROID/'core/go.mod').read_text())[1]
        expected = '/tools/gomobile: go1.26.0\npath\tgolang.org/x/mobile/cmd/gomobile\nmod\tgolang.org/x/mobile\t' + version + '\thash\n'
        with patch.object(android_build.shutil, 'which', return_value='/tools/gomobile'), \
             patch.object(android_build.subprocess, 'check_output', return_value='go1.26.0\n'), \
             patch.object(android_build.subprocess, 'run') as inspect, patch.object(android_build, 'run') as install:
            inspect.return_value = subprocess.CompletedProcess([], 0, expected, '')
            android_build.ensure_binding_tool('gomobile')
            install.assert_not_called()
            inspect.return_value = subprocess.CompletedProcess([], 0, expected.replace(version, 'v0.0.0-outdated'), '')
            android_build.ensure_binding_tool('gomobile')
            install.assert_called_once_with(['go', 'install', 'golang.org/x/mobile/cmd/gomobile@' + version])
            install.reset_mock()
            inspect.return_value = subprocess.CompletedProcess([], 0, expected.replace('go1.26.0', 'go1.25.0'), '')
            android_build.ensure_binding_tool('gomobile')
            install.assert_called_once_with(['go', 'install', 'golang.org/x/mobile/cmd/gomobile@' + version])

    def test_android_identity_accepts_old_and_current_aapt2_but_rejects_wrong_hosts(self):
        identity="package: name='org.clashofrust.android' versionCode='1000281' versionName='0.5.12-dev.1'\n"
        for name in ('sdkVersion','minSdkVersion'):
            output=identity+name+":'26'\ntargetSdkVersion:'36'\n"
            self.assertEqual(android_build.apk_identity(output,'0.5.12-dev.1'),1000281)
            for invalid in (output.replace(":'26'",":'25'"),output.replace(":'36'",":'35'"),
                            output.replace('org.clashofrust.android','another.app'),output.replace('0.5.12-dev.1','0.5.11')):
                with self.assertRaises(ValueError): android_build.apk_identity(invalid,'0.5.12-dev.1')

    def test_shared_core_has_no_desktop_toolkit_or_host_dependency(self):
        core = tomllib.loads((build.core_directory()/'Cargo.toml').read_text())
        # Mobile hosts must be able to use all shared policies without linking
        # the desktop application, platform authorization code or GUI toolkits.
        dependencies = set(core.get('dependencies', {}))
        for target in core.get('target', {}).values():
            dependencies.update(target.get('dependencies', {}))
        forbidden = {'clash-of-rust', 'clash-client-ui', 'slint', 'iced', 'iced_graphics', 'cosmic-text',
                     'tray-icon', 'windows', 'windows-sys', 'winreg', 'zbus',
                     'objc2', 'objc2-app-kit', 'objc2-foundation', 'block2'}
        self.assertFalse(dependencies & forbidden)
        for module in (build.core_directory()/'src').rglob('*.rs'):
            text = module.read_text()
            self.assertNotIn('crate::platform', text, str(module))
            self.assertNotIn('crate::engine', text, str(module))

    def test_workspace_and_legacy_version_and_source_paths(self):
        self.assertEqual(build.manifest_version('[package]\nversion="0.4.12"\n'), '0.4.12')
        self.assertEqual(build.manifest_version('[workspace.package]\nversion="0.5.11-dev.1"\n'), '0.5.11-dev.1')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root/'Cargo.toml').write_text('[package]\nversion="0.4.12"\n')
            self.assertEqual(build.package_version(root), '0.4.12')
            self.assertEqual(build.desktop_directory(root), root)
            self.assertEqual(build.core_directory(root), root)
            (root/'apps/desktop').mkdir(parents=True)
            (root/'crates/client-core').mkdir(parents=True)
            (root/'apps/desktop/Cargo.toml').touch()
            (root/'crates/client-core/Cargo.toml').touch()
            (root/'Cargo.toml').write_text('[workspace.package]\nversion="0.5.11-dev.1"\n')
            self.assertEqual(build.package_version(root), '0.5.11-dev.1')
            self.assertEqual(build.desktop_directory(root), root/'apps/desktop')
            self.assertEqual(build.core_directory(root), root/'crates/client-core')

    def test_release_job_protocol_matches_workflow_display_names(self):
        workflow = (build.ROOT/'.github/workflows/ci.yml').read_text()
        for name in release.CI_JOBS:
            if name.startswith('Ubuntu 20.04-'):
                self.assertIn('    name: Ubuntu 20.04-${{ matrix.arch }} installed proxy and TUN\n', workflow)
                arch = name.removeprefix('Ubuntu 20.04-').split(' ', 1)[0]
                self.assertIn('            arch: '+arch+'\n', workflow)
            elif name.startswith('Ubuntu latest-'):
                self.assertIn('    name: '+name+'\n', workflow)
        publishing = (build.ROOT/'.github/workflows/release.yml').read_text()
        self.assertIn('    name: package-android-${{ matrix.arch }}\n', publishing)
        self.assertNotIn('  ubuntu_20:', publishing)
        self.assertNotIn('  ubuntu_latest:', publishing)
        self.assertNotIn('scripts/test-release-package.py', publishing)

    def test_failed_cargo_json_preserves_human_diagnostics(self):
        message = json.dumps({'reason': 'compiler-message', 'message': {'rendered': 'error[E0308]: expected borrowed window'}})+'\n'
        with tempfile.TemporaryDirectory() as directory, patch('build_support.subprocess.run') as runner, patch('sys.stdout', new_callable=io.StringIO) as output:
            runner.return_value = subprocess.CompletedProcess(['cargo'], 101, stdout=message)
            log = Path(directory)/'build.jsonl'
            with self.assertRaises(subprocess.CalledProcessError):
                build.cargo_messages(['cargo', 'test'], log=log)
            self.assertEqual(log.read_text(), message)
            self.assertIn('error[E0308]', output.getvalue())

    def test_asset_verification_checks_each_pe_independently(self):
        spec = importlib.util.spec_from_file_location('prepare_assets', build.ROOT/'scripts/prepare-assets.py')
        assets = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(assets)

        def executable(path, magic, group=True, complete=True):
            data = bytearray(0x400)
            pe, root = 0x80, 0x200
            struct.pack_into('<I', data, 0x3c, pe)
            data[pe:pe + 4] = b'PE\0\0'
            struct.pack_into('<H', data, pe + 6, 1)
            optional = pe + 24
            size = 240 if magic == 0x20b else 224
            struct.pack_into('<H', data, pe + 20, size)
            struct.pack_into('<H', data, optional, magic)
            directory = optional + (112 if magic == 0x20b else 96)
            struct.pack_into('<I', data, directory + 16, 0x1000)
            struct.pack_into('<IIII', data, optional + size + 8, 0x200, 0x1000, 0x200, root)
            directories = [
                (0, [(3, 0x80000020)] + ([(14, 0x80000040)] if group else [])),
                (0x20, [(1, 0x80000060), (2, 0x80000080)]),
                (0x40, [(1, 0x800000a0)]),
                (0x60, [(1033, 0xc0)]), (0x80, [(1033, 0xd0)]), (0xa0, [(1033, 0xe0)]),
            ]
            for offset, entries in directories:
                struct.pack_into('<HH', data, root + offset + 12, 0, len(entries))
                for index, (name, target) in enumerate(entries):
                    struct.pack_into('<II', data, root + offset + 16 + index * 8, name, target)
            for offset, rva, length in ((0xc0, 0x1100, 2), (0xd0, 0x1102, 3), (0xe0, 0x1105, 1)):
                struct.pack_into('<II', data, root + offset, rva, length)
            data[root + 0x100:root + 0x106] = b'abcdeg' if complete else b'abxxxg'
            path.write_bytes(data)

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            icons = root/'resources/icons'
            icons.mkdir(parents=True)
            frames = [(2, 38), (3, 40)]
            ico = struct.pack('<HHH', 0, 1, 2)
            ico += b''.join(struct.pack('<BBBBHHII', 1, 1, 0, 0, 1, 32, size, offset)
                            for size, offset in frames) + b'abcde'
            (icons/'app.ico').write_bytes(ico)
            first, second, bad = root/'app.exe', root/'installer.exe', root/'bad.exe'
            executable(first, 0x20b)
            executable(second, 0x10b)
            with patch.object(assets, 'ROOT', root), patch('sys.stdout', io.StringIO()):
                assets.verify_icons([first, second])
                executable(bad, 0x20b, group=False)
                with self.assertRaises(AssertionError):
                    assets.verify_icons([first, bad])
                executable(bad, 0x10b, complete=False)
                with self.assertRaises(AssertionError):
                    assets.verify_icons([second, bad])

    def test_disk_image_retries_only_busy_errors_and_bounds_attempts(self):
        import macos_package as mac
        busy = subprocess.CompletedProcess([], 1, '', 'hdiutil: create failed - Resource busy\n')
        ok = subprocess.CompletedProcess([], 0, 'created\n', '')
        denied = subprocess.CompletedProcess([], 1, '', 'Permission denied\n')
        with patch.object(mac.time, 'sleep'), patch.object(mac.sys, 'stdout', io.StringIO()), patch.object(mac.sys, 'stderr', io.StringIO()):
            with patch.object(mac.subprocess, 'run', side_effect=[busy, ok]) as execute:
                mac.disk_image(['create', 'path with spaces.dmg'])
                self.assertEqual(execute.call_count, 2)
                self.assertEqual(execute.call_args.args[0][-1], 'path with spaces.dmg')
            with patch.object(mac.subprocess, 'run', return_value=denied) as execute:
                with self.assertRaises(subprocess.CalledProcessError):
                    mac.disk_image(['attach', 'package.dmg'])
                self.assertEqual(execute.call_count, 1)
            with patch.object(mac.subprocess, 'run', return_value=busy) as execute:
                with self.assertRaises(subprocess.CalledProcessError):
                    mac.disk_image(['detach', '/Volumes/test'])
                self.assertEqual(execute.call_count, 5)
            with patch.object(mac.time, 'monotonic', side_effect=[0, 2]), patch.object(mac.subprocess, 'run') as execute:
                with self.assertRaises(subprocess.TimeoutExpired):
                    mac.disk_image(['create', 'package.dmg'], timeout=1)
                execute.assert_not_called()

    def test_defender_update_waits_for_busy_installation_and_refreshes_binary(self):
        busy = subprocess.CompletedProcess([], 2, 'ERROR: hr=0x80070652\n')
        ok = subprocess.CompletedProcess([], 0, 'Signature update finished\n')
        old, new = Path('old/MpCmdRun.exe'), Path('new/MpCmdRun.exe')
        with patch.object(windows, 'defender_command', side_effect=[old, new, new]), \
             patch.object(windows.time, 'monotonic', return_value=0), \
             patch.object(windows.time, 'sleep') as sleep, \
             patch.object(windows.subprocess, 'run', side_effect=[busy, ok]) as execute, \
             patch('sys.stdout', io.StringIO()):
            self.assertEqual(windows.update_signatures(), new)
            sleep.assert_called_once_with(15)
            self.assertEqual([call.args[0] for call in execute.call_args_list], [
                [str(old), '-SignatureUpdate', '-MMPC'],
                [str(new), '-SignatureUpdate', '-MMPC'],
            ])

    def test_defender_update_retries_only_exact_busy_error_and_bounds_waits(self):
        with patch.object(windows, 'defender_command', return_value=Path('MpCmdRun.exe')), \
             patch.object(windows.time, 'monotonic', return_value=0), \
             patch.object(windows.time, 'sleep') as sleep, \
             patch('sys.stdout', io.StringIO()):
            for message in ('ERROR: hr=0x80070005', 'ERROR: hr=0x800706520', ''):
                with self.subTest(message=message), \
                     patch.object(windows.subprocess, 'run', return_value=subprocess.CompletedProcess([], 2, message)) as execute:
                    with self.assertRaises(subprocess.CalledProcessError):
                        windows.update_signatures()
                    execute.assert_called_once()
            sleep.assert_not_called()
            # Some native tools redirect UTF-16 text; the HRESULT must remain recognizable.
            busy = subprocess.CompletedProcess([], 2, '\0'.join('ERROR: 0X80070652'))
            with patch.object(windows.subprocess, 'run', return_value=busy) as execute:
                with self.assertRaises(subprocess.CalledProcessError):
                    windows.update_signatures()
                self.assertEqual(execute.call_count, 5)
                self.assertEqual([call.args[0] for call in sleep.call_args_list], [15, 30, 45, 60])

    def test_defender_update_preserves_total_timeout(self):
        with patch.object(windows, 'defender_command', return_value=Path('MpCmdRun.exe')), \
             patch.object(windows.time, 'monotonic', side_effect=[0, 2]), \
             patch.object(windows.subprocess, 'run') as execute:
            with self.assertRaises(subprocess.TimeoutExpired):
                windows.update_signatures(timeout=1)
            execute.assert_not_called()
        busy = subprocess.CompletedProcess([], 2, 'ERROR: hr=0x80070652')
        with patch.object(windows, 'defender_command', return_value=Path('MpCmdRun.exe')), \
             patch.object(windows.time, 'monotonic', side_effect=[0, 0, 295]), \
             patch.object(windows.time, 'sleep') as sleep, \
             patch.object(windows.subprocess, 'run', return_value=busy) as execute, \
             patch('sys.stdout', io.StringIO()):
            with self.assertRaises(subprocess.TimeoutExpired):
                windows.update_signatures()
            self.assertEqual(execute.call_args.kwargs['timeout'], 300)
            sleep.assert_not_called()

    def test_defender_failed_update_cannot_proceed_to_scan(self):
        with tempfile.TemporaryDirectory() as directory:
            installer = Path(directory)/'installer.exe'
            installer.write_bytes(b'installer fixture')
            with patch.object(windows, 'update_signatures', side_effect=subprocess.CalledProcessError(2, ['update'])), \
                 patch.object(windows.subprocess, 'run') as execute:
                with self.assertRaises(subprocess.CalledProcessError):
                    windows.scan_windows_files([installer])
                execute.assert_not_called()

    def test_defender_scan_blocks_threats_and_changed_files(self):
        with tempfile.TemporaryDirectory() as directory:
            installer = Path(directory)/'installer.exe'
            original = b'installer fixture'
            installer.write_bytes(original)
            versions = {'AMEngineVersion': '1.1.1', 'AMProductVersion': '4.18.1',
                        'AntivirusSignatureVersion': '1.2.3'}
            status = subprocess.CompletedProcess([], 0, json.dumps(versions))
            with patch.object(windows, 'update_signatures', return_value=Path('new/MpCmdRun.exe')), \
                 patch('sys.stdout', io.StringIO()):
                with patch.object(windows.subprocess, 'run', side_effect=[
                    status, subprocess.CalledProcessError(2, ['scan']),
                ]) as execute:
                    with self.assertRaises(subprocess.CalledProcessError):
                        windows.scan_windows_files([installer])
                    self.assertEqual(execute.call_args.args[0][0], str(Path('new/MpCmdRun.exe')))
                    self.assertIn('-DisableRemediation', execute.call_args.args[0])
                    self.assertTrue(execute.call_args.kwargs['check'])
                def changed(*args, **kwargs):
                    if args[0][0] == 'powershell.exe':
                        return status
                    installer.write_bytes(b'changed fixture')
                    return subprocess.CompletedProcess(args[0], 0)
                with patch.object(windows.subprocess, 'run', side_effect=changed):
                    with self.assertRaisesRegex(RuntimeError, 'File changed'):
                        windows.scan_windows_files([installer])

    def test_defender_scan_records_unpacked_executables_and_libraries(self):
        with tempfile.TemporaryDirectory() as directory:
            files = [Path(directory)/name for name in ('setup.exe', 'clash-of-rust.exe', 'wintun.dll')]
            for path in files:
                path.write_bytes(path.name.encode('ascii'))
            versions = {'AMEngineVersion': '1.1.1', 'AMProductVersion': '4.18.1',
                        'AntivirusSignatureVersion': '1.2.3'}
            with patch.object(windows, 'update_signatures', return_value=Path('MpCmdRun.exe')), \
                 patch('sys.stdout', io.StringIO()), \
                 patch.object(windows.subprocess, 'run', side_effect=[
                     subprocess.CompletedProcess([], 0, json.dumps(versions)),
                     *[subprocess.CompletedProcess([], 0) for _ in files],
                 ]) as execute:
                report = windows.scan_windows_files(files)
            self.assertEqual(report['versions'], versions)
            self.assertEqual(report['files'], [
                {'file': path.name, 'sha256': build.sha256(path)} for path in files
            ])
            self.assertEqual(execute.call_count, 4)
            scanned = [Path(call.args[0][call.args[0].index('-File')+1])
                       for call in execute.call_args_list[1:]]
            self.assertEqual(scanned, [path.resolve() for path in files])

    def test_defender_scan_requires_version_evidence_before_scanning(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'program.exe'
            path.write_bytes(b'fixture')
            with patch.object(windows, 'update_signatures', return_value=Path('MpCmdRun.exe')), \
                 patch('sys.stdout', io.StringIO()), \
                 patch.object(windows.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, '{}')) as execute:
                with self.assertRaisesRegex(RuntimeError, 'version information'):
                    windows.scan_windows_files([path])
                self.assertEqual(execute.call_count, 1)

    def test_defender_status_timeout_retries_but_never_skips_the_scan(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'program.exe'
            path.write_bytes(b'fixture')
            versions = {'AMEngineVersion': '1.1.1', 'AMProductVersion': '4.18.1',
                        'AntivirusSignatureVersion': '1.2.3'}
            timeout = subprocess.TimeoutExpired(['powershell.exe'], 120)
            with patch.object(windows, 'update_signatures', return_value=Path('MpCmdRun.exe')), \
                 patch.object(windows.time, 'sleep') as delay, \
                 patch.object(windows.subprocess, 'run', side_effect=[
                     timeout, subprocess.CompletedProcess([], 0, json.dumps(versions)),
                     subprocess.CompletedProcess([], 0),
                 ]) as execute:
                report = windows.scan_windows_files([path])
                self.assertEqual(report['versions'], versions)
                self.assertEqual(execute.call_count, 3)
                self.assertIn('-Scan', execute.call_args.args[0])
                delay.assert_called_once_with(5)
            with patch.object(windows, 'update_signatures', return_value=Path('MpCmdRun.exe')), \
                 patch.object(windows.time, 'sleep'), \
                 patch.object(windows.subprocess, 'run', side_effect=timeout) as execute:
                with self.assertRaises(subprocess.TimeoutExpired):
                    windows.scan_windows_files([path])
                self.assertEqual(execute.call_count, 2)

    def test_windows_package_scans_payload_before_installing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            payload = root/'build/payload'
            (payload/'resources').mkdir(parents=True)
            names = ['clash-of-rust.exe', 'clash-desktop-ui.exe', 'cor.exe',
                     'clash-tun-launcher.exe', 'resources/mihomo.exe', 'resources/wintun.dll']
            for name in [*names, 'resources/manifest.json']:
                (payload/name).write_bytes(b'fixture')
            package, smoke = root/'setup.exe', root/'build/tests/core_integration.exe'
            evidence = {'versions': {}, 'files': []}
            with patch.object(windows, 'os') as platform, \
                 patch.object(windows, 'ROOT', root), \
                 patch.object(windows, 'scan_windows_files', return_value=evidence) as scan, \
                 patch.object(windows, '_verify_installed_package') as verify:
                platform.name = 'nt'
                manifest = {'tests': {'core_integration': 'tests/core_integration.exe'}}
                package.write_bytes(b'installer')
                smoke.parent.mkdir(parents=True)
                smoke.write_bytes(b'test fixture')
                self.assertEqual(windows.package_tests(package, manifest, root/'build'), evidence)
                self.assertEqual(scan.call_args.args[0], [package, *sorted(payload/name for name in names)])
                verify.assert_called_once()
                verify.reset_mock()
                scan.side_effect = subprocess.CalledProcessError(2, ['scan'])
                with self.assertRaises(subprocess.CalledProcessError):
                    windows.package_tests(package, manifest, root/'build')
                verify.assert_not_called()

    def test_linux_helpers_are_distinct_inactive_native_copies(self):
        spec = importlib.util.spec_from_file_location('build_installer',build.ROOT/'scripts/build-installer.py')
        installer = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(installer)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root/'clash-tun-launcher'
            header = bytearray(64)
            header[:6] = b'\x7fELF\x02\x01'
            struct.pack_into('<H',header,18,62)
            source.write_bytes(header)
            stage = root/'stage'
            stage.mkdir()
            installer.stage_linux_helpers(root/'clash-of-rust',stage,'x64')
            for path in (stage/'clash-tun-launcher',stage/'libexec/resolvectl'):
                self.assertEqual(path.read_bytes(),source.read_bytes())
                self.assertFalse(path.samefile(source))
                if __import__('os').name != 'nt':
                    self.assertEqual(path.stat().st_mode & 0o7777,0o755)
            self.assertFalse((stage/'clash-tun-launcher').samefile(stage/'libexec/resolvectl'))

    def test_linux_tun_authorization_requires_an_admin_in_every_session(self):
        import xml.etree.ElementTree as xml
        policy = xml.parse(build.ROOT/'resources/linux/org.clashofrust.tun.policy')
        actions = policy.findall('action')
        self.assertEqual(len(actions), 1)
        action = actions[0]
        for session in ('allow_any', 'allow_inactive', 'allow_active'):
            self.assertEqual(action.findtext('defaults/'+session), 'auth_admin')
        annotations = {item.attrib['key']: item.text for item in action.findall('annotate')}
        self.assertEqual(annotations['org.freedesktop.policykit.exec.path'], '/opt/clash-of-rust/clash-of-rust')
        self.assertEqual(annotations['org.freedesktop.policykit.exec.argv1'], '--authorize-tun')
        self.assertNotIn('org.freedesktop.policykit.exec.allow_gui', annotations)

    def test_linux_package_validation_requires_the_terminal_agent_and_pkexec(self):
        for missing in (None, 'pkexec', 'pkttyagent'):
            with self.subTest(missing=missing), \
                    patch.object(release_package.shutil, 'which',
                                 side_effect=lambda name: None if name == missing else '/usr/bin/' + name):
                if missing:
                    with self.assertRaisesRegex(RuntimeError, missing):
                        release_package.require_linux_polkit_tools()
                else:
                    release_package.require_linux_polkit_tools()

    def test_arm64_bundles_are_separate_and_native_architecture_is_detected(self):
        self.assertEqual(build.bundle_directory('windows','x64'),build.ROOT/'bundle')
        self.assertEqual(build.bundle_directory('windows','arm64'),build.ROOT/'bundle/windows-arm64')
        self.assertEqual(build.bundle_directory('linux','arm64'),build.ROOT/'bundle/linux-arm64')
        for machine, expected in [('AMD64','x64'),('x86_64','x64'),('ARM64','arm64'),('aarch64','arm64')]:
            with patch.object(build.platform,'machine',return_value=machine):
                self.assertEqual(build.host_arch(),expected)
        with patch.object(build.platform,'machine',return_value='armv7l'):
            with self.assertRaises(ValueError):
                build.host_arch()

    def test_package_rejects_wrong_binary_architecture(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder)/'binary'
            for system, arch, machine in [('windows','x64',0x8664),('windows','arm64',0xaa64),('linux','x64',62),('linux','arm64',183),('macos','x64',0x01000007),('macos','arm64',0x0100000c)]:
                header = bytearray(64)
                if system == 'windows':
                    header[:2] = b'MZ'
                    struct.pack_into('<I',header,0x3c,64)
                    header.extend(b'PE\0\0'+struct.pack('<H',machine))
                elif system == 'macos':
                    header[:4] = b'\xcf\xfa\xed\xfe'
                    struct.pack_into('<I',header,4,machine)
                else:
                    header[:6] = b'\x7fELF\x02\x01'
                    struct.pack_into('<H',header,18,machine)
                path.write_bytes(header)
                build.validate_binary_arch(path,system,arch)
                with self.assertRaises(ValueError):
                    build.validate_binary_arch(path,system,'arm64' if arch == 'x64' else 'x64')
            path.write_bytes(b'not an executable')
            with self.assertRaises(ValueError):
                build.validate_binary_arch(path,'windows','arm64')

    def test_version_validation(self):
        self.assertEqual(build.validate_version('0.4.4'),'0.4.4')
        for invalid in ('1.2','1.2.3.4','1.2.x','1.2.65536','../0.4.4','01.2.3','0.4.12-dev.01','0.4.12-','0.4.12-dev/1',
                        '0.6.5-dev:fake-ip-sources','0.6.5-fix:android-protection'):
            with self.assertRaises(ValueError):
                build.validate_version(invalid)

    def test_prerelease_installer_versions_preserve_semver_and_upgrade_order(self):
        self.assertEqual(build.deb_version('0.4.12'),'0.4.12')
        for version, stable, previous in [('0.4.12-dev.1','0.4.12','0.4.11'),
                                          ('0.6.5-dev.1.fake-ip-sources','0.6.5','0.6.4'),
                                          ('0.6.5-fix.1.android-protection','0.6.5','0.6.4')]:
            with self.subTest(version=version):
                self.assertEqual(build.validate_version(version),version)
                self.assertEqual(build.numeric_version(version),stable+'.0')
                self.assertEqual(build.deb_version(version),version.replace('-','~',1))
                if shutil.which('dpkg'):
                    subprocess.run(['dpkg','--compare-versions',build.deb_version(version),'lt',stable],check=True)
                    subprocess.run(['dpkg','--compare-versions',build.deb_version(version),'gt',previous],check=True)

    def test_build_token_is_sent_only_to_secure_github_api_and_removed_on_redirect(self):
        import io
        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self,*_): self.close()
        class Opener:
            def open(self,request,**kwargs):
                self.request = request
                return Response(b'{"ok":true}')
        mocked = Opener()
        with patch.dict(build.os.environ, {'GH_TOKEN':'test-only-token'}), patch.object(build,'opener',return_value=mocked):
            for url, expected in [('https://api.github.com/repos/example/repo', 'Bearer test-only-token'),
                                  ('https://example.test/data', None), ('http://api.github.com/data', None)]:
                self.assertEqual(build.read_json(url), {'ok':True})
                self.assertEqual(mocked.request.get_header('Authorization'), expected)
        source = build.urllib.request.Request('https://api.github.com/data', headers={'Authorization':'Bearer test-only-token'})
        for destination in ('https://example.test/data', 'http://api.github.com/data'):
            redirected = build.SafeRedirect().redirect_request(source,None,302,'Found',{},destination)
            self.assertIsNone(redirected.get_header('Authorization'))

    def resources(self,source):
        source.mkdir()
        (source/'mihomo.exe').write_bytes(b'core')
        build.write_json(source/'core.json',{'version':'v1.20.1','exe_sha256':build.sha256(source/'mihomo.exe')})
        records = []
        for name in build.GEO_NAMES:
            path = source/name
            path.write_bytes(name.encode())
            records.append({'name':name,'size':path.stat().st_size,'sha256':build.sha256(path)})
        build.write_json(source/'geodata.json',{'files':records})

    def test_staging_excludes_old_runtime_and_checks_all_payload_hashes(self):
        with tempfile.TemporaryDirectory() as folder:
            base = Path(folder)
            source = base/'resources'
            self.resources(source)
            stale = source/'ip-check/runtime/usr/bin'
            stale.mkdir(parents=True)
            (stale/'bash.exe').write_bytes(b'stale tool')
            stage = base/'stage'
            build.stage_resources(source,stage,'windows')
            self.assertEqual({p.name for p in (stage/'resources/ip-check').iterdir()},{'LICENSE','SOURCE.md'})
            notice = (stage/'resources/THIRD-PARTY-NOTICES.txt').read_text()
            self.assertIn('Version: v1.20.1. Distributed under GPL-3.0.', notice)
            self.assertIn('https://github.com/MetaCubeX/mihomo/tree/v1.20.1', notice)
            (source/'GeoIP.dat').write_bytes(b'corruption')
            with self.assertRaises(ValueError):
                build.validate_resources(source,'windows')

    def test_android_notice_uses_resolved_go_version_and_pseudo_revision(self):
        with tempfile.TemporaryDirectory() as folder:
            module = Path(folder)/'go.mod'
            notice = Path(folder)/'notice.txt'
            module.write_text('module example.test\nrequire (\n github.com/metacubex/mihomo v1.2.4-0.20261029120000-123456789abc\n)\n')
            build.write_mihomo_notice(notice, build.mihomo_go_version(module))
            self.assertIn('Version: v1.2.4-0.20261029120000-123456789abc.', notice.read_text())
            self.assertIn('/mihomo/tree/123456789abc', notice.read_text())
            module.write_text('require github.com/metacubex/mihomo v1.2.3\nreplace github.com/metacubex/mihomo => ../local\n')
            with self.assertRaisesRegex(ValueError, 'explicit source notice'):
                build.mihomo_go_version(module)
            with self.assertRaisesRegex(ValueError, 'Invalid mihomo version'):
                build.write_mihomo_notice(notice, '../invalid')

    def test_manifest_rejects_unexpected_paths(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder)/'resources'
            self.resources(source)
            value = json.loads((source/'geodata.json').read_text())
            value['files'][0]['name'] = '../outside'
            build.write_json(source/'geodata.json',value)
            with self.assertRaises(ValueError):
                build.validate_resources(source,'windows')

    def test_verified_download_rejects_corruption_without_replacing_good_file(self):
        import io
        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self,*_): self.close()
        class Opener:
            def open(self,*args,**kwargs): return Response(b'bad')
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder)/'asset'
            path.write_bytes(b'previous')
            with patch.object(build,'opener',return_value=Opener()), patch.object(build.time,'sleep'):
                with self.assertRaises(ValueError):
                    build.download('https://example.test/asset',path,digest=hashlib.sha256(b'expected').hexdigest())
            self.assertEqual(path.read_bytes(),b'previous')
            self.assertFalse(path.with_name('asset.partial').exists())

    def test_windows_pe_version_is_read_without_executing_binary(self):
        import os
        if os.name != 'nt':
            self.skipTest('Windows version API')
        binary = build.ROOT/'target/release/clash-of-rust.exe'
        if not binary.is_file():
            self.skipTest('Application binary not prepared')
        version = build.file_version(binary)
        self.assertEqual(build.validate_version(version),version)


class CompatibilityTests(unittest.TestCase):
    source_commit = 'a' * 40
    repository = 'example/project'

    def client(self, suite, run_changes=None, jobs=None, receipt_changes=None,
               bad_digest=False, expired=False, missing_input=False):
        source = {'id': 123, 'path': '.github/workflows/ci.yml', 'head_sha': self.source_commit,
                  'repository': {'full_name': self.repository},
                  'head_repository': {'full_name': self.repository},
                  'status': 'completed', 'conclusion': 'success', 'event': 'push',
                  'head_branch': 'main'}
        run = source | {'id': 456, 'event': 'workflow_dispatch', 'run_attempt': 2,
                        'head_branch': 'different-ref',
                        'path': '.github/workflows/' + compatibility.WORKFLOWS[suite]}
        run.update(run_changes or {})
        inputs = [{'id': number, 'name': name, 'digest': 'sha256:' + 'b' * 64,
                   'expired': False, 'workflow_run': {'id': 123, 'head_sha': self.source_commit}}
                  for number, name in enumerate(compatibility.input_names(suite), 1)]
        receipt = {'schema': 1, 'suite': suite, 'commit': self.source_commit,
                   'ci_run_id': 123, 'run_id': 456, 'run_attempt': 2,
                   'checks': ['system_proxy', 'tun'],
                   'inputs': [{key: item[key] for key in ('id', 'name', 'digest')} for item in inputs]}
        receipt.update(receipt_changes or {})
        stream = io.BytesIO()
        with zipfile.ZipFile(stream, 'w') as archive:
            archive.writestr('compatibility.json', json.dumps(receipt))
        blob = stream.getvalue()
        artifact = {'id': 789, 'name': compatibility.report_name(suite, 2), 'expired': expired,
                    'size_in_bytes': len(blob),
                    'digest': 'sha256:' + ('0' * 64 if bad_digest else hashlib.sha256(blob).hexdigest()),
                    'workflow_run': {'id': 456, 'head_sha': self.source_commit}}
        records = jobs if jobs is not None else [
            {'name': name, 'status': 'completed', 'conclusion': 'success'}
            for name in compatibility.required_jobs(suite) | {compatibility.REPORT_JOBS[suite]}]

        def request(path, raw=False):
            if path.startswith('/actions/workflows/'):
                return {'workflow_runs': [run]}
            if path == '/actions/runs/123':
                return source
            if path == '/actions/runs/456':
                return run
            if path.startswith('/actions/runs/123/artifacts?'):
                return {'artifacts': inputs[:-1] if missing_input else inputs}
            if path.startswith('/actions/runs/456/artifacts?'):
                return {'artifacts': [artifact]}
            if '/jobs?' in path:
                return {'jobs': records, 'total_count': len(records)}
            if path == '/actions/artifacts/789/zip' and raw:
                return blob
            self.fail('Unexpected compatibility request: ' + path)
        request.repo = self.repository
        return request, source, receipt

    def test_release_accepts_only_complete_same_source_compatibility_receipts(self):
        for suite in compatibility.WORKFLOWS:
            client, source, _ = self.client(suite)
            self.assertEqual(compatibility.verified_suite(client, source, suite), 456)
            for change in ({'status': 'in_progress'}, {'conclusion': 'failure'},
                           {'head_sha': 'b' * 40}, {'event': 'pull_request'},
                           {'head_repository': {'full_name': 'external/fork'}}):
                with self.subTest(suite=suite, change=change), self.assertRaises(ValueError):
                    selected, source, _ = self.client(suite, run_changes=change)
                    compatibility.verified_suite(selected, source, suite, 456)

    def test_receipt_rejects_expiry_corruption_missing_inputs_and_stale_identity(self):
        for suite in compatibility.WORKFLOWS:
            for changes in ({'bad_digest': True}, {'expired': True}, {'missing_input': True},
                            {'receipt_changes': {'ci_run_id': 122}},
                            {'receipt_changes': {'run_attempt': 1}},
                            {'receipt_changes': {'schema': True}},
                            {'receipt_changes': {'checks': ['system_proxy']}},
                            {'receipt_changes': {'inputs': []}},
                            {'receipt_changes': {'commit': 'b' * 40}}):
                with self.subTest(suite=suite, changes=changes), self.assertRaises(ValueError):
                    client, source, _ = self.client(suite, **changes)
                    compatibility.verified_suite(client, source, suite)

    def test_compatibility_never_accepts_missing_skipped_or_failed_jobs(self):
        for suite in compatibility.WORKFLOWS:
            jobs = [{'name': name, 'status': 'completed', 'conclusion': 'success'}
                    for name in sorted(compatibility.required_jobs(suite)
                                       | {compatibility.REPORT_JOBS[suite]})]
            for selected in (jobs[:-1],
                             [jobs[0] | {'conclusion': 'skipped'}, *jobs[1:]],
                             [jobs[0] | {'conclusion': 'failure'}, *jobs[1:]]):
                with self.subTest(suite=suite, jobs=selected), self.assertRaises(ValueError):
                    client, source, _ = self.client(suite, jobs=selected)
                    compatibility.verified_suite(client, source, suite)

    def test_record_accepts_its_own_running_job_and_binds_exact_ci_inputs(self):
        for suite in compatibility.WORKFLOWS:
            jobs = [{'name': name, 'status': 'completed', 'conclusion': 'success'}
                    for name in compatibility.required_jobs(suite)]
            jobs.append({'name': compatibility.REPORT_JOBS[suite],
                         'status': 'in_progress', 'conclusion': None})
            client, _, receipt = self.client(suite, jobs=jobs)
            with tempfile.TemporaryDirectory() as temporary, \
                    patch.object(compatibility, 'commit', return_value=self.source_commit), \
                    patch.dict(os.environ, {'GITHUB_RUN_ID': '456', 'GITHUB_RUN_ATTEMPT': '2',
                                            'GITHUB_STEP_SUMMARY': ''}):
                output = Path(temporary) / 'compatibility.json'
                compatibility.record(client, suite, 123, output)
                self.assertEqual(json.loads(output.read_text()), receipt)

    def test_workflow_jobs_and_inputs_cover_all_android_versions_and_both_ubuntu_architectures(self):
        self.assertEqual(len(compatibility.required_jobs('android')), 12)
        self.assertEqual(len(compatibility.required_jobs('ubuntu')), 3)
        for suite in compatibility.WORKFLOWS:
            workflow = (build.ROOT / '.github/workflows' / compatibility.WORKFLOWS[suite]).read_text()
            self.assertIn('name: ' + compatibility.REPORT_JOBS[suite], workflow)
            self.assertIn(f'name: {suite}-compatibility-attempt-', workflow)
            self.assertIn('COR_CI_EXTENDED_TESTS:', workflow)
            self.assertNotIn('pull_request_target:', workflow)
        workflow = (build.ROOT / '.github/workflows/ci.yml').read_text()
        self.assertNotIn('  ubuntu_20:', workflow)
        self.assertEqual(len(release.CI_JOBS), 12)


class ReleaseTests(unittest.TestCase):
    commit = 'a' * 40
    version = '0.4.11'
    tag = 'v0.4.11'
    notes = 'Reviewed release notes\n'

    def setUp(self):
        printing = patch('builtins.print')
        printing.start()
        self.addCleanup(printing.stop)

    def client(self, run_changes=None, jobs=None, main=None):
        run = {'id':77, 'path':'.github/workflows/ci.yml@main', 'event':'push', 'head_branch':'main',
               'head_repository':{'full_name':'example/project'}, 'head_sha':self.commit,
               'status':'completed', 'conclusion':'success'}
        run.update(run_changes or {})
        jobs = jobs if jobs is not None else [{'name':name,'status':'completed','conclusion':'success'} for name in release.CI_JOBS]
        def request(path, *args, **kwargs):
            if path.startswith('/actions/workflows/ci.yml/runs?'):return {'workflow_runs':[run]}
            if path == '/actions/runs/77':return run
            if path.startswith('/actions/runs/77/attempts/') and '/jobs?' in path:return {'jobs':jobs,'total_count':len(jobs)}
            if path.startswith('/branches/'):return {'commit':{'sha':main or self.commit}}
            self.fail('Unexpected CI request: '+path)
        request.repo = 'example/project'
        return request

    def files(self, directory):
        for suffix in release.PACKAGES.values():
            path = directory/f'Clash-of-Rust-{self.version}-{suffix}'
            path.write_bytes(('installer '+suffix).encode())
            path.with_name(path.name+'.sha256').write_text(build.sha256(path)+'\n')
        return release.validate_files(list(directory.iterdir()), self.version)

    def test_release_environment_tokens_do_not_require_git_credential_helper(self):
        for key in ('GH_TOKEN','GITHUB_TOKEN'):
            with patch.dict(build.os.environ,{key:'test-only-token'},clear=True), patch.object(release.subprocess,'run') as git:
                self.assertEqual(release.credential('example/project'),'test-only-token')
                git.assert_not_called()
        with self.assertRaises(ValueError):
            release.GitHub('example/project','test-only-token')('https://example.test/upload','POST',b'installer')

    def test_release_artifact_get_recovers_from_dns_and_interrupted_download(self):
        reply = MagicMock()
        reply.read.side_effect = release.http.client.IncompleteRead(b'partial')
        transport = MagicMock()
        transport.open.side_effect = [
            urllib.error.URLError('temporary DNS failure'),
            nullcontext(reply),
            nullcontext(io.BytesIO(b'complete verified archive')),
        ]
        with patch.object(release, 'opener', return_value=transport), patch.object(release.time, 'sleep') as sleep:
            data = release.GitHub('example/project','test-only-token')('/actions/artifacts/1/zip',raw=True)
        self.assertEqual(data,b'complete verified archive')
        self.assertEqual(transport.open.call_count,3)
        self.assertEqual([call.args[0] for call in sleep.call_args_list],[1,2])

    def test_release_metadata_does_not_reuse_a_stale_jobs_snapshot(self):
        current = {'jobs':[{'status':'in_progress','conclusion':None}]}
        cached = {}
        queries = []
        def response(request, **kwargs):
            queries.append(urllib.parse.parse_qs(urllib.parse.urlparse(request.full_url).query))
            body = cached.setdefault(request.full_url,json.dumps(current).encode())
            return nullcontext(io.BytesIO(body))
        transport = MagicMock()
        transport.open.side_effect = response
        with patch.object(release,'opener',return_value=transport), patch.object(release.time,'time_ns',side_effect=[1,2]):
            client = release.GitHub('example/project','test-only-token')
            path = '/actions/runs/77/jobs?filter=latest&per_page=100'
            self.assertEqual(client(path)['jobs'][0]['status'],'in_progress')
            current['jobs'][0].update(status='completed',conclusion='success')
            self.assertEqual(client(path)['jobs'][0]['conclusion'],'success')
        self.assertEqual(len(cached),2)
        for query in queries:
            self.assertEqual(query['filter'],['latest'])
            self.assertEqual(query['per_page'],['100'])

    def test_release_reads_attempt_jobs_and_keeps_the_newest_result_after_partial_reruns(self):
        previous = [{'name':name,'status':'completed','conclusion':'success'} for name in release.CI_JOBS]
        attempts = {1:previous, 2:[{'name':'macos-arm64','status':'completed','conclusion':'failure'}]}
        def client(path):
            match = re.fullmatch(r'/actions/runs/77/attempts/([12])/jobs\?per_page=100&page=1',path)
            self.assertIsNotNone(match, 'Must not depend on the incomplete generic jobs index')
            jobs = attempts[int(match.group(1))]
            return {'jobs':jobs,'total_count':len(jobs)}
        jobs = {j['name']:j for j in release.workflow_jobs(client,{'id':77,'run_attempt':2})}
        self.assertEqual(set(jobs),release.CI_JOBS)
        self.assertEqual(jobs['macos-arm64']['conclusion'],'failure')
        self.assertEqual(jobs['linux-x64']['conclusion'],'success')
        attempts[2][0]['conclusion']='success'
        self.assertTrue(all(j['conclusion']=='success' for j in release.workflow_jobs(client,{'id':77,'run_attempt':2})))

    def test_release_attempt_job_pages_must_be_complete_and_unique(self):
        records = [{'name':f'job-{index}'} for index in range(101)]
        def client(path):
            page = int(urllib.parse.parse_qs(urllib.parse.urlparse(path).query)['page'][0])
            return {'jobs':records[(page-1)*100:page*100],'total_count':len(records)}
        self.assertEqual(len(release.workflow_jobs(client,{'id':77})),101)
        records[-1]['name']=records[0]['name']
        with self.assertRaisesRegex(ValueError,'Duplicate'):
            release.workflow_jobs(client,{'id':77})
        with self.assertRaisesRegex(ValueError,'incomplete'):
            release.workflow_jobs(lambda _: {'jobs':[],'total_count':1},{'id':77})

    def test_release_partial_rerun_stops_before_obsolete_skipped_matrix_placeholders(self):
        records = {
            2:[{'name':name,'status':'completed','conclusion':'success'} for name in release.CI_JOBS],
            3:[{'name':'macos-arm64','status':'completed','conclusion':'success'}],
        }
        queried = []
        def client(path):
            number = int(path.split('/attempts/',1)[1].split('/',1)[0])
            queried.append(number)
            self.assertIn(number,records, 'Older failed-input attempts must not reintroduce matrix placeholders')
            jobs = records[number]
            return {'jobs':jobs,'total_count':len(jobs)}
        jobs = release.workflow_jobs(client,{'id':77,'run_attempt':3},release.CI_JOBS)
        self.assertEqual({job['name'] for job in jobs},release.CI_JOBS)
        self.assertEqual(queried,[3,2])

    def test_release_refresh_preserves_artifact_and_mutation_urls(self):
        for path,method,data,raw in [
            ('/actions/artifacts/1/zip?archive=verified','GET',None,True),
            ('https://uploads.github.com/repos/example/project/releases/1/assets?name=package','POST',b'package',False),
            ('/releases/1','PATCH',{'draft':False},False),
        ]:
            with self.subTest(path=path):
                transport = MagicMock()
                transport.open.return_value = nullcontext(io.BytesIO(b'archive' if raw else b'{}'))
                with patch.object(release,'opener',return_value=transport):
                    release.GitHub('example/project','test-only-token')(path,method,data,raw=raw)
                request = transport.open.call_args.args[0]
                expected = 'https://api.github.com/repos/example/project'+path if path.startswith('/') else path
                self.assertEqual(request.full_url,expected)
                self.assertEqual(request.get_method(),method)

    def test_release_get_retries_only_temporary_http_errors(self):
        for status in (403,404,429,503):
            with self.subTest(status=status):
                error = urllib.error.HTTPError('https://api.github.com/test',status,'HTTP failure',{},None)
                transport = MagicMock()
                transport.open.side_effect = [error,nullcontext(io.BytesIO(b'{"ok":true}'))]
                with patch.object(release, 'opener', return_value=transport), patch.object(release.time, 'sleep') as sleep:
                    client = release.GitHub('example/project','test-only-token')
                    if status in (403,404):
                        with self.assertRaises(urllib.error.HTTPError): client('/test')
                        self.assertEqual(transport.open.call_count,1)
                        sleep.assert_not_called()
                    else:
                        self.assertEqual(client('/test'),{'ok':True})
                        self.assertEqual(transport.open.call_count,2)
                        sleep.assert_called_once_with(1)

    def test_release_network_retries_are_bounded_and_never_replay_mutations(self):
        for method, attempts in [('GET',3),('POST',1),('PATCH',1),('DELETE',1)]:
            with self.subTest(method=method):
                transport = MagicMock()
                transport.open.side_effect = urllib.error.URLError('offline')
                with patch.object(release, 'opener', return_value=transport), patch.object(release.time, 'sleep'):
                    with self.assertRaises(urllib.error.URLError):
                        release.GitHub('example/project','test-only-token')('/test',method=method)
                self.assertEqual(transport.open.call_count,attempts)

    def test_release_check_mode_never_calls_publisher(self):
        with patch.object(release.sys,'argv',['publish-release.py','--check','--run-id','77','--package-run-id','88']), \
             patch.object(release,'repository',return_value='example/project'), \
             patch.object(release,'credential',return_value='test-only-token'), \
             patch.object(release,'GitHub'), patch.object(release,'git',return_value=self.commit), \
             patch.object(release,'verified_run',return_value={'id':77,'head_branch':'main','html_url':'https://example.test/ci'}), \
             patch.object(compatibility,'verified_suite'), \
             patch.object(release,'release_metadata',return_value=(self.version,self.tag,self.notes)), \
             patch.object(release,'verified_package_run'), patch.object(release,'download_package_files',return_value={}), patch.object(release,'publish') as publishing:
            release.main()
            publishing.assert_not_called()

    def test_release_requires_current_main_push_ci_and_every_job(self):
        self.assertEqual(release.verified_run(self.client(),self.commit)['id'],77)
        for changes in ({'event':'pull_request'}, {'path':'.github/workflows/release.yml'}, {'head_branch':'other'},
                        {'head_sha':'b'*40}, {'head_repository':{'full_name':'other/project'}},
                        {'status':'in_progress'}, {'conclusion':'failure'}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                release.verified_run(self.client(changes),self.commit,77)
        jobs = [{'name':name,'status':'completed','conclusion':'success'} for name in release.CI_JOBS]
        with self.assertRaises(ValueError):release.verified_run(self.client(jobs=jobs[:-1]),self.commit,77)
        jobs[0]['conclusion']='skipped'
        with self.assertRaises(ValueError):release.verified_run(self.client(jobs=jobs),self.commit,77)
        with self.assertRaises(ValueError):release.verified_run(self.client(main='b'*40),self.commit,77)

    def test_release_allows_skipped_optional_matrix_without_omitting_required_jobs(self):
        required = [{'name':name,'status':'completed','conclusion':'success'} for name in release.CI_JOBS]
        optional = {'name':'Android 16 IME','status':'completed','conclusion':'skipped'}
        changes = {'event':'pull_request','head_branch':'codex/fix-android'}
        for mode in ({'check_only':True}, {'prerelease':True}):
            with self.subTest(mode=mode):
                self.assertEqual(release.verified_run(self.client(changes,jobs=required+[optional]),
                                                     self.commit,77,**mode)['id'],77)
                with self.assertRaisesRegex(ValueError,'does not cover'):
                    release.verified_run(self.client(changes,jobs=required[:-1]+[optional]),
                                         self.commit,77,**mode)
                skipped = [dict(job) for job in required]
                skipped[0]['conclusion'] = 'skipped'
                with self.assertRaisesRegex(ValueError,'did not pass'):
                    release.verified_run(self.client(changes,jobs=skipped+[optional]),
                                         self.commit,77,**mode)
        for conclusion in ['failure','cancelled','success']:
            with self.subTest(conclusion=conclusion), self.assertRaises(ValueError):
                release.verified_run(self.client(jobs=required+[optional|{'conclusion':conclusion}]),
                                     self.commit,77)

    def test_preview_packaging_accepts_current_same_repository_pr_ci_only_in_check_mode(self):
        changes = {'event':'pull_request', 'head_branch':'codex/dev-0.5.2'}
        with self.assertRaises(ValueError):
            release.verified_run(self.client(changes),self.commit,77)
        self.assertEqual(release.verified_run(self.client(changes),self.commit,77,check_only=True)['id'],77)
        for invalid in ({'head_repository':{'full_name':'fork/project'}}, {'head_sha':'b'*40},
                        {'status':'in_progress'}, {'conclusion':'failure'}, {'event':'workflow_dispatch'}):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                release.verified_run(self.client(changes | invalid),self.commit,77,check_only=True)
        with self.assertRaises(ValueError):
            release.verified_run(self.client(changes, main='b'*40),self.commit,77,check_only=True)
        with self.assertRaises(ValueError):
            release.verified_run(self.client(changes,jobs=[]),self.commit,77,check_only=True)

    def test_release_version_and_notes_are_read_from_tested_commit(self):
        with patch.object(release,'git',return_value='[package]\nversion="0.4.11"'), patch.object(release.subprocess,'check_output',return_value=self.notes):
            self.assertEqual(release.release_metadata(self.commit),(self.version,self.tag,self.notes))
            with self.assertRaises(ValueError):release.release_metadata(self.commit,'v0.4.12')
            with tempfile.TemporaryDirectory() as folder:
                path=Path(folder)/'notes.md';path.write_text('Uncommitted notes')
                with self.assertRaises(ValueError):release.release_metadata(self.commit,notes_path=path)

    def test_download_badges_target_each_asset_for_stable_and_prerelease(self):
        for version in ['0.6.6', '0.6.7-fix.1.example']:
            with self.subTest(version=version):
                tag = 'v' + version
                notes = release.release_notes_with_downloads(self.notes, 'example/project', version, tag)
                self.assertTrue(notes.startswith(self.notes.rstrip()))
                rows = [line.split('|')[1].strip() for line in notes.splitlines()
                        if line.startswith('| ')][2:]
                self.assertEqual(rows, ['Windows', 'Linux', 'macOS', 'Android'])
                self.assertEqual(notes.count('（多数手机和平板）'), 1)
                self.assertRegex(notes, r'\[!\[Android ARM64 downloads\]\([^)]+\)\]'
                                        r'\([^)]+\) （多数手机和平板）<br>')
                self.assertEqual(notes.count('https://img.shields.io/badge/'), len(release.PACKAGES))
                self.assertEqual(notes.count('https://img.shields.io/github/downloads/'), len(release.PACKAGES))
                self.assertEqual(notes.count('displayAssetName=false'), len(release.PACKAGES))
                buttons = re.findall(r'\[!\[([^]]+)\]\(([^)]+)\)\]\(([^)]+)\)', notes)
                self.assertEqual(len(buttons), 2 * len(release.PACKAGES))
                downloads = []
                for label, image_url, download_url in buttons:
                    image = urllib.parse.urlparse(image_url)
                    download = urllib.parse.urlparse(download_url)
                    self.assertEqual(image.netloc, 'img.shields.io')
                    self.assertEqual(download.netloc, 'github.com')
                    self.assertEqual(urllib.parse.unquote(download.path).split('/')[-2], tag)
                    if label.endswith(' downloads'):
                        self.assertEqual(urllib.parse.parse_qs(image.query),
                                         {'label': ['downloads'], 'displayAssetName': ['false']})
                        self.assertEqual(urllib.parse.unquote(image.path).split('/')[-2], tag)
                        self.assertEqual(image.path.split('/')[-1], download.path.split('/')[-1])
                    downloads.append(urllib.parse.unquote(download.path).split('/')[-1])
                self.assertEqual(set(downloads),
                                 {'Clash-of-Rust-' + version + '-' + suffix for suffix in release.PACKAGES.values()})
                for suffix in release.PACKAGES.values():
                    filename = 'Clash-of-Rust-' + version + '-' + suffix
                    self.assertEqual(downloads.count(filename), 2)
                self.assertNotIn('/downloads-pre/', notes)
                self.assertNotIn('/latest/', notes)
                self.assertNotIn('下载次数包含重复下载', notes)
                self.assertNotIn('SHA256 校验文件位于下方 Assets', notes)
        with self.assertRaises(ValueError):
            release.release_notes_with_downloads(self.notes, 'example/project', '0.6.6', 'v0.6.5')

    def test_download_table_precedes_long_release_details(self):
        version = '0.6.7-fix.7.settings-storage'
        details = '## 更新\n\n' + '完整保留更新说明。\n' * 2000
        notes = '# ' + version + '\n\n' + details
        result = release.release_notes_with_downloads(notes, 'example/project', version, 'v' + version)
        self.assertTrue(result.startswith('# ' + version + '\n\n## 下载\n'))
        self.assertTrue(result.endswith(details.rstrip() + '\n'))
        downloads = result.split('## 更新', 1)[0]
        self.assertEqual(downloads.count(' downloads]('), len(release.PACKAGES))
        buttons = re.findall(r'\[!\[[^]]+\]\([^)]+\)\]\([^)]+\)', downloads)
        self.assertEqual(len(buttons), 2 * len(release.PACKAGES))
        self.assertLess(len(downloads), 9000)

    def test_prerelease_requires_explicit_current_same_repository_ci(self):
        changes = {'event':'pull_request', 'head_branch':'codex/fix-android'}
        self.assertEqual(release.verified_run(self.client(changes), self.commit, 77, prerelease=True)['id'], 77)
        for invalid in ({'head_repository':{'full_name':'fork/project'}},
                        {'event':'push'}, {'head_sha':'b'*40}, {'conclusion':'failure'}):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                release.verified_run(self.client(changes | invalid), self.commit, 77, prerelease=True)
        with self.assertRaises(ValueError):
            release.verified_run(self.client(changes), self.commit, prerelease=True)
        with self.assertRaises(ValueError):
            release.verified_run(self.client(changes), self.commit, 77, check_only=True, prerelease=True)
        with self.assertRaises(ValueError):
            release.verified_run(self.client(changes, main='b'*40), self.commit, 77, prerelease=True)
        with patch.object(release, 'git', return_value='[package]\nversion=\"0.6.5\"'):
            with self.assertRaises(ValueError):
                release.release_metadata(self.commit, prerelease=True)
        with patch.object(release, 'git', return_value='[package]\nversion=\"0.6.5-fix.2.android-parity\"'), \
             patch.object(release.subprocess, 'check_output', return_value=self.notes):
            self.assertEqual(release.release_metadata(self.commit, prerelease=True)[1], 'v0.6.5-fix.2.android-parity')

    def test_preview_packaging_requires_no_release_notes_and_still_checks_version(self):
        with patch.object(release,'git',return_value='[package]\nversion="0.5.2-dev.1"'), \
             patch.object(release.subprocess,'check_output') as notes:
            self.assertEqual(release.release_metadata(self.commit,check_only=True),
                             ('0.5.2-dev.1','v0.5.2-dev.1',''))
            notes.assert_not_called()
            with self.assertRaises(ValueError):
                release.release_metadata(self.commit,'v0.5.1',check_only=True)
        with patch.object(release,'git',return_value='[package]\nversion="invalid"'):
            with self.assertRaises(ValueError):
                release.release_metadata(self.commit,check_only=True)

    def test_release_rejects_missing_assets_and_bad_or_misnamed_checksums(self):
        with tempfile.TemporaryDirectory() as folder:
            directory=Path(folder);files=self.files(directory)
            with self.assertRaises(ValueError):release.validate_files(list(files.values())[:-1],self.version)
            checksum=next(p for p in files.values() if p.suffix=='.sha256')
            digest=checksum.read_text().strip()
            checksum.write_text(digest+'  unrelated.exe\n')
            with self.assertRaises(ValueError):release.validate_files(list(files.values()),self.version)
            checksum.write_text('0'*64+'\n')
            with self.assertRaises(ValueError):release.validate_files(list(files.values()),self.version)

    def artifact_client(self, bad_name=False, expired=False, bad_digest=False, duplicate=False, bad_receipt=False, bad_build=False,
                        bad_signing=False, missing_candidate=False):
        artifacts=[];candidates=[];archives={};builds=[]
        for number,(package,suffix) in enumerate(release.PACKAGES.items()):
            name=f'Clash-of-Rust-{self.version}-{suffix}'
            data=('installer '+suffix).encode();stream=io.BytesIO()
            system,arch=package.removeprefix('package-').split('-')
            source={'id':100+number,'name':f'build-{system}-{arch}','digest':'sha256:'+hashlib.sha256(('build '+suffix).encode()).hexdigest(),'expired':False}
            builds.append(source)
            receipt={'schema':1,'commit':self.commit,'version':self.version,'ci_run_id':77,
                     'system':system,'arch':arch,'package':name,'tests_passed':True,
                     'sha256':hashlib.sha256(data).hexdigest(),
                     'build_artifact':{key:source[key] for key in ('id','name','digest')}}
            if system == 'android':
                receipt['distribution_signed'] = not bad_signing
            if bad_receipt:receipt['commit']='b'*40
            if bad_build:receipt['build_artifact']['id']=999
            with zipfile.ZipFile(stream,'w') as zipped:
                zipped.writestr('../outside' if bad_name and number==0 else name,data)
                zipped.writestr(name+'.sha256',hashlib.sha256(data).hexdigest()+'\n')
                zipped.writestr('package-receipt.json',json.dumps(receipt))
            blob=stream.getvalue();archives[number]=blob
            artifacts.append({'id':number,'name':package,'expired':expired,
                              'digest':'sha256:'+('0'*64 if bad_digest else hashlib.sha256(blob).hexdigest())})
            candidate = copy.deepcopy(receipt)
            if system == 'android':
                candidate['distribution_signed'] = False
            candidate_stream = io.BytesIO()
            with zipfile.ZipFile(candidate_stream, 'w') as zipped:
                zipped.writestr('../outside' if bad_name and number==0 else name, data)
                zipped.writestr(name+'.sha256', hashlib.sha256(data).hexdigest()+'\n')
                zipped.writestr('package-receipt.json', json.dumps(candidate))
            candidate_blob = candidate_stream.getvalue()
            archives[1000+number] = candidate_blob
            candidates.append({'id': 1000+number, 'name': package, 'expired': expired,
                               'digest': 'sha256:'+hashlib.sha256(candidate_blob).hexdigest()})
        artifacts = [artifact for artifact in artifacts if artifact['name'] in release.PACKAGE_JOBS]
        if duplicate:artifacts.append(artifacts[0])
        if missing_candidate:candidates.pop()
        def request(path, raw=False):
            if path.startswith('/actions/runs/77/artifacts?'):return {'artifacts':builds+candidates}
            if path.startswith('/actions/runs/88/artifacts?'):return {'artifacts':artifacts}
            return archives[int(path.split('/')[3])]
        return request

    def test_release_artifact_download_enforces_archive_hash_and_safe_complete_contents(self):
        with tempfile.TemporaryDirectory() as folder:
            directory=Path(folder)
            self.assertEqual(len(release.download_package_files(self.artifact_client(),88,77,self.commit,self.version,directory/'good')),len(release.PACKAGES)*2)
            for key in ('bad_name','expired','bad_digest','duplicate','bad_receipt','bad_build','bad_signing','missing_candidate'):
                with self.subTest(key=key), self.assertRaises(ValueError):
                    release.download_package_files(self.artifact_client(**{key:True}),88,77,self.commit,self.version,directory/key)
            self.assertFalse((directory/'outside').exists())

    def test_check_mode_reads_only_ci_candidates_and_rejects_them_for_distribution(self):
        selected = self.artifact_client()
        queries = []

        def client(path, raw=False):
            queries.append(path)
            self.assertNotIn('/actions/runs/88/', path)
            return selected(path, raw=raw)

        with tempfile.TemporaryDirectory() as directory:
            files = release.download_package_files(client, 88, 77, self.commit, self.version,
                                                    Path(directory)/'check', check_only=True)
            self.assertEqual(len(files), 18)
            self.assertTrue(queries)
            with self.assertRaisesRegex(ValueError, 'separate release'):
                release.download_package_files(client, 77, 77, self.commit, self.version,
                                               Path(directory)/'signed')

    def test_release_requires_every_package_and_compatibility_job(self):
        def client(changes=None,jobs=None):
            run={'id':88,'path':'.github/workflows/release.yml@main','event':'workflow_dispatch',
                 'head_branch':'main','head_sha':self.commit,'head_repository':{'full_name':'example/project'},
                 'status':'completed','conclusion':'success'}
            run.update(changes or {})
            records=jobs if jobs is not None else [{'name':name,'status':'completed','conclusion':'success'}
                for name in release.PACKAGE_JOBS | {'Resolve verified CI'}]
            def request(path):
                return {'jobs':records,'total_count':len(records)} if '/jobs?' in path else run
            request.repo='example/project'
            return request
        with patch.dict(build.os.environ,{},clear=True):
            self.assertEqual(release.verified_package_run(client(),88,self.commit)['id'],88)
            for change in ({'event':'push'},{'head_sha':'b'*40},{'path':'.github/workflows/ci.yml'},
                           {'status':'in_progress'},{'conclusion':'failure'},{'head_branch':'codex/dev-0.5.2'}):
                with self.subTest(change=change),self.assertRaises(ValueError):
                    release.verified_package_run(client(change),88,self.commit)
            jobs=[{'name':name,'status':'completed','conclusion':'success'}
                  for name in sorted(release.PACKAGE_JOBS | {'Resolve verified CI'})]
            with self.assertRaises(ValueError):release.verified_package_run(client(jobs=jobs[:-1]),88,self.commit)
            jobs[0]['conclusion']='skipped'
            with self.assertRaises(ValueError):release.verified_package_run(client(jobs=jobs),88,self.commit)
            self.assertEqual(release.verified_package_run(client({'head_branch':'codex/dev-0.5.2'}),
                                                         88,self.commit,check_only=True)['id'],88)
            self.assertEqual(release.verified_package_run(client({'head_branch':'codex/fix-android'}),
                             88,self.commit,prerelease=True,source_branch='codex/fix-android')['id'],88)
            with self.assertRaises(ValueError):
                release.verified_package_run(client({'head_branch':'other'}),
                    88,self.commit,prerelease=True,source_branch='codex/fix-android')
            with self.assertRaises(ValueError):
                release.verified_package_run(client({'head_branch':'codex/dev-0.5.2'},jobs=jobs),
                                             88,self.commit,check_only=True)
        with patch.dict(build.os.environ,{'GITHUB_RUN_ID':'88'},clear=True):
            release.verified_package_run(client({'status':'in_progress','conclusion':None}),88,self.commit)

    def test_resolve_emits_ci_identity_without_downloading_packages(self):
        with tempfile.TemporaryDirectory() as directory:
            output=Path(directory)/'outputs'
            with patch.object(release.sys,'argv',['publish-release.py','--resolve','--run-id','77']), \
                 patch.object(release,'repository',return_value='example/project'), \
                 patch.object(release,'credential',return_value='test-only-token'), \
                 patch.object(release,'GitHub'),patch.object(release,'git',return_value=self.commit), \
                 patch.object(release,'verified_run',return_value={'id':77}), \
                 patch.object(release,'release_metadata',return_value=(self.version,self.tag,self.notes)), \
                 patch.object(release,'build_artifacts'),patch.object(release,'candidate_artifacts'), \
                 patch.object(compatibility,'verified_suite') as verify_compatibility, \
                 patch.object(release,'download_package_files') as download, \
                 patch.object(release,'publish') as publishing,patch.dict(build.os.environ,{'GITHUB_OUTPUT':str(output)}):
                release.main()
                download.assert_not_called();publishing.assert_not_called()
                self.assertEqual([call.args[2] for call in verify_compatibility.call_args_list],
                                 ['android', 'ubuntu'])
            self.assertIn('commit='+self.commit,output.read_text())
            self.assertIn('ci_run_id=77',output.read_text())

    class PublisherClient:
        def __init__(self, owner, published=False, bad_upload=False, main_changed=False, wrong_tag=False, starter=False, branch='main'):
            self.owner=owner;self.bad_upload=bad_upload;self.main_changed=main_changed;self.writes=[]
            self.branch=branch
            self.tag_commit='b'*40 if wrong_tag else owner.commit if published else None
            self.release=None
            if published or starter:
                self.release={'id':1,'tag_name':owner.tag,'target_commitish':owner.commit,'body':owner.notes,
                    'draft':not published,'prerelease':False,'upload_url':'https://uploads.github.com/repos/example/project/releases/1{?name}',
                    'assets':[],'html_url':'https://github.com/example/project/releases/tag/'+owner.tag}
            if starter:
                self.release['assets']=[{'id':42,'name':sorted(release.filenames(owner.version))[0],'state':'starter','size':0}]

        def __call__(self,path,method='GET',data=None,**kwargs):
            owner=self.owner
            if method!='GET':self.writes.append((path,method,data))
            if path.startswith('/releases?'):return [copy.deepcopy(self.release)] if self.release else []
            if path=='/git/ref/tags/'+owner.tag:
                if self.tag_commit is None:raise urllib.error.HTTPError(path,404,'Not Found',{},None)
                return {'object':{'type':'commit','sha':self.tag_commit}}
            if path=='/branches/'+urllib.parse.quote(self.branch,safe=''):return {'commit':{'sha':'b'*40 if self.main_changed else owner.commit}}
            if path=='/releases' and method=='POST':
                self.release={**data,'id':1,'assets':[],
                    'upload_url':'https://uploads.github.com/repos/example/project/releases/1{?name}',
                    'html_url':'https://github.com/example/project/releases/tag/'+owner.tag}
            elif path=='/releases/1' and method=='PATCH':
                self.release.update(data)
                if not self.release['draft']:self.tag_commit=owner.commit
            elif method=='POST' and path.startswith('https://uploads.github.com/'):
                name=urllib.parse.parse_qs(urllib.parse.urlparse(path).query)['name'][0]
                asset={'id':100+len(self.release['assets']),'name':name,'size':len(data),'state':'uploaded',
                    'digest':'sha256:'+('0'*64 if self.bad_upload else hashlib.sha256(data).hexdigest())}
                self.release['assets'].append(asset);return copy.deepcopy(asset)
            elif path=='/releases/assets/42' and method=='DELETE':
                self.release['assets']=[];return None
            elif path not in ('/releases/1','/releases/tags/'+owner.tag):
                owner.fail('Unexpected publication request: '+path)
            return copy.deepcopy(self.release)

    def test_release_publishes_only_after_all_verified_uploads_and_can_resume_empty_starter(self):
        with tempfile.TemporaryDirectory() as folder:
            files=self.files(Path(folder))
            for starter in (False,True):
                client=self.PublisherClient(self,starter=starter)
                release.publish(client,self.commit,self.tag,self.notes,files)
                self.assertFalse(client.release['draft'])
                self.assertEqual(len(client.release['assets']),len(release.PACKAGES)*2)
                if starter:self.assertTrue(any(method=='DELETE' for _,method,_ in client.writes))

    def test_development_version_is_published_as_a_preview(self):
        self.version='0.4.13-dev.1';self.tag='v'+self.version
        with tempfile.TemporaryDirectory() as folder:
            files=self.files(Path(folder));client=self.PublisherClient(self)
            release.publish(client,self.commit,self.tag,self.notes,files)
            self.assertTrue(client.release['prerelease'])
            self.assertEqual(client.release['make_latest'],'false')

    def test_release_refuses_existing_public_release_or_wrong_tag_without_writes(self):
        with tempfile.TemporaryDirectory() as folder:
            files=self.files(Path(folder))
            for kwargs in ({'published':True},{'wrong_tag':True}):
                client=self.PublisherClient(self,**kwargs)
                with self.assertRaises(ValueError):release.publish(client,self.commit,self.tag,self.notes,files)
                self.assertFalse(client.writes)

    def test_fix_branch_prerelease_does_not_change_latest_and_checks_source_after_upload(self):
        self.version='0.6.5-fix.2.android-parity';self.tag='v'+self.version
        with tempfile.TemporaryDirectory() as folder:
            files=self.files(Path(folder))
            branch='codex/fix-android'
            refused=self.PublisherClient(self,branch=branch)
            with self.assertRaises(ValueError):
                release.publish(refused,self.commit,self.tag,self.notes,files,source_branch=branch)
            self.assertFalse(refused.writes)
            for changed in (False, True):
                client=self.PublisherClient(self,branch=branch,main_changed=changed)
                if changed:
                    with self.assertRaises(ValueError):
                        release.publish(client,self.commit,self.tag,self.notes,files,source_branch=branch,prerelease=True)
                    self.assertTrue(client.release['draft'])
                else:
                    release.publish(client,self.commit,self.tag,self.notes,files,source_branch=branch,prerelease=True)
                    self.assertFalse(client.release['draft'])
                    self.assertTrue(client.release['prerelease'])
                    self.assertEqual(client.release['make_latest'],'false')

    def test_release_leaves_draft_unpublished_after_bad_upload_or_main_change(self):
        with tempfile.TemporaryDirectory() as folder:
            files=self.files(Path(folder))
            for kwargs in ({'bad_upload':True},{'main_changed':True}):
                client=self.PublisherClient(self,**kwargs)
                with self.assertRaises(ValueError):release.publish(client,self.commit,self.tag,self.notes,files)
                self.assertTrue(client.release['draft'])
                self.assertFalse(any(isinstance(data,dict) and data.get('draft') is False for _,_,data in client.writes))


class OriginalDownloadTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='cor-original-download-tests-')
        self.addCleanup(temporary.cleanup)
        environment = patch.dict(os.environ, {
            'GITHUB_ACTIONS': 'true',
            'BUILD_INPUTS_DIR': temporary.name,
            'RUNNER_TEMP': temporary.name,
            'COR_UPSTREAM_CACHE_CONFIG': '',
            **{name: str(Path(temporary.name) / name.lower()) for name in (
                'GITHUB_ENV', 'GITHUB_OUTPUT', 'GITHUB_STATE', 'GITHUB_STEP_SUMMARY')},
        })
        environment.start()
        self.addCleanup(environment.stop)

    def originals(self, count=1):
        folder = upstream_cache.directory()
        folder.mkdir(parents=True, exist_ok=True)
        files = []
        for index in range(count):
            checksum = hashlib.sha256(str(index).encode()).hexdigest()
            item = {'url': f'https://github.com/example/tool/releases/download/v1/file-{index}',
                    'size': 100 + index, 'sha256': checksum}
            (folder / (checksum + '.json')).write_text(json.dumps(item))
            files.append(item)
        return sorted(files, key=lambda item: item['sha256'])

    def queue_page(self, payload, offset=0, state='queued', complete=False):
        items = payload['files'][offset:offset + 100]
        identifier = hashlib.sha256(upstream_cache.encoded(payload)).hexdigest()
        return {
            'schema': 1, 'group': upstream_cache.GROUP, 'sequence': payload['sequence'],
            'manifest_id': identifier, 'status': 'ready' if state == 'ready' else 'pending',
            'total': len(payload['files']), 'counts': {state: len(payload['files'])},
            'complete': complete, 'next_offset': offset + len(items) if offset + len(items) < len(payload['files']) else None,
            'files': [{'key': 'sha256-' + item['sha256'], 'sequence': payload['sequence'],
                       'job_id': item['sha256'], 'status': state,
                       'error_code': 'upstream_rejected' if state == 'failed' else None}
                      for item in items],
        }

    def queue_submit(self, handler, **kwargs):
        def reply(config, path, **options):
            status, value = handler(path, options)
            response = io.BytesIO(json.dumps(value).encode())
            response.status = status
            return nullcontext(response)
        with patch.object(upstream_cache, 'configuration', return_value={
                'url': 'http://100.64.0.1:9080', 'token': 'never-publish-this'}), \
                patch.object(local_cache, 'server_capabilities', return_value={
                    *upstream_cache.IMPORT_CAPABILITIES, upstream_cache.QUEUE_CAPABILITY}), \
                patch.object(local_cache, 'response', side_effect=reply):
            return upstream_cache.submit(Path(os.environ['RUNNER_TEMP']) / 'report.json', **kwargs)

    def test_queue_submits_every_batch_before_polling_and_verifies_all_pages(self):
        self.originals(1025)
        requests = []
        batches = {}
        def server(path, options):
            requests.append((path, options))
            if options:
                payload = json.loads(options['file'])
                self.assertFalse(payload['retry_failed'])
                self.assertLessEqual(len(payload['files']), 1024)
                self.assertLessEqual(len(options['file']), 1024 * 1024)
                page = self.queue_page(payload)
                batches[page['manifest_id']] = payload
                return 202, page
            query = urllib.parse.parse_qs(urllib.parse.urlsplit(path).query)
            self.assertEqual(len(batches), 2)
            return 200, self.queue_page(batches[query['id'][0]], int(query['offset'][0]), 'ready', True)
        report = self.queue_submit(server)
        self.assertEqual([len(item['request']['files']) for item in report['manifests']], [1024, 1])
        self.assertEqual(len(requests), 14)
        self.assertTrue(all(item['status'] == 'ready' and item['receipt_verified'] for item in report['manifests']))
        self.assertTrue(all(item['status'] == 'ready' for item in report['files']))
        self.assertNotIn('never-publish-this', json.dumps(report))
        self.assertNotIn(os.environ['RUNNER_TEMP'], json.dumps(report))

    def test_manifest_batches_also_enforce_the_encoded_byte_limit(self):
        files = self.originals(3)
        for item in files:
            item['url'] += 'a' * 400_000
        batches = list(upstream_cache.manifest_batches(files, 123))
        self.assertEqual([len(item['files']) for item in batches], [2, 1])
        self.assertTrue(all(len(upstream_cache.encoded(item)) <= 1024 * 1024 for item in batches))

    def test_lost_post_response_reuses_identical_identity_and_later_checkpoint_does_not_resubmit(self):
        self.originals(1)
        bodies = []
        payload = None
        def server(path, options):
            nonlocal payload
            if options:
                bodies.append(options['file'])
                payload = json.loads(options['file'])
                if len(bodies) == 1:
                    raise ConnectionResetError('server committed but response was lost')
                return 202, self.queue_page(payload)
            return 200, self.queue_page(payload, state='ready')
        first = self.queue_submit(server, poll_seconds=0)
        self.assertEqual(bodies[0], bodies[1])
        self.assertEqual(first['files'][0]['status'], 'pending')
        final = self.queue_submit(server)
        self.assertEqual(len(bodies), 2)
        self.assertEqual(final['files'][0]['status'], 'ready')
        self.assertEqual(first['manifests'][0]['manifest_id'], final['manifests'][0]['manifest_id'])

    def test_complete_failed_or_superseded_is_never_a_success_or_automatic_retry(self):
        self.originals()
        for state in ('queued', 'retry_wait', 'failed', 'superseded'):
            with self.subTest(state=state):
                (Path(os.environ['RUNNER_TEMP']) / 'report.json').unlink(missing_ok=True)
                payload = None
                def server(path, options):
                    nonlocal payload
                    if options:
                        payload = json.loads(options['file'])
                        self.assertFalse(payload['retry_failed'])
                    return 200, self.queue_page(payload, state=state, complete=True)
                report = self.queue_submit(server)
                self.assertEqual(report['files'][0]['status'], state if state in ('failed', 'superseded') else 'pending')
                self.assertNotEqual(report['manifests'][0]['status'], 'ready')
                if state == 'failed':
                    self.assertEqual(report['files'][0]['error_code'], 'upstream_rejected')

    def test_rejected_batch_is_preserved_as_not_received_and_later_batches_are_still_submitted(self):
        self.originals(1025)
        calls = []
        def server(path, options):
            calls.append(options['file'])
            if len(calls) == 1:
                return 503, {'error': 'queue_full'}
            return 202, self.queue_page(json.loads(options['file']))
        report = self.queue_submit(server, poll_seconds=0)
        self.assertEqual(len(calls), 2)
        self.assertEqual(sum(item['status'] == 'not_submitted' for item in report['files']), 1024)
        self.assertEqual(report['manifests'][0]['error'], 'queue_full')
        self.assertEqual(report['manifests'][0]['http_status'], 503)
        self.assertEqual(report['files'][-1]['status'], 'pending')

    def test_wrong_receipt_identity_or_ready_counts_do_not_confirm_acceptance(self):
        self.originals()
        for changed in ({'sequence': -1}, {'group': 'rust/linux-x64'},
                        {'total': 2}, {'status': 'ready'}, {'manifest_id': '../private'}):
            with self.subTest(changed=changed):
                (Path(os.environ['RUNNER_TEMP']) / 'report.json').unlink(missing_ok=True)
                def server(path, options):
                    return 202, {**self.queue_page(json.loads(options['file'])), **changed}
                report = self.queue_submit(server, poll_seconds=0)
                self.assertEqual(report['files'][0]['status'], 'not_submitted')
                self.assertFalse(report['files'][0]['accepted'])

    def test_wrong_item_or_repeated_page_cannot_mark_the_manifest_verified(self):
        self.originals(201)
        for kind in ('item', 'pagination'):
            with self.subTest(kind=kind):
                (Path(os.environ['RUNNER_TEMP']) / 'report.json').unlink(missing_ok=True)
                payload = None
                def server(path, options):
                    nonlocal payload
                    if options:
                        payload = json.loads(options['file'])
                        return 202, self.queue_page(payload)
                    query = urllib.parse.parse_qs(urllib.parse.urlsplit(path).query)
                    page = self.queue_page(payload, int(query['offset'][0]), 'ready')
                    if kind == 'item':
                        page['files'][0]['sequence'] = -1
                    elif int(query['offset'][0]) == 100:
                        page['files'] = self.queue_page(payload, 0, 'ready')['files']
                    return 200, page
                report = self.queue_submit(server)
                self.assertFalse(report['manifests'][0]['receipt_verified'])
                self.assertNotEqual(report['manifests'][0]['status'], 'ready')
                self.assertTrue(all(item['accepted'] for item in report['files']))

    def test_expired_budget_preserves_the_complete_unsent_manifest(self):
        self.originals(205)
        with patch.object(upstream_cache.time, 'monotonic', side_effect=[0, 0, 61, 61, 61]), \
                patch.object(upstream_cache, 'configuration', return_value={'token': 'test'}), \
                patch.object(local_cache, 'server_capabilities', return_value={
                    *upstream_cache.IMPORT_CAPABILITIES, upstream_cache.QUEUE_CAPABILITY}), \
                patch.object(local_cache, 'response') as request:
            report = upstream_cache.submit(Path(os.environ['RUNNER_TEMP']) / 'report.json')
        request.assert_not_called()
        self.assertEqual(len(report['manifests'][0]['request']['files']), 205)
        self.assertTrue(all(item['status'] == 'not_submitted' for item in report['files']))

    def test_action_configures_without_network_and_bounds_post_submission(self):
        node = shutil.which('node')
        if not node:
            self.skipTest('Node is required for the Actions wrapper test')
        script = build.ROOT / '.github/actions/upstream-cache/index.cjs'
        harness = r'''
const assert=require('node:assert/strict'),vm=require('node:vm'),realfs=require('node:fs');
const script=process.argv[1],mode=process.argv[2],calls=[],writes=[];
const fs={writeFileSync(path,value,options){writes.push({path,value,options})},
  appendFileSync(path,value){writes.push({path,value})},existsSync(){return mode==='reported'},
  rmSync(path){calls.push({remove:path})}};
const env={RUNNER_TEMP:'/runner/temp',GITHUB_STATE:'state',GITHUB_ENV:'env',INPUT_TOKEN:'private-token'};
if(mode!=='main')env.STATE_original_download_config='/runner/temp/config.json';
if(mode==='reported')env.COR_UPSTREAM_REPORT_ATTEMPTED='true';
const context={__dirname:require('node:path').dirname(script),console:{log(){}},
  process:{env,platform:'linux'},require(name){
    if(name==='node:fs')return fs;
    if(name==='node:child_process')return {spawnSync(command,args,options){
      calls.push({command,args,options});return {status:null,error:{code:'ETIMEDOUT'}}}};
    return require(name)}};
vm.runInNewContext(realfs.readFileSync(script,'utf8'),context,{filename:script});
if(mode==='main'){
  assert.equal(calls.length,0);assert.equal(writes[0].options.mode,0o600);
  assert.equal(JSON.parse(writes[0].value).token,'private-token');
  assert.ok(writes.slice(1).every(record=>!record.value.includes('private-token')));
}else{
  assert.equal(calls.at(-1).remove,'/runner/temp/config.json');
  assert.equal(calls.length,mode==='reported'?1:2);
  if(mode!=='reported'){assert.equal(calls[0].args[1],'submit');assert.equal(calls[0].options.timeout,90000);}
}
'''
        for mode in ('main', 'post', 'reported'):
            with self.subTest(mode=mode):
                result = subprocess.run([node, '-e', harness, str(script), mode], capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_sources_require_public_https_and_strong_content_identity(self):
        checksum = 'a' * 64
        self.assertTrue(upstream_cache.valid_source('https://github.com/owner/tool/file.zip', checksum, 123))
        for url in ('http://github.com/file', 'https://100.64.0.1/file',
                    'https://user:password@github.com/file', 'https://github.com:444/file',
                    'https://github.com/file?token=secret', 'https://github.com/file#fragment',
                    'https://github.com/file\ninjection', 'https://example.com/file'):
            self.assertFalse(upstream_cache.valid_source(url, checksum, 123), url)
        for checksum, size in (('a' * 63, 123), ('g' * 64, 123), ('a' * 64, 0),
                               ('a' * 64, True), ('a' * 64, 5 * 1024**3)):
            self.assertFalse(upstream_cache.valid_source('https://github.com/file', checksum, size))

    def test_official_success_never_looks_up_the_private_mirror(self):
        payload = b'official original'
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / 'original'
            with patch.object(build, 'opener') as http, \
                    patch.object(upstream_cache, 'restore') as private:
                http.return_value.open.return_value = io.BytesIO(payload)
                build.download('https://github.com/owner/tool/file', target,
                               digest=hashlib.sha256(payload).hexdigest())
                private.assert_not_called()
            self.assertEqual(target.read_bytes(), payload)

    def test_official_retries_precede_the_private_fallback(self):
        checksum = hashlib.sha256(b'original').hexdigest()
        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / 'original'
            with patch.object(build, 'opener') as http, \
                    patch.object(build.time, 'sleep'), \
                    patch.object(upstream_cache, 'restore', return_value=True) as private:
                http.return_value.open.side_effect = OSError('offline')
                build.download('https://github.com/tool/original', target, digest=checksum)
                self.assertEqual(http.return_value.open.call_count, 3)
                private.assert_called_once_with('https://github.com/tool/original', target, checksum)

    def test_private_envelope_and_original_are_both_verified_before_replacement(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            original = root / 'source'
            original.write_bytes(b'verified original')
            archive = root / 'blob'
            local_cache.pack([original], archive)
            metadata = {'id': '1' * 64, 'size': archive.stat().st_size, 'sha256': build.sha256(archive)}
            target = root / 'target'
            target.write_bytes(b'existing good data')
            for corrupt_envelope, checksum, expected in (
                    (True, build.sha256(original), False),
                    (False, '0' * 64, False),
                    (False, build.sha256(original), True)):
                with self.subTest(corrupt_envelope=corrupt_envelope, expected=expected):
                    reply = io.BytesIO(archive.read_bytes())
                    reply.status = 200
                    value = {**metadata, 'sha256': '0' * 64} if corrupt_envelope else metadata
                    with patch.object(upstream_cache, 'configuration', return_value={'token': 'test'}), \
                            patch.object(upstream_cache, 'lookup', return_value=value), \
                            patch.object(local_cache, 'response', return_value=nullcontext(reply)):
                        self.assertEqual(upstream_cache.restore(
                            'https://github.com/tool/original', target, checksum), expected)
                    self.assertEqual(target.read_bytes(), original.read_bytes() if expected else b'existing good data')

    def test_private_original_is_staged_in_destination_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            original = root / 'source'
            original.write_bytes(b'verified original')
            archive = root / 'blob'
            local_cache.pack([original], archive)
            metadata = {'id': '1' * 64, 'size': archive.stat().st_size, 'sha256': build.sha256(archive)}
            reply = io.BytesIO(archive.read_bytes())
            reply.status = 200
            target = root / 'target'
            with patch.object(upstream_cache, 'configuration', return_value={'token': 'test'}), \
                    patch.object(upstream_cache, 'lookup', return_value=metadata), \
                    patch.object(local_cache, 'response', return_value=nullcontext(reply)), \
                    patch.object(tempfile, 'mkstemp', wraps=tempfile.mkstemp) as staged:
                self.assertTrue(upstream_cache.restore(
                    'https://github.com/tool/original', target, build.sha256(original)))
            self.assertEqual(staged.call_args.kwargs['dir'], target.parent)
            self.assertEqual(target.read_bytes(), original.read_bytes())
            self.assertEqual(list(root.glob('.target-*.partial')), [])

    def test_private_envelope_rejects_directories_links_and_extra_payloads(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / 'blob'
            for kind in ('directory', 'symlink', 'extra'):
                with self.subTest(kind=kind):
                    with tarfile.open(archive, 'w:gz') as output:
                        entry = tarfile.TarInfo('0')
                        entry.type = {'directory': tarfile.DIRTYPE, 'symlink': tarfile.SYMTYPE,
                                      'extra': tarfile.REGTYPE}[kind]
                        entry.linkname = '/etc/passwd'
                        output.addfile(entry)
                        if kind == 'extra':
                            output.addfile(tarfile.TarInfo('1'))
                    reply = io.BytesIO(archive.read_bytes())
                    reply.status = 200
                    metadata = {'id': '1' * 64, 'size': archive.stat().st_size, 'sha256': build.sha256(archive)}
                    with patch.object(upstream_cache, 'configuration', return_value={'token': 'test'}), \
                            patch.object(upstream_cache, 'lookup', return_value=metadata), \
                            patch.object(local_cache, 'response', return_value=nullcontext(reply)):
                        self.assertFalse(upstream_cache.restore(
                            'https://github.com/tool/original', root / 'target', 'a' * 64))
                    self.assertFalse((root / 'target').exists())

    def test_import_failure_never_uploads_an_archive_or_deletes_cache_entries(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.dict(os.environ, {'RUNNER_TEMP': temporary, 'GITHUB_ACTIONS': 'true'}):
            source = Path(temporary) / 'original'
            source.write_bytes(b'original')
            build.record_download('https://github.com/tool/original', source, build.sha256(source))
            source.unlink()
            for reply_value in (None, {'schema': 1, 'group': 'wrong', 'status': 'ready'}):
                with self.subTest(reply_value=reply_value):
                    reply = io.BytesIO(json.dumps(reply_value).encode())
                    reply.status = 502 if reply_value is None else 200
                    with patch.object(upstream_cache, 'configuration', return_value={'token': 'secret'}), \
                            patch.object(local_cache, 'server_capabilities', return_value={
                                'validated-cache-commit-v1', 'complete-cache-retention-v1', 'upstream-fetch-v1'}), \
                            patch.object(upstream_cache, 'lookup', return_value=None), \
                            patch.object(local_cache, 'response', return_value=nullcontext(reply)) as request:
                        result = upstream_cache.submit(Path(temporary) / 'report.json')
                    self.assertEqual(result['files'][0]['status'], 'unavailable')
                    self.assertEqual(request.call_count, 1)
                    self.assertEqual(request.call_args.args[1], '/v1/import')
                    self.assertEqual(request.call_args.kwargs['method'], 'POST')
                    self.assertNotIn(temporary, json.dumps(result))

    def test_receipts_deduplicate_content_and_do_not_export_runner_paths(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.dict(os.environ, {'RUNNER_TEMP': temporary, 'GITHUB_ACTIONS': 'true'}):
            source = Path(temporary) / 'original'
            source.write_bytes(b'original')
            for name in ('one', 'two'):
                build.record_download('https://github.com/tool/' + name, source, build.sha256(source))
            with patch.object(upstream_cache, 'configuration', return_value={}):
                report = upstream_cache.submit(Path(temporary) / 'report.json')
            self.assertEqual(len(report['files']), 1)
            self.assertNotIn(temporary, json.dumps(report))
            self.assertEqual(report['files'][0]['status'], 'not_submitted')

    def test_lookup_rejects_a_compatible_or_unrelated_record(self):
        for changed in ({'group': 'rust/linux-x64'}, {'key': 'sha256-' + 'b' * 64},
                        {'id': '../escape'}, {'size': -1}):
            reply = io.BytesIO(json.dumps({'schema': 1, 'group': upstream_cache.GROUP,
                                           'key': 'sha256-' + 'a' * 64, 'id': 'c' * 64,
                                           'sha256': 'd' * 64, 'size': 100, **changed}).encode())
            reply.status = 200
            with self.subTest(changed=changed), \
                    patch.object(local_cache, 'response', return_value=nullcontext(reply)):
                with self.assertRaises(ValueError):
                    upstream_cache.lookup({}, 'a' * 64)

    def test_lookup_reports_only_rejected_field_names(self):
        value = {'schema': True, 'group': 'private-value', 'key': 'wrong-value',
                 'id': None, 'sha256': ['private-value'], 'size': True}
        reply = io.BytesIO(json.dumps(value).encode())
        reply.status = 200
        with patch.object(local_cache, 'response', return_value=nullcontext(reply)):
            with self.assertRaises(upstream_cache.LookupMetadataError) as raised:
                upstream_cache.lookup({}, 'a' * 64)
        self.assertEqual(str(raised.exception),
                         'Invalid original download metadata: schema, group, key, id, sha256, size')

    def test_restore_failure_reports_its_stage_without_private_error_contents(self):
        for error, expected in (
                (OSError('private-address secret-token'), 'lookup: OSError'),
                (upstream_cache.LookupMetadataError(('schema',)), 'lookup: Invalid original download metadata: schema')):
            with self.subTest(error=type(error).__name__), \
                    patch.object(upstream_cache, 'configuration', return_value={'token': 'test'}), \
                    patch.object(upstream_cache, 'lookup', side_effect=error), \
                    patch('sys.stdout', new_callable=io.StringIO) as output:
                self.assertFalse(upstream_cache.restore(
                    'https://github.com/tool/original', Path('unused'), 'a' * 64))
            self.assertIn(expected, output.getvalue())
            self.assertNotIn('private-address', output.getvalue())
            self.assertNotIn('secret-token', output.getvalue())

    def test_main_rust_platforms_survive_budget_pressure(self):
        def entry(number, key):
            return {'id': number, 'key': key, 'ref': 'refs/heads/main', 'size_in_bytes': 100,
                    'last_accessed_at': str(number), 'created_at': str(number)}
        slow = entry(1, 'native-release-v7-windows-11-arm-check-aabbccdd-11223344')
        desktop = entry(2, 'native-release-v7-macos-15-intel-check-aabbccdd-11223344')
        linux = entry(3, 'native-release-ubuntu20-v1-ubuntu-24.04-check-aabbccdd-11223344')
        memory = entry(4, 'memory-ubuntu22-v2-release-aabbccdd-11223344')
        tool = entry(5, 'runtime-v1-java-linux-x64')
        removed = storage.eviction_plan([slow, desktop, linux, memory, tool], 200)
        self.assertEqual(removed, [tool])

    def test_main_targets_and_bootstrap_survive_even_when_budget_is_too_small(self):
        native_prefix = 'native-release-'+'v2-'
        keys = [
            'action-setup-tailscale/1.104.5/macOS-arm64',
            native_prefix+'macos-latest',
            native_prefix+'macos-15-intel',
            'android-v2-arm64',
            'android-v2-armv7',
            'android-v2-x64',
            native_prefix+'windows-11-arm',
            'native-release-ubuntu20-v1-ubuntu-22.04',
            native_prefix+'windows-2025',
            'native-release-ubuntu20-v1-ubuntu-22.04-arm',
            'memory-ubuntu22-v2-release',
        ]
        entries = [{'id':index,'key':key if key.startswith('action-setup-tailscale/') else key+'-13131313-11223344','ref':'refs/heads/main',
                    'size_in_bytes':100,'last_accessed_at':f'{index:02d}','created_at':f'{index:02d}'}
                   for index,key in enumerate(keys)]
        self.assertEqual(storage.eviction_plan(entries, 100), [])


class AptCacheTests(unittest.TestCase):
    def test_records_require_strong_hashes_and_safe_names(self):
        checksum = 'a'*64
        valid = f"'https://example.invalid/pkg.deb' pkg_1_amd64.deb 3 SHA256:{checksum}\n"
        self.assertEqual(linux_apt.archive_records(valid), {
            'pkg_1_amd64.deb': {'size': 3, 'sha256': checksum, 'url': 'https://example.invalid/pkg.deb'}})
        self.assertEqual(linux_apt.archive_records(valid.replace('SHA256:', 'MD5Sum:')), {})
        self.assertEqual(linux_apt.archive_records(valid.replace('https:', 'file:')), {})
        for name in ('../pkg.deb', '/pkg.deb', 'pkg.txt', 'pkg;cmd.deb'):
            with self.subTest(name=name), self.assertRaises(ValueError):
                linux_apt.archive_records(valid.replace('pkg_1_amd64.deb', name))
        with self.assertRaises(ValueError):
            linux_apt.archive_records(valid + valid.replace(checksum, 'b'*64))

    def test_local_application_records_without_hash_do_not_disable_dependency_cache(self):
        remote = "'https://example.invalid/pkg.deb' pkg_1_amd64.deb 3 SHA256:" + 'a'*64 + '\n'
        for uri in ('file:/tmp/application.deb', '/tmp/application.deb'):
            with self.subTest(uri=uri):
                local = f"'{uri}' application.deb 100\n"
                self.assertEqual(linux_apt.archive_records(local + remote), {
                    'pkg_1_amd64.deb': {'size': 3, 'sha256': 'a'*64, 'url': 'https://example.invalid/pkg.deb'}})
        with self.assertRaises(ValueError):
            linux_apt.archive_records("'https://example.invalid/pkg.deb' pkg_1_amd64.deb 3\n")

    def test_install_reuses_verified_bytes_and_replaces_same_size_corruption(self):
        content = b'verified package'
        name = 'pkg_1_amd64.deb'
        query = f"'https://example.invalid/pkg.deb' {name} {len(content)} SHA256:{hashlib.sha256(content).hexdigest()}\n"
        for initial in (None, content, b'x'*len(content)):
            with self.subTest(initial=initial), tempfile.TemporaryDirectory() as temporary:
                cache = Path(temporary)/'cache'
                cache.mkdir()
                if initial is not None:
                    (cache/name).write_bytes(initial)
                directories = []

                def execute(command, timeout, **kwargs):
                    if command[0] == 'rm':
                        shutil.rmtree(command[-1])
                        return
                    directory = Path(next(item.split('=', 1)[1] for item in command
                                          if item.startswith('Dir::Cache::archives=')))
                    directories.append(directory)
                    if '--print-uris' in command:
                        self.assertFalse((directory/name).exists())
                        return subprocess.CompletedProcess(command, 0, stdout=query)
                    self.assertEqual((directory/name).exists(), initial == content)
                    (directory/name).write_bytes(content)
                    return subprocess.CompletedProcess(command, 0)

                with patch.dict(os.environ, {'GITHUB_ACTIONS': 'true',
                                            'COR_LINUX_APT_CACHE': str(cache)}), \
                     patch.object(linux_apt, 'privileged', side_effect=execute):
                    linux_apt.install(['pkg'])
                self.assertEqual((cache/name).read_bytes(), content)
                self.assertTrue(directories)
                self.assertTrue(all(not item.exists() for item in directories))

    def test_symlinks_are_never_trusted(self):
        if not hasattr(os, 'symlink'):
            self.skipTest('No symlink support')
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root/'real').write_bytes(b'valid')
            try:
                (root/'alias').symlink_to(root/'real')
            except OSError:
                self.skipTest('Symlinks unavailable')
            record = {'size': 5, 'sha256': hashlib.sha256(b'valid').hexdigest()}
            self.assertTrue(linux_apt.matches(root/'real', record))
            self.assertFalse(linux_apt.matches(root/'alias', record))

    def test_metadata_and_cleanup_failure_do_not_hide_install_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            archives = Path(temporary)/'archives'
            archives.mkdir()
            with patch.dict(os.environ, {'GITHUB_ACTIONS': 'true',
                                        'COR_LINUX_APT_CACHE': temporary}), \
                 patch.object(linux_apt.tempfile, 'mkdtemp', return_value=str(archives)), \
                 patch.object(linux_apt, 'privileged', side_effect=[
                     subprocess.CalledProcessError(100, 'metadata'),
                     subprocess.CalledProcessError(101, 'install'),
                     subprocess.CalledProcessError(102, 'cleanup')]):
                with self.assertRaises(subprocess.CalledProcessError) as failure:
                    linux_apt.install(['pkg'])
                self.assertEqual(failure.exception.returncode, 101)


class CosmicStartupInvestigationTests(unittest.TestCase):
    @unittest.skipUnless(sys.platform.startswith('linux'), 'Requires a Linux terminal')
    def test_serial_probe_recovers_from_an_incomplete_command_in_a_real_terminal(self):
        import pty
        import select
        import signal
        import time

        child, terminal = pty.fork()
        if child == 0:
            os.execve('/bin/bash', ['bash', '--noprofile', '--norc', '-i'], {
                **os.environ, 'PS1': 'fixture-ready> ', 'PS2': 'fixture-incomplete> ',
                'PROMPT_COMMAND': '', 'TERM': 'dumb',
            })
        try:
            def receive():
                return os.read(terminal, 65536) if select.select([terminal], [], [], .05)[0] else b''

            def expect(marker):
                output = b''
                deadline = time.monotonic() + 5
                while time.monotonic() < deadline:
                    output += receive()
                    if marker in output:
                        return
                self.fail('The terminal fixture did not reach its expected prompt')

            expect(b'fixture-ready> ')
            os.write(terminal, b"id() { printf '0\\n'; }\n")
            expect(b'fixture-ready> ')
            os.write(terminal, b"printf '\n")
            expect(b'fixture-incomplete> ')
            console = object.__new__(cosmic_startup_host.Console)
            console.connection = SimpleNamespace(sendall=lambda payload: os.write(terminal, payload))
            console.receive = receive
            real_clock = time.monotonic
            started = real_clock()
            with patch.object(cosmic_startup_host.time, 'monotonic',
                              side_effect=lambda: started + (real_clock() - started) * 200):
                console.wait_shell(SimpleNamespace(poll=lambda: None))
        finally:
            os.close(terminal)
            try:
                os.kill(child, signal.SIGKILL)
            except ProcessLookupError:
                pass
            os.waitpid(child, 0)

    def test_package_manager_lock_wait_is_bounded_and_does_not_hide_dependency_errors(self):
        lock = 'E: Could not get lock /var/lib/apt/lists/lock. It is held by process 1254 (apt-get)\n'
        clock = [0]
        results = [
            subprocess.CompletedProcess(['apt-get'], 100, lock),
            subprocess.CompletedProcess(['apt-get'], 0, 'Fetched package lists\n'),
        ]
        with patch.object(cosmic_startup_guest.subprocess, 'run', side_effect=results) as invoke, \
                patch.object(cosmic_startup_guest.time, 'monotonic', side_effect=lambda: clock[0]), \
                patch.object(cosmic_startup_guest.time, 'sleep',
                             side_effect=lambda delay: clock.__setitem__(0, clock[0] + delay)), \
                contextlib.redirect_stdout(io.StringIO()):
            cosmic_startup_guest.apt('update', timeout=30)
        self.assertEqual(invoke.call_count, 2)
        command = invoke.call_args.args[0]
        self.assertEqual(command[:2], ['apt-get', '-o'])
        self.assertEqual(command[-1], 'update')
        self.assertEqual(invoke.call_args.kwargs['env']['LC_ALL'], 'C')
        failure = subprocess.CompletedProcess(['apt-get'], 100, 'E: Unmet dependencies\n')
        with patch.object(cosmic_startup_guest.subprocess, 'run', return_value=failure) as invoke, \
                patch.object(cosmic_startup_guest.time, 'sleep') as pause, \
                contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaises(subprocess.CalledProcessError):
                cosmic_startup_guest.apt('install', 'missing')
        self.assertEqual(invoke.call_count, 1)
        pause.assert_not_called()
        clock[0] = 0
        with patch.object(cosmic_startup_guest.subprocess, 'run', return_value=results[0]) as invoke, \
                patch.object(cosmic_startup_guest.time, 'monotonic', side_effect=lambda: clock[0]), \
                patch.object(cosmic_startup_guest.time, 'sleep',
                             side_effect=lambda delay: clock.__setitem__(0, clock[0] + delay)), \
                contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaises(TimeoutError):
                cosmic_startup_guest.apt('update', timeout=10)
        self.assertEqual(invoke.call_count, 2)
        self.assertEqual(clock[0], 10)

    def test_session_environment_uses_a_real_compositor_process(self):
        account = SimpleNamespace(pw_uid=os.getuid(), pw_dir='/home/example', pw_name='example')
        with tempfile.TemporaryDirectory() as temporary:
            processes = Path(temporary)

            def process(pid, command, environment):
                directory = processes/str(pid)
                directory.mkdir()
                (directory/'comm').write_text(command+'\n')
                (directory/'environ').write_text(
                    '\0'.join(key+'='+value for key, value in environment.items()))

            process(10, 'cosmic-session', {})
            process(20, 'cosmic-panel', {
                'DISPLAY': ':1', 'WAYLAND_DISPLAY': 'wayland-1', 'PASSWORD': 'not-exported',
            })
            with patch.object(Path, 'is_socket', return_value=True):
                environment = cosmic_startup_guest.session_environment(account, processes)
            self.assertEqual(environment['DISPLAY'], ':1')
            self.assertEqual(environment['WAYLAND_DISPLAY'], 'wayland-1')
            self.assertEqual(environment['XDG_RUNTIME_DIR'], f'/run/user/{account.pw_uid}')
            self.assertNotIn('PASSWORD', environment)
            with patch.object(Path, 'is_socket', return_value=False):
                self.assertIsNone(cosmic_startup_guest.session_environment(account, processes))
            account.pw_uid += 1
            with patch.object(Path, 'is_socket', return_value=True):
                self.assertIsNone(cosmic_startup_guest.session_environment(account, processes))
            account.pw_uid -= 1
            shutil.rmtree(processes/'10')
            shutil.rmtree(processes/'20')
            process(30, 'browser', {'DISPLAY': ':1', 'WAYLAND_DISPLAY': 'wayland-1'})
            with patch.object(Path, 'is_socket', return_value=True):
                self.assertIsNone(cosmic_startup_guest.session_environment(account, processes))

    def test_startup_record_requires_a_new_live_session_and_reused_manual_process(self):
        account = SimpleNamespace(pw_uid=os.getuid())

        def record(session, *, background=True, visible=True, pid=None):
            return {
                'schema': 1, 'kind': 'desktop_startup', 'session_id': session,
                'pid': os.getpid() if pid is None else pid,
                'background_requested': background,
                'status': {'window_visible': visible, 'tray_state': 'created'},
            }

        def trace(records, **arguments):
            with patch.object(cosmic_startup_guest, 'startup_traces',
                              return_value=iter(records)):
                return cosmic_startup_guest.trace(account, **arguments)

        old = record('old')
        new = record('new', visible=False)
        self.assertIs(trace([old, new], background=True, visible=False,
                            excluded={'old'}), new)
        self.assertIsNone(trace([old], background=True, visible=False, excluded={'old'}))
        background = record('background', visible=False)
        manual = record('background', visible=True)
        self.assertIs(trace([manual], background=True, visible=True,
                            session='background'), manual)
        self.assertIsNone(trace([manual], background=True, visible=True, session='other'))
        self.assertIsNone(trace([record('dead', pid=999999999)],
                                background=True, visible=True))
        missing = record('missing')
        missing['status']['tray_state'] = 'failed'
        self.assertIsNone(trace([missing], background=True, visible=True))
        with self.assertRaises(RuntimeError):
            trace([record('one'), record('two')], background=True, visible=True)


if __name__ == '__main__':
    unittest.main()
