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


class CliTuiTests(unittest.TestCase):
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
