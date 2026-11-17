"""Capture the desktop TUI's pages and editors using the production PTY driver."""
import html
import json
import os
from pathlib import Path
import re
import subprocess
import time

from cli_tui import Session
from workflow_artifacts import commit


PAGES = ('首页', '节点', '连接', '网络', '设置', '操作', '订阅', '快捷规则')
DOWN = b'\x1b[B'
UP = b'\x1b[A'
ESCAPE = b'\x1b'


def settle(terminal):
    deadline = time.monotonic()+.12
    while time.monotonic() < deadline:
        terminal.pump(.02)


def selected(terminal):
    return '\n'.join(line.strip() for line in terminal.text().splitlines()
                     if line.lstrip().startswith('›'))


class Gallery:
    def __init__(self, report):
        self.directory = Path(report)/'desktop'
        self.directory.mkdir(parents=True, exist_ok=True)
        self.path = self.directory/'index.json'
        self.commit = commit()
        previous = json.loads(self.path.read_text()) if self.path.exists() else None
        if previous and previous['commit'] != self.commit:
            raise RuntimeError('Desktop capture directory belongs to a different source commit')
        self.frames = previous['frames'] if previous else []

    def capture(self, terminal, page, title, *, demo=True, size=(120, 40), note=''):
        settle(terminal)
        if max(terminal.screen, default=0) != size[1]-1:
            raise RuntimeError(f'Desktop capture has the wrong height: {title}')
        name = f'{len(self.frames)+1:03d}'
        terminal.capture(self.directory, name)
        self.frames.append(dict(file=name+'.json', page=page, title=title,
                                columns=size[0], rows=size[1], demo=demo, note=note))
        self.path.write_text(json.dumps(dict(source='actual-cor-pty',
                                             commit=self.commit,
                                             run_id=os.environ.get('GITHUB_RUN_ID'),
                                             frames=self.frames), ensure_ascii=False, indent=2))

    def editor(self, terminal, page, title, *, demo=True):
        self.capture(terminal, page, title, demo=demo)
        visited = {selected(terminal)}
        for _ in range(100):
            current = selected(terminal)
            if '[← / →]' in current:
                choices = {current}
                for _ in range(32):
                    terminal.send(b'\x1b[C')
                    settle(terminal)
                    current = selected(terminal)
                    if current in choices:
                        break
                    choices.add(current)
                    self.capture(terminal, page, title+' · '+current.removeprefix('› '),
                                 demo=demo)
                else:
                    raise RuntimeError(f'Desktop candidates did not return to the first value: {title}')
            previous = selected(terminal)
            terminal.send(DOWN)
            settle(terminal)
            current = selected(terminal)
            if current == previous:
                break
            if current in visited:
                raise RuntimeError(f'Desktop editor looped without reaching its action: {title}')
            visited.add(current)
            if any(action in current for action in ('检查并执行 →', '保存设置 →',
                                                    '应用参数 →', '导出 JSON →')):
                self.capture(terminal, page, title+' · 提交入口', demo=demo)
                return
            if len(visited) % 8 == 0:
                self.capture(terminal, page, title+' · 后续字段', demo=demo)
        if not any(action in selected(terminal) for action in
                   ('检查并执行 →', '保存设置 →', '应用参数 →', '导出 JSON →')):
            raise RuntimeError(f'Desktop editor did not expose its action: {title}')

    def render(self):
        sections = []
        for page in PAGES:
            cards = []
            for frame in self.frames:
                if frame['page'] != page:
                    continue
                payload = json.loads((self.directory/frame['file']).read_text())
                if payload['source'] != 'actual-cor-pty':
                    raise RuntimeError('Desktop gallery contains a synthetic terminal frame')
                lines = '\n'.join(
                    f'<span class="{html.escape(line["tone"])}">{html.escape(line["text"])}</span>'
                    for line in payload['lines'])
                title = html.escape(frame['title'])
                mode = '演示数据 · 未执行系统或联网操作' if frame['demo'] else '隔离后台 · 实际只读或夹具操作'
                cards.append(f'<details class="card"><summary>{title}</summary>'
                             f'<p>{frame["columns"]} × {frame["rows"]} 列/行 · {mode}</p>'
                             f'<p>{html.escape(frame["note"])}</p>'
                             f'<pre aria-label="{title}">{lines}</pre>'
                             f'<a href="{frame["file"][:-5]}.txt">原始终端文本</a></details>')
            sections.append(f'<section id="page-{PAGES.index(page)+1}"><h2>{page}'
                            f' <small>{len(cards)} 帧</small></h2>{"".join(cards)}</section>')
        manifest = json.loads(self.path.read_text())
        links = ''.join(f'<a href="#page-{index}">{index} {page}</a>'
                        for index, page in enumerate(PAGES, 1))
        source = html.escape(manifest['commit'] or '未声明；不能据此认定源码版本')
        document = '''<!doctype html><html lang="zh-CN"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Clash of Rust · 桌面终端完整验收</title>
<style>
*{box-sizing:border-box}body{margin:0;background:#10151c;color:#dee7ef;font:16px/1.6 system-ui,sans-serif}
header,main,nav{max-width:1500px;margin:auto;padding:20px 30px}header p{margin:8px 0;color:#afbdcd}
nav{position:sticky;top:0;display:flex;flex-wrap:wrap;gap:16px;background:#1a2330;z-index:1}
a{color:#7ce4b1}h1{margin:0;font-size:28px}h2{margin-top:36px}small{font-size:14px;color:#9eafc2}
.card{margin:14px 0;padding:16px;background:#18212b;border:1px solid #344454;border-radius:10px}
summary{cursor:pointer;font-weight:600}pre{overflow-x:auto;background:#0b1016;padding:18px;
font:14px/1.5 "Noto Sans Mono CJK SC","Sarasa Mono SC","DejaVu Sans Mono",monospace;white-space:pre}
pre span{display:inline}.accent{color:#79dfac}.selected{color:#10151c;background:#79dfac}
.muted{color:#94a3b5}.warning{color:#ffb46b}button,input{font:inherit;padding:7px 12px;margin:5px}
input{width:320px;max-width:100%}.tools{display:flex;flex-wrap:wrap;align-items:center}
</style><header><h1>桌面终端 · 完整验收</h1>
<p>主尺寸 120 × 40，另含 100 × 30 主页面。所有画面由实际 cor 程序经 PTY 捕获，
每页列出当前操作，表单包含后续字段和提交入口。点击条目展开终端原文。</p>
<p>演示数据用于安全检查导航、表单与确认入口；隔离后台帧另行标注。
系统授权、安装、真实代理切换的执行结果不能由这些画面代替。</p>
<p>全局操作：1–8 / Tab 切页，r 刷新，q / Ctrl+C 退出。
列表使用 ↑↓、Enter、/ 搜索与 Esc 返回；表单另有 ←→ 候选值、
Enter 编辑、Ctrl+U 清空、保存/执行确认与取消，具体按键保留在每帧底部。</p>
<p>来源提交：SOURCE · CI：RUN · 共 COUNT 帧</p>
<div class="tools"><input id="search" aria-label="筛选页面或子页面" placeholder="搜索页面、命令、字段或操作">
<button id="expand">展开当前结果</button><button id="collapse">收起全部</button>
<a href="index.json">捕获索引</a></div></header><nav>LINKS</nav><main>SECTIONS</main>
<script>
const cards=[...document.querySelectorAll('.card')];
document.querySelector('#search').addEventListener('input',event=>{
 const query=event.target.value.trim().toLowerCase();
 cards.forEach(card=>{card.hidden=!card.textContent.toLowerCase().includes(query)});
 document.querySelectorAll('section').forEach(section=>{
  section.hidden=![...section.querySelectorAll('.card')].some(card=>!card.hidden);
 });
});
document.querySelector('#expand').onclick=()=>cards.filter(card=>!card.hidden).forEach(card=>card.open=true);
document.querySelector('#collapse').onclick=()=>cards.forEach(card=>card.open=false);
</script></html>'''
        for key, value in (('SOURCE', source), ('RUN', html.escape(manifest['run_id'] or '本地')),
                           ('COUNT', str(len(self.frames))), ('LINKS', links),
                           ('SECTIONS', ''.join(sections))):
            document = document.replace(key, value)
        (self.directory/'index.html').write_text(document, encoding='utf-8')


def command_catalog(console):
    result = subprocess.run([str(console), 'help', 'all'], capture_output=True,
                            text=True, check=True, timeout=15)
    commands = [(match[1], match[2]) for match in re.finditer(
        r'^  ([a-z][a-z-]*(?: [a-z][a-z-]*)*) {2,}(\S.*)$', result.stdout, re.MULTILINE)]
    commands = [(path, description) for path, description in commands
                if path not in ('ui', 'run') and not description.startswith('已移除')]
    if not commands or len({path for path, _ in commands}) != len(commands):
        raise RuntimeError('Cannot derive a unique TUI operation catalogue from CLI help')
    return commands


def visit(terminal, number, expected):
    terminal.send(str(number).encode())
    terminal.wait(expected)


def search(terminal, query):
    terminal.send(b'/\x15'+query.encode()+b'\r')
    settle(terminal)


def review_operation(terminal, gallery, path):
    terminal.send(b'\r')
    settle(terminal)
    gallery.capture(terminal, '操作', path+' · 校验或执行确认',
                    note='必填项为空时展示校验拒绝；本帧不代表执行成功。')
    if '执行 cor '+path not in terminal.text():
        examples = {
            'profile import': {'--name': '演示订阅'},
            'profile edit': {'--name': '演示订阅'},
            'node test': {'NODE': '演示节点'},
            'connection close': {'ID': 'preview-id'},
            'logs export': {'--output': '/tmp/cor-preview-only.json'},
            'update download': {'--output': '/tmp/cor-preview-only.json'},
            'rule add': {"'TYPE,VALUE,POLICY'": 'DOMAIN-SUFFIX,example.test,DIRECT'},
            'rule edit': {"'TYPE,VALUE,POLICY'": 'DOMAIN-SUFFIX,example.test,DIRECT'},
            'rule move': {'POSITION': '1'},
            'rule enable': {'on|off': 'on'},
            'completion': {'SHELL': 'bash'},
        }.get(path, {})
        for _ in range(100):
            previous = selected(terminal)
            terminal.send(UP)
            settle(terminal)
            if selected(terminal) == previous:
                break
        for _ in range(100):
            current = selected(terminal)
            if '检查并执行 →' in current:
                break
            label = current.removeprefix('› ').split('：', 1)[0].split('（', 1)[0]
            if '（必填）' in current or label in examples:
                terminal.send(b'\r')
                terminal.wait('输入中')
                terminal.send(examples.get(label, 'preview-value').encode()+b'\r')
                settle(terminal)
            terminal.send(DOWN)
            settle(terminal)
        else:
            raise RuntimeError('Operation fields did not reach the confirmation action: '+path)
        terminal.send(b'\r')
        terminal.wait('执行 cor '+path)
        gallery.capture(terminal, '操作', path+' · 演示参数与执行确认',
                        note='参数均为虚构输入；仅通过 CLI 参数校验并展示确认，未执行业务操作。')
    terminal.send(ESCAPE)
    settle(terminal)
    terminal.send(ESCAPE)
    terminal.wait('管理操作')


def verify_desktop(console, command, report):
    gallery = Gallery(report)
    with Session([*command, '--demo'], columns=120, rows=40) as terminal:
        for number, (page, expected) in enumerate(zip(PAGES, (
                '参考线路', '策略组', '活跃连接', '网站检查', '混合代理端口',
                '管理操作', '订阅 · Enter', '快捷规则 · 当前订阅')), 1):
            visit(terminal, number, expected)
            gallery.capture(terminal, page, page+' · 主页面')
            terminal.resize(100, 30)
            terminal.wait(expected, rows=30)
            gallery.capture(terminal, page, page+' · 紧凑桌面', size=(100, 30))
            terminal.resize(120, 40)
            terminal.wait(expected, rows=40)

        visit(terminal, 2, '策略组')
        search(terminal, 'PROXY')
        terminal.send(b'\r')
        terminal.wait('PROXY   ·')
        gallery.capture(terminal, '节点', '手动策略组 · 成员与当前节点')
        for mode in ('延迟从高到低', '名称排序', '延迟从低到高'):
            terminal.send(b's')
            terminal.wait(mode)
            gallery.capture(terminal, '节点', '排序 · '+mode)
        search(terminal, '日本')
        terminal.send(b'\r')
        terminal.wait('切换 PROXY')
        gallery.capture(terminal, '节点', '节点切换 · 确认与取消')
        terminal.send(ESCAPE)
        settle(terminal)
        terminal.send(ESCAPE)
        search(terminal, 'AUTO')
        terminal.send(b'\r')
        terminal.wait('AUTO   ·')
        gallery.capture(terminal, '节点', '自动策略组 · 只读状态')

        visit(terminal, 3, 'github.com')
        search(terminal, 'api.github.com')
        gallery.capture(terminal, '连接', '连接搜索 · 嗅探域名匹配')
        terminal.send(b'\r')
        terminal.wait('连接详情')
        gallery.capture(terminal, '连接', '连接详情 · 域名、嗅探、规则与策略链')
        for key, title in ((b'd', '补充解析'), (b'x', '关闭连接')):
            terminal.send(key)
            terminal.wait('cor connection')
            gallery.editor(terminal, '连接', title+' · 命令表单')
            terminal.send(b'\r')
            terminal.wait('执行 cor connection')
            gallery.capture(terminal, '连接', title+' · 确认与取消')
            terminal.send(ESCAPE)
            settle(terminal)
            terminal.send(ESCAPE)
            terminal.wait('连接详情')

        visit(terminal, 4, '网站检查')
        tools = [path.split()[1] for path, _ in command_catalog(console)
                 if path.startswith('network ')]
        for index, tool in enumerate(tools):
            if index:
                terminal.send(b']')
                settle(terminal)
            gallery.capture(terminal, '网络', tool+' · 工具说明和当前参数')
            terminal.send(b'o')
            terminal.wait(' · 参数')
            gallery.editor(terminal, '网络', tool+' · 参数编辑')
            terminal.send(b'\r')
            terminal.wait('参数仅本页生效')
            terminal.send(b'\r')
            terminal.wait('实际线路' if index == 0 else '未运行网络或系统探针')
            gallery.capture(terminal, '网络', tool+' · 演示结果')
        terminal.send(b'e')
        terminal.wait('演示模式：未导出文件')
        gallery.capture(terminal, '网络', '演示结果 · 禁止导出虚构报告')
        terminal.send(ESCAPE)

        visit(terminal, 5, '混合代理端口')
        settings = []
        for _ in range(100):
            row = selected(terminal)
            if row in settings:
                break
            settings.append(row)
            terminal.send(b'\r')
            terminal.wait('草稿不会自动保存')
            title = next(line.strip() for line in terminal.text().splitlines()
                         if line.strip().startswith('设置 · '))
            gallery.editor(terminal, '设置', title)
            terminal.send(b'\r')
            terminal.wait('保存'+title.removeprefix('设置 · ')+'为')
            gallery.capture(terminal, '设置', title+' · 保存确认')
            terminal.send(ESCAPE)
            settle(terminal)
            terminal.send(ESCAPE+DOWN)
            settle(terminal)
        else:
            raise RuntimeError('Desktop setting catalogue did not reach its end')

        visit(terminal, 6, '管理操作')
        for path, description in command_catalog(console):
            search(terminal, path)
            for _ in range(100):
                if re.search(r'(?:^|\s)'+re.escape(path)+r'$', selected(terminal)):
                    break
                previous = selected(terminal)
                terminal.send(DOWN)
                settle(terminal)
                if selected(terminal) == previous:
                    raise RuntimeError('TUI operation is missing: '+path)
            else:
                raise RuntimeError('Cannot locate the exact TUI operation: '+path)
            terminal.send(b'\r')
            terminal.wait('cor '+path+' ·')
            gallery.editor(terminal, '操作', path+' · '+description)
            review_operation(terminal, gallery, path)
    gallery.render()


def verify_desktop_catalog(call, command, report, first, second):
    """Capture real profile/shortcut editors in the existing disposable backend."""
    gallery = Gallery(report)
    with Session([*command, 'ui'], columns=120, rows=40) as terminal:
        terminal.wait('内核未运行')
        visit(terminal, 7, '订阅 · Enter')
        gallery.capture(terminal, '订阅', '订阅目录 · 当前项和来源类型', demo=False)
        terminal.send(b'\r')
        terminal.wait('设置 · 当前订阅')
        choices = set()
        for index in range(100):
            current = selected(terminal)
            if current in choices:
                break
            choices.add(current)
            gallery.capture(terminal, '订阅', f'订阅选择 · 候选 {index+1}', demo=False)
            terminal.send(b'\x1b[C')
            settle(terminal)
        else:
            raise RuntimeError('Subscription candidates did not reach the original choice')
        gallery.editor(terminal, '订阅', '订阅切换 · 保存入口', demo=False)
        terminal.send(b'\r')
        terminal.wait('保存当前订阅为')
        gallery.capture(terminal, '订阅', '订阅切换 · 确认与取消', demo=False)
        terminal.send(ESCAPE)
        settle(terminal)
        terminal.send(ESCAPE+b'8')
        terminal.wait('广告拦截：订阅已包含')
        gallery.capture(terminal, '快捷规则', '快捷规则 · 订阅包含与去重', demo=False)
        terminal.send(b'\r')
        terminal.wait('无需重复添加')
        gallery.capture(terminal, '快捷规则', '订阅已有规则 · 禁止重复添加', demo=False)
        call('profile', 'use', second)
        terminal.wait('备用线路')
        deadline = time.monotonic()+15
        while '订阅已包含' in terminal.text() or '尚未读取' in terminal.text():
            if time.monotonic() > deadline:
                raise RuntimeError('Shortcut gallery did not refresh after changing subscriptions')
            terminal.pump(.05)
        gallery.capture(terminal, '快捷规则', '快捷规则 · 可编辑的独立订阅状态', demo=False)
        for index in range(5):
            terminal.send(b'\r')
            terminal.wait('草稿不会自动保存')
            title = next(line.strip() for line in terminal.text().splitlines()
                         if line.strip().startswith('设置 · '))
            gallery.capture(terminal, '快捷规则', title+' · 当前值', demo=False)
            terminal.send(b'\x1b[C')
            settle(terminal)
            gallery.editor(terminal, '快捷规则', title+' · 候选值与保存', demo=False)
            terminal.send(b'\r')
            terminal.wait('保存'+title.removeprefix('设置 · ')+'为')
            gallery.capture(terminal, '快捷规则', title+' · 保存确认', demo=False)
            terminal.send(ESCAPE)
            settle(terminal)
            terminal.send(ESCAPE)
            settle(terminal)
            terminal.send(DOWN)
            settle(terminal)
        call('profile', 'use', first)
    gallery.render()
