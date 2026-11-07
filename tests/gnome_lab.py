"""Test-only virtual input and screenshots for an isolated GNOME compositor.

This driver clicks the real portal dialogs; Desklink still requests and receives
normal portal permissions. Never point this at the user's session bus.
Requires PyGObject, supplied by the disposable Wayland test image.
"""
import time
from gi.repository import Gio, GLib


class GnomeInput:
    def __init__(self, address):
        self.bus = Gio.DBusConnection.new_for_address_sync(address,
            Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT |
            Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION, None, None)
        self.path = self.call("CreateSession", path="/org/gnome/Mutter/RemoteDesktop",
                              interface="org.gnome.Mutter.RemoteDesktop")[0]
        self.call("Start")
        self.position = None

    def call(self, method, signature=None, args=(), path=None, interface=None):
        return self.bus.call_sync("org.gnome.Mutter.RemoteDesktop", path or self.path,
            interface or "org.gnome.Mutter.RemoteDesktop.Session", method,
            GLib.Variant(signature, args) if signature else None, None,
            Gio.DBusCallFlags.NONE, 5000, None).unpack()

    def move(self, x, y):
        # Relative motion works without binding a screencast/video stream.
        if self.position is None:
            self.call("NotifyPointerMotionRelative", "(dd)", (-10000., -10000.))
            time.sleep(.05)
            self.position = (0., 0.)
        self.call("NotifyPointerMotionRelative", "(dd)",
                  (float(x)-self.position[0], float(y)-self.position[1]))
        self.position = (float(x), float(y))
        time.sleep(.025)

    def button(self, down, button=1):
        self.call("NotifyPointerButton", "(ib)", ({1:272, 2:274, 3:273}[button], down))

    def keysym(self, key, down):
        self.call("NotifyKeyboardKeysym", "(ub)", (key, down))

    def mouse(self, *args):
        args = list(map(str, args))
        while args:
            cmd = args.pop(0)
            if cmd == "mousemove":
                self.move(float(args.pop(0)), float(args.pop(0)))
            elif cmd == "mousemove_relative":
                if args[0] == "--": args.pop(0)
                self.call("NotifyPointerMotionRelative", "(dd)", (float(args.pop(0)),float(args.pop(0))))
            elif cmd in ("click", "mousedown", "mouseup"):
                b = int(args.pop(0))
                if cmd != "mouseup": self.button(True, b)
                if cmd == "click": time.sleep(.03)
                if cmd != "mousedown": self.button(False, b)
            elif cmd == "key":
                name = args.pop(0)
                key = {"Escape":0xff1b, "Return":0xff0d}.get(name, ord(name) if len(name)==1 else 0)
                assert key, name
                self.keysym(key, True); self.keysym(key, False)
            elif cmd == "type":
                while args and args[0].startswith("--"):
                    if args.pop(0) == "--delay": args.pop(0)
                for char in args.pop(0):
                    key = ord(char)
                    self.keysym(key, True); self.keysym(key, False)
            elif cmd == "windowfocus":
                if args and args[0] == "--sync": args.pop(0)
                args.pop(0)  # Focus follows the clicked native surface.
            else:
                raise AssertionError(f"unsupported native lab command: {cmd}")
        return ""

    def screenshot(self, path):
        # Shell authorizes its screenshot client by bus name. This private bus
        # belongs to the disposable test compositor, never the real desktop.
        self.bus.call_sync("org.freedesktop.DBus", "/org/freedesktop/DBus",
            "org.freedesktop.DBus", "RequestName",
            GLib.Variant("(su)", ("org.gnome.Screenshot", 0)), None, 0, 5000, None)
        result = self.bus.call_sync("org.gnome.Shell.Screenshot", "/org/gnome/Shell/Screenshot",
            "org.gnome.Shell.Screenshot", "Screenshot", GLib.Variant("(bbs)",
            (False, False, str(path))), None, 0, 5000, None).unpack()
        assert result[0], result

    def close(self):
        self.call("Stop")
        self.bus.close_sync(None)
