"""Locate and operate the installed COSMIC authorization dialog in a disposable VM."""
import csv
import os
import re
import shlex
import struct
import subprocess
import time

from linux_vm_monitor import Monitor, capture_screen


def dialog_button(tsv, label, width, height):
    words = list(csv.DictReader(tsv.splitlines(), delimiter='\t'))
    visible = {word.get('text', '').casefold() for word in words}
    if not {'authentication', 'tun', 'clash', 'rust'} <= visible:
        return None
    matches = [word for word in words if word.get('text', '').casefold() == label.casefold()]
    if len(matches) != 1:
        return None
    button = matches[0]
    try:
        if float(button['conf']) < 70:
            return None
        left, top, span, depth = (int(button[key]) for key in ('left', 'top', 'width', 'height'))
    except (KeyError, ValueError):
        return None
    if not (0 <= left < left+span <= width and 0 <= top < top+depth <= height):
        return None
    return left + span//2, top + depth//2


def capture_button(monitor, screenshot, label, environment):
    method = capture_screen(monitor, screenshot, environment)
    with screenshot.open('rb') as stream:
        data = stream.read(24)
    if len(data) != 24 or not data.startswith(b'\x89PNG\r\n\x1a\n') or data[12:16] != b'IHDR':
        raise RuntimeError('The QEMU display capture is not a PNG')
    width, height = struct.unpack('>II', data[16:24])
    if not (800 <= width <= 4096 and 450 <= height <= 4096):
        return None
    try:
        tsv = subprocess.check_output(
            ['tesseract', screenshot, 'stdout', '--psm', '11', '-l', 'eng', 'tsv'],
            text=True, stderr=subprocess.DEVNULL, timeout=10,
            env={**os.environ, 'OMP_THREAD_LIMIT': '1'})
    except subprocess.TimeoutExpired:
        return None
    button = dialog_button(tsv, label, width, height)
    if button is None:
        return None
    if method == 'x11':
        return button, method
    return (tuple(round(coordinate * 0x7fff / (limit-1))
                  for coordinate, limit in zip(button, (width, height))), method)


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
    label = 'Authenticate' if password is not None else 'Cancel'
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        target = capture_button(monitor, screenshot, label, environment)
        if target is None:
            time.sleep(1)
            continue
        point, method = target
        if password is not None:
            type_password(monitor, password)
        if method == 'x11':
            subprocess.run(['xdotool', 'mousemove', str(point[0]), str(point[1]), 'click', '1'],
                           env=environment, check=True, timeout=10)
        else:
            click(monitor, point)
        return
    raise TimeoutError(f'COSMIC authorization did not show the {label} button')


def verify_dialogs(guest, report):
    from linux_cosmic_desktop import AUTH_MARKER, AUTH_PASSWORD, AUTH_RESULT
    from linux_desktop_login import HOME, USER

    def read(path):
        command = shlex.join(['sudo', '-u', USER, '--', 'cat', str(HOME/path)])
        return guest.output(f'{command} 2>/dev/null || true', timeout=15).strip()

    monitor = Monitor(guest.monitor_path)
    try:
        for phase in ('cancel', 'accept'):
            deadline = time.monotonic() + 150
            while time.monotonic() < deadline:
                if read(AUTH_RESULT):
                    raise RuntimeError('COSMIC authorization exited before the expected '+phase+' dialog')
                if read(AUTH_MARKER) == phase:
                    break
                time.sleep(1)
            else:
                raise TimeoutError('COSMIC authorization did not request '+phase)
            password = None if phase == 'cancel' else read(AUTH_PASSWORD)
            answer_dialog(monitor, report/f'cosmic-authorization-{phase}.png', guest.display.environment, password)
    finally:
        monitor.close()
