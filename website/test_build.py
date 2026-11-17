"""Offline checks for release identity, generated navigation and embedded previews."""
import copy
from html.parser import HTMLParser
import json
from pathlib import Path
import re
import shutil
import struct
import tempfile
import unittest
from unittest import mock
from urllib.parse import unquote, urlsplit
import xml.etree.ElementTree as ET
import build
from release_content import validate

REPO_ROOT = build.ROOT.parent

class Document(HTMLParser):
    def __init__(self):
        super().__init__(); self.ids=set(); self.refs=[]; self.elements=[]
    def handle_starttag(self, tag, attrs):
        attrs=dict(attrs)
        self.elements.append((tag, attrs))
        if 'id' in attrs:
            if attrs['id'] in self.ids: raise ValueError('Duplicate HTML id: '+attrs['id'])
            self.ids.add(attrs['id'])
        self.refs.extend(attrs[key] for key in ('href','src') if key in attrs)
        if 'srcset' in attrs: self.refs.extend(candidate.strip().split()[0] for candidate in attrs['srcset'].split(','))

class SiteTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        temporary = tempfile.TemporaryDirectory(prefix="cor-website-test-")
        cls.addClassCleanup(temporary.cleanup)
        original_root, original_dist = build.ROOT, build.DIST
        cls.addClassCleanup(setattr, build, "ROOT", original_root)
        cls.addClassCleanup(setattr, build, "DIST", original_dist)
        build.ROOT = Path(temporary.name) / "website"
        build.DIST = build.ROOT / "dist"
        build.ROOT.mkdir()
        for name in ("_headers", "_redirects", "robots.txt", "sitemap.xml"):
            shutil.copyfile(original_root/name, build.ROOT/name)
        icons = build.ROOT.parent / "resources/icons"
        icons.mkdir(parents=True)
        for theme in ("light", "dark"):
            shutil.copyfile(REPO_ROOT/f"resources/icons/wordmark-{theme}.svg", icons/f"wordmark-{theme}.svg")
        cls.data=json.loads((build.SRC/'content/releases.json').read_text(encoding='utf-8'))
        build.build()
    def test_current_snapshot(self):
        validate(self.data)
    def test_tool_catalog_matches_the_shared_cli_and_both_languages(self):
        catalog = json.loads((build.SRC/'content/tools.json').read_text())
        core = (REPO_ROOT/'crates/client-core/src/detection/tools/mod.rs').read_text()
        commands = (REPO_ROOT/'apps/cli/src/args.rs').read_text()
        identifier_body = core.split("pub fn id(self)")[1].split("pub fn title(self)")[0]
        identifiers = re.findall(r'Self::\w+ => "([^"]+)"', identifier_body)
        self.assertEqual([tool['id'] for tool in catalog['tools']], identifiers)
        self.assertEqual({tool for row in catalog['symptoms'] for tool in row['tools']}, set(identifiers))
        for tool in catalog['tools']:
            self.assertIn('"network '+tool['id']+'"', commands)
            for lang in ('zh', 'en'):
                for field in ('name', 'short', 'checks', 'when', 'note'):
                    self.assertTrue(tool[lang][field].strip())
            for option in tool['options']:
                self.assertIn(option, catalog['options'])
            if not tool['arg']:
                self.assertFalse(build.tool_command(tool).endswith(' '))
    def test_inconsistent_release_data_rejected(self):
        for field in ('channel','translation','package','date','duplicate','series','highlight','known','prerelease',
                      'history-duplicate','history-translation','history-empty'):
            bad=copy.deepcopy(self.data)
            if field=='channel': bad['releases'][0]['channel']='invalid'
            if field=='translation': bad['releases'][0]['zh']['highlights']=[]
            if field=='package': bad['downloads']['stable']['sizes'].pop('android-arm64.apk')
            if field=='date': bad['downloads']['stable']['published']='2000-01-01T00:00:00Z'
            if field=='duplicate': bad['releases'].append(bad['releases'][0])
            if field=='series': bad['series']='0.6'
            if field=='highlight': bad['releases'][0]['en']['highlights'].pop()
            if field=='known': bad['releases'][0]['en'].pop('known')
            if field=='prerelease': bad['releases'][0]['version'] += '-test.1'
            if field=='history-duplicate': bad['history'].append(bad['history'][0])
            if field=='history-translation': bad['history'][0].pop('en')
            if field=='history-empty': bad['history'][0]['zh']['summary']=' '
            with self.subTest(field=field), self.assertRaises(ValueError): validate(bad)
    def test_navigation_and_assets(self):
        docs={}
        for path in build.DIST.rglob('*.html'):
            parser=Document();parser.feed(path.read_text(encoding='utf-8'));docs[path.resolve()]=parser
        self.assertEqual(len(docs), 2 * len(build.PAGES))
        for path,doc in docs.items():
            for ref in doc.refs:
                url=urlsplit(ref)
                if url.scheme or url.netloc: continue
                target=(path.parent/unquote(url.path)).resolve() if url.path else path
                if target.is_dir(): target/='index.html'
                with self.subTest(page=path.name,ref=ref):
                    self.assertTrue(target.exists())
                    if url.fragment and target in docs:self.assertIn(unquote(url.fragment),docs[target].ids)
    def test_current_navigation_includes_versions_and_manuals(self):
        for language in ("", "en"):
            for name in build.PAGES:
                document = Document()
                source = (build.DIST/language/(name+".html")).read_text()
                document.feed(re.search(r"<header\b.*?</header>", source, flags=re.S)[0])
                current = [attrs["href"] for tag, attrs in document.elements
                           if tag == "a" and attrs.get("aria-current") == "page"]
                expected = "guide" if name in ("tui", "cli") else name
                if name != "index":
                    self.assertTrue(current)
                    self.assertEqual(set(current), {expected+".html"})
                else:
                    self.assertFalse(current)
    def test_theme_assets_and_initialization(self):
        self.assertEqual((build.DIST/"assets/wordmark-dark.svg").read_bytes(),
                         (REPO_ROOT/"resources/icons/wordmark-dark.svg").read_bytes())
        for page in build.DIST.rglob("*.html"):
            source = page.read_text()
            document = Document()
            document.feed(source)
            self.assertTrue(any(tag == "button" and "data-theme-toggle" in attrs
                                for tag, attrs in document.elements))
            self.assertTrue(any(tag == "source" and "data-night" in attrs
                                for tag, attrs in document.elements))
            self.assertLess(source.index('theme.js"></script>'), source.index('rel="stylesheet"'))
            if page.stem != "tui":
                self.assertIn("-night-small.webp", source)
        self.assertEqual((build.DIST/"theme.js").read_bytes(), (build.SRC/"theme.js").read_bytes())
    def test_deployment_metadata_and_content_policy(self):
        for name in ('_headers', '_redirects', 'robots.txt', 'sitemap.xml'):
            self.assertEqual((build.DIST/name).read_bytes(), (build.ROOT/name).read_bytes())
        redirects = (build.DIST/'_redirects').read_text()
        self.assertIn('/features /guide 301', redirects)
        self.assertIn('/zh/:page /:page 301', redirects)
        self.assertIn('Sitemap: https://clash-of-rust.fu021.shop/sitemap.xml',
                      (build.DIST/'robots.txt').read_text())
        entries = ET.parse(build.DIST/'sitemap.xml').getroot()
        self.assertEqual(len(entries), 2 * len(build.PAGES))
        self.assertEqual({urlsplit(entry[0].text).path for entry in entries},
                         {prefix + ('' if name == 'index' else name)
                          for prefix in ('/', '/en/') for name in build.PAGES})
        for entry in entries:
            url = urlsplit(entry[0].text)
            self.assertEqual(url.netloc, 'clash-of-rust.fu021.shop')
            path = build.DIST / url.path.lstrip('/')
            target = path/'index.html' if url.path.endswith('/') else path.with_suffix('.html')
            self.assertTrue(target.is_file(), url.path)
        for page in build.DIST.rglob('*.html'):
            self.assertNotIn(' style=', page.read_text(encoding='utf-8'))
    def test_stable_downloads_work_without_javascript(self):
        tag = self.data['downloads']['stable']['tag']
        for language in ('', 'en'):
            parser = Document()
            parser.feed((build.DIST/language/'download.html').read_text(encoding='utf-8'))
            downloads = [ref for ref in parser.refs if '/releases/download/' + tag + '/' in ref]
            self.assertEqual(len(downloads), 18)
            for suffix in build.SUFFIXES:
                name = 'Clash-of-Rust-' + tag[1:] + '-' + suffix
                self.assertTrue(any(ref.endswith('/' + name) for ref in downloads))
                self.assertTrue(any(ref.endswith('/' + name + '.sha256') for ref in downloads))
            source = (build.DIST/language/'download.html').read_text(encoding='utf-8')
            self.assertIn('<b data-version>' + tag[1:] + '</b>', source)
            self.assertIn(build.REPO_URL + '/releases/tag/' + tag, parser.refs)
            self.assertFalse(any('data-channel' in attrs for _, attrs in parser.elements))
            self.assertNotIn('preview', parser.ids)
            self.assertIn(build.REPO_URL + '/releases/latest', parser.refs)
    def test_historical_summaries_are_available_in_both_languages(self):
        from html import escape
        for language, directory in (('zh', ''), ('en', 'en')):
            source = (build.DIST/directory/'releases.html').read_text(encoding='utf-8')
            document = Document()
            document.feed(source)
            self.assertEqual({identifier for identifier in document.ids if identifier.startswith('v')},
                             {'v'+release['version'] for release in self.data['releases'] + self.data['history']})
            for release in self.data['history']:
                self.assertIn(escape(release[language]['summary']), source)
                self.assertIn(build.REPO_URL + '/releases/tag/v' + release['version'], document.refs)
    def test_chinese_default_and_explicit_english_routes(self):
        for language, html_language in (('', 'zh-CN'), ('en', 'en')):
            for name in build.PAGES:
                source = (build.DIST/language/(name+'.html')).read_text()
                document = Document(); document.feed(source)
                self.assertIn(('html', {'lang': html_language}), document.elements)
                path = '' if name == 'index' else name
                prefix = '/en/' if language else '/'
                links = [attrs for tag, attrs in document.elements if tag == 'link']
                self.assertIn({'rel':'canonical','href':build.SITE_URL+prefix+path}, links)
                alternate = next(attrs for attrs in links if attrs.get('hreflang') == 'x-default')
                self.assertEqual(alternate['href'], build.SITE_URL+'/'+path)
                toggle = next(attrs for tag,attrs in document.elements if 'data-set-lang' in attrs)
                self.assertEqual(toggle['href'], ('../' if language else 'en/')+name+'.html')
                self.assertNotIn('Illustrations were made', source)
                self.assertNotIn('插画为本站绘制', source)
        self.assertNotIn('navigator.language', (build.SRC/'site.js').read_text())
    def test_bilingual_structure_and_manual_commands_match(self):
        for name in build.PAGES:
            chinese = (build.DIST/(name+'.html')).read_text()
            english = (build.DIST/'en'/(name+'.html')).read_text()
            zh = Document(); zh.feed(chinese)
            en = Document(); en.feed(english)
            self.assertEqual(zh.ids, en.ids, name)
            if name in ('start', 'guide', 'tui', 'cli', 'tools'):
                commands = r'<pre class="term"[^>]*>(.*?)</pre>'
                self.assertEqual(re.findall(commands,chinese,re.S), re.findall(commands,english,re.S))
    def test_independent_manuals_and_legacy_anchors(self):
        self.assertTrue({'start', 'guide', 'tui', 'cli'} <= set(build.PAGES))
        legacy = {'gui', 'tui', 'cli', 'tui-first', 'tui-pages', 'tui-manage', 'first',
                  'subscriptions', 'modes', 'rules', 'android', 'checks', 'protection', 'updates', 'trouble'}
        for language in ('', 'en'):
            guide = (build.DIST/language/'guide.html').read_text()
            parser = Document(); parser.feed(guide)
            self.assertTrue(legacy <= parser.ids)
            self.assertNotIn('command-side', guide)
            self.assertNotIn('data-guide-panel', guide)
            for name in ('guide', 'tui', 'cli'):
                document = Document()
                document.feed((build.DIST/language/(name+'.html')).read_text())
                manual_links = {attrs['href'] for tag, attrs in document.elements
                                if tag == 'a' and attrs.get('href') in ('guide.html', 'tui.html', 'cli.html')}
                self.assertEqual(manual_links, {'guide.html', 'tui.html', 'cli.html'})
                self.assertTrue(any(tag == 'details' and 'data-toc' in attrs for tag, attrs in document.elements))
    def test_all_pages_have_illustrations_and_exact_readme_wordmark(self):
        self.assertEqual((build.DIST/'assets/wordmark.svg').read_bytes(),
                         (REPO_ROOT/'resources/icons/wordmark-light.svg').read_bytes())
        for page in build.DIST.rglob('*.html'):
            document = Document(); document.feed(page.read_text())
            self.assertTrue(any(tag == 'img' and attrs.get('src', '').endswith('.webp')
                                and attrs.get('alt') for tag, attrs in document.elements), page.name)
            self.assertTrue(any(tag == 'img' and attrs.get('src', '').endswith('wordmark.svg')
                                for tag, attrs in document.elements), page.name)
        for language in ('', 'en'):
            home = (build.DIST/language/'index.html').read_text()
            self.assertNotIn('hero-narrow.webp', home)
            self.assertNotIn('data-demo-play', home)
    def test_traceable_gui_and_real_terminal_captures(self):
        captures = json.loads((build.SRC/'content/screenshots.json').read_text())
        terminal = json.loads((build.SRC/'content/terminal.json').read_text())
        published = {release['version'] for release in self.data['releases']}
        self.assertEqual(len({capture['version'] for capture in captures.values()}), 1)
        for record in [*captures.values(), *terminal.values()]:
            self.assertIn(record['version'], published)
            self.assertRegex(record['commit'], r'^[0-9a-f]{40}$')
        self.assertEqual(set(captures), set(build.CAPTURES))
        for capture in captures.values():
            image = (build.SRC/'assets'/capture['file']).read_bytes()
            self.assertEqual(image[:8], b'\x89PNG\r\n\x1a\n')
            self.assertEqual(struct.unpack('>II',image[16:24]), (capture['width'],capture['height']))
            self.assertRegex(capture['commit'],r'^[0-9a-f]{40}$')
            self.assertEqual(capture['source'],'production-slint-preview')
        for frame in terminal.values():
            self.assertEqual(frame['source'],'actual-cor-pty')
            self.assertTrue(any(frame['version'] in line['text'] for line in frame['lines']))
        for language in ('','en'):
            for page in ('index','start','guide','tui','tools'):
                source = (build.DIST/language/(page+'.html')).read_text()
                self.assertIn('data-capture-version=',source)
                if page != 'guide':
                    self.assertIn('terminal-capture',source)
                self.assertNotIn('app-home-light.webp',source)
                self.assertNotIn('app-network.webp',source)
    def test_preview_embeds_assets(self):
        import export_preview
        with mock.patch.multiple(export_preview, ROOT=build.ROOT, DIST=build.DIST):
            export_preview.export()
        parser=Document();parser.feed((build.ROOT/'preview.html').read_text(encoding='utf-8'))
        self.assertEqual(parser.refs,[])
        source=(build.ROOT/'preview.html').read_text(encoding='utf-8')
        self.assertTrue('data:font/woff2;base64,' in source, 'Missing embedded WOFF2 font')
        self.assertTrue('data:image/webp;base64,' in source, 'Missing embedded WebP image')
        self.assertTrue('data:image/svg+xml;base64,' in source, 'Missing embedded README wordmark')

if __name__=='__main__':unittest.main()
