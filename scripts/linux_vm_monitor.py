"""QMP client for private sockets created by the compatibility runner."""
import json
import os
import select
import socket
import subprocess


class MonitorError(RuntimeError):
    def __init__(self, response):
        self.response = response
        super().__init__(str(response))


class Monitor:
    def __init__(self, path, *, handshake_timeout=60):
        self.connection = socket.socket(socket.AF_UNIX)
        self.stream = None
        try:
            self.connection.settimeout(handshake_timeout)
            self.connection.connect(str(path))
            self.stream = self.connection.makefile('rb')
            if 'QMP' not in self.read():
                raise RuntimeError('QEMU monitor did not send its protocol greeting')
            self.execute('qmp_capabilities')
            self.connection.settimeout(10)
        except BaseException:
            self.close()
            raise

    def read(self):
        while True:
            line = self.stream.readline()
            if not line:
                raise RuntimeError('QEMU monitor disconnected')
            value = json.loads(line)
            if 'event' not in value:
                return value

    def execute(self, command, arguments=None):
        self.connection.sendall(json.dumps({
            'execute': command, 'arguments': arguments or {},
        }).encode() + b'\n')
        result = self.read()
        if 'error' in result:
            raise MonitorError(result['error'])
        if 'return' not in result:
            raise RuntimeError('Unexpected QEMU monitor response')
        return result['return']

    def close(self):
        try:
            if self.stream is not None:
                self.stream.close()
        finally:
            self.connection.close()


def capture_screen(monitor, destination, environment=None):
    try:
        monitor.execute('screendump', {'filename': str(destination), 'format': 'png'})
    except MonitorError as error:
        if error.response.get('desc') != 'no surface' or environment is None:
            raise
        code = ('from PIL import ImageGrab; import sys; '
                'ImageGrab.grab(xdisplay=sys.argv[2]).save(sys.argv[1])')
        subprocess.run(['/usr/bin/python3', '-c', code, destination, environment['DISPLAY']],
                       check=True, timeout=10)
        return 'x11'
    return 'qmp'


class SoftwareDisplay:
    def __init__(self, log):
        with log.open('w') as output:
            self.process = subprocess.Popen(
                ['Xvfb', '-displayfd', '1', '-screen', '0', '1920x1200x24',
                 '+extension', 'GLX', '-nolisten', 'tcp'],
                stdout=subprocess.PIPE, stderr=output,
            )
        try:
            if not select.select([self.process.stdout], [], [], 20)[0]:
                raise TimeoutError('The VM software display did not initialize')
            number = self.process.stdout.readline(16).strip()
            if not number.isdigit():
                raise RuntimeError('Xvfb did not report its display')
        except BaseException:
            self.close()
            raise
        self.environment = {**os.environ, 'DISPLAY': ':'+number.decode(),
                            'LIBGL_ALWAYS_SOFTWARE': '1', 'SDL_VIDEODRIVER': 'x11',
                            'SDL_VIDEO_WINDOW_POS': '0,0'}

    def close(self):
        self.process.stdout.close()
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
