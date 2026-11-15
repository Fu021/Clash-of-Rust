"""Offline checks for release identity, generated navigation and embedded previews."""
import copy
from html.parser import HTMLParser
import json
from pathlib import Path
import unittest
from urllib.parse import unquote, urlsplit
import build
from release_content import validate

class Document(HTMLParser):
    def __init__(self):
        super().__init__(); self.ids=set(); self.refs=[]
    def handle_starttag(self, tag, attrs):
        attrs=dict(attrs)
        if 'id' in attrs:
            if attrs['id'] in self.ids: raise ValueError('Duplicate HTML id: '+attrs['id'])
            self.ids.add(attrs['id'])
        self.refs.extend(attrs[key] for key in ('href','src') if key in attrs)
        if 'srcset' in attrs: self.refs.extend(candidate.strip().split()[0] for candidate in attrs['srcset'].split(','))

class SiteTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data=json.loads((build.SRC/'content/releases.json').read_text(encoding='utf-8'))
        build.build()
    def test_current_snapshot(self):
        validate(self.data)
    def test_inconsistent_release_data_rejected(self):
        for field in ('channel','translation','package','date','duplicate'):
            bad=copy.deepcopy(self.data)
            if field=='channel': bad['releases'][0]['channel']='invalid'
            if field=='translation': bad['releases'][0]['zh']['highlights']=[]
            if field=='package': bad['downloads']['stable']['sizes'].pop('android-arm64.apk')
            if field=='date': bad['downloads']['stable']['published']='2000-01-01T00:00:00Z'
            if field=='duplicate': bad['releases'].append(bad['releases'][0])
            with self.subTest(field=field), self.assertRaises(ValueError): validate(bad)
    def test_navigation_and_assets(self):
        docs={}
        for path in build.DIST.rglob('*.html'):
            parser=Document();parser.feed(path.read_text(encoding='utf-8'));docs[path.resolve()]=parser
        self.assertEqual(len(docs),10)
        for path,doc in docs.items():
            for ref in doc.refs:
                url=urlsplit(ref)
                if url.scheme or url.netloc: continue
                target=(path.parent/unquote(url.path)).resolve() if url.path else path
                if target.is_dir(): target/='index.html'
                with self.subTest(page=path.name,ref=ref):
                    self.assertTrue(target.exists())
                    if url.fragment and target in docs:self.assertIn(unquote(url.fragment),docs[target].ids)
    def test_preview_embeds_assets(self):
        import export_preview
        export_preview.export()
        parser=Document();parser.feed((build.ROOT/'preview.html').read_text(encoding='utf-8'))
        self.assertEqual(parser.refs,[])
        source=(build.ROOT/'preview.html').read_text(encoding='utf-8')
        self.assertTrue('data:font/woff2;base64,' in source, 'Missing embedded WOFF2 font')
        self.assertTrue('data:image/webp;base64,' in source, 'Missing embedded WebP image')

if __name__=='__main__':unittest.main()
