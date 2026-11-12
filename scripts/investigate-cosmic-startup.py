"""Temporary official Pop!_OS live VM; no private cache or guest credentials."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shlex
import shutil
import socket
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
REPORT = ROOT / 'target/cosmic-startup-report'
WORK = ROOT / 'target/cosmic-startup-vm'
IMAGE_URL = 'https://iso.pop-os.org/24.04/amd64/intel/20/pop-os_24.04_amd64_intel_20.iso'
IMAGE_SIZE = 2955067392
IMAGE_SHA256 = 'a0ef3842ab710db4f4407cf3499560b59dddbbcd59bee17beab7b0e99dc22b4c'


class MonitorError(RuntimeError):
    def __init__(self, response):
        self.response = response
        super().__init__(str(response))


class Monitor:
    def __init__(self, path, display):
        self.display = display
        self.captures = {}
        self.connection = socket.socket(socket.AF_UNIX)
        self.connection.settimeout(10)
        self.connection.connect(str(path))
        self.stream = self.connection.makefile('rb')
        self.read()
        self.execute('qmp_capabilities')

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
        return result.get('return')

    def screenshot(self, name):
        from PIL import Image, ImageGrab
        path = REPORT / (name + '.ppm')
        try:
            self.execute('screendump', {'filename': str(path)})
        except MonitorError as error:
            if error.response.get('desc') != 'no surface':
                raise
            image = ImageGrab.grab(xdisplay=self.display)
            method = 'host-x11-display'
        else:
            image = Image.open(path)
            method = 'qmp-screendump'
        with image:
            image.save(path.with_suffix('.png'))
            self.captures[name] = {'method': method, 'size': list(image.size)}
        path.unlink(missing_ok=True)
        (REPORT / 'captures.json').write_text(json.dumps(self.captures, indent=2))

    def close(self):
        self.stream.close()
        self.connection.close()


class Console:
    def __init__(self, path):
        self.connection = socket.socket(socket.AF_UNIX)
        self.connection.settimeout(1)
        self.connection.connect(str(path))
        self.log = (REPORT / 'serial.log').open('wb')
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
            raise RuntimeError('Guest serial log exceeds investigation budget')
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
        next_probe = 0.
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise RuntimeError('QEMU exited before the login probe')
            if time.monotonic() >= next_probe:
                self.connection.sendall(b"\nprintf '\\nCOR_ROOT_READY:%s\\n' \"$(id -u)\"\n")
                next_probe = time.monotonic() + 10
            data = (data + self.receive()).replace(b'\r', b'')[-16384:]
            if b'\nCOR_ROOT_READY:0\n' in data:
                return
        raise TimeoutError('The live image did not expose its isolated debug console')

    def close(self):
        self.log.close()
        self.connection.close()


def run(*arguments, **options):
    return subprocess.run(arguments, check=True, **options)


def fetch_candidate():
    from compatibility import validate_run
    from workflow_artifacts import commit, fetch_package_inputs, release_module
    module = release_module()
    repository = module.repository()
    client = module.GitHub(repository, module.credential(repository))
    run_id = int(os.environ['SOURCE_RUN'])
    expected_commit = commit()
    if validate_run(client(f'/actions/runs/{run_id}'), repository, expected_commit) != run_id:
        raise ValueError('The source CI run identity differs from the requested run')
    fetch_package_inputs(client, run_id, 'x64', ROOT, expected_commit, run_id)


def prepare():
    WORK.mkdir(parents=True, exist_ok=True)
    REPORT.mkdir(parents=True, exist_ok=True)
    receipt = json.loads((ROOT / 'dist/package-receipt.json').read_text())
    package = ROOT / 'dist' / receipt['package']
    if (receipt['system'], receipt['arch'], receipt['tests_passed']) != ('linux', 'x64', True):
        raise ValueError('Expected the validated Linux x64 candidate')
    with package.open('rb') as source:
        if hashlib.file_digest(source, 'sha256').hexdigest() != receipt['sha256']:
            raise ValueError('CI package checksum changed')
    (REPORT / 'candidate.json').write_text(json.dumps(receipt, indent=2))
    image = WORK / 'pop.iso'
    run('curl', '--fail', '--location', '--retry', '2', '--connect-timeout', '30',
        '--max-time', '900', '--proto', '=https', '--proto-redir', '=https',
        '--output', str(image), IMAGE_URL)
    with image.open('rb') as source:
        if image.stat().st_size != IMAGE_SIZE or hashlib.file_digest(source, 'sha256').hexdigest() != IMAGE_SHA256:
            raise ValueError('Official Pop image identity mismatch')
    (REPORT / 'image.json').write_text(json.dumps({
        'url': IMAGE_URL, 'size': IMAGE_SIZE, 'sha256': IMAGE_SHA256,
        'metadata_source': 'https://api.pop-os.org/builds/24.04/intel',
    }, indent=2))
    members = subprocess.check_output(['bsdtar', '-tf', str(image)], text=True).splitlines()
    kernels = [name for name in members if PurePosixPath(name).name in ('vmlinuz', 'vmlinuz.efi')
               and PurePosixPath(name).parent.name.startswith('casper')]
    if len(kernels) != 1:
        raise ValueError('Expected one live-system kernel')
    media = PurePosixPath(kernels[0]).parent
    initrds = [name for name in members if PurePosixPath(name).parent == media
               and PurePosixPath(name).name in ('initrd', 'initrd.gz', 'initrd.lz')]
    if len(initrds) != 1:
        raise ValueError('Expected one live-system initramfs')
    for source, name in [(kernels[0], 'vmlinuz'), (initrds[0], 'initrd')]:
        with (WORK / name).open('wb') as output:
            run('bsdtar', '-xOf', str(image), source, stdout=output)
    share = WORK / 'share'
    share.mkdir(exist_ok=True)
    shutil.copyfile(package, share / 'candidate.deb')
    shutil.copyfile(ROOT / 'scripts/cosmic_startup_guest.py', share / 'guest.py')
    return image, media, share


def main():
    image, media, share = prepare()
    serial_path = WORK / 'serial.sock'
    qmp_path = WORK / 'qmp.sock'
    for path in (serial_path, qmp_path):
        path.unlink(missing_ok=True)
    kvm = Path('/dev/kvm').exists()
    command = [
        'qemu-system-x86_64', '-machine', 'q35,accel=' + ('kvm' if kvm else 'tcg'),
        '-cpu', 'host' if kvm else 'max', '-m', '6144', '-smp', '4',
        '-kernel', str(WORK / 'vmlinuz'), '-initrd', str(WORK / 'initrd'),
        '-append', f'boot=casper live-media-path=/{str(media).lstrip("./")} noprompt '
                   'systemd.debug_shell=ttyS0 systemd.mask=serial-getty@ttyS0.service',
        '-cdrom', str(image), '-boot', 'd', '-display', 'sdl,gl=on', '-device', 'virtio-vga-gl',
        '-nic', 'user,model=virtio-net-pci',
        '-virtfs', f'local,path={share},mount_tag=cor-test,security_model=none,readonly=on',
        '-serial', f'unix:{serial_path},server=on,wait=off',
        '-qmp', f'unix:{qmp_path},server=on,wait=off', '-no-reboot',
    ]
    console = monitor = None
    environment = {**os.environ, 'DISPLAY': ':93', 'LIBGL_ALWAYS_SOFTWARE': '1',
                   'SDL_VIDEODRIVER': 'x11'}
    with (REPORT / 'qemu.log').open('w') as log, (REPORT / 'xvfb.log').open('w') as display_log:
        display = subprocess.Popen(
            ['Xvfb', ':93', '-screen', '0', '1280x800x24', '+extension', 'GLX', '-nolisten', 'tcp'],
            stdout=display_log, stderr=subprocess.STDOUT,
        )
        display_deadline = time.monotonic() + 20
        while not Path('/tmp/.X11-unix/X93').exists():
            if display.poll() is not None or time.monotonic() >= display_deadline:
                display.terminate()
                display.wait(timeout=10)
                raise RuntimeError('The isolated software display did not initialize')
            time.sleep(.2)
        process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, env=environment)
        try:
            deadline = time.monotonic() + 30
            while not serial_path.exists() or not qmp_path.exists():
                if process.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError('QEMU socket initialization failed')
                time.sleep(.2)
            monitor = Monitor(qmp_path, environment['DISPLAY'])
            console = Console(serial_path)
            console.wait_shell(process)
            console.command('mkdir -p /mnt/cor-test && mount -t 9p '
                            '-o trans=virtio,version=9p2000.L,ro cor-test /mnt/cor-test')
            reports = {}
            for stage in ('prepare', 'first-login', 'background-login', 'manual-launch'):
                print('Starting COSMIC stage:', stage, flush=True)
                failure = None
                try:
                    console.command('python3 /mnt/cor-test/guest.py ' + shlex.quote(stage), timeout=900)
                except Exception as error:
                    failure = error
                evidence_errors = {}
                try:
                    encoded = console.command(
                        'base64 -w0 /var/tmp/cor-startup-' + stage + '.json', timeout=30
                    ).strip()
                    reports[stage] = json.loads(base64.b64decode(encoded, validate=True))
                    (REPORT / (stage + '.json')).write_text(json.dumps(reports[stage], indent=2))
                except Exception as error:
                    evidence_errors['report'] = type(error).__name__ + ': ' + str(error)
                try:
                    monitor.screenshot(stage)
                except Exception as error:
                    evidence_errors['screenshot'] = type(error).__name__ + ': ' + str(error)
                if evidence_errors:
                    (REPORT / (stage + '-evidence-errors.json')).write_text(
                        json.dumps(evidence_errors, indent=2)
                    )
                if failure is not None:
                    raise failure
                if evidence_errors:
                    raise RuntimeError('COSMIC stage evidence is incomplete: ' + str(evidence_errors))
                print(stage, 'verified', flush=True)
            (REPORT / 'result.json').write_text(json.dumps({
                'complete': True, 'platform': 'Pop!_OS 24.04 live COSMIC Wayland',
                'kvm': kvm, 'stages': list(reports),
            }, indent=2))
        finally:
            if console:
                try:
                    journal = console.command('journalctl -b --no-pager -n 600', timeout=30)
                    (REPORT / 'guest-journal.log').write_text(journal)
                except Exception as error:
                    print('Guest journal unavailable:', type(error).__name__)
            if monitor:
                try:
                    monitor.screenshot('final')
                except Exception as error:
                    print('Final screenshot unavailable:', type(error).__name__)
                monitor.close()
            process.terminate()
            try:
                process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
            if console:
                console.close()
            display.terminate()
            display.wait(timeout=10)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('fetch', 'run'), nargs='?', default='run')
    arguments = parser.parse_args()
    if arguments.operation == 'fetch':
        fetch_candidate()
    else:
        main()
