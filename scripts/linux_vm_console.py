"""Run commands on the private serial console of a disposable live guest."""
import re
import socket
import time
import uuid


class Console:
    def __init__(self, path, log):
        self.connection = socket.socket(socket.AF_UNIX)
        try:
            self.connection.settimeout(1)
            self.connection.connect(str(path))
            self.log = log.open('wb')
        except BaseException:
            self.connection.close()
            raise
        self.total = 0

    def receive(self):
        try:
            chunk = self.connection.recv(65536)
        except TimeoutError:
            return b''
        if not chunk:
            raise RuntimeError('Guest serial console disconnected')
        self.total += len(chunk)
        if self.total > 16 * 1024 * 1024:
            raise RuntimeError('Guest serial log exceeds the diagnostic limit')
        self.log.write(chunk)
        self.log.flush()
        return chunk

    def command(self, command, timeout=180):
        marker = uuid.uuid4().hex
        self.connection.sendall((
            f"\nprintf '\\nBEGIN_{marker}\\n'; {command}; "
            f"cor_result=$?; printf '\\nEND_{marker}:%s\\n' \"$cor_result\"\n"
        ).encode())
        pattern = re.compile(
            rb'\nBEGIN_' + marker.encode() + rb'\n(.*?)\nEND_'
            + marker.encode() + rb':(\d+)\n', re.DOTALL,
        )
        data = b''
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            data += self.receive().replace(b'\r', b'')
            match = pattern.search(data)
            if match:
                output = match[1].decode(errors='replace')
                if int(match[2]):
                    raise RuntimeError('Guest command failed: ' + output[-3000:])
                return output
        raise TimeoutError('Guest serial command timed out')

    def wait_shell(self, process):
        deadline = time.monotonic() + 600
        data = b''
        next_probe = 0.0
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise RuntimeError('QEMU exited before the serial shell was ready')
            if time.monotonic() >= next_probe:
                self.connection.sendall(b"\x03\nprintf '\\nCOR_ROOT_READY:%s\\n' \"$(id -u)\"\n")
                next_probe = time.monotonic() + 10
            data = (data + self.receive()).replace(b'\r', b'')[-16384:]
            if b'\nCOR_ROOT_READY:0\n' in data:
                return
        raise TimeoutError('The live image did not expose its isolated debug console')

    def close(self):
        try:
            self.log.close()
        finally:
            self.connection.close()
