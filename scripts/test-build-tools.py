"""Offline tests for resource validation and clean package staging."""
import hashlib
import json
from pathlib import Path
import tempfile
import tomllib
import struct
import shutil
import subprocess
import unittest
from unittest.mock import MagicMock, patch
import build_support as build
import android as android_build
import windows_ci as windows
import workflow_storage as storage
import action_cache as local_cache
import cache_shards
import build_inputs
import os
import copy
import importlib.util
import io
import urllib.error
import urllib.parse
import zipfile
import tarfile
from contextlib import nullcontext

_spec = importlib.util.spec_from_file_location('publish_release', build.ROOT/'scripts/publish-release.py')
release = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(release)



class BuildTests(unittest.TestCase):
    def setUp(self):
        # Fixtures must not inherit the workflow's real dependency snapshot.
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        override = patch.dict(os.environ, {'BUILD_INPUTS_DIR': directory.name})
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
            (root/'.github/workflows').mkdir(parents=True)
            workflow = root/'.github/workflows/ci.yml'
            workflow.write_text('ndk: 28')
            before = local_cache.rust_keys(root, b'compiler one', {})[0]
            workflow.write_text('ndk: 29')
            self.assertNotEqual(before, local_cache.rust_keys(root, b'compiler one', {})[0])
            (root/'Cargo.lock').write_text('slint = 1.19.0')
            new_prefix = local_cache.rust_keys(root, b'compiler one', {})[1]
            self.assertNotEqual(prefix, new_prefix)
            # The compiler-compatible fallback may retain unrelated libraries;
            # Cargo still resolves/builds against the new lockfile.
            self.assertTrue(prefix.startswith(local_cache.rust_environment_key(b'compiler one', {})))
            self.assertTrue(new_prefix.startswith(local_cache.rust_environment_key(b'compiler one', {})))
            self.assertNotEqual(new_prefix, local_cache.rust_keys(root, b'compiler two', {})[1])
            self.assertNotEqual(new_prefix, local_cache.rust_keys(root, b'compiler one', {'RUSTFLAGS':'-C target-cpu=native'})[1])

    def test_copy_static_is_idempotent_for_an_existing_wsl_resource_tree(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root/'resources').mkdir()
            (root/'resources/ip-check').mkdir()
            wsl = root/'scripts/wsl-secure-egress'
            for name in build.RESOURCE_NAMES:
                (root/'resources'/name).write_text(name)
            for relative in ('fonts/LICENSE.txt', 'flags/LICENSE-Twemoji.txt',
                             'flags/LICENSE-Unicode.txt'):
                path = root/'resources'/relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(relative)
            for name in ('LICENSE', 'SOURCE.md'):
                (root/'resources/ip-check'/name).write_text(name)
            for name in build.WSL_EGRESS_TOP:
                path = wsl/name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(name)
            for directory, names in (('linux', build.WSL_EGRESS_LINUX),
                                     ('server', build.WSL_EGRESS_SERVER)):
                for name in names:
                    path = wsl/directory/name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(name)
            (root/'LICENSE').write_text('license')
            bundle = root/'bundle/linux-x64'
            bundle.mkdir(parents=True)
            with patch.object(build, 'ROOT', root):
                build.copy_static(bundle)
                build.copy_static(bundle)
            copied = bundle/'resources/wsl-secure-egress'
            self.assertEqual(
                {path.relative_to(copied).as_posix() for path in copied.rglob('*') if path.is_file()},
                set(build.WSL_EGRESS_TOP)
                | {'linux/'+name for name in build.WSL_EGRESS_LINUX}
                | {'server/'+name for name in build.WSL_EGRESS_SERVER},
            )

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

    def test_workspace_cache_verifies_contents_before_restoring_times_and_invalidates_changed_crates(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            for name in ('ui','core'):
                folder=root/'crates'/name;folder.mkdir(parents=True)
                (folder/'Cargo.toml').write_text('[package]\nname="clash-'+name+'"\nversion="1.0.0"\n')
                (folder/'lib.rs').write_bytes(b'original source')
            java=root/'apps/android/app/MainActivity.java';java.parent.mkdir(parents=True)
            java.write_bytes(b'original Java host')
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
                v2=root/'target/release/.fingerprint/clash-ui/another-hash'
                v2.mkdir(parents=True);(v2/'lib-info').write_bytes(b'fingerprint')
                # Checkout made unchanged sources newer. Conversely, changed
                # contents can belong to an older branch timestamp.
                os.utime(core,ns=(previous+1_000_000,previous+1_000_000))
                ui.write_bytes(b'changed source');os.utime(ui,ns=(1,1))
                java.write_bytes(b'changed Android host; unrelated to desktop Rust libraries')
                local_cache.workspace_sources(root)
            self.assertEqual(core.stat().st_mtime_ns,previous)
            self.assertEqual(ui.read_bytes(),b'changed source')
            self.assertFalse((root/'target/release/.fingerprint/clash-ui-hash').exists())
            self.assertFalse(v2.exists())
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
                shard=next(s for s in restored['shard_index']['shards'] if s)
                store.path(shard['group'],shard['id']).write_bytes(b'corrupt')
                with self.assertRaisesRegex(ValueError,'Incomplete|checksum'):
                    cache_shards.restore(restored,False)

    def test_rust_shards_reject_indexes_pointing_outside_their_cache_group(self):
        record={'schema':local_cache.SCHEMA,'group':'rust/linux-x64/p00','key':'compiler-object',
                'id':'a'*64,'sha256':'b'*64,'size':1024}
        cache_shards.validate(record,'rust/linux-x64/p00')
        with self.assertRaises(ValueError): cache_shards.validate(record,'rust/windows-x64/p00')
        for field,value in (('id','../escape'),('size',-1),('sha256','bad')):
            with self.subTest(field=field),self.assertRaises(ValueError):
                cache_shards.validate({**record,field:value},record['group'])

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
                    patch.object(android_build,'ensure_binding_tool'),patch.object(build_inputs.subprocess,'run'):
                build_inputs.shared_go(out)
                module.unlink()
                build_inputs.shared_go(out,restore=True)
                self.assertEqual(module.read_bytes(),b'verified source zip')
                mod.write_bytes(b'changed dependency')
                with self.assertRaises(ValueError): build_inputs.shared_go(out,restore=True)
                mod.write_bytes(b'module test')
                archive=out/'go-source.tar.gz';archive.write_bytes(archive.read_bytes()+b'tampered')
                with self.assertRaises(ValueError): build_inputs.shared_go(out,restore=True)

    def test_cache_timing_report_never_copies_credentials_or_private_action_state(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            config={'group':'rust/linux-x64','key':'verified','token':'never-publish-this-token',
                    'url':'http://100.64.3.1:19081','started':10,
                    'timings':{'download':1.25,'token':'never-publish-this-token'}}
            with patch.dict(os.environ,{'GITHUB_WORKSPACE':str(root),'GITHUB_STEP_SUMMARY':str(root/'summary.txt')}), \
                    patch.object(local_cache.time,'monotonic',return_value=12):
                local_cache.cache_report(config,'restore','exact',1024)
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
                         'dependency-snapshot/inputs.json','android-report/logcat.txt'):
                path=target/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(b'preserve only reports/inputs')
            unpacked=root/'cargo/registry/src/crate/lib.rs'
            unpacked.parent.mkdir(parents=True);unpacked.write_bytes(b'incomplete restored source')
            with patch.dict(os.environ,{'GITHUB_ACTIONS':'true','CARGO_HOME':str(root/'cargo'),
                                       'WORKSPACE_SOURCE_CACHE':str(root/'external/source.json')}):
                local_cache.discard_restore({'kind':'rust','source':str(root)})
            self.assertFalse((target/'release').exists())
            self.assertFalse((target/'aarch64-linux-android/release').exists())
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
            with patch.object(local_cache,'response',side_effect=upload),patch.object(local_cache,'cache_report'):
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
                    'available':True,'save':True,'hit':False,'mirror_required':False}
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
                     'dependency-snapshot/inputs.json', 'cache-network.json', 'android-report/logcat.txt')
            for name in names:
                path = source/name; path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(b'binary')
            archive = root/'cache.tar.gz'
            local_cache.pack([source], archive, rust=True)
            local_cache.unpack(archive, [root/'restored'])
            files = sorted(p.relative_to(root/'restored').as_posix() for p in (root/'restored').rglob('*') if p.is_file())
            self.assertEqual(files, ['release/build/tool/build-script-build.exe', 'release/deps/dep.rlib'])

    def test_source_dependency_network_failure_restores_previous_snapshot_but_bad_constraints_fail(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            lock = root/'Cargo.lock'; lock.write_bytes(b'cached')
            for message in ('503 Service Unavailable', '429 Too Many Requests',
                            'reading https://proxy.example/module: 429 Too Many Requests',
                            'go: Get "https://proxy.example": dial tcp: lookup proxy.example: no such host',
                            'dial tcp 192.0.2.1:443: no route to host'):
                def failure(*args, **kwargs):
                    lock.write_bytes(b'incomplete replacement')
                    return subprocess.CompletedProcess(args[0], 1, message)
                with self.subTest(message=message), patch.object(build_inputs, 'ROOT', root), \
                     patch.object(build_inputs.subprocess, 'run', side_effect=failure):
                    warnings = []
                    build_inputs.refresh_command(['cargo','update'], ('Cargo.lock',), warnings)
                    self.assertTrue(warnings)
                    self.assertEqual(lock.read_bytes(), b'cached')
            with patch.object(build_inputs, 'ROOT', root), patch.object(build_inputs.subprocess, 'run',
                    return_value=subprocess.CompletedProcess([],1,'failed to select a version: incompatible constraints')):
                with self.assertRaisesRegex(RuntimeError, 'not a network'):
                    build_inputs.refresh_command(['cargo','update'], ('Cargo.lock',), [])
            with patch.object(build_inputs, 'ROOT', root), patch.object(build_inputs.subprocess, 'run',
                    side_effect=subprocess.TimeoutExpired(['cargo','update'],180)):
                warnings = []
                build_inputs.refresh_command(['cargo','update'], ('Cargo.lock',), warnings)
                self.assertTrue(warnings)
                self.assertEqual(lock.read_bytes(), b'cached')

    def test_go_refresh_checks_exact_modules_without_downgrades_and_retains_unreachable_versions(self):
        modules=[{'Path':'go4.org/netipx','Version':'v0.0.0-20260823151212-3075585bcbeb'},
                 {'Path':'example.org/Module','Version':'v1.2.0'},
                 {'Path':'example.org/offline','Version':'v2.0.0'}]
        queries=[]
        class Fetch:
            def open(self,url,**kwargs):
                queries.append(url)
                if '/offline/' in url: raise urllib.error.URLError('offline')
                value='v1.3.0' if '!module' in url else 'v0.0.0-20260112195520-a5071408f32f'
                return io.BytesIO(json.dumps({'Version':value}).encode())
        with patch.object(build_inputs.subprocess,'check_output',return_value=json.dumps({'Require':modules})), \
                patch.object(build_inputs,'opener',return_value=Fetch()),patch.object(build_inputs,'refresh_command') as update:
            warnings=build_inputs.refresh_go()
        commands=[call.args[0] for call in update.call_args_list]
        self.assertTrue(any(command[-1]=='example.org/Module@v1.3.0' for command in commands))
        self.assertFalse(any('-u' in command or './...' in command for command in commands))
        self.assertIn('https://proxy.golang.org/go4.org/netipx/@latest',queries)
        self.assertFalse(any(url.endswith('/go4.org/@latest') for url in queries))
        self.assertTrue(any('offline' in warning for warning in warnings))
        self.assertGreater(build_inputs.go_version('v1.0.0'),build_inputs.go_version('v1.0.0-rc.1'))
        self.assertGreater(build_inputs.go_version('v1.0.0-rc.10'),build_inputs.go_version('v1.0.0-rc.2'))

    def test_release_metadata_network_fallback_respects_explicit_version(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)/'core.json'
            cached = {'tag_name':'v1.2.3','assets':[]}
            path.write_text(json.dumps({'release':cached}))
            with patch.object(build_inputs, 'component', return_value=None), \
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
                 patch.object(build_inputs, 'nsis_release', side_effect=urllib.error.URLError('unavailable')):
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

    def test_cache_budget_evicts_old_lockfiles_then_least_recently_used_entries(self):
        def entry(number, key, size, ref='refs/heads/main'):
            return {'id': number, 'key': key, 'size_in_bytes': size, 'ref': ref,
                    'last_accessed_at': f'2026-10-16T10:00:{number:02d}Z',
                    'created_at': f'2026-10-16T09:00:{number:02d}Z'}
        old = entry(1, 'native-release-linux-check-Linux-x64-aaaaaaaa-bbbbbbbb', 30)
        new = entry(2, 'native-release-linux-check-Linux-x64-aaaaaaaa-cccccccc', 30)
        go = entry(3, 'source-go-version-modules', 40)
        gradle = entry(4, 'gradle-content-unique-blob', 40)
        # Stale revisions are removed even below budget; current dependencies
        # are otherwise preserved. An 80-byte budget also evicts the oldest.
        self.assertEqual([c['id'] for c in storage.eviction_plan([old, new, go, gradle], 200)], [1])
        self.assertEqual([c['id'] for c in storage.eviction_plan([old, new, go, gradle], 80)], [1, 2])
        other_branch = entry(5, old['key'], 30, 'refs/pull/18/merge')
        self.assertEqual(storage.eviction_plan([new, other_branch], 100), [])
        compiler_old = entry(6, 'macos-compiler-arm64-' + 'a'*16 + '-' + 'b'*64, 20)
        compiler_new = entry(7, 'macos-compiler-arm64-' + 'a'*16 + '-' + 'c'*64, 20)
        self.assertEqual(storage.eviction_plan([compiler_old, compiler_new], 100), [compiler_new,compiler_old])
        legacy=entry(12,'android-go-Linux-X64-old-modules',20)
        self.assertEqual(storage.eviction_plan([legacy],100),[legacy])
        self.assertEqual(storage.eviction_plan([gradle], 10), [gradle])
        desktop=entry(8,'native-release-v2-macos-15-intel-check-Linux-x64-aaaaaaaa-bbbbbbbb',60)
        linux=entry(9,'native-release-v2-ubuntu-22.04-check-Linux-x64-aaaaaaaa-bbbbbbbb',60)
        self.assertEqual([c['id'] for c in storage.eviction_plan([desktop,linux],60)],[9])
        source_old=entry(10,'source-cargo-'+'a'*64+'-1-1',10)
        source_new=entry(11,'source-cargo-'+'b'*64+'-2-1',10)
        self.assertEqual([c['id'] for c in storage.eviction_plan([source_old,source_new],100)],[10])
        for older,newer in (
                ('android-go-Linux-X64-1.26.0-'+'a'*64,'android-go-Linux-X64-1.27.0-'+'b'*64),
                ('skia-source-Linux-X64-0.153.3-'+'a'*64,'skia-source-Linux-X64-0.154.0-'+'b'*64),
                ('resources-macos-x64-'+'a'*64+'-2026-10-16','resources-macos-x64-'+'b'*64+'-'+'c'*64),
                ('tools-nsis-'+'a'*64,'tools-nsis-'+'b'*64),
                ('action-setup-tailscale/1.94.2/Linux-amd64','action-setup-tailscale/1.104.1/Linux-amd64')):
            self.assertEqual(storage.lineage(older),storage.lineage(newer))
        with self.assertRaises(ValueError): storage.eviction_plan([], 0)

    def test_cache_cleanup_rechecks_size_and_fork_report_never_deletes(self):
        entries = [{'id': 1, 'key': 'tool-one', 'ref': 'refs/heads/main', 'size_in_bytes': 30,
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
            report = storage.prune(client, 50)
            self.assertEqual(writes, ['/actions/caches/1'])
            self.assertEqual(report['after'], 50)
            self.assertEqual(report['removed'], 1)

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
        workflow = (build.ROOT/'.github/workflows/release.yml').read_text()
        for name in release.PACKAGE_JOBS - set(release.PACKAGES):
            self.assertIn('    name: '+name+'\n', workflow)

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
                    windows.scan_installers([installer])
                execute.assert_not_called()

    def test_defender_scan_blocks_threats_and_changed_installer(self):
        with tempfile.TemporaryDirectory() as directory:
            installer = Path(directory)/'installer.exe'
            original = b'installer fixture'
            installer.write_bytes(original)
            with patch.object(windows, 'update_signatures', return_value=Path('new/MpCmdRun.exe')), \
                 patch('sys.stdout', io.StringIO()):
                with patch.object(windows.subprocess, 'run', side_effect=subprocess.CalledProcessError(2, ['scan'])) as execute:
                    with self.assertRaises(subprocess.CalledProcessError):
                        windows.scan_installers([installer])
                    self.assertEqual(execute.call_args.args[0][0], str(Path('new/MpCmdRun.exe')))
                    self.assertIn('-DisableRemediation', execute.call_args.args[0])
                    self.assertTrue(execute.call_args.kwargs['check'])
                def changed(*args, **kwargs):
                    installer.write_bytes(b'changed fixture')
                    return subprocess.CompletedProcess(args[0], 0)
                with patch.object(windows.subprocess, 'run', side_effect=changed):
                    with self.assertRaisesRegex(RuntimeError, 'Installer changed'):
                        windows.scan_installers([installer])

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

    def test_wsl_egress_resource_allowlist_rejects_missing_and_extra_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            resources = root/'resources'
            wsl = resources/'wsl-secure-egress'
            wsl.mkdir(parents=True)
            for name in build.WSL_EGRESS_TOP:
                (wsl/name).write_text('fixture '+name)
            for folder, names in (('linux', build.WSL_EGRESS_LINUX),
                                  ('server', build.WSL_EGRESS_SERVER)):
                (wsl/folder).mkdir()
                for name in names:
                    (wsl/folder/name).write_text('fixture '+name)
            (resources/'mihomo.exe').write_bytes(b'core')
            build.write_json(resources/'core.json', {'exe_sha256': build.sha256(resources/'mihomo.exe')})
            geo = []
            for name in build.GEO_NAMES:
                (resources/name).write_text('fixture '+name)
                geo.append({'name': name, 'sha256': build.sha256(resources/name),
                            'size': (resources/name).stat().st_size})
            build.write_json(resources/'geodata.json', {'files': geo})
            build.validate_resources(resources, 'windows')
            (wsl/'injected.txt').write_text('unexpected')
            with self.assertRaisesRegex(ValueError, 'resource set'):
                build.validate_resources(resources, 'windows')
            (wsl/'injected.txt').unlink()
            (wsl/'server/manage.py').unlink()
            with self.assertRaisesRegex(ValueError, 'resource set'):
                build.validate_resources(resources, 'windows')

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
        wsl = source/'wsl-secure-egress'
        wsl.mkdir()
        for name in build.WSL_EGRESS_TOP:
            (wsl/name).write_bytes(('fixture '+name).encode())
        for folder, names in (('linux', build.WSL_EGRESS_LINUX),
                              ('server', build.WSL_EGRESS_SERVER)):
            (wsl/folder).mkdir()
            for name in names:
                (wsl/folder/name).write_bytes(('fixture '+name).encode())

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

    def test_release_check_mode_never_calls_publisher(self):
        with patch.object(release.sys,'argv',['publish-release.py','--check','--run-id','77','--package-run-id','88']), \
             patch.object(release,'repository',return_value='example/project'), \
             patch.object(release,'credential',return_value='test-only-token'), \
             patch.object(release,'GitHub'), patch.object(release,'git',return_value=self.commit), \
             patch.object(release,'verified_run',return_value={'id':77,'html_url':'https://example.test/ci'}), \
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

    def artifact_client(self, bad_name=False, expired=False, bad_digest=False, duplicate=False, bad_receipt=False, bad_build=False):
        artifacts=[];archives={};builds=[]
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
            if bad_receipt:receipt['commit']='b'*40
            if bad_build:receipt['build_artifact']['id']=999
            with zipfile.ZipFile(stream,'w') as zipped:
                zipped.writestr('../outside' if bad_name and number==0 else name,data)
                zipped.writestr(name+'.sha256',hashlib.sha256(data).hexdigest()+'\n')
                zipped.writestr('package-receipt.json',json.dumps(receipt))
            blob=stream.getvalue();archives[number]=blob
            artifacts.append({'id':number,'name':package,'expired':expired,
                              'digest':'sha256:'+('0'*64 if bad_digest else hashlib.sha256(blob).hexdigest())})
        if duplicate:artifacts.append(artifacts[0])
        def request(path, raw=False):
            if path.startswith('/actions/runs/77/artifacts?'):return {'artifacts':builds}
            if path.startswith('/actions/runs/88/artifacts?'):return {'artifacts':artifacts}
            return archives[int(path.split('/')[3])]
        return request

    def test_release_artifact_download_enforces_archive_hash_and_safe_complete_contents(self):
        with tempfile.TemporaryDirectory() as folder:
            directory=Path(folder)
            self.assertEqual(len(release.download_package_files(self.artifact_client(),88,77,self.commit,self.version,directory/'good')),len(release.PACKAGES)*2)
            for key in ('bad_name','expired','bad_digest','duplicate','bad_receipt','bad_build'):
                with self.subTest(key=key), self.assertRaises(ValueError):
                    release.download_package_files(self.artifact_client(**{key:True}),88,77,self.commit,self.version,directory/key)
            self.assertFalse((directory/'outside').exists())

    def test_release_requires_every_package_and_compatibility_job(self):
        def client(changes=None,jobs=None):
            run={'id':88,'path':'.github/workflows/release.yml@main','event':'workflow_dispatch',
                 'head_branch':'main','head_sha':self.commit,'head_repository':{'full_name':'example/project'},
                 'status':'completed','conclusion':'success'}
            run.update(changes or {})
            records=jobs if jobs is not None else [{'name':name,'status':'completed','conclusion':'success'} for name in release.PACKAGE_JOBS]
            def request(path):
                return {'jobs':records} if '/jobs?' in path else run
            request.repo='example/project'
            return request
        with patch.dict(build.os.environ,{},clear=True):
            self.assertEqual(release.verified_package_run(client(),88,self.commit)['id'],88)
            for change in ({'event':'push'},{'head_sha':'b'*40},{'path':'.github/workflows/ci.yml'},
                           {'status':'in_progress'},{'conclusion':'failure'},{'head_branch':'codex/dev-0.5.2'}):
                with self.subTest(change=change),self.assertRaises(ValueError):
                    release.verified_package_run(client(change),88,self.commit)
            jobs=[{'name':name,'status':'completed','conclusion':'success'} for name in release.PACKAGE_JOBS]
            with self.assertRaises(ValueError):release.verified_package_run(client(jobs=jobs[:-1]),88,self.commit)
            jobs[0]['conclusion']='skipped'
            with self.assertRaises(ValueError):release.verified_package_run(client(jobs=jobs),88,self.commit)
            self.assertEqual(release.verified_package_run(client({'head_branch':'codex/dev-0.5.2'}),
                                                         88,self.commit,check_only=True)['id'],88)
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
                 patch.object(release,'build_artifacts'),patch.object(release,'download_package_files') as download, \
                 patch.object(release,'publish') as publishing,patch.dict(build.os.environ,{'GITHUB_OUTPUT':str(output)}):
                release.main()
                download.assert_not_called();publishing.assert_not_called()
            self.assertIn('commit='+self.commit,output.read_text())
            self.assertIn('ci_run_id=77',output.read_text())

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
