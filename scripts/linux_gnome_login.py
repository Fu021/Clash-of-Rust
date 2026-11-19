"""Inspect the real GNOME Wayland session without exposing its shell evaluator."""
import ast
import json
import re
import subprocess
import tempfile
from pathlib import Path

from build_support import run

UUID = 'compatibility-probe@clash-of-rust.invalid'
DESTINATION = 'org.gnome.Shell'
OBJECT = '/org/clashofrust/TestWindows'
INTERFACE = 'org.clashofrust.TestWindows'

EXTENSION = """\
import Gio from 'gi://Gio';
import Meta from 'gi://Meta';
import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';

export default class CompatibilityProbe extends Extension {
    windows() {
        return global.get_window_actors().map(actor => actor.meta_window).filter(window =>
            window && window.get_title() === 'Clash of Rust' &&
            window.get_client_type() === Meta.WindowClientType.WAYLAND);
    }
    ListWindows() {
        return JSON.stringify(this.windows().map(window => {
            const rect = window.get_frame_rect();
            return { width: rect.width, height: rect.height, pid: window.get_pid(),
                     sequence: window.get_stable_sequence() };
        }));
    }
    CloseWindow(sequence) {
        for (const window of this.windows())
            if (window.get_stable_sequence() === sequence)
                window.delete(global.get_current_time());
    }
    enable() {
        this.probe = Gio.DBusExportedObject.wrapJSObject(
            '<node><interface name="org.clashofrust.TestWindows">' +
            '<method name="ListWindows"><arg type="s" direction="out"/></method>' +
            '<method name="CloseWindow"><arg type="u" direction="in"/></method>' +
            '</interface></node>', this);
        this.probe.export(Gio.DBus.session, '/org/clashofrust/TestWindows');
    }
    disable() {
        this.probe.unexport();
        this.probe = null;
    }
}
"""


def install(as_user):
    from linux_gui import appindicator_extension, output

    version = re.search(r'\b(\d+)\.\d+', output(['gnome-shell', '--version']))
    if not version or int(version[1]) < 45:
        raise RuntimeError('The GNOME Wayland login probe requires GNOME 45 or newer')
    directory = Path('/usr/share/gnome-shell/extensions')/UUID
    with tempfile.TemporaryDirectory(prefix='cor-gnome-probe-') as temporary:
        source = Path(temporary)
        metadata = {'uuid': UUID, 'name': 'Compatibility window probe',
                    'description': 'Inspect candidate windows in a disposable login',
                    'shell-version': [version[1]]}
        (source/'metadata.json').write_text(json.dumps(metadata))
        (source/'extension.js').write_text(EXTENSION)
        run(['sudo', 'install', '-d', '-m', '0755', directory])
        run(['sudo', 'install', '-m', '0644', source/'metadata.json', source/'extension.js', directory])
    extensions = [appindicator_extension(), UUID]
    run(as_user('dbus-run-session', '--', 'gsettings', 'set', 'org.gnome.shell',
                'enabled-extensions', repr(extensions)))


def call(desktop_call, method, *arguments):
    return desktop_call('gdbus', 'call', '--session', '--dest', DESTINATION,
                        '--object-path', OBJECT, '--method', INTERFACE + '.' + method, *arguments)


def windows(desktop_call):
    try:
        raw = call(desktop_call, 'ListWindows')
    except subprocess.CalledProcessError as error:
        if any(reason in (error.stderr or '') for reason in ('UnknownMethod', 'UnknownObject')):
            return []
        raise
    value, = ast.literal_eval(raw)
    return [window for window in json.loads(value) if window['width'] >= 800 and window['height'] >= 450]
