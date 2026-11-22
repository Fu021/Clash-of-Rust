"""Drive the real terminal frontend through a PTY for previews and memory scenes."""
import contextlib
import json
import os
from pathlib import Path
import re
import subprocess
import time


class Session:
    def __init__(self, command, env=None, columns=100, rows=30):
        import pty
        import termios
        self.master, self.slave = pty.openpty()
        self.original = termios.tcgetattr(self.slave)
        self.pending = bytearray()
        self.screen = {}
        self.styles = {}
        self.resize(columns, rows)
        self.process = None
        try:
            self.process = subprocess.Popen(list(map(str, command)),
                                            stdin=self.slave, stdout=self.slave, stderr=self.slave,
                                            env=dict(os.environ if env is None else env, TERM='xterm-256color'),
                                            start_new_session=True)
        except BaseException:
            os.close(self.master)
            os.close(self.slave)
            raise

    def resize(self, columns, rows):
        import fcntl
        import struct
        import termios
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack('HHHH', rows, columns, 0, 0))

    def pump(self, timeout=.1):
        import select
        if not select.select([self.master], [], [], timeout)[0]:
            return
        try:
            chunk = os.read(self.master, 65536)
        except OSError:
            return
        self.pending.extend(chunk)
        clear = self.pending.rfind(b'\x1b[2J')
        if clear >= 0:
            self.screen.clear()
            self.styles.clear()
            del self.pending[:clear+4]
        pattern = rb'\x1b\[(\d+);1H((?:[^\x1b]|\x1b\[[0-9;]*m)*)\x1b\[K'
        consumed = 0
        for match in re.finditer(pattern, self.pending):
            index = int(match[1])-1
            raw = match[2].decode('utf-8', errors='strict')
            self.screen[index] = re.sub(r'\x1b\[[0-9;]*m', '', raw).rstrip()
            self.styles[index] = ('selected' if '\x1b[0;30;48;5;79m' in raw else
                                  'accent' if '\x1b[0;38;5;79m' in raw else
                                  'warning' if '\x1b[0;38;5;215m' in raw else
                                  'muted' if '\x1b[0;90m' in raw else 'plain')
            consumed = match.end()
        del self.pending[:consumed]
        if len(self.pending) > 1048576:
            raise RuntimeError('Terminal output did not form bounded frames')

    def text(self):
        return '\n'.join(self.screen.get(index, '') for index in range(max(self.screen, default=0)+1))

    def page_key(self, *titles):
        for title in titles:
            match = re.search(r'(?:^|[\s\[])([1-8]) '+re.escape(title)+r'(?=$|[\s\]])',
                              self.text(), re.MULTILINE)
            if match:
                return match[1].encode('ascii')
        raise RuntimeError('TUI navigation did not contain: '+', '.join(titles))

    def open_shortcuts(self):
        if '快捷规则 · 当前订阅' in self.text():
            return
        try:
            key = self.page_key('规则')
        except RuntimeError:
            self.send(self.page_key('快捷规则'))
        else:
            self.send(key)
            self.wait('快捷规则 →')
            self.send(b'\x1b\r')
        self.wait('快捷规则 · 当前订阅')

    def wait(self, text, timeout=15, rows=None, regex=False):
        deadline = time.monotonic()+timeout
        while True:
            self.pump(.05)
            matched = re.search(text, self.text()) if regex else text in self.text()
            if matched and (rows is None or max(self.screen, default=0) < rows):
                return
            if self.process.poll() is not None or time.monotonic() >= deadline:
                raise RuntimeError('TUI did not show '+repr(text)+': '+self.text()[-4000:])

    def send(self, data):
        os.write(self.master, data)

    def capture(self, directory, name):
        directory = Path(directory)
        directory.mkdir(parents=True, exist_ok=True)
        (directory/(name+'.txt')).write_text(self.text()+'\n', encoding='utf-8')
        lines = [{'text': self.screen.get(index, ''), 'tone': self.styles.get(index, 'plain')}
                 for index in range(max(self.screen, default=0)+1)]
        (directory/(name+'.json')).write_text(json.dumps({'source': 'actual-cor-pty',
                                                         'lines': lines}, ensure_ascii=False),
                                             encoding='utf-8')

    def close(self, key=b'q'):
        import termios
        try:
            if self.process.poll() is None:
                self.send(key)
                deadline = time.monotonic()+5
                while self.process.poll() is None and time.monotonic() < deadline:
                    self.pump(.05)
                if self.process.poll() is None:
                    self.process.terminate()
                    self.process.wait(timeout=5)
                    raise RuntimeError('TUI did not exit on request')
            if self.process.returncode != 0:
                raise RuntimeError('TUI failed: '+str(self.process.returncode))
            if termios.tcgetattr(self.slave) != self.original:
                raise RuntimeError('TUI did not restore terminal attributes')
        finally:
            if self.process.poll() is None:
                self.process.kill()
                self.process.wait(timeout=5)
            os.close(self.master)
            os.close(self.slave)

    def __enter__(self):
        return self

    def __exit__(self, kind, value, traceback):
        if kind is None:
            self.close()
        else:
            with contextlib.suppress(Exception):
                self.close(b'\x03')


def verify(console, resources, report):
    import tempfile
    report = Path(report)
    report.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='cor-tui-check-') as temporary:
        data = Path(temporary)/'absent'
        command = [str(console), '--lang=zh-CN', '--data-dir', str(data), '--resources', str(resources), 'ui']
        rejected = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True,
                                  text=True, timeout=10)
        if rejected.returncode == 0 or data.exists():
            raise RuntimeError('Nonterminal TUI started a backend or accepted redirected input')
        with Session(command) as terminal:
            terminal.wait('尚未连接后台')
            if data.exists():
                raise RuntimeError('Opening TUI created a backend')
        with Session([*command, '--demo']) as terminal:
            terminal.wait('参考线路')
            terminal.capture(report, 'home')
            terminal.send(b'2')
            terminal.wait('策略组')
            terminal.send(b'\x1b[B\r')
            terminal.wait('PROXY   ·')
            terminal.wait('延迟从低到高')
            for mode in ['延迟从高到低', '名称排序', '延迟从低到高']:
                terminal.send(b's')
                terminal.wait(mode)
            terminal.capture(report, 'nodes')
            for columns, rows in [(40, 12), (36, 16), (48, 24), (49, 30), (50, 30), (53, 30), (54, 30),
                                  (60, 30), (79, 30), (79, 23), (60, 18), (100, 30)]:
                terminal.resize(columns, rows)
                deadline = time.monotonic()+5
                actions = ['↑↓ 选择', 'Enter 打开', 's 排序', '/ 搜索', 'Esc 返回', 'q 退出']
                while True:
                    terminal.pump(.05)
                    text = terminal.text()
                    title = terminal.screen.get(0, '')
                    if columns < 50 or rows < 24:
                        navigation_ready = (title.strip() == '[节点]'
                                            and title.find('[') == (columns-1-6)//2)
                    else:
                        header = '\n'.join(terminal.screen.get(index, '') for index in range(5))
                        navigation_ready = (
                            all(label in header for label in ('CLASH OF RUST', '[2 节点]',
                                '1 首页', '3 订阅', '4 连接', '5 规则', '6 工具', '7 设置', '8 操作'))
                            and terminal.screen.get(4) == '  '+'─'*(columns-5)
                            and '…' not in header)
                    if max(terminal.screen, default=0) == rows-1 and all(
                            action in text for action in actions) and navigation_ready:
                        break
                    if terminal.process.poll() is not None or time.monotonic() >= deadline:
                        raise RuntimeError(f'Resized {columns}x{rows} hid navigation or node actions: {text}')
                if text.count('q 退出') != 1 or text.count('Esc 返回') != 1:
                    raise RuntimeError('Phone footer repeated exit or back actions')
                if any('…' in line for line in text.splitlines()[-3:]):
                    raise RuntimeError('Phone footer truncated actions instead of wrapping')
                terminal.capture(report, f'nodes-{columns}x{rows}')
            terminal.send('/日本'.encode())
            terminal.wait('🇯🇵 日本 01')
            terminal.send(b'\r\r')
            terminal.wait('切换 PROXY')
            terminal.capture(report, 'confirmation')
            terminal.send(b'\x1b')
            terminal.wait('退出界面保留后台')
            terminal.send(b'/q')
            terminal.wait('输入中')
            if terminal.process.poll() is not None:
                raise RuntimeError('Typing q into search exited TUI')
            terminal.send(b'\x1b')
            time.sleep(.15)
            terminal.send(b'4')
            terminal.wait('github.com')
            terminal.capture(report, 'connections')
            terminal.send(b'\r')
            terminal.wait('连接详情')
            terminal.wait('匹配规则')
            terminal.wait('域名：github.com')
            terminal.wait('嗅探域名：api.github.com')
            terminal.capture(report, 'connection-detail')
            terminal.send(b'\x1b7')
            terminal.wait('混合端口')
            terminal.wait('── 网络 ──')
            terminal.send(b'/fake_ip\r')
            terminal.wait('Fake-IP')
            if re.search(r'Fake-IP\s+(?:开启|关闭)', terminal.text()) is None:
                raise RuntimeError('TUI Fake-IP setting did not show its actual boolean state')
            terminal.send(b'/\x15fake_ip_exclusions\r')
            terminal.wait('项自定义')
            terminal.capture(report, 'settings')
            terminal.send(b'\r')
            terminal.wait('设置 · Fake-IP 排除项')
            terminal.send(b'\r')
            terminal.wait('输入中')
            terminal.send(b'q6')
            terminal.wait('新增：q6')
            if terminal.process.poll() is not None:
                raise RuntimeError('Typing q into a setting exited TUI')
            terminal.send(b'\x1b')
            terminal.wait('新增：q6')
            terminal.send(b'\x1b8')
            terminal.wait('管理操作')
            terminal.send(b'/service status\r\r')
            terminal.wait('cor service status')
            terminal.capture(report, 'operation')
            terminal.send(b'\r')
            terminal.wait('执行 cor service status')
            terminal.send(b'\r')
            terminal.wait('演示模式')
            terminal.send(b'\x1b6')
            terminal.wait('网站检查')
            terminal.send(b'\r')
            terminal.wait('实际线路')
            terminal.capture(report, 'network')
            terminal.send(b'\x1b[C')
            terminal.wait('DNS 查询')
            terminal.send(b'o')
            terminal.wait('DNS 查询 · 参数')
            terminal.send(b'\x1b[B\x1b[C')
            terminal.wait('记录类型：A')
            terminal.resize(40, 20)
            terminal.wait('[工具]', rows=20)
            terminal.wait('记录类型：A', rows=20)
            terminal.capture(report, 'network-parameters-40x20')
            terminal.resize(100, 30)
            terminal.wait('CLASH OF RUST')
            terminal.send(b'\x1b[B\r')
            terminal.wait('参数：记录 A')
            terminal.send(b'o')
            terminal.wait('记录类型：A')
            terminal.send(b'\x1b[B\x1b[C\x1b')
            terminal.wait('参数：记录 A')
            terminal.send(b'\x1b[D')
            terminal.wait('网站检查')
            terminal.send(b'o')
            terminal.wait('网站检查 · 参数')
            terminal.send(b'\x1b[B\r\r')
            terminal.wait('实际线路')
            for columns, rows in ((36, 16), (40, 20), (55, 16), (80, 24)):
                terminal.resize(columns, rows)
                terminal.wait('工具', rows=rows)
                for _ in range(64):
                    terminal.send(b'\x1b[B')
                    terminal.pump(.01)
                terminal.wait(r'\)(?:\s|$)', rows=rows, regex=True)
                terminal.capture(report, f'network-{columns}x{rows}')
            terminal.send(b'\x1b')
            terminal.send(b'4')
            terminal.wait('活跃连接')
            terminal.resize(55, 16)
            terminal.send(b'/\x15api.github.com')
            terminal.wait('输入中', rows=16)
            terminal.send(b'\r\r')
            terminal.wait('嗅探域名：api.github.com', rows=16)
            terminal.wait('[连接详情]', rows=16)
            terminal.capture(report, 'small')
            terminal.resize(40, 2)
            terminal.wait('api.github.com', rows=2)
            for _ in range(24):
                terminal.send(b'\x1b[B')
                terminal.pump(.05)
            terminal.wait('demo-1', rows=2)
            terminal.capture(report, 'tiny')
            terminal.resize(80, 24)
            terminal.wait('连接详情')
            terminal.capture(report, 'compact')
        if data.exists():
            raise RuntimeError('Demonstration mode wrote data or started a backend')
        from cli_tui_preview import verify_desktop
        verify_desktop(console, command, report)
        if data.exists():
            raise RuntimeError('Desktop demonstration gallery created backend data')
        terminal = Session([*command, '--demo'])
        terminal.wait('参考线路')
        terminal.close(b'\x03')
    verify_settings(console, resources, report)
    (report/'verification.json').write_text(json.dumps({
        'status': 'passed', 'source': 'real-pty',
        'checks': ['no-tty-rejected', 'read-only-no-backend', 'search-and-confirmation',
                   'literal-q-search', 'connection-details', 'settings-edit-and-cancel',
                   'operation-demo-no-side-effects', 'literal-q-setting',
                   'network-tools-and-cancel', 'network-inline-parameters',
                   'network-result-wrap-scroll-and-resize', 'network-export-and-recovery',
                   'node-sort-cycle', 'phone-footer-wraps-actions',
                   'navigation-fit-and-centered-compact-title',
                   'resize-and-tiny-detail-scroll',
                   'native-setting-save-failure-persistence-and-cross-client-refresh',
                   'fake-ip-list-add-disable-restore-and-stale-confirmation',
                   'profile-list-selection-and-included-shortcut-state',
                   'long-profile-name-and-source-survive-phone-resize',
                   'rule-list-sources-add-move-disable-delete-and-stale-confirmation',
                   'q-restores-terminal', 'ctrl-c-restores-terminal'],
    }), encoding='utf-8')


def verify_settings(console, resources, report):
    import tempfile
    with tempfile.TemporaryDirectory(prefix='cor-tui-settings-') as temporary:
        data = Path(temporary)
        command = [str(console), '--lang=zh-CN', '--data-dir', str(data), '--resources', str(resources)]

        def call(*arguments):
            result = subprocess.run([*command, '--json', *arguments], capture_output=True,
                                    text=True, check=True, timeout=45)
            return json.loads(result.stdout)['data']

        try:
            call('config', 'set', 'delay_interval_minutes', '17')
            call('config', 'set', 'update_check_interval_minutes', '0')
            call('config', 'set', 'tui_node_sort', 'latency_descending')
            owner = json.loads((data/'control.json').read_text())['pid']
            original_port = call('config', 'get', 'mixed_port')['mixed_port']
            verify_home(call, command, data, report)
            with Session([*command, 'ui']) as terminal:
                terminal.wait('内核未运行')
                terminal.send(b'7/delay_interval_minutes\r')
                terminal.wait(r'定时测速[^\n]* 17(?:\n|$)', regex=True)
                terminal.send(b'\r\r')
                terminal.wait('输入中')
                terminal.send(b'\x1523\r\x1b[B\r')
                terminal.wait('保存定时测速')
                terminal.send(b'\r')
                terminal.wait('已保存')
                if call('config', 'get', 'delay_interval_minutes')['delay_interval_minutes'] != 23:
                    raise RuntimeError('TUI saved a setting without persisting through the owner')
                if json.loads((data/'settings.json').read_text())['delay_interval_minutes'] != 23:
                    raise RuntimeError('TUI setting was not written to disk')
                terminal.capture(report, 'setting-saved')
                terminal.send(b'/\x15mixed_port\r\r\r')
                terminal.wait('输入中')
                terminal.wait('值：'+str(original_port))
                terminal.send(b'\x150\r\x1b[B\r')
                terminal.wait('保存混合端口')
                terminal.send(b'\r')
                terminal.wait('草稿保留')
                terminal.wait('代理端口必须非零')
                if call('config', 'get', 'mixed_port')['mixed_port'] != original_port:
                    raise RuntimeError('Rejected TUI setting changed the active value')
                terminal.capture(report, 'setting-rejected')
                terminal.send(b'\x1b2r')
                terminal.wait('延迟从高到低')
                deadline = time.monotonic()+15
                while '读取失败' in terminal.text():
                    if time.monotonic() >= deadline:
                        raise RuntimeError('TUI did not refresh after rejecting a setting')
                    terminal.pump(.05)
                terminal.send(b's')
                terminal.wait('名称排序')
            if call('config', 'get', 'tui_node_sort')['tui_node_sort'] != 'name':
                raise RuntimeError('TUI sort was not persisted')
            with Session([*command, 'ui']) as terminal:
                terminal.wait('内核未运行')
                terminal.send(b'2')
                terminal.wait('名称排序')
                call('config', 'set', 'tui_node_sort', 'latency_ascending')
                terminal.wait('延迟从低到高')
            verify_network_export(command, data, report)
            verify_fake_ip(call, command, data, report)
            verify_profile_catalog(call, command, data, report)
            verify_rules(call, command, data, report)
            if json.loads((data/'control.json').read_text())['pid'] != owner:
                raise RuntimeError('Opening or closing TUI replaced the existing owner')
            if call('status')['running']:
                raise RuntimeError('TUI settings or sorting started a core')
        except BaseException:
            with contextlib.suppress(Exception):
                call('stop')
            raise
        else:
            call('stop')


def verify_home(call, command, data, report):
    original = call('config', 'get', 'run_mode')['run_mode']
    with Session([*command, 'ui']) as terminal:
        terminal.wait('启动后台与内核')
        terminal.wait('自动检查已关闭')
        terminal.send(b'\r\r')
        terminal.wait('执行 cor start')
        terminal.capture(report, 'home-start-confirmation')
        terminal.send(b'\x1b')
        terminal.wait('cor start')
        terminal.send(b'\x1b')
        terminal.wait('首页 · Enter 操作')
        terminal.send(b'\x1b[B\x1b[B\r\x1b[C\r')
        terminal.wait('保存运行模式为')
        terminal.send(b'\r')
        terminal.wait('已保存')
        expected = 'global' if original == 'rule' else 'direct' if original == 'global' else 'rule'
        if call('config', 'get', 'run_mode')['run_mode'] != expected:
            raise RuntimeError('Home mode selection did not use the existing owner')
        if json.loads((data/'settings.json').read_text())['run_mode'] != expected:
            raise RuntimeError('Home mode selection did not persist')
        terminal.capture(report, 'home-mode-saved')
        call('mode', original)
        terminal.wait('运行模式 · '+{'rule': '规则', 'global': '全局', 'direct': '直连'}[original])
        terminal.send(b'\x1b[B\x1b[B\x1b[B')
        terminal.wait('› 检查更新')
        for columns, rows in ((40, 24), (60, 24), (100, 30)):
            terminal.resize(columns, rows)
            terminal.wait('› 检查更新', rows=rows)
            if '首页' not in terminal.text() or 'Enter 操作' not in terminal.text():
                raise RuntimeError('Resized home hid its title or operation help')
            terminal.capture(report, f'home-update-disabled-{columns}x{rows}')
    if call('status')['running']:
        raise RuntimeError('Home preview started a core before explicit confirmation')


def verify_fake_ip(call, command, data, report):
    from cli_tui_preview import Gallery
    gallery = Gallery(report)
    domain = 'tui.example.test'
    with Session([*command, 'ui']) as terminal:
        terminal.wait('内核未运行')
        terminal.send(b'7/fake_ip_exclusions\r')
        terminal.wait('项自定义')
        terminal.send(b'\r')
        terminal.wait('设置 · Fake-IP 排除项')
        terminal.wait('localhost')
        terminal.capture(report, 'fake-ip-existing')
        gallery.capture(terminal, '设置', 'Fake-IP · 已有项及来源', demo=False, size=(100, 30))
        terminal.send(b'\r'+domain.encode()+b'\r')
        terminal.wait('新增排除项：'+domain)
        terminal.send(b'\r')
        terminal.wait('已保存 Fake-IP 排除项')
        if call('config', 'get', 'fake_ip_exclusions')['fake_ip_exclusions'] != [domain]:
            raise RuntimeError('TUI Fake-IP addition was not persisted')
        terminal.send(b'\x1b[B'*3+b'\r')
        terminal.wait('停用默认排除项：localhost')
        terminal.send(b'\r')
        terminal.wait('已保存 Fake-IP 排除项')
        terminal.wait('已停用')
        if 'localhost' not in call('config', 'get', 'fake_ip_disabled_defaults')['fake_ip_disabled_defaults']:
            raise RuntimeError('TUI Fake-IP default was not disabled')
        terminal.capture(report, 'fake-ip-disabled')
        gallery.capture(terminal, '设置', 'Fake-IP · 停用默认项后仍可查看', demo=False, size=(100, 30))
        terminal.send(b'\x1b[A'*100+b'\x1b[B\r')
        terminal.wait('恢复全部默认排除项')
        terminal.send(b'\r')
        terminal.wait('已保存 Fake-IP 排除项')
        saved = json.loads((data/'settings.json').read_text())
        if saved['fake_ip_disabled_defaults'] or saved['fake_ip_exclusions'] != [domain]:
            raise RuntimeError('TUI Fake-IP restore lost custom values or left defaults disabled')
        gallery.capture(terminal, '设置', 'Fake-IP · 恢复默认并保留自定义项', demo=False, size=(100, 30))
        terminal.send(b'\x1b[A\r\x15invalid/domain\r')
        terminal.wait('排除项只能包含')
        terminal.wait('新增：invalid/domain')
        terminal.capture(report, 'fake-ip-invalid-input-kept')
        terminal.send(b'\r\x15stale.example.test\r')
        terminal.wait('新增排除项：stale.example.test')
        original = call('config', 'get', 'ipv6')['ipv6']
        call('config', 'set', 'ipv6', 'false' if original else 'true')
        terminal.send(b'\r')
        terminal.wait(r'设置已变化|设置已被另一客户端修改', regex=True)
        if call('config', 'get', 'fake_ip_exclusions')['fake_ip_exclusions'] != [domain]:
            raise RuntimeError('Stale Fake-IP confirmation changed settings')
        terminal.wait('stale.example.test')
        terminal.capture(report, 'fake-ip-stale-confirmation')

def verify_rules(call, command, data, report):
    from cli_tui_preview import Gallery
    gallery = Gallery(report)
    source = data/'rules.yaml'
    original = 'proxies: []\nrules:\n  - DOMAIN,original.test,DIRECT\n  - MATCH,DIRECT\n'
    source.write_text(original, encoding='utf-8')
    call('profile', 'add', '规则管理', str(source))
    profile = next(item['id'] for item in call('profile', 'list') if item['name'] == '规则管理')
    call('profile', 'use', profile)
    for shortcut in ('ads', 'private-domain', 'private-ip', 'cn-domain', 'cn-ip'):
        call('rule', 'shortcut', shortcut, 'off', '--profile', profile)
    raw = 'DOMAIN,tui-rule.test,DIRECT'

    def rows():
        return call('rule', 'list', '--profile', profile)['items']

    def operate(terminal, label):
        terminal.send(b'\r')
        terminal.wait('稳定 ID：')
        for _ in range(8):
            terminal.pump(.1)
            if re.search(r'^\s*› '+re.escape(label)+r'\s*$', terminal.text(), re.MULTILINE):
                terminal.send(b'\r')
                return
            terminal.send(b'\x1b[B')
        raise RuntimeError('Rule operation was not reachable: '+label)

    with Session([*command, 'ui'], columns=120, rows=40) as terminal:
        terminal.wait('内核未运行')
        terminal.send(b'5')
        terminal.wait('快捷规则 →')
        terminal.wait('original.test')
        gallery.capture(terminal, '规则', '规则列表 · 实际来源与原文', demo=False)
        for columns in (40, 60):
            terminal.resize(columns, 24)
            terminal.wait('original.test', rows=24)
            gallery.capture(terminal, '规则', '规则列表 · 手机 SSH', demo=False, size=(columns, 24))
        terminal.resize(120, 40)
        terminal.wait('original.test', rows=40)
        terminal.send(b'\x1b[B\r')
        terminal.wait('新增规则 · 单行文本')
        gallery.capture(terminal, '规则', '新增自定义规则 · 输入与策略提示', demo=False)
        terminal.send(raw.encode()+b'\r')
        terminal.wait('保存规则：'+raw)
        gallery.capture(terminal, '规则', '新增自定义规则 · 原子保存确认', demo=False)
        terminal.send(b'\r')
        terminal.wait('已保存规则')
        added = next(row for row in rows() if row['rule'] == raw)
        if added['source'] != 'Custom' or added['position'] != 1:
            raise RuntimeError('TUI rule addition lost its source or position')
        terminal.send(b'\r')
        terminal.wait('稳定 ID：')
        gallery.capture(terminal, '规则', '自定义规则 · 全部适用操作', demo=False)
        terminal.resize(40, 24)
        terminal.wait('稳定 ID：', rows=24)
        gallery.capture(terminal, '规则', '自定义规则操作 · 手机 SSH', demo=False, size=(40, 24))
        terminal.resize(120, 40)
        terminal.wait('稳定 ID：', rows=40)
        terminal.send(b'\x1b')
        terminal.wait('快捷规则 →')
        operate(terminal, '下移')
        terminal.wait('下移规则')
        terminal.send(b'\r')
        terminal.wait('已保存规则')
        moved = next(row for row in rows() if row['id'] == added['id'])
        if moved['position'] != 2:
            raise RuntimeError('TUI rule move did not preserve the stable identity')
        operate(terminal, '停用')
        terminal.wait('停用规则')
        terminal.send(b'\r')
        terminal.wait('已保存规则')
        if next(row for row in rows() if row['id'] == added['id'])['enabled']:
            raise RuntimeError('TUI did not persist the disabled custom rule')
        operate(terminal, '编辑')
        terminal.wait('编辑规则 · 单行文本')
        terminal.send(b'\x15INVALID\r')
        terminal.wait('Enter 编辑')
        terminal.wait('INVALID')
        gallery.capture(terminal, '规则', '规则校验失败 · 保留输入', demo=False)
        draft = 'DOMAIN,tui-edited.test,DIRECT'
        terminal.send(b'\r\x15'+draft.encode()+b'\r')
        terminal.wait('保存规则：'+draft)
        call('rule', 'add', 'DOMAIN,concurrent.test,DIRECT', '--profile', profile)
        terminal.send(b'\r')
        terminal.wait(r'规则已(?:被另一客户端修改|变化)', regex=True)
        terminal.wait(draft)
        gallery.capture(terminal, '规则', '并发修改拒绝 · 保留输入和外部规则', demo=False)
        current = rows()
        if next(row for row in current if row['id'] == added['id'])['rule'] != raw:
            raise RuntimeError('Old TUI confirmation overwrote a changed rule revision')
        if not any(row['rule'] == 'DOMAIN,concurrent.test,DIRECT' for row in current):
            raise RuntimeError('Old TUI confirmation discarded a concurrent addition')
        terminal.send(b'\x1b')
        terminal.wait('稳定 ID：')
        terminal.send(b'\x1b')
        terminal.wait('快捷规则 →')
        terminal.wait('concurrent.test')
        terminal.send(b'/tui-rule.test\r\x1b[B\x1b[B')
        operate(terminal, '删除')
        terminal.wait('删除规则')
        terminal.send(b'\r')
        terminal.wait('已保存规则')
        if any(row['id'] == added['id'] for row in rows()):
            raise RuntimeError('TUI rule deletion was not persisted')
        if source.read_text(encoding='utf-8') != original:
            raise RuntimeError('TUI rule management changed the original subscription')
        gallery.capture(terminal, '规则', '删除自定义规则 · 保存后读回', demo=False)
    gallery.render()


def verify_network_export(command, data, report):
    from cli_tui_preview import Gallery
    gallery = Gallery(report)
    path = data/'network.json'
    with Session([*command, 'ui'], columns=120, rows=40) as terminal:
        terminal.wait('内核未运行')
        terminal.send(b'6\x1b[D')
        terminal.wait('HTTP 检查')
        terminal.send(b'/http://127.0.0.1:9/private?token=hidden\r\r')
        terminal.wait('检查状态 · 失败')
        gallery.capture(terminal, '工具', 'HTTP · 实际失败报告和总耗时', demo=False)
        terminal.send(b'e')
        terminal.wait('工具 · 导出本次结果')
        gallery.capture(terminal, '工具', '导出本次报告 · 路径和导出入口', demo=False)
        terminal.send(b'\r')
        terminal.wait('输入中')
        gallery.capture(terminal, '工具', '导出本次报告 · 编辑文件路径', demo=False)
        terminal.send(b'\x15'+str(data).encode()+b'\r\x1b[B\r')
        terminal.wait('导出失败，路径和报告保留')
        terminal.capture(report, 'network-export-rejected')
        gallery.capture(terminal, '工具', '导出失败 · 保留路径和原报告', demo=False)
        terminal.send(b'\x1b[A\r\x15'+str(path).encode()+b'\r\x1b[B\r')
        terminal.wait('已导出：')
        terminal.capture(report, 'network-exported')
        gallery.capture(terminal, '工具', '修正路径 · 导出成功', demo=False)
        saved = path.read_text()
        payload = json.loads(saved)
        result = payload['data']
        if (payload['command'] != 'network http' or result['status'] != 'failed'
                or not result['entries'] or 'elapsed_ms' not in result):
            raise RuntimeError('TUI export lost the original failed measurement')
        if '/private' in saved or 'token=' in saved or 'hidden' in saved:
            raise RuntimeError('TUI export included private URL components')
    gallery.render()


def verify_profile_catalog(call, command, data, report):
    from cli_tui_preview import selected, verify_desktop_catalog
    included = data/'included.yaml'
    plain = data/'plain.yaml'
    included_raw = 'proxies: []\nrules:\n  - GEOSITE,category-ads-all,REJECT\n  - MATCH,DIRECT\n'
    plain_raw = 'proxies: []\nrules:\n  - MATCH,DIRECT\n'
    included.write_text(included_raw, encoding='utf-8')
    plain.write_text(plain_raw, encoding='utf-8')
    call('profile', 'add', '含规则线路', str(included))
    call('profile', 'add', '备用线路', str(plain))
    profiles = call('profile', 'list')
    first = next(profile['id'] for profile in profiles if profile['name'] == '含规则线路')
    second = next(profile['id'] for profile in profiles if profile['name'] == '备用线路')
    call('profile', 'use', first)
    verify_desktop_catalog(call, command, report, first, second)
    with Session([*command, 'ui']) as terminal:
        terminal.wait('内核未运行')
        terminal.open_shortcuts()
        terminal.wait('广告拦截：订阅已包含')
        terminal.send(b'\r')
        terminal.wait('无需重复添加')
        terminal.capture(report, 'shortcut-included')
        terminal.send(b'3')
        terminal.wait('[当前] 含规则线路')
        terminal.resize(40, 10)
        terminal.wait('[当前] 含规则线路', rows=10)
        terminal.capture(report, 'profile-small')
        terminal.resize(100, 30)
        terminal.wait('订阅 · Enter')
        terminal.send('/备用线路\r\r'.encode())
        terminal.wait('设置 · 当前订阅')
        terminal.wait('值：备用线路')
        terminal.send(b'\x1b[B\r')
        terminal.wait('保存当前订阅')
        terminal.send(b'\r')
        terminal.wait('已保存 · 当前订阅：备用线路')
        if call('config', 'get', 'active_profile')['active_profile'] != second:
            raise RuntimeError('TUI profile selection did not persist through the owner')
        terminal.capture(report, 'profile-selected')
        call('rule', 'shortcut', 'ads', 'off', '--profile', second)
        terminal.open_shortcuts()
        terminal.wait('广告拦截：关闭')
        if '广告拦截：订阅已包含' in terminal.text():
            raise RuntimeError('TUI reused shortcut metadata from the previous profile')
        terminal.send(b'\r')
        terminal.wait('设置 · 广告拦截')
        call('rule', 'shortcut', 'private-domain', 'off', '--profile', second)
        terminal.send(b'\x1b[C\x1b[B\r')
        terminal.wait('保存广告拦截')
        terminal.send(b'\r')
        terminal.wait('已保存 · 广告拦截')
        terminal.capture(report, 'shortcut-saved')
        saved = json.loads((data/'settings.json').read_text())
        if not saved['rule_profiles'][second]['enabled'][0]:
            raise RuntimeError('TUI shortcut choice was not persisted')
        if saved['rule_profiles'][second]['enabled'][1]:
            raise RuntimeError('TUI shortcut save overwrote a concurrent CLI rule change')
        if plain.read_text(encoding='utf-8') != plain_raw:
            raise RuntimeError('TUI shortcut changes rewrote an original subscription')
        terminal.send(b'\r')
        terminal.wait('设置 · 广告拦截')
        terminal.send(b'\x1b[C')
        terminal.wait('值：关闭')
        plain.write_text(included_raw, encoding='utf-8')
        call('profile', 'update', second)
        terminal.send(b'\x1b[B')
        terminal.wait(r'(?ms)^[ \t]*› 保存设置 →.*退出界面保留后台和代理',
                      timeout=30, regex=True)
        terminal.send(b'\r')
        terminal.wait('保存广告拦截')
        terminal.send(b'\r')
        terminal.wait('草稿保留')
        terminal.wait('订阅已包含此规则')
        after = json.loads((data/'settings.json').read_text())
        if not after['rule_profiles'][second]['enabled'][0]:
            raise RuntimeError('Stale TUI shortcut draft changed a now-included rule')
        terminal.capture(report, 'shortcut-stale-source-rejected')
        terminal.send(b'\x1b')
        call('profile', 'use', first)
        terminal.wait('广告拦截：订阅已包含')
        terminal.capture(report, 'shortcut-external-profile-refresh')
    if included.read_text(encoding='utf-8') != included_raw or plain.read_text(encoding='utf-8') != included_raw:
        raise RuntimeError('TUI shortcut changes rewrote an original subscription')
    long_name = '跨地域远程线路' * 5 + '验收终点'
    call('profile', 'edit', second, '--name', long_name)
    with Session([*command, 'ui'], columns=120, rows=40) as terminal:
        terminal.wait('内核未运行')
        terminal.send('3/验收终点\r'.encode())
        terminal.wait(long_name)
        for columns, heading in ((40, '[订阅]'), (60, '[3 订阅]')):
            terminal.resize(columns, 24)
            terminal.wait(heading, rows=24)
            terminal.wait(r'验\s*收\s*终\s*点\s*·\s*本\s*地', rows=24, regex=True)
            if long_name+'·本地' not in ''.join(selected(terminal).split()):
                raise RuntimeError(f'{columns}-column profile selection lost its name or source')
            terminal.capture(report, f'profile-long-{columns}x24')
