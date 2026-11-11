"""Offline regression tests for immutable CI payloads and package-only builds."""
import argparse
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import struct
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import build_support as build
import workflow_artifacts as artifacts


class ArtifactTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        override = patch.dict(os.environ, {'BUILD_INPUTS_DIR': directory.name, 'GITHUB_ACTIONS': 'false'})
        override.start()
        self.addCleanup(override.stop)

    commit = 'a'*40
    version = '0.4.12'

    def test_guest_source_marker_is_validated_without_falling_back_to_git(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            marker = root / 'source-commit.txt'
            with patch.object(artifacts, 'ROOT', root), \
                    patch.object(artifacts.subprocess, 'check_output') as git, \
                    patch.dict(os.environ, {'COR_LINUX_GUEST': 'true', 'GITHUB_ACTIONS': 'true'}):
                marker.write_text(self.commit + '\n')
                self.assertEqual(artifacts.commit(), self.commit)
                marker.write_text('unverified-source')
                with self.assertRaises(ValueError):
                    artifacts.commit()
                marker.write_text(self.commit)
                with patch.dict(os.environ, {'GITHUB_ACTIONS': 'false'}), self.assertRaises(ValueError):
                    artifacts.commit()
                git.assert_not_called()

    def test_package_wait_preserves_source_identity_and_waits_for_both_artifacts(self):
        queries = []
        polls = 0

        def client(path):
            nonlocal polls
            queries.append(path)
            if path.endswith('/artifacts?per_page=100'):
                polls += 1
                names = ['package-linux-arm64']
                if polls > 1:
                    names.append('linux-tun-test-arm64')
                return {'artifacts': [{'name': name, 'expired': False} for name in names]}
            if '/jobs?' in path:
                return {'jobs': [{'name': 'package-linux-arm64', 'status': 'in_progress'}]}
            return {'head_sha': self.commit, 'status': 'in_progress'}

        with patch.object(artifacts.time, 'sleep') as pause:
            artifacts.wait_package(client, 42, 'linux', 'arm64', self.commit)
        pause.assert_called_once_with(15)
        self.assertEqual(polls, 2)
        self.assertTrue(all(path.startswith('/actions/runs/42') for path in queries))

    def test_package_wait_rejects_a_different_commit(self):
        with self.assertRaisesRegex(ValueError, 'source differs'):
            artifacts.wait_package(lambda path: {'head_sha': 'b'*40},
                                   42, 'linux', 'x64', self.commit)

    def test_package_wait_stops_when_the_native_ci_producer_failed(self):
        def client(path):
            if '/artifacts?' in path:
                return {'artifacts': []}
            if '/jobs?' in path:
                return {'jobs': [{'name': 'linux-arm64', 'status': 'completed',
                                  'conclusion': 'failure'}]}
            return {'path': '.github/workflows/ci.yml', 'head_sha': self.commit,
                    'status': 'in_progress'}

        with patch.object(artifacts.time, 'sleep') as pause, self.assertRaisesRegex(RuntimeError, 'did not produce'):
            artifacts.wait_package(client, 42, 'linux', 'arm64', self.commit)
        pause.assert_not_called()

    def test_package_wait_stops_after_producer_failure_or_timeout(self):
        for failed in (False, True):
            def client(path):
                if '/artifacts?' in path:
                    return {'artifacts': []}
                if '/jobs?' in path:
                    return {'jobs': [{'name': 'package-linux-x64', 'status': 'completed',
                                      'conclusion': 'failure'}] if failed else []}
                return {'head_sha': self.commit, 'status': 'in_progress'}

            with self.subTest(failed=failed), \
                 patch.object(artifacts.time, 'monotonic', side_effect=[0, 1200]), \
                 patch.object(artifacts.time, 'sleep') as pause:
                with self.assertRaises(RuntimeError if failed else TimeoutError):
                    artifacts.wait_package(client, 42, 'linux', 'x64', self.commit)
                pause.assert_not_called()

    def test_package_wait_rejects_duplicate_or_expired_artifacts(self):
        for entries in (
                [{'name': 'package-linux-x64', 'expired': True}],
                [{'name': 'package-linux-x64', 'expired': False}]*2):
            def client(path):
                if '/artifacts?' in path:
                    return {'artifacts': entries}
                return {'head_sha': self.commit, 'status': 'in_progress'}

            with self.subTest(entries=entries), self.assertRaisesRegex(ValueError, 'duplicated or expired'):
                artifacts.wait_package(client, 42, 'linux', 'x64', self.commit)

    def test_vm_package_fetch_checks_archives_and_receipts_before_using_them(self):
        package = f'Clash-of-Rust-{self.version}-linux-amd64.deb'
        binary = b'isolated-tun-test'
        payload = b'verified-deb'
        receipt = {'schema': 1, 'tests_passed': True, 'system': 'linux', 'arch': 'x64',
                   'version': self.version, 'commit': self.commit, 'ci_run_id': 17, 'package': package,
                   'sha256': hashlib.sha256(payload).hexdigest(),
                   'linux_tun_test_sha256': hashlib.sha256(binary).hexdigest()}

        def zipped(entries):
            stream = io.BytesIO()
            with zipfile.ZipFile(stream, 'w') as archive:
                for name, data in entries.items():
                    archive.writestr(name, data)
            return stream.getvalue()

        originals = [
            zipped({package: payload, package+'.sha256': receipt['sha256'],
                    'package-receipt.json': json.dumps(receipt)}),
            zipped({'linux-tun-test': binary}),
        ]
        for scenario in ('valid', 'archive-changed', 'wrong-receipt', 'wrong-ci', 'unsafe-member'):
            archives = originals.copy()
            if scenario in ('wrong-receipt', 'wrong-ci'):
                changed = dict(receipt, **({'commit': 'b'*40} if scenario == 'wrong-receipt'
                                          else {'ci_run_id': 18}))
                archives[0] = zipped({package: payload, package+'.sha256': receipt['sha256'],
                                      'package-receipt.json': json.dumps(changed)})
            if scenario == 'unsafe-member':
                archives[1] = zipped({'../linux-tun-test': binary})
            records = [{'id': index, 'name': name, 'expired': False,
                        'digest': 'sha256:'+hashlib.sha256(data).hexdigest()}
                       for index, (name, data) in enumerate(zip(
                           ['package-linux-x64', 'linux-tun-test-x64'], archives))]
            if scenario == 'archive-changed':
                archives[0] += b'altered'

            def client(path, raw=False):
                if raw:
                    return archives[int(path.split('/')[3])]
                if '/artifacts?' in path:
                    return {'artifacts': records}
                return {'head_sha': self.commit, 'status': 'in_progress'}

            with self.subTest(scenario=scenario), tempfile.TemporaryDirectory() as temporary, \
                    patch.object(artifacts, 'package_version', return_value=self.version):
                root = Path(temporary)
                if scenario == 'valid':
                    artifacts.fetch_package_inputs(client, 42, 'x64', root, self.commit, 17)
                    self.assertEqual((root/'dist'/package).read_bytes(), payload)
                    self.assertEqual((root/'target/linux-tun-test').read_bytes(), binary)
                else:
                    with self.assertRaises(ValueError):
                        artifacts.fetch_package_inputs(client, 42, 'x64', root, self.commit, 17)

    def test_only_proxy_tun_test_is_exported_after_units_run_in_ci(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def message(name, kind):
                return json.dumps({'reason': 'compiler-artifact', 'profile': {'test': True},
                                   'executable': str(root/'target'/name),
                                   'target': {'name': name, 'kind': [kind], 'src_path': str(root/'source.rs')}})
            entries = [message('clash_of_rust', 'lib'), message('clash_client_core', 'lib'),
                       message('clash-of-rust', 'bin'), message('ui_models', 'test'),
                       message('memory_benchmark', 'example')]
            with patch.object(artifacts, 'ROOT', root):
                with self.assertRaisesRegex(ValueError, 'Required proxy/TUN'):
                    artifacts.test_binaries('\n'.join(entries))
                entries.append(message('core_integration', 'test'))
                self.assertEqual(artifacts.test_binaries('\n'.join(entries)),
                                 {'core_integration': str(root/'target/core_integration')})
                with self.assertRaisesRegex(ValueError, 'Duplicate proxy/TUN'):
                    artifacts.test_binaries('\n'.join([*entries, entries[-1]]))

    def fixture(self, root):
        (root/'Cargo.lock').write_text('fixture dependencies')
        payload = root/'input/payload'
        resources = payload/'resources'
        resources.mkdir(parents=True)
        header = bytearray(64)
        header[:6] = b'\x7fELF\x02\x01'
        struct.pack_into('<H', header, 18, 62)
        for path in (payload/'clash-of-rust', payload/'clash-desktop-ui', payload/'clash-tun-launcher', payload/'cor', resources/'mihomo'):
            path.write_bytes(header)
        (payload/'LICENSE').write_text('fixture license')
        build.write_json(resources/'core.json', {'version': 'v1.20.1', 'exe_sha256': build.sha256(resources/'mihomo')})
        geo = []
        for name in build.GEO_NAMES:
            path = resources/name
            path.write_text('fixture '+name)
            geo.append({'name': name, 'sha256': build.sha256(path), 'size': path.stat().st_size})
        build.write_json(resources/'geodata.json', {'files': geo})
        tests = {}
        for name in ('core_integration',):
            path = root/'input/tests'/name
            path.parent.mkdir(exist_ok=True)
            path.write_bytes(header)
            tests[name] = 'tests/'+name
        records = {path.relative_to(root/'input').as_posix():
                   {'sha256': build.sha256(path), 'size': path.stat().st_size,
                    'executable': path.name in tests or path.name in ('clash-of-rust', 'clash-desktop-ui', 'clash-tun-launcher', 'cor', 'mihomo')}
                   for path in (root/'input').rglob('*') if path.is_file()}
        manifest = {'schema': artifacts.BUILD_SCHEMA, 'commit': self.commit, 'version': self.version, 'system': 'linux',
                    'arch': 'x64', 'profile': 'release', 'cargo_lock_sha256': build.sha256(root/'Cargo.lock'),
                    'tests': tests, 'files': records}
        build.write_json(root/'input/manifest.json', manifest)
        return manifest

    def test_payload_checks_source_architecture_dependencies_and_every_byte(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            original = self.fixture(root)
            with patch.object(artifacts, 'ROOT', root), patch.object(artifacts, 'package_version', return_value=self.version):
                self.assertEqual(artifacts.verify_build(root/'input', 'linux', 'x64', self.commit)['commit'], self.commit)
                for field, value in (('schema', 1), ('commit', 'b'*40), ('arch', 'arm64'), ('profile', 'dev'), ('cargo_lock_sha256', '0'*64)):
                    manifest = copy.deepcopy(original); manifest[field] = value
                    build.write_json(root/'input/manifest.json', manifest)
                    with self.subTest(field=field), self.assertRaises(ValueError):
                        artifacts.verify_build(root/'input', 'linux', 'x64', self.commit)
                # Cleanup must still reject a payload missing the smoke test
                # consumed by the packaging jobs.
                manifest = copy.deepcopy(original)
                del manifest['files']['payload/clash-desktop-ui']
                build.write_json(root/'input/manifest.json', manifest)
                with self.assertRaisesRegex(ValueError, 'required native payload'):
                    artifacts.verify_build(root/'input', 'linux', 'x64', self.commit)
                manifest = copy.deepcopy(original)
                del manifest['tests']['core_integration']
                build.write_json(root/'input/manifest.json', manifest)
                with self.assertRaises(ValueError):
                    artifacts.verify_build(root/'input', 'linux', 'x64', self.commit)
                manifest = copy.deepcopy(original)
                manifest['tests']['library'] = 'tests/core_integration'
                build.write_json(root/'input/manifest.json', manifest)
                with self.assertRaises(ValueError):
                    artifacts.verify_build(root/'input', 'linux', 'x64', self.commit)
                build.write_json(root/'input/manifest.json', original)
                (root/'input/payload/clash-of-rust').write_bytes(b'corrupt executable')
                with self.assertRaises(ValueError):artifacts.verify_build(root/'input', 'linux', 'x64', self.commit)

    def test_local_staging_preserves_uploaded_identity_and_checks_payload_and_archive(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            archive = root/artifacts.ARCHIVE_NAME
            with tarfile.open(archive, 'w:gz') as stream:
                for path in sorted((root/'input').rglob('*')):
                    if path.is_file():
                        stream.add(path, arcname=path.relative_to(root/'input'))
            archive.with_name(archive.name+'.sha256').write_text(build.sha256(archive)+'\n')
            with patch.object(artifacts, 'ROOT', root), \
                 patch.object(artifacts, 'package_version', return_value=self.version), \
                 patch.object(artifacts, 'release_module') as network:
                artifacts.stage_local_build(archive, root/'staged', 'linux', 'x64',
                                            self.commit, 123, 'a'*64)
                self.assertEqual(json.loads((root/'staged/source-artifact.json').read_text()),
                                 {'id': 123, 'name': 'build-linux-x64', 'digest': 'sha256:'+'a'*64})
                network.assert_not_called()
                with self.assertRaises(ValueError):
                    artifacts.stage_local_build(archive, root/'wrong-source', 'linux', 'x64',
                                                'b'*40, 123, 'a'*64)
                for identity, digest in ((0, 'a'*64), (True, 'a'*64), (123, 'invalid')):
                    with self.subTest(identity=identity, digest=digest), self.assertRaises(ValueError):
                        artifacts.stage_local_build(archive, root/'invalid', 'linux', 'x64',
                                                    self.commit, identity, digest)
                archive.write_bytes(b'corrupt archive')
                with self.assertRaisesRegex(ValueError, 'checksum'):
                    artifacts.stage_local_build(archive, root/'corrupt', 'linux', 'x64',
                                                self.commit, 123, 'a'*64)

    def test_linux_artifact_requires_the_frozen_baseline_and_complete_elf_audit(self):
        import build_inputs
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = self.fixture(root)
            image = 'ubuntu@sha256:'+'a'*64
            snapshot = {'commit': self.commit, 'baseline': {}, 'constraints': {}, 'files': {},
                        'upstream': {'linux': {'image': image}}}
            manifest['build_inputs'] = {key: snapshot[key] for key in ('commit', 'baseline', 'constraints', 'files')}
            manifest['linux_baseline'] = {'distribution': '20.04', 'image': image}
            names = ('payload/clash-of-rust', 'payload/clash-desktop-ui', 'payload/clash-tun-launcher',
                     'payload/cor', 'payload/resources/mihomo', 'tests/core_integration')
            manifest['linux_elf'] = {name: {'sha256': manifest['files'][name]['sha256'],
                                            'glibc': ['GLIBC_2.31']} for name in names}
            with patch.object(artifacts, 'ROOT', root), patch.object(artifacts, 'package_version', return_value=self.version), \
                    patch.object(build_inputs, 'manifest', return_value=snapshot):
                build.write_json(root/'input/manifest.json', manifest)
                artifacts.verify_build(root/'input', 'linux', 'x64', self.commit)
                for case in ('image', 'missing', 'hash', 'glibc'):
                    altered = copy.deepcopy(manifest)
                    if case == 'image':
                        altered['linux_baseline']['image'] = 'ubuntu@sha256:'+'b'*64
                    elif case == 'missing':
                        del altered['linux_elf'][names[0]]
                    elif case == 'hash':
                        altered['linux_elf'][names[0]]['sha256'] = '0'*64
                    else:
                        altered['linux_elf'][names[0]]['glibc'] = ['GLIBC_2.32']
                    build.write_json(root/'input/manifest.json', altered)
                    with self.subTest(case=case), self.assertRaises(ValueError):
                        artifacts.verify_build(root/'input', 'linux', 'x64', self.commit)
                local = copy.deepcopy(manifest)
                del local['linux_baseline'], local['linux_elf']
                build.write_json(root/'input/manifest.json', local)
                artifacts.verify_build(root/'input', 'linux', 'x64', self.commit)
                with patch.dict(os.environ, {'GITHUB_ACTIONS': 'true'}), self.assertRaises(ValueError):
                    artifacts.verify_build(root/'input', 'linux', 'x64', self.commit)
            with patch.object(artifacts, 'ROOT', root), patch.object(artifacts, 'package_version', return_value=self.version), \
                    patch.object(build_inputs, 'manifest', return_value=None), patch.dict(os.environ, {'GITHUB_ACTIONS': 'true'}):
                with self.assertRaisesRegex(ValueError, 'requires the CI dependency snapshot'):
                    artifacts.verify_build(root/'input', 'linux', 'x64', self.commit)

    def test_archive_rejects_traversal_links_duplicates_and_special_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for number, kind in enumerate(('traversal', 'symlink', 'duplicate', 'device')):
                archive = root/f'{number}.tar.gz'
                with tarfile.open(archive, 'w:gz') as tar:
                    info = tarfile.TarInfo('../outside' if kind == 'traversal' else 'payload/file')
                    if kind == 'symlink':
                        info.type = tarfile.SYMTYPE; info.linkname = '../outside'
                    elif kind == 'device':
                        info.type = tarfile.CHRTYPE
                    tar.addfile(info)
                    if kind == 'duplicate':tar.addfile(info)
                destination = root/f'extracted-{number}'; destination.mkdir()
                with self.subTest(kind=kind), self.assertRaises(ValueError):
                    artifacts.extract_build(archive, destination)
            self.assertFalse((root/'outside').exists())

    def test_authenticated_build_fetch_checks_both_archive_hashes_and_restores_modes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); self.fixture(root)
            archive = root/'payload.tar.gz'
            with tarfile.open(archive, 'w:gz') as tar:
                for path in (root/'input').rglob('*'):
                    if path.is_file():tar.add(path, arcname=path.relative_to(root/'input').as_posix())
            data = archive.read_bytes()
            def client(bad_outer=False, bad_inner=False):
                stream = io.BytesIO()
                with zipfile.ZipFile(stream, 'w') as zip:
                    zip.writestr(artifacts.ARCHIVE_NAME, data)
                    zip.writestr(artifacts.ARCHIVE_NAME+'.sha256', ('0'*64 if bad_inner else hashlib.sha256(data).hexdigest())+'\n')
                blob = stream.getvalue()
                artifact = {'id': 1, 'name': 'build-linux-x64', 'expired': False,
                            'digest': 'sha256:'+('0'*64 if bad_outer else hashlib.sha256(blob).hexdigest())}
                return lambda path, raw=False: blob if raw else {'artifacts': [artifact]}
            with patch.object(artifacts, 'ROOT', root), patch.object(artifacts, 'package_version', return_value=self.version):
                destination = root/'restored'
                artifacts.fetch_build(client(), 77, 'linux', 'x64', destination, self.commit)
                self.assertEqual(build.sha256(destination/'payload/clash-of-rust'), build.sha256(root/'input/payload/clash-of-rust'))
                if build.os.name != 'nt':
                    self.assertEqual((destination/'payload/clash-of-rust').stat().st_mode & 0o777, 0o755)
                for key in ('bad_outer', 'bad_inner'):
                    with self.subTest(key=key), self.assertRaises(ValueError):
                        artifacts.fetch_build(client(**{key: True}), 77, 'linux', 'x64', root/key, self.commit)

    def test_prebuilt_packaging_never_invokes_cargo_or_downloads(self):
        spec = importlib.util.spec_from_file_location('installer', build.ROOT/'scripts/build-installer.py')
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        args = argparse.Namespace(prebuilt=Path('ci-build'), system='linux', arch='x64')
        with patch.object(artifacts, 'verify_build'), patch.object(artifacts, 'commit', return_value=self.commit), \
             patch.object(module, 'package_version', return_value=self.version), \
             patch.object(module, 'run') as run, patch.object(build, 'download') as download:
            resources, binary = module.build_inputs(args, self.version)
            self.assertEqual(binary, Path('ci-build/payload/clash-of-rust'))
            self.assertEqual(resources, Path('ci-build/payload/resources'))
            run.assert_not_called(); download.assert_not_called()
            with self.assertRaises(ValueError):module.build_inputs(args, '9.9.9')

    def test_gui_fixture_and_report_preserve_all_workloads_and_unknown_metrics(self):
        spec = importlib.util.spec_from_file_location('memory', build.ROOT/'scripts/run-memory-benchmark.py')
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        self.assertEqual(module.fixture(), module.fixture('light'))
        for workload, nodes, groups, rules in (('light', 100, 5, 1000), ('stress', 2000, 40, 20000)):
            fixture = module.fixture(workload)
            with self.subTest(workload=workload):
                self.assertEqual(fixture.count('type: http'), nodes)
                self.assertEqual(fixture.count('type: select'), groups)
                self.assertEqual(fixture.count('DOMAIN-SUFFIX'), rules)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            rows = [{'type': 'api', 'workload': workload, 'mode': mode, 'refreshes': refresh,
                     'peak_extra_heap_bytes': 1048576, 'retained_extra_heap_bytes': 524288}
                    for workload in ('rules', 'connections', 'proxies')
                    for mode in ('streamed', 'buffered') for refresh in (1, 5)]
            (output/'api.jsonl').write_text('\n'.join(json.dumps(row) for row in rows))
            sample = {'elapsed_ms': 0, 'gui': {'rss_bytes': 1048576}, 'core': {'rss_bytes': 2097152}, 'total': {'pss_bytes': None}}
            for scene in ('home', 'proxies-collapsed', 'proxies-search', 'home-after-proxies'):
                (output/(scene+'.jsonl')).write_text(json.dumps(sample))
            metadata = {'source_commit': self.commit, 'version': self.version, 'profile': 'release',
                        'repetitions': 1, 'workload': 'light', 'rows': 1000,
                        'gui_fixture': {'nodes': 100, 'groups': 5, 'rules': 1000}}
            with patch.dict(build.os.environ, {}, clear=True):module.summarize(output, metadata)
            report = (output/'README.md').read_text()
            self.assertIn('| proxies | streamed | 5 | 1.00 | 0.50 |', report)
            self.assertIn('| home | 1.00 | 2.00 | unknown |', report)
            self.assertIn('100 nodes, 5 groups × 100 members, 1,000 rules', report)
            self.assertNotIn('2,000 nodes', report)


if __name__ == '__main__':
    unittest.main()
