"""Check ordinary-user process initialization before testing the TUN service."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from windows_ci import allow_ordinary_user_directory, run_unelevated


def main():
    if os.name != 'nt':
        raise RuntimeError('Windows token checks require Windows')
    with tempfile.TemporaryDirectory(prefix='windows-token-') as temporary:
        # Python's private temporary directories can be owned by the elevated
        # Administrators SID, which the ordinary child cannot use.
        allow_ordinary_user_directory(temporary)
        report = Path(temporary)/'initialized.txt'
        child = (
            'import ctypes, os; from pathlib import Path; '
            'assert not ctypes.windll.shell32.IsUserAnAdmin(); '
            'assert ctypes.windll.user32.GetProcessWindowStation(); '
            f'Path({str(report)!r}).write_text(os.environ["CLASH_TOKEN_TEST"])'
        )
        env = dict(os.environ, CLASH_TOKEN_TEST='ordinary user32 initialized')
        run_unelevated([sys.executable, '-c', child], env)
        assert report.read_text() == env['CLASH_TOKEN_TEST']
        run_unelevated([sys.executable, '-c', 'raise SystemExit(7)'], env, expected_code=7)
        try:
            run_unelevated([sys.executable, '-c', 'import time; time.sleep(30)'], env, timeout=0.2)
        except (TimeoutError, subprocess.TimeoutExpired):
            pass
        else:
            raise AssertionError('The ordinary-user process did not time out')
        called = []

        def terminate(kernel, process):
            if not kernel.TerminateProcess(process, 0xCAFE):
                raise RuntimeError('The ordinary-user process handle is invalid')
            called.append(True)

        run_unelevated([sys.executable, '-c', 'import time; time.sleep(30)'], env,
                       on_start=terminate, expected_code=0xCAFE)
        assert called
    print('PASS: ordinary user32 initialization, environment, exits, timeout and crash callback')


if __name__ == '__main__':
    main()
