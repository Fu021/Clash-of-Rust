"""Offline tests for resource validation and clean package staging."""
import hashlib
import json
from pathlib import Path
import tempfile
import struct
import shutil
import subprocess
import unittest
from unittest.mock import patch
import build_support as build
import copy
import importlib.util
import io
import urllib.error
import urllib.parse
import zipfile

_spec = importlib.util.spec_from_file_location('publish_release', build.ROOT/'scripts/publish-release.py')
release = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(release)


class BuildTests(unittest.TestCase):
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
            for system, arch, machine in [('windows','x64',0x8664),('windows','arm64',0xaa64),('linux','x64',62),('linux','arm64',183)]:
                header = bytearray(64)
                if system == 'windows':
                    header[:2] = b'MZ'
                    struct.pack_into('<I',header,0x3c,64)
                    header.extend(b'PE\0\0'+struct.pack('<H',machine))
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

    def test_wsl_mirror_refuses_source_and_unowned_directory(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location('build_wsl',build.ROOT/'scripts/build-wsl.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)/'source'
            source.mkdir()
            original = source/'keep.txt'
            original.write_text('Original source data')
            unowned = Path(directory)/'unowned'
            unowned.mkdir()
            (unowned/'keep.txt').write_text('Existing unrelated data')
            for destination in (source,unowned):
                with self.assertRaises(ValueError):
                    module.native_build(source,destination,None,None,'')
            self.assertEqual(original.read_text(),'Original source data')
            self.assertEqual((unowned/'keep.txt').read_text(),'Existing unrelated data')

    def test_version_validation(self):
        self.assertEqual(build.validate_version('0.4.4'),'0.4.4')
        for invalid in ('1.2','1.2.3.4','1.2.x','1.2.65536','../0.4.4','01.2.3','0.4.12-dev.01','0.4.12-','0.4.12-dev/1'):
            with self.assertRaises(ValueError):
                build.validate_version(invalid)

    def test_prerelease_installer_versions_preserve_semver_and_upgrade_order(self):
        self.assertEqual(build.validate_version('0.4.12-dev.1'),'0.4.12-dev.1')
        self.assertEqual(build.numeric_version('0.4.12-dev.1'),'0.4.12.0')
        self.assertEqual(build.deb_version('0.4.12-dev.1'),'0.4.12~dev.1')
        self.assertEqual(build.deb_version('0.4.12'),'0.4.12')
        if shutil.which('dpkg'):
            subprocess.run(['dpkg','--compare-versions',build.deb_version('0.4.12-dev.1'),'lt','0.4.12'],check=True)
            subprocess.run(['dpkg','--compare-versions',build.deb_version('0.4.12-dev.1'),'gt','0.4.11'],check=True)

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
        build.write_json(source/'core.json',{'exe_sha256':build.sha256(source/'mihomo.exe')})
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
            (source/'GeoIP.dat').write_bytes(b'corruption')
            with self.assertRaises(ValueError):
                build.validate_resources(source,'windows')

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
            if path.startswith('/actions/runs/77/jobs?'):return {'jobs':jobs}
            if path == '/branches/main':return {'commit':{'sha':main or self.commit}}
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

    def test_release_check_mode_never_calls_publisher(self):
        with patch.object(release.sys,'argv',['publish-release.py','--check','--run-id','77']), \
             patch.object(release,'repository',return_value='example/project'), \
             patch.object(release,'credential',return_value='test-only-token'), \
             patch.object(release,'GitHub'), patch.object(release,'git',return_value=self.commit), \
             patch.object(release,'verified_run',return_value={'id':77,'html_url':'https://example.test/ci'}), \
             patch.object(release,'release_metadata',return_value=(self.version,self.tag,self.notes)), \
             patch.object(release,'download_ci_files',return_value={}), patch.object(release,'publish') as publishing:
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

    def test_release_version_and_notes_are_read_from_tested_commit(self):
        with patch.object(release,'git',return_value='[package]\nversion="0.4.11"'), patch.object(release.subprocess,'check_output',return_value=self.notes):
            self.assertEqual(release.release_metadata(self.commit),(self.version,self.tag,self.notes))
            with self.assertRaises(ValueError):release.release_metadata(self.commit,'v0.4.12')
            with tempfile.TemporaryDirectory() as folder:
                path=Path(folder)/'notes.md';path.write_text('Uncommitted notes')
                with self.assertRaises(ValueError):release.release_metadata(self.commit,notes_path=path)

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

    def artifact_client(self, bad_name=False, expired=False, bad_digest=False, duplicate=False):
        artifacts=[];archives={}
        for number,(package,suffix) in enumerate(release.PACKAGES.items()):
            name=f'Clash-of-Rust-{self.version}-{suffix}'
            data=('installer '+suffix).encode();stream=io.BytesIO()
            with zipfile.ZipFile(stream,'w') as zipped:
                zipped.writestr('../outside' if bad_name and number==0 else name,data)
                zipped.writestr(name+'.sha256',hashlib.sha256(data).hexdigest()+'\n')
            blob=stream.getvalue();archives[number]=blob
            artifacts.append({'id':number,'name':package,'expired':expired,
                              'digest':'sha256:'+('0'*64 if bad_digest else hashlib.sha256(blob).hexdigest())})
        if duplicate:artifacts.append(artifacts[0])
        def request(path, raw=False):
            if path.startswith('/actions/runs/77/artifacts?'):return {'artifacts':artifacts}
            return archives[int(path.split('/')[3])]
        return request

    def test_release_artifact_download_enforces_archive_hash_and_safe_complete_contents(self):
        with tempfile.TemporaryDirectory() as folder:
            directory=Path(folder)
            self.assertEqual(len(release.download_ci_files(self.artifact_client(),77,self.version,directory/'good')),8)
            for key in ('bad_name','expired','bad_digest','duplicate'):
                with self.subTest(key=key), self.assertRaises(ValueError):
                    release.download_ci_files(self.artifact_client(**{key:True}),77,self.version,directory/key)
            self.assertFalse((directory/'outside').exists())

    class PublisherClient:
        def __init__(self, owner, published=False, bad_upload=False, main_changed=False, wrong_tag=False, starter=False):
            self.owner=owner;self.bad_upload=bad_upload;self.main_changed=main_changed;self.writes=[]
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
            if path=='/branches/main':return {'commit':{'sha':'b'*40 if self.main_changed else owner.commit}}
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

    def test_release_publishes_only_after_eight_verified_uploads_and_can_resume_empty_starter(self):
        with tempfile.TemporaryDirectory() as folder:
            files=self.files(Path(folder))
            for starter in (False,True):
                client=self.PublisherClient(self,starter=starter)
                release.publish(client,self.commit,self.tag,self.notes,files)
                self.assertFalse(client.release['draft'])
                self.assertEqual(len(client.release['assets']),8)
                if starter:self.assertTrue(any(method=='DELETE' for _,method,_ in client.writes))

    def test_release_refuses_existing_public_release_or_wrong_tag_without_writes(self):
        with tempfile.TemporaryDirectory() as folder:
            files=self.files(Path(folder))
            for kwargs in ({'published':True},{'wrong_tag':True}):
                client=self.PublisherClient(self,**kwargs)
                with self.assertRaises(ValueError):release.publish(client,self.commit,self.tag,self.notes,files)
                self.assertFalse(client.writes)

    def test_release_leaves_draft_unpublished_after_bad_upload_or_main_change(self):
        with tempfile.TemporaryDirectory() as folder:
            files=self.files(Path(folder))
            for kwargs in ({'bad_upload':True},{'main_changed':True}):
                client=self.PublisherClient(self,**kwargs)
                with self.assertRaises(ValueError):release.publish(client,self.commit,self.tag,self.notes,files)
                self.assertTrue(client.release['draft'])
                self.assertFalse(any(isinstance(data,dict) and data.get('draft') is False for _,_,data in client.writes))


if __name__ == '__main__':
    unittest.main()
