"""Locate and operate the installed COSMIC authorization dialog in a disposable VM."""
import csv
import json
import os
import re
import shlex
import struct
import subprocess
import time

from build_support import write_json
from linux_vm_monitor import Monitor, capture_screen


def dialog_control(tsv, label, width, height):
    words = list(csv.DictReader(tsv.splitlines(), delimiter='\t'))
    visible = {word.get('text', '').casefold() for word in words}
    if not {'authentication', 'tun', 'clash', 'rust'} <= visible:
        return None
    matches = [word for word in words if word.get('text', '').casefold() == label.casefold()]
    if len(matches) != 1:
        return None
    control = matches[0]
    minimum_confidence = 40 if label == 'Password' else 70
    try:
        if float(control['conf']) < minimum_confidence:
            return None
        left, top, span, depth = (int(control[key]) for key in ('left', 'top', 'width', 'height'))
    except (KeyError, ValueError):
        return None
    if not (0 <= left < left+span <= width and 0 <= top < top+depth <= height):
        return None
    return left + span//2, top + depth//2


def recognize_control(source, label, width, height, *, page_mode, image=None):
    try:
        tsv = subprocess.check_output(
            ['tesseract', source, 'stdout', '--psm', str(page_mode), '-l', 'eng', 'tsv'],
            input=image, stderr=subprocess.DEVNULL, timeout=10,
            env={**os.environ, 'OMP_THREAD_LIMIT': '1'})
    except subprocess.TimeoutExpired:
        return None, False
    return dialog_control(tsv.decode(), label, width, height), True


def enlarged_grayscale(screenshot, *, trim):
    code = (
        'from PIL import Image\n'
        'import struct, sys\n'
        'image = Image.open(sys.argv[1]).convert("L")\n'
        'bounds = (image.getbbox() if sys.argv[2] == "trim" else None) or (0, 0, *image.size)\n'
        'image = image.crop(bounds)\n'
        'sys.stdout.buffer.write(struct.pack(">4I", *bounds))\n'
        'image.resize((image.width * 2, image.height * 2)).save(sys.stdout.buffer, format="PNG")\n'
    )
    data = subprocess.check_output(
        ['/usr/bin/python3', '-c', code, screenshot, 'trim' if trim else 'full'], timeout=10)
    left, top, right, bottom = struct.unpack('>4I', data[:16])
    return data[16:], (left, top, right-left, bottom-top)


def capture_control(monitor, screenshot, label, environment):
    method = capture_screen(monitor, screenshot, environment)
    with screenshot.open('rb') as stream:
        data = stream.read(24)
    if len(data) != 24 or not data.startswith(b'\x89PNG\r\n\x1a\n') or data[12:16] != b'IHDR':
        raise RuntimeError('The QEMU display capture is not a PNG')
    width, height = struct.unpack('>II', data[16:24])
    if not (800 <= width <= 4096 and 450 <= height <= 4096):
        return None
    control, complete = recognize_control(screenshot, label, width, height, page_mode=11)
    for trim, page_mode in ((True, 11), (False, 6)):
        if control is not None or not complete:
            break
        try:
            image, (left, top, crop_width, crop_height) = enlarged_grayscale(screenshot, trim=trim)
        except subprocess.TimeoutExpired:
            return None
        scaled, complete = recognize_control('-', label, crop_width*2, crop_height*2,
                                             page_mode=page_mode, image=image)
        if scaled is not None:
            control = left + scaled[0]//2, top + scaled[1]//2
    if control is None:
        return None
    if method == 'x11':
        return control, method
    return (tuple(round(coordinate * 0x7fff / (limit-1))
                  for coordinate, limit in zip(control, (width, height))), method)


def click(monitor, point):
    motion = [{'type': 'abs', 'data': {'axis': axis, 'value': value}}
              for axis, value in zip(('x', 'y'), point)]
    monitor.execute('input-send-event', {'events': motion})
    for pressed in (True, False):
        monitor.execute('input-send-event', {
            'events': [{'type': 'btn', 'data': {'down': pressed, 'button': 'left'}}],
        })


def type_password(monitor, password):
    if not re.fullmatch(r'[a-f0-9]{48}', password):
        raise ValueError('Invalid disposable graphical authentication fixture')
    for character in password:
        monitor.execute('send-key', {'keys': [{'type': 'qcode', 'data': character}], 'hold-time': 15})
        time.sleep(.03)


def answer_dialog(monitor, screenshot, environment, password=None, timeout=90):
    label = 'Password' if password is not None else 'Cancel'
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        target = capture_control(monitor, screenshot, label, environment)
        if target is None:
            time.sleep(1)
            continue
        point, method = target
        if method == 'x11':
            subprocess.run(['xdotool', 'mousemove', str(point[0]), str(point[1]), 'click', '1'],
                           env=environment, check=True, timeout=10)
        else:
            click(monitor, point)
        if password is not None:
            type_password(monitor, password)
            monitor.execute('send-key', {'keys': [{'type': 'qcode', 'data': 'ret'}], 'hold-time': 30})
        return
    raise TimeoutError(f'COSMIC authorization did not show the {label} control')


def verify_dialogs(guest, report):
    from linux_cosmic_desktop import AUTH_LOG, AUTH_MARKER, AUTH_PASSWORD, AUTH_RESULT
    from linux_desktop_login import HOME, USER

    def read(path):
        command = shlex.join(['sudo', '-u', USER, '--', 'head', '-c', '4096', '--', str(HOME/path)])
        return guest.output(f'{command} 2>/dev/null || true', timeout=15).strip()

    def agent_ready(state):
        session = re.fullmatch(r'login ([a-zA-Z0-9]+)', state)
        if session is None:
            return False
        pattern = rf'^(Registered|Unregistered) Authentication Agent for unix-session:{session[1]} \('
        command = shlex.join(['sudo', '-n', 'journalctl', '--boot', '--unit=polkit.service',
                              '--lines=1', '--output=cat', '--no-pager', '--grep', pattern])
        registration = guest.output(f'{command} || test $? -eq 1', timeout=15).strip()
        if not (registration.startswith('Registered Authentication Agent for ')
                and re.search(r'\[(?:[^ \]]*/)?cosmic-osd(?:[ \]])', registration)):
            return False
        command = shlex.join(['sudo', '-u', USER, '--', 'tee', str(HOME/AUTH_MARKER)])
        guest.remote(f'printf %s ready | {command} >/dev/null', timeout=15)
        return True

    monitor = Monitor(guest.monitor_path)
    try:
        for phase in ('agent registration', 'cancel', 'accept'):
            deadline = time.monotonic() + 150
            while time.monotonic() < deadline:
                result = read(AUTH_RESULT)
                try:
                    result = json.loads(result)
                except ValueError:
                    result = None
                if isinstance(result, dict):
                    write_json(report/'cosmic-authorization-early-result.json', {
                        'expected_phase': phase, 'guest_result': result,
                        'authorization_log': read(AUTH_LOG),
                    })
                    raise RuntimeError(f'COSMIC authorization exited before {phase}')
                state = read(AUTH_MARKER)
                ready = agent_ready(state) if phase == 'agent registration' else state == phase
                if ready:
                    break
                time.sleep(1)
            else:
                raise TimeoutError(f'COSMIC authorization did not reach {phase}')
            if phase == 'agent registration':
                continue
            password = None if phase == 'cancel' else read(AUTH_PASSWORD)
            answer_dialog(monitor, report/f'cosmic-authorization-{phase}.png', guest.display.environment, password)
    finally:
        monitor.close()
