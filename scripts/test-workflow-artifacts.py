"""Offline regression tests for immutable CI payloads and package-only builds."""
import argparse
import copy
import hashlib
import importlib.util
import io
import json
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
    commit = 'a'*40
    version = '0.4.12'

    def fixture(self, root):
        (root/'Cargo.lock').write_text('fixture dependencies')
        payload = root/'input/payload'
        resources = payload/'resources'
        resources.mkdir(parents=True)
        header = bytearray(64)
        header[:6] = b'\x7fELF\x02\x01'
        struct.pack_into('<H', header, 18, 62)
        for path in (payload/'clash-of-rust', payload/'clash-tun-launcher', resources/'mihomo'):
            path.write_bytes(header)
        (payload/'LICENSE').write_text('fixture license')
        build.write_json(resources/'core.json', {'exe_sha256': build.sha256(resources/'mihomo')})
        geo = []
        for name in build.GEO_NAMES:
            path = resources/name
            path.write_text('fixture '+name)
            geo.append({'name': name, 'sha256': build.sha256(path), 'size': path.stat().st_size})
        build.write_json(resources/'geodata.json', {'files': geo})
        tests = {}
        for name in ('library', 'clash-of-rust', 'core_integration', 'tray_integration', 'memory_benchmark', 'linux_tun'):
            path = root/'input/tests'/name
            path.parent.mkdir(exist_ok=True)
            path.write_bytes(header)
            tests[name] = 'tests/'+name
        records = {path.relative_to(root/'input').as_posix():
                   {'sha256': build.sha256(path), 'size': path.stat().st_size,
                    'executable': path.name in tests or path.name in ('clash-of-rust', 'clash-tun-launcher', 'mihomo')}
                   for path in (root/'input').rglob('*') if path.is_file()}
        manifest = {'schema': 1, 'commit': self.commit, 'version': self.version, 'system': 'linux',
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
                for field, value in (('commit', 'b'*40), ('arch', 'arm64'), ('profile', 'dev'), ('cargo_lock_sha256', '0'*64)):
                    manifest = copy.deepcopy(original); manifest[field] = value
                    build.write_json(root/'input/manifest.json', manifest)
                    with self.subTest(field=field), self.assertRaises(ValueError):
                        artifacts.verify_build(root/'input', 'linux', 'x64', self.commit)
                build.write_json(root/'input/manifest.json', original)
                (root/'input/payload/clash-of-rust').write_bytes(b'corrupt executable')
                with self.assertRaises(ValueError):artifacts.verify_build(root/'input', 'linux', 'x64', self.commit)

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
             patch.object(module, 'run') as run, patch.object(module, 'prepare') as prepare:
            resources, binary = module.build_inputs(args, self.version)
            self.assertEqual(binary, Path('ci-build/payload/clash-of-rust'))
            self.assertEqual(resources, Path('ci-build/payload/resources'))
            run.assert_not_called(); prepare.assert_not_called()
            with self.assertRaises(ValueError):module.build_inputs(args, '9.9.9')

    def test_gui_fixture_and_report_preserve_all_workloads_and_unknown_metrics(self):
        spec = importlib.util.spec_from_file_location('memory', build.ROOT/'scripts/run-memory-benchmark.py')
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        fixture = module.fixture()
        self.assertEqual(fixture.count('type: http'), 2000)
        self.assertEqual(fixture.count('type: select'), 40)
        self.assertEqual(fixture.count('DOMAIN-SUFFIX'), 20000)
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
            metadata = {'source_commit': self.commit, 'version': self.version, 'profile': 'release', 'repetitions': 1}
            with patch.dict(build.os.environ, {}, clear=True):module.summarize(output, metadata)
            report = (output/'README.md').read_text()
            self.assertIn('| proxies | streamed | 5 | 1.00 | 0.50 |', report)
            self.assertIn('| home | 1.00 | 2.00 | unknown |', report)


if __name__ == '__main__':
    unittest.main()
