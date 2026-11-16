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
        command = [str(console), '--data-dir', str(data), '--resources', str(resources), 'ui']
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
            terminal.send(b'3')
            terminal.wait('github.com')
            terminal.capture(report, 'connections')
            terminal.send(b'\r')
            terminal.wait('连接详情')
            terminal.wait('匹配规则')
            terminal.wait('域名：github.com')
            terminal.wait('嗅探域名：api.github.com')
            terminal.capture(report, 'connection-detail')
            terminal.send(b'\x1b5')
            terminal.wait('混合代理端口')
            terminal.capture(report, 'settings')
            terminal.send(b'/fake_ip_exclusions\r\r')
            terminal.wait('设置 · Fake-IP 排除项')
            terminal.send(b'\r')
            terminal.wait('输入中')
            terminal.send(b'q6')
            terminal.wait('[]q6')
            if terminal.process.poll() is not None:
                raise RuntimeError('Typing q into a setting exited TUI')
            terminal.send(b'\x1b')
            terminal.wait('值：[]')
            terminal.send(b'\x1b6')
            terminal.wait('管理操作')
            terminal.send(b'/service status\r\r')
            terminal.wait('cor service status')
            terminal.capture(report, 'operation')
            terminal.send(b'\r')
            terminal.wait('执行 cor service status')
            terminal.send(b'\r')
            terminal.wait('演示模式')
            terminal.send(b'\x1b4')
            terminal.wait('网站检查')
            terminal.send(b'\r')
            terminal.wait('实际线路')
            terminal.capture(report, 'network')
            terminal.send(b']')
            terminal.wait('DNS 查询')
            terminal.send(b'[')
            terminal.wait('网站检查')
            terminal.send(b'\x1b')
            terminal.send(b'3')
            terminal.wait('活跃连接')
            terminal.resize(55, 16)
            terminal.send(b'/\x15api.github.com')
            terminal.wait('输入中', rows=16)
            terminal.send(b'\r\r')
            terminal.wait('嗅探域名：api.github.com', rows=16)
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
        terminal = Session([*command, '--demo'])
        terminal.wait('参考线路')
        terminal.close(b'\x03')
    verify_settings(console, resources, report)
    (report/'verification.json').write_text(json.dumps({
        'status': 'passed', 'source': 'real-pty',
        'checks': ['no-tty-rejected', 'read-only-no-backend', 'search-and-confirmation',
                   'literal-q-search', 'connection-details', 'settings-edit-and-cancel',
                   'operation-demo-no-side-effects', 'literal-q-setting',
                   'network-tools-and-cancel', 'node-sort-cycle', 'resize-and-tiny-detail-scroll',
                   'native-setting-save-failure-persistence-and-cross-client-refresh',
                   'q-restores-terminal', 'ctrl-c-restores-terminal'],
    }), encoding='utf-8')


def verify_settings(console, resources, report):
    import tempfile
    with tempfile.TemporaryDirectory(prefix='cor-tui-settings-') as temporary:
        data = Path(temporary)
        command = [str(console), '--data-dir', str(data), '--resources', str(resources)]

        def call(*arguments):
            result = subprocess.run([*command, '--json', *arguments], capture_output=True,
                                    text=True, check=True, timeout=45)
            return json.loads(result.stdout)['data']

        try:
            call('config', 'set', 'delay_interval_minutes', '17')
            call('config', 'set', 'tui_node_sort', 'latency_descending')
            owner = json.loads((data/'control.json').read_text())['pid']
            original_port = call('config', 'get', 'mixed_port')['mixed_port']
            with Session([*command, 'ui']) as terminal:
                terminal.wait('内核未运行')
                terminal.send(b'5/delay_interval_minutes\r')
                terminal.wait(r'节点测速间隔[^\n]* 17(?:\n|$)', regex=True)
                terminal.send(b'\r\r')
                terminal.wait('输入中')
                terminal.send(b'\x1523\r\x1b[B\r')
                terminal.wait('保存节点测速间隔')
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
                terminal.wait('保存混合代理端口')
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
