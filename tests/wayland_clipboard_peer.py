#!/usr/bin/env python3
"""A real GTK Wayland clipboard owner/reader for the isolated desktop test."""
import base64
import json
from pathlib import Path
import sys
import gi
gi.require_version("Gtk", "4.0")
gi.require_version("Gdk", "4.0")
from gi.repository import Gdk, Gio, GLib, Gtk

root = Path(sys.argv[1])
root.mkdir(exist_ok=True)
last = None
app = Gtk.Application(application_id="org.desklink.ClipboardTest")


def respond(request, **data):
    temporary = root / "reply.tmp"
    temporary.write_text(json.dumps({"id":request["id"], **data}))
    temporary.replace(root / "reply.json")


def activate(application):
    display = Gdk.Display.get_default()
    assert "Wayland" in display.__gtype__.name, display.__gtype__.name
    clipboard = display.get_clipboard()
    window = Gtk.ApplicationWindow(application=application, title="Desklink clipboard test")
    window.set_default_size(400, 180)
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
    box.append(Gtk.Label(label="Synthetic clipboard data\nPrivate Wayland test desktop"))
    entry = Gtk.Entry()
    entry.connect("changed", lambda field: (root / "typed.txt").write_text(field.get_text()))
    box.append(entry)
    window.set_child(box)
    window.present()
    (root / "ready").write_text(display.__gtype__.name)

    def read_done(source, result, request):
        try:
            stream, mime = source.read_finish(result)
            content = bytearray()
            def bytes_done(stream, result, *_):
                try:
                    chunk = stream.read_bytes_finish(result).get_data()
                    if not chunk:
                        respond(request, data=base64.b64encode(content).decode(), mime=mime)
                    else:
                        content.extend(chunk)
                        assert len(content) <= 1024 * 1024
                        stream.read_bytes_async(4096,GLib.PRIORITY_DEFAULT,None,bytes_done)
                except Exception as error:
                    respond(request,error=str(error))
            stream.read_bytes_async(4096,GLib.PRIORITY_DEFAULT,None,bytes_done)
        except Exception as error:
            respond(request, error=str(error))

    def poll():
        global last
        path = root / "request.json"
        if not path.exists(): return True
        request = json.loads(path.read_text())
        if request["id"] == last: return True
        last = request["id"]
        try:
            if request["action"] == "active":
                respond(request,data=base64.b64encode(b"yes" if window.is_active() else b"no").decode())
            elif request["action"] == "focus":
                window.present();entry.grab_focus();respond(request,ok=True)
            elif request["action"] == "copy":
                data = base64.b64decode(request["data"])
                if request["mime"] == "image/png":
                    provider = Gdk.ContentProvider.new_for_bytes("image/png", GLib.Bytes.new(data))
                    assert clipboard.set_content(provider)
                else:
                    clipboard.set(data.decode())
                respond(request, ok=True)
            else:
                clipboard.read_async([request["mime"]], GLib.PRIORITY_DEFAULT, None, read_done, request)
        except Exception as error:
            respond(request, error=str(error))
        return True
    GLib.timeout_add(50, poll)


app.connect("activate", activate)
app.run([])
