"""Owned desktop fixtures for Linux memory measurements, outside measured PIDs."""
import ctypes as c
import json
import os
from pathlib import Path
import subprocess
import sys
import time


WATCHER = 'org.kde.StatusNotifierWatcher'
OBJECT = '/StatusNotifierWatcher'
INTERFACE = '''<node><interface name="org.kde.StatusNotifierWatcher">
  <method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
  <method name="RegisterStatusNotifierHost"><arg type="s" direction="in"/></method>
  <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
  <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
  <property name="ProtocolVersion" type="i" access="read"/>
  <signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
  <signal name="StatusNotifierItemUnregistered"><arg type="s"/></signal>
  <signal name="StatusNotifierHostRegistered"/>
  <signal name="StatusNotifierHostUnregistered"/>
</interface></node>'''


def wait_until(check, description, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if check():
            return
        time.sleep(0.1)
    raise RuntimeError('Timed out waiting for ' + description)


class TrayHost:
    def __init__(self, output):
        self.state = output/'tray-host.json'
        self.log_path = output/'tray-host.log'

    def read(self):
        return json.loads(self.state.read_text()) if self.state.exists() else {}

    def __enter__(self):
        if not os.environ.get('DBUS_SESSION_BUS_ADDRESS'):
            raise RuntimeError('Tray memory measurements require dbus-run-session')
        self.log = self.log_path.open('x')
        self.process = subprocess.Popen(
            ['/usr/bin/python3', str(Path(__file__).resolve()), str(self.state)],
            stdout=self.log, stderr=subprocess.STDOUT,
        )
        try:
            wait_until(lambda: self.ready(), 'isolated tray host')
        except BaseException:
            self.__exit__(None, None, None)
            raise
        return self

    def ready(self):
        if self.process.poll() is not None:
            raise RuntimeError('Tray fixture exited; see tray-host.log')
        return self.read().get('ready', False)

    def registered(self, pid):
        return any(item['pid'] == pid for item in self.read().get('items', []))

    def __exit__(self, *_):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        self.log.close()


def close_window(window, env):
    """Send the WM_DELETE_WINDOW request used by a window manager's close button.

    XUnmapWindow would bypass the application's hidden-state cleanup. Destroying
    the window would not exercise the close-to-tray path either.
    """
    class Data(c.Union):
        _fields_ = [('b', c.c_char*20), ('s', c.c_short*10), ('l', c.c_long*5)]

    class ClientMessage(c.Structure):
        _fields_ = [('type', c.c_int), ('serial', c.c_ulong), ('send_event', c.c_int),
                    ('display', c.c_void_p), ('window', c.c_ulong), ('message_type', c.c_ulong),
                    ('format', c.c_int), ('data', Data)]

    class Event(c.Union):
        _fields_ = [('client', ClientMessage), ('padding', c.c_long*24)]

    x = c.CDLL('libX11.so.6')
    x.XOpenDisplay.argtypes = [c.c_char_p]
    x.XOpenDisplay.restype = c.c_void_p
    x.XInternAtom.argtypes = [c.c_void_p, c.c_char_p, c.c_int]
    x.XInternAtom.restype = c.c_ulong
    x.XSendEvent.argtypes = [c.c_void_p, c.c_ulong, c.c_int, c.c_long, c.POINTER(Event)]
    x.XFlush.argtypes = [c.c_void_p]
    x.XCloseDisplay.argtypes = [c.c_void_p]
    display = x.XOpenDisplay(env['DISPLAY'].encode())
    if not display:
        raise RuntimeError('Cannot open owned Xvfb display')
    try:
        event = Event()
        event.client.type = 33  # ClientMessage, X11/X.h
        event.client.display = display
        event.client.window = int(window)
        event.client.message_type = x.XInternAtom(display, b'WM_PROTOCOLS', False)
        event.client.format = 32
        event.client.data.l[0] = x.XInternAtom(display, b'WM_DELETE_WINDOW', False)
        if not x.XSendEvent(display, int(window), False, 0, c.byref(event)):
            raise RuntimeError('Could not request close on the measured window')
        x.XFlush(display)
    finally:
        x.XCloseDisplay(display)


def tray_host_main(path):
    # Use Ubuntu's system Python: python3-gi is installed for that interpreter.
    from gi.repository import Gio, GLib
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    result = bus.call_sync(
        'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'RequestName',
        GLib.Variant('(su)', (WATCHER, 4)), GLib.VariantType.new('(u)'),
        Gio.DBusCallFlags.NONE, 5000, None,
    )
    if result.unpack()[0] != 1:
        raise RuntimeError('Refusing to replace an existing desktop tray watcher')
    state = {'ready': True, 'items': []}

    def save():
        temporary = path.with_suffix('.tmp')
        temporary.write_text(json.dumps(state))
        temporary.replace(path)

    def method(connection, sender, _path, _interface, name, parameters, invocation):
        if name == 'RegisterStatusNotifierItem':
            value = parameters.unpack()[0]
            service = sender if value.startswith('/') else value
            item_path = value if value.startswith('/') else '/StatusNotifierItem'
            try:
                pid = bus.call_sync(
                    'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
                    'GetConnectionUnixProcessID', GLib.Variant('(s)', (service,)),
                    GLib.VariantType.new('(u)'), Gio.DBusCallFlags.NONE, 5000, None,
                ).unpack()[0]
            except GLib.Error as error:
                invocation.return_dbus_error(WATCHER+'.InvalidItem', str(error))
                return
            item = {'service': service, 'path': item_path, 'pid': pid}
            if item not in state['items']:
                state['items'].append(item)
                save()
                connection.emit_signal(None, OBJECT, WATCHER, 'StatusNotifierItemRegistered',
                                       GLib.Variant('(s)', (service+item_path,)))
        invocation.return_value(GLib.Variant('()', ()))

    def property_value(_connection, _sender, _path, _interface, name):
        return {
            'RegisteredStatusNotifierItems': GLib.Variant('as', [
                item['service']+item['path'] for item in state['items']]),
            'IsStatusNotifierHostRegistered': GLib.Variant('b', True),
            'ProtocolVersion': GLib.Variant('i', 0),
        }[name]

    info = Gio.DBusNodeInfo.new_for_xml(INTERFACE)
    bus.register_object(OBJECT, info.interfaces[0], method, property_value, None)
    save()
    GLib.MainLoop().run()


if __name__ == '__main__':
    if len(sys.argv) != 2:
        raise SystemExit('memory_desktop.py TRAY_STATE_FILE (private benchmark session only)')
    tray_host_main(Path(sys.argv[1]))
