"""Export the built bilingual site as one self-contained preview.html for sharing."""
import base64
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DIST = ROOT / 'dist'

def export():
    assets = {}
    for path in (DIST / 'assets').rglob('*'):
        if path.is_file() and path.suffix in ('.webp','.png','.woff2','.svg'):
            mime = {'.woff2':'font/woff2', '.webp':'image/webp', '.png':'image/png', '.svg':'image/svg+xml'}[path.suffix]
            assets[path.relative_to(DIST).as_posix()] = 'data:'+mime+';base64,'+base64.b64encode(path.read_bytes()).decode()
    pages = {p.relative_to(DIST).as_posix(): p.read_text(encoding='utf-8') for p in DIST.rglob('*.html')}
    css = (DIST/'site.css').read_text(encoding='utf-8')
    for key,value in assets.items(): css=css.replace(key,value)
    payload = json.dumps({'pages':pages,'assets':assets,'css':css,'js':(DIST/'site.js').read_text(encoding='utf-8')},ensure_ascii=True).replace('<','\\u003c')
    shell = '''<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Clash of Rust — 网站预览</title><style>html,body{margin:0;height:100%;background:#f8f3e7}iframe{width:100%;height:100%;border:0;display:block}</style><iframe title="Clash of Rust website"></iframe><script id="bundle" type="application/json">PAYLOAD</script><script>
const bundle=JSON.parse(document.getElementById('bundle').textContent);
function showPage(route){
 const [path,hash]=route.split('#');const key=path||'index.html';if(!bundle.pages[key])return;
 let html=bundle.pages[key].replace(/<link[^>]+rel="(?:stylesheet|preload)"[^>]*>/g,'');
 html=html.replace('</head>',()=>'<style>'+bundle.css+'</style></head>');
 html=html.replace(/<script src="[^"]*site.js" defer><\\/script>/,'');
 for(const [name,data] of Object.entries(bundle.assets))html=html.split((key.startsWith('en/')?'../':'')+name).join(data);
 const bridge=`<script>document.addEventListener('click',e=>{const a=e.target.closest('a');if(!a)return;const raw=a.getAttribute('href');if(!raw)return;const u=new URL(raw,'https://preview.invalid/${key}');if(u.origin==='https://preview.invalid'){e.preventDefault();parent.showPage(u.pathname.slice(1)+u.hash)}});${hash?'addEventListener("load",()=>document.getElementById('+JSON.stringify(decodeURIComponent(hash))+')?.scrollIntoView());':''}<\\/script>`;
 const virtual='window.previewLocation={hash:'+JSON.stringify(hash?'#'+hash:'')+',replaceState(_state,_title,next){this.hash=new URL(next,"https://preview.invalid/'+key+'").hash;}};';
 const script=bundle.js.replaceAll('location.hash','window.previewLocation.hash').replaceAll('history.replaceState','window.previewLocation.replaceState');
 html=html.replace('</body>',()=>'<script>'+virtual+script+'<\\/script>'+bridge+'</body>');document.querySelector('iframe').srcdoc=html;
 document.documentElement.lang=key.startsWith('en/')?'en':'zh-CN';
}
showPage(location.hash.slice(1)||'index.html');
</script></html>'''
    (ROOT/'preview.html').write_text(shell.replace('PAYLOAD',payload),encoding='utf-8')
    print('Single-file preview:',ROOT/'preview.html')

if __name__ == '__main__': export()
