"""Terminal protocol and lifecycle checks without compiling the application."""
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import cli_tui
import cli_tui_preview
import ci_test_policy


class CliTuiTests(unittest.TestCase):
    def test_desktop_selection_includes_wrapped_command_continuations(self):
        terminal = cli_tui.Session.__new__(cli_tui.Session)
        terminal.screen = {
            0: '[操作]', 1: '› 显示完整说明并保留命令',
            2: '  · profile directory', 3: '  其他命令',
        }
        terminal.styles = {0: 'accent', 1: 'selected', 2: 'selected', 3: 'plain'}
        selected = cli_tui_preview.selected(terminal)
        self.assertEqual(selected, '› 显示完整说明并保留命令\n· profile directory')
        self.assertRegex(selected, r'(?:^|\s)profile directory$')

    def test_navigation_keys_follow_current_and_historical_visible_headers(self):
        terminal = cli_tui.Session.__new__(cli_tui.Session)
        layouts = (
            ('1 首页  2 节点  3 连接  4 网络\n[5 设置]  6 操作  7 订阅  8 快捷规则',
             {'连接': b'3', '网络': b'4', '设置': b'5', '订阅': b'7', '快捷规则': b'8'}),
            ('1 首页  2 节点  3 订阅  4 连接\n5 规则  [6 工具]  7 设置  8 操作',
             {'连接': b'4', '工具': b'6', '设置': b'7', '订阅': b'3', '规则': b'5'}),
        )
        for header, expected in layouts:
            terminal.screen = dict(enumerate(header.splitlines()))
            for title, key in expected.items():
                with self.subTest(header=header, title=title):
                    self.assertEqual(terminal.page_key(title), key)
            self.assertEqual(terminal.page_key('工具', '网络'),
                             expected.get('工具', expected.get('网络')))
            if '快捷规则' in expected:
                with self.assertRaisesRegex(RuntimeError, 'navigation'):
                    terminal.page_key('规则')
            with self.assertRaisesRegex(RuntimeError, 'navigation'):
                terminal.page_key('日')

    def test_desktop_capture_uses_checkout_identity_when_the_container_has_no_github_sha(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(cli_tui_preview, 'commit', return_value='a'*40), \
                patch.dict(os.environ, {}, clear=True):
            gallery = cli_tui_preview.Gallery(temporary)
            terminal = cli_tui.Session.__new__(cli_tui.Session)
            terminal.screen = {0: '首页', 1: 'q 退出'}
            terminal.styles = {}
            with patch.object(cli_tui_preview, 'settle'):
                gallery.capture(terminal, '首页', '首页', size=(100, 2))
            self.assertEqual(json.loads(gallery.path.read_text())['commit'], 'a'*40)
            with patch.object(cli_tui_preview, 'commit', return_value='b'*40):
                with self.assertRaisesRegex(RuntimeError, 'different source'):
                    cli_tui_preview.Gallery(temporary)

    def test_desktop_catalog_uses_actual_help_and_omits_retired_or_recursive_entries(self):
        help_text = ('命令：\n  ui                        打开终端界面\n'
                     '  run                       前台运行\n'
                     '  profile import            从文件导入\n'
                     '  network config-diff       已移除；请用 config get\n'
                     '  network http              读取响应\n'
                     '  --timeout N               通用选项\n')
        with patch.object(cli_tui_preview.subprocess, 'run') as run:
            run.return_value.stdout = help_text
            self.assertEqual(cli_tui_preview.command_catalog('cor'),
                             [('profile import', '从文件导入'), ('network http', '读取响应')])
            self.assertEqual(run.call_args.args[0], ['cor', 'help', 'all'])

    def test_desktop_preview_escapes_terminal_text_and_identifies_the_frame_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            gallery = cli_tui_preview.Gallery(temporary)
            terminal = cli_tui.Session.__new__(cli_tui.Session)
            terminal.screen = {0: '<script>bad()</script>', 1: 'q 退出'}
            terminal.styles = {0: 'warning', 1: 'muted'}
            with patch.object(cli_tui_preview, 'settle'):
                gallery.capture(terminal, '首页', '实际 <终端>', size=(100, 2))
            gallery.render()
            document = (gallery.directory/'index.html').read_text()
            self.assertNotIn('<script>bad()', document)
            self.assertIn('&lt;script&gt;bad()', document)
            self.assertIn('实际 &lt;终端&gt;', document)
            self.assertEqual(json.loads(gallery.path.read_text())['frames'][0]['rows'], 2)
            frame = gallery.directory/gallery.frames[0]['file']
            frame.write_text(json.dumps({'source': 'synthetic', 'lines': []}))
            with self.assertRaisesRegex(RuntimeError, 'synthetic'):
                gallery.render()

    def test_terminal_driver_changes_select_the_actual_cli_and_tui_checks(self):
        for path in ('scripts/cli_tui.py', 'scripts/cli_tui_preview.py', 'scripts/test_cli_tui.py'):
            with self.subTest(path=path):
                plan = ci_test_policy.select([path])
                self.assertTrue(plan['linux_user_tests'])
                self.assertFalse(plan['extended_tests'])

    def test_wait_matches_the_setting_row_instead_of_an_unrelated_value(self):
        if os.name != 'posix':
            self.skipTest('PTY requires Unix')
        code = (
            "import os,termios,tty\n"
            "original=termios.tcgetattr(0)\n"
            "tty.setraw(0)\n"
            "os.write(1,'\\x1b[2J\\x1b[1;1H17 connections\\x1b[K"
            "\\x1b[2;1H节点测速间隔 后台未连接\\x1b[K'.encode())\n"
            "os.read(0,1)\n"
            "os.write(1,'\\x1b[2J\\x1b[1;1H17 connections\\x1b[K"
            "\\x1b[2;1H节点测速间隔 17\\x1b[K'.encode())\n"
            "os.read(0,1)\n"
            "termios.tcsetattr(0,termios.TCSANOW,original)\n"
        )
        with cli_tui.Session([sys.executable, '-c', code]) as terminal:
            terminal.wait('后台未连接')
            terminal.send(b'r')
            terminal.wait(r'节点测速间隔[^\n]* 17(?:\n|$)', regex=True)
            self.assertNotIn('后台未连接', terminal.text())

    def test_resize_waits_for_a_new_bounded_frame_even_when_old_text_already_matches(self):
        if os.name != 'posix':
            self.skipTest('PTY termios checks require a Unix terminal')
        code = (
            "import os,termios,time,tty\n"
            "original=termios.tcgetattr(0)\n"
            "tty.setraw(0)\n"
            "os.write(1,b'\\x1b[2J\\x1b[20;1Hdomain.test\\x1b[K')\n"
            "os.read(0,1)\n"
            "time.sleep(.1)\n"
            "os.write(1,b'\\x1b[2J\\x1b[1;1Hdomain.test\\x1b[K\\x1b[2;1Hq exit\\x1b[K')\n"
            "os.read(0,1)\n"
            "termios.tcsetattr(0,termios.TCSANOW,original)\n"
        )
        with cli_tui.Session([sys.executable, '-c', code]) as terminal:
            terminal.wait('domain.test')
            self.assertEqual(max(terminal.screen), 19)
            terminal.resize(40, 2)
            terminal.send(b'r')
            terminal.wait('domain.test', rows=2)
            self.assertLess(max(terminal.screen), 2)

    def test_pty_frames_survive_split_utf8_styles_and_real_terminal_exit(self):
        if os.name != 'posix':
            self.skipTest('PTY termios checks require a Unix terminal')
        code = (
            "import os,termios,tty\n"
            "original=termios.tcgetattr(0)\n"
            "tty.setraw(0)\n"
            "os.write(1, b'\\x1b[2J\\x1b[1;1H\\x1b[0;38;5;79m'+"
            "'参考线路'.encode()[:4])\n"
            "os.write(1, '参考线路'.encode()[4:]+b'\\x1b[0m\\x1b[K')\n"
            "os.read(0,1)\n"
            "termios.tcsetattr(0,termios.TCSANOW,original)\n"
        )
        with cli_tui.Session([sys.executable, '-c', code]) as terminal:
            terminal.wait('参考线路')
            self.assertEqual(terminal.styles[0], 'accent')
            with tempfile.TemporaryDirectory() as directory:
                terminal.capture(directory, 'frame')
                frame = json.loads((Path(directory)/'frame.json').read_text())
                self.assertEqual(frame['source'], 'actual-cor-pty')
                self.assertEqual(frame['lines'][0]['text'], '参考线路')

    def test_terminal_constructor_closes_both_handles_on_spawn_failure(self):
        if os.name != 'posix':
            self.skipTest('PTY requires Unix')
        import pty
        import termios
        with patch.object(pty, 'openpty', return_value=(101, 102)), \
                patch.object(termios, 'tcgetattr', return_value=[]), \
                patch.object(cli_tui.Session, 'resize'), \
                patch.object(cli_tui.subprocess, 'Popen', side_effect=OSError('denied')), \
                patch.object(cli_tui.os, 'close') as close:
            with self.assertRaises(OSError):
                cli_tui.Session(['missing'])
            self.assertEqual([call.args[0] for call in close.call_args_list], [101, 102])

    def test_terminal_memory_rejects_missing_frontend_duplicate_pid_and_early_exit(self):
        spec = importlib.util.spec_from_file_location(
            'tui_memory_test', Path(__file__).with_name('run-memory-benchmark.py'))
        memory = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(memory)
        processes = [{'role': 'daemon', 'pid': 10},
                     {'role': 'core', 'pid': 11}, {'role': 'tui', 'pid': 12}]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'sample.jsonl'
            def write(rows, elapsed=10000):
                path.write_text(json.dumps({'elapsed_ms': elapsed, 'processes': rows})+'\n')
            write(processes)
            self.assertEqual(len(memory.read_scene(path, 10, 'daemon', frontend=12)), 1)
            for rows in (processes[:2], processes+[processes[-1]],
                         [*processes[:2], {'role': 'tui', 'pid': 99}]):
                write(rows)
                with self.assertRaises(ValueError):
                    memory.read_scene(path, 10, 'daemon', frontend=12)
            write(processes, elapsed=100)
            with self.assertRaises(ValueError):
                memory.read_scene(path, 10, 'daemon', frontend=12)
