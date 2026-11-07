#!/usr/bin/env python3
"""Native layout editor integration test; sends input only to an isolated Xvfb display."""
import argparse
import base64
import json
import io
import os
import re
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import time
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib


def wait(check, label, seconds=15):
    end = time.monotonic() + seconds
    last = None
    while time.monotonic() < end:
        try:
            value = check()
            if value:
                return value
        except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
            last = error
        time.sleep(0.07)
    raise AssertionError(f"Timed out: {label}: {last}")


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def run(binary, screenshot=None, capture_only=False, wayland=False, sharing=False, lab_env=None, headless=False, approve_gnome=False):
    processes, handles = [], []
    with tempfile.TemporaryDirectory(prefix="desklink-ui-") as temp:
        root = Path(temp)
        statefile = root / "state.json"
        profiles = [root / f"computer-{i}/config.toml" for i in range(4)]
        def spawn(args, env=None, log=None, **options):
            if log:
                handle = open(log, "ab", buffering=0)
                handles.append(handle)
                options.update(stdout=handle, stderr=handle)
            proc = subprocess.Popen(args, env=env, **options)
            processes.append(proc)
            return proc
        def display():
            proc = spawn(["Xvfb", "-displayfd", "1", "-screen", "0", "1600x1000x24", "-nolisten", "tcp", "-ac"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            number = proc.stdout.readline().decode().strip()
            assert number.isdigit()
            env = dict(os.environ, DISPLAY=f":{number}", XDG_SESSION_TYPE="x11", LIBGL_ALWAYS_SOFTWARE="1", WINIT_UNIX_BACKEND="x11", NO_COLOR="1")
            env.pop("WAYLAND_DISPLAY", None)
            return env
        def cli(profile, *args):
            return subprocess.check_output([str(binary), "--config", str(profile), *args], stderr=subprocess.PIPE).decode().strip()
        def state():
            return json.loads(statefile.read_text())
        def mouse(*args):
            if headless:
                return native.mouse(*args)
            return subprocess.check_output(["xdotool", *map(str, args)], env=env, stderr=subprocess.PIPE).decode()
        def capture(path):
            if headless:
                native.screenshot(path)
            else:
                subprocess.run(["import", "-display", env["DISPLAY"], "-window", window, str(path)], check=True)
        def window_offset():
            if wayland:
                return {"X":0,"Y":0}
            data = mouse("getwindowgeometry", "--shell", window)
            return {k: int(v) for k, v in (line.split("=", 1) for line in data.splitlines())}
        def target(name):
            box = wait(lambda: state()["targets"].get(name), f"control {name}")
            # Dialog content can change its centered bounds after a paste.
            # Click only once a subsequent rendered snapshot agrees on placement.
            for _ in range(8):
                time.sleep(.18)
                next_box = state()["targets"].get(name)
                if next_box == box:
                    break
                if next_box: box = next_box
            offset = window_offset()
            return offset["X"] + box[0] + box[2] / 2, offset["Y"] + box[1] + box[3] / 2
        def click(name):
            x, y = target(name)
            mouse("mousemove", int(x), int(y), "click", "1")
            time.sleep(0.2)
        def drag(name, x, y):
            startx, starty = target(name)
            mouse("mousemove", int(startx), int(starty), "mousedown", "1")
            for step in range(1, 15):
                mouse("mousemove", int(startx+(x-startx)*step/14), int(starty+(y-starty)*step/14))
                time.sleep(0.018)
            mouse("mouseup", "1")
            time.sleep(0.3)
        def control(profile, request):
            with socket.socket(socket.AF_UNIX) as sock:
                sock.settimeout(2)
                sock.connect(str(profile.with_name("control.sock")))
                data = json.dumps(request).encode()
                sock.sendall(len(data).to_bytes(4, "big")+data)
                header = sock.recv(4)
                if len(header) != 4:
                    return None
                needed = int.from_bytes(header, "big")
                body = b""
                while len(body) < needed:
                    body += sock.recv(needed-len(body))
                return json.loads(body)

        try:
            env = display()
            if wayland:
                runtime = root / "runtime"
                runtime.mkdir(mode=0o700)
                busfile = root / "bus-address"
                shellenv = dict(env, XDG_RUNTIME_DIR=str(runtime), XDG_CURRENT_DESKTOP="GNOME", XDG_SESSION_TYPE="wayland", LAB_BUS_FILE=str(busfile), MUTTER_DEBUG_DUMMY_MODE_SPECS="1600x1000")
                system_address = f"unix:path={runtime}/system-bus"
                spawn(["dbus-daemon", "--session", "--nofork", f"--address={system_address}"], env=shellenv, log=root / "system-bus.log")
                wait(lambda: (runtime / "system-bus").exists(), "isolated desktop system bus")
                shellenv["DBUS_SYSTEM_BUS_ADDRESS"] = system_address
                shellargs = "--headless --virtual-monitor 1600x1000" if headless else "--nested"
                spawn(["dbus-run-session", "--", "sh", "-c", 'printf %s "$DBUS_SESSION_BUS_ADDRESS" > "$LAB_BUS_FILE"; exec gnome-shell '+shellargs+' --wayland --wayland-display desklink-test-0 --sm-disable'], env=shellenv, log=root / "gnome.log")
                wait(lambda: (runtime / "desklink-test-0").exists(), "nested GNOME Wayland compositor", 30)
                env.update(XDG_RUNTIME_DIR=str(runtime), XDG_CURRENT_DESKTOP="GNOME", XDG_SESSION_TYPE="wayland", WAYLAND_DISPLAY="desklink-test-0", DBUS_SESSION_BUS_ADDRESS=busfile.read_text(), DBUS_SYSTEM_BUS_ADDRESS=system_address, DESKLINK_UI_TEST_FULLSCREEN="1")
                env.pop("WINIT_UNIX_BACKEND", None)
                def compositor_window():
                    tree = subprocess.check_output(["xwininfo", "-root", "-tree"], env=env).decode()
                    windows = [(int(w)*int(h), wid) for wid, w, h in re.findall(r'(0x[0-9a-f]+)[^\n]*?\s(\d+)x(\d+)\+', tree) if int(w)>=640 and int(h)>=480]
                    return max(windows)[1] if windows else None
                if headless:
                    from gnome_lab import GnomeInput
                    native = wait(lambda: GnomeInput(env["DBUS_SESSION_BUS_ADDRESS"]), "native GNOME virtual test input", 30)
                    shellwindow = "native"
                else:
                    shellwindow = wait(compositor_window, "nested compositor host window")
                    wait(lambda: subprocess.run(["xdotool", "windowfocus", "--sync", shellwindow], env=env, capture_output=True).returncode == 0, "mapped compositor window")
                    mouse("windowsize", shellwindow, "1600", "1000")
                    mouse("windowmove", shellwindow, "0", "0")
                    mouse("windowfocus", "--sync", shellwindow)
                time.sleep(1)
                mouse("key", "Escape")
            names = ["Workstation", "Design laptop", "Studio display", "Travel laptop"]
            ports = [free_port() for _ in profiles]
            keys = []
            for profile, name, port in zip(profiles, names, ports):
                cli(profile, "init", "--name", name, "--listen", f"127.0.0.1:{port}")
                keys.append(cli(profile, "identity"))
            for i in (1, 2):
                cli(profiles[0], "trust", keys[i], "--name", names[i], "--address", f"127.0.0.1:{ports[i]}")
                cli(profiles[i], "trust", keys[0], "--name", names[0], "--address", f"127.0.0.1:{ports[0]}")
            with open(profiles[0], "a") as file:
                file.write(f'\n[layout]\nschema = 1\nrevision = 1\nowner = "{keys[0]}"\n')
                for i, x in ((0, 0), (1, -240), (2, 240)):
                    file.write(f'\n[[layout.screens]]\nid = "{keys[i]}"\nname = "{names[i]}"\n[layout.screens.rect]\nx = {x}\ny = 0\nwidth = 240\nheight = 150\n')
            gui = spawn([str(binary), "--config", str(profiles[0]), "ui", "--test-state", str(statefile)], env=env, log=root / "gui.log")
            wait(lambda: statefile.exists() or gui.poll() is not None, "native window startup", 30)
            assert gui.poll() is None, (root / "gui.log").read_text()
            window = shellwindow if wayland else wait(lambda: mouse("search", "--name", "Desklink.*Screen layout").splitlines()[0], "native window")
            if headless or wayland:
                mouse("windowfocus", "--sync", window)
            else:
                # The window is named before it is mapped; focusing an unmapped window fails.
                wait(lambda: subprocess.run(["xdotool", "windowfocus", "--sync", window], env=env, capture_output=True).returncode == 0, "mapped native window")
            wait(lambda: len(state()["targets"]) >= 10, "rendered controls")
            if lab_env:
                Path(lab_env).write_text(json.dumps({"env":env,"root":str(root),"profiles":[str(p) for p in profiles],"state":str(statefile),"keys":keys}))
            assert state()["window_backend"] == ("wayland" if wayland else "x11"), state()["window_backend"]
            click("canvas")
            if screenshot:
                Path(screenshot).parent.mkdir(parents=True, exist_ok=True)
                time.sleep(0.4)
                capture(screenshot)
            print("PASS: real native window renders the configured computer arrangement", flush=True)
            if capture_only:
                return

            main = state()["targets"][f"screen:{keys[0]}"]
            right = state()["targets"][f"screen:{keys[2]}"]
            offset = window_offset()
            # Preserve a shared horizontal edge: move the right card immediately below the main one.
            x = offset["X"] + main[0] + main[2]/2
            y = offset["Y"] + main[1] + main[3] + right[3]/2
            drag(f"screen:{keys[2]}", x, y)
            wait(lambda: state()["dirty"] and state()["layout_error"] is None, "valid snapped drag")
            click("save")
            wait(lambda: not state()["dirty"], "layout save")
            saved = tomllib.loads(profiles[0].read_text())
            moved = next(s for s in saved["layout"]["screens"] if s["id"] == keys[2])
            assert (moved["rect"]["x"], moved["rect"]["y"]) == (0, 150), moved
            click("reload")
            time.sleep(.4)  # Reload schedules a fitted view on the next rendered frame.
            assert next(s for s in state()["layout"]["screens"] if s["id"] == keys[2])["rect"]["y"] == 150
            print("PASS: dragging snaps a computer to the bottom; saved placement survives reload", flush=True)

            original = target(f"screen:{keys[1]}")
            owncenter = target(f"screen:{keys[0]}")
            drag(f"screen:{keys[1]}", *owncenter)
            wait(lambda: state()["layout_error"] and "overlap" in state()["layout_error"], "overlap validation")
            before = profiles[0].read_bytes()
            click("save")
            assert profiles[0].read_bytes() == before, "invalid draft must not overwrite the saved layout"
            drag(f"screen:{keys[1]}", *original)
            wait(lambda: state()["layout_error"] is None, "corrected placement")
            click("save")
            wait(lambda: not state()["dirty"], "corrected layout saved")
            print("PASS: overlapping screens are reported and corrected through real pointer input", flush=True)

            click("add")
            click("pair-code")
            code = "desklink://pair/" + base64.urlsafe_b64encode(json.dumps({"version":1,"name":names[3],"public_key":keys[3],"address":f"127.0.0.1:{ports[3]}"}).encode()).decode().rstrip("=")
            mouse("type", "--clearmodifiers", "--delay", "0", code)
            time.sleep(0.3)
            click("confirm-add")
            wait(lambda: len(state()["layout"]["screens"]) == 4, "computer added through pairing dialog")
            click("save")
            wait(lambda: not state()["dirty"], "paired computer saved")
            print("PASS: pairing dialog adds a real identity to the arrangement", flush=True)
            if sharing:
                # Real receivers, each on its own private X11 desktop. The controller
                # uses the window's native backend (X11 or the nested Wayland compositor).
                peer_envs = {}
                for i in (1, 2):
                    text = profiles[i].read_text()
                    text = f'layout_owner = "{keys[0]}"\n' + text
                    profiles[i].write_text(text)
                    peer_env = display()
                    peer_envs[i] = peer_env
                    spawn([str(binary), "--config", str(profiles[i]), "run"], env=peer_env, log=root / f"receiver-{i}.log")
                    wait(lambda p=profiles[i]: control(p, "Status")["running"], "receiver started")
                if lab_env:
                    Path(lab_env).write_text(json.dumps({"env":env,"root":str(root),"profiles":[str(p) for p in profiles],"state":str(statefile),"keys":keys}))
                click("sharing")
                print("WAITING: approve native desktop permission dialogs if shown", flush=True)
                last_permission = [0.]
                approved = set()
                def ready():
                    if headless and (screenshot or approve_gnome) and time.monotonic() - last_permission[0] > .4:
                        last_permission[0] = time.monotonic()
                        permission = Path(screenshot).with_suffix(".permission.png") if screenshot else root / "permission.png"
                        permission.unlink(missing_ok=True)
                        native.screenshot(permission)
                        if approve_gnome:
                            # The pinned GNOME 48 consent UI on this test's private
                            # 1600×1000 desktop. Wait until each real dialog renders.
                            # No portal responses or grants are mocked.
                            from PIL import Image
                            image = Image.open(permission).convert("RGB")
                            def blue(x,y):
                                r,g,b=image.getpixel((x,y));return r<205 and b>r+25 and g>r+10
                            if "capture" not in approved and blue(1002,377):
                                mouse("mousemove",980,489,"click",1);time.sleep(.2)
                                mouse("mousemove",1002,389,"click",1)
                                approved.add("capture")
                            elif "remote" not in approved and blue(1083,277):
                                mouse("mousemove",1045,371,"click",1);time.sleep(.2)
                                mouse("mousemove",1083,289,"click",1)
                                approved.add("remote")
                    return state()["running"] or (not state()["starting"] and state()["error"])
                wait(ready, "UI starts the sharing agent", 110)
                assert state()["running"], state()["error"]
                if headless and not approve_gnome: native.position = None
                wait(lambda: sum(p["connected"] for p in state()["status"]["peers"]) == 2, "UI reports both connected computers")
                for profile in profiles[1:3]:
                    wait(lambda p=profile: control(p, "Status")["layout_revision"] == state()["layout"]["revision"], "receivers get the saved UI layout")
                print("PASS: Start sharing launches the real agent and reports connected receivers", flush=True)
                # A graphical layout edit must reach the existing running agents.
                main = state()["targets"][f"screen:{keys[0]}"]
                below = state()["targets"][f"screen:{keys[2]}"]
                offset = window_offset()
                drag(f"screen:{keys[2]}", offset["X"] + main[0] + main[2] + below[2]/2, offset["Y"] + main[1] + main[3]/2)
                wait(lambda: state()["dirty"] and state()["layout_error"] is None, "live edit is valid")
                pid = state()["agent_pid"]
                click("save")
                revision = tomllib.loads(profiles[0].read_text())["layout"]["revision"]
                if wayland:
                    approved.clear()
                    wait(lambda: state()["agent_pid"] not in (None,pid) and state()["starting"], "new Wayland permission session")
                    wait(ready, "approve changed Wayland capture edges", 110)
                    assert state()["running"], state()["error"]
                wait(lambda: state()["status"]["layout_revision"] == revision, "UI applies the edit")
                if not wayland:
                    assert state()["agent_pid"] == pid, "an X11 layout edit must not restart sharing"
                for profile in profiles[1:3]:
                    wait(lambda p=profile: control(p, "Status")["layout_revision"] == revision, "live UI edit reaches receiver")
                print("PASS: Save layout applies and synchronizes changes" + (" through a fresh Wayland permission session" if wayland else " without restarting sharing"), flush=True)
                if headless:
                    # Exercise real Wayland capture/EI and portal clipboard using
                    # synthetic input and data on these private desktops only.
                    events = root / "wayland-received.log"
                    spawn(["stdbuf", "-oL", "xev", "-name", "Wayland receiver", "-geometry", "1580x950+0+0"], env=peer_envs[1], log=events)
                    def receiver_mouse(*args):
                        return subprocess.check_output(["xdotool", *map(str,args)],env=peer_envs[1]).decode()
                    wid = wait(lambda: receiver_mouse("search","--name","Wayland receiver").splitlines()[0], "X11 receiver observer")
                    receiver_mouse("windowfocus","--sync",wid)
                    native.move(800,500)
                    native.mouse("mousemove_relative","--",-7000,0)
                    wait(lambda: control(profiles[0],"Status")["active_computer"]==keys[1], "Wayland edge capture enters the left computer")
                    time.sleep(.2)
                    native.mouse("mousemove_relative","--",-1000,0)
                    time.sleep(.1)
                    native.mouse("key","a")
                    native.mouse("click",1)
                    native.call("NotifyPointerAxisDiscrete", "(ui)", (0, 1))
                    wait(lambda: "keysym 0x61, a" in events.read_text(), "Wayland keyboard reaches the receiver")
                    wait(lambda: "ButtonPress event" in events.read_text(), "Wayland button reaches the receiver")
                    wait(lambda: "button 5" in events.read_text(), "Wayland scroll reaches the receiver")
                    native.mouse("mousemove_relative","--",7000,0)
                    wait(lambda: control(profiles[0],"Status")["active_computer"]==keys[0], "Wayland returns to the controller")
                    time.sleep(.4)
                    before_pointer = state()["pointer"]
                    native.call("NotifyPointerMotionRelative", "(dd)", (2.,0.))
                    wait(lambda: state()["pointer"] and state()["pointer"] != before_pointer, "local pointer position after portal release")
                    native.position = tuple(state()["pointer"])
                    print("PASS: Wayland barrier capture, EI keyboard/pointer/buttons/scroll and edge return", flush=True)

                    clipboard_root = root / "gtk-clipboard"
                    spawn(["python3", str(Path(__file__).with_name("wayland_clipboard_peer.py")), str(clipboard_root)],
                          env=dict(env,GDK_BACKEND="wayland"), log=root / "clipboard-owner.log")
                    wait(lambda: (clipboard_root / "ready").exists(), "real native GTK Wayland clipboard client")
                    time.sleep(.3)
                    native.move(800,530);native.mouse("click",1)
                    time.sleep(.2)
                    sequence = [0]
                    def gtk_clipboard(action, mime, data=None):
                        sequence[0] += 1
                        request = {"id":sequence[0],"action":action,"mime":mime}
                        if data is not None: request["data"] = base64.b64encode(data).decode()
                        temporary = clipboard_root / "request.tmp"
                        temporary.write_text(json.dumps(request));temporary.replace(clipboard_root / "request.json")
                        def reply():
                            value = json.loads((clipboard_root / "reply.json").read_text())
                            return value if value["id"] == sequence[0] else None
                        value = wait(reply, "native GTK clipboard response", 4)
                        if "error" in value: raise ValueError(value["error"])
                        return base64.b64decode(value["data"]) if "data" in value else None

                    def copy_data(data, mime, wl):
                        if wl: return gtk_clipboard("copy",mime,data)
                        args = ["xclip", "-quiet", "-selection", "clipboard", "-in", "-target", mime]
                        owner = spawn(args, env=peer_envs[1], stdin=subprocess.PIPE, log=root / "x11-clipboard-owner.log")
                        owner.stdin.write(data); owner.stdin.close()
                    def paste(mime, wl):
                        if wl: return gtk_clipboard("paste",mime)
                        args = ["xclip", "-selection", "clipboard", "-out", "-target", mime]
                        return subprocess.check_output(args,env=peer_envs[1],stderr=subprocess.DEVNULL,timeout=3)
                    text = "Desklink Wayland → X11 — café 日本語 🖥\n".encode()
                    copy_data(text, "text/plain;charset=utf-8", True)
                    wait(lambda: paste("UTF8_STRING",False)==text, "Wayland text clipboard reaches X11")
                    reverse = "X11 → Wayland clipboard ✓\n".encode()
                    copy_data(reverse,"UTF8_STRING",False)
                    wait(lambda: paste("text/plain;charset=utf-8",True)==reverse, "X11 text clipboard reaches Wayland")
                    from PIL import Image
                    image = Image.new("RGBA",(7,5),(53,127,231,96))
                    encoded = io.BytesIO(); image.save(encoded,format="PNG")
                    def same_image(data):
                        other = Image.open(io.BytesIO(data)).convert("RGBA")
                        return other.size == image.size and other.tobytes() == image.tobytes()
                    copy_data(encoded.getvalue(),"image/png",True)
                    wait(lambda: same_image(paste("image/png",False)), "Wayland PNG clipboard reaches X11")
                    image.putpixel((2,3),(190,42,65,183));encoded=io.BytesIO();image.save(encoded,format="PNG")
                    copy_data(encoded.getvalue(),"image/png",False)
                    wait(lambda: same_image(paste("image/png",True)), "X11 PNG clipboard reaches Wayland")
                    print("PASS: portal text and PNG clipboard sharing in both directions retains pixels and alpha", flush=True)
                    # Return focus to Desklink after the native clipboard test window.
                    native.move(1400,800);native.mouse("click",1)
                    time.sleep(.2)
                    click(f"computer:{keys[0]}")
                    click("copy-code")
                    wait(lambda: paste("UTF8_STRING",False).startswith(b"desklink://pair/"), "native UI pairing code is copied and shared")
                    print("PASS: native Wayland UI copies a pairing code to the shared clipboard", flush=True)
                click("sharing")
                wait(lambda: not state()["running"], "UI stops sharing")
                wait(lambda: state()["agent_pid"] is None, "agent exits cleanly")
                print("PASS: Stop sharing cleanly releases and stops the real agent", flush=True)
                if headless:
                    # Reverse the controller through the real GUI, then exercise
                    # EI injection into a native GTK Wayland entry.
                    from layout_e2e import write_config
                    control(profiles[1],"Stop")
                    wait(lambda: not profiles[1].with_name("control.sock").exists(), "old receiver exits")
                    reverse_layout={"owner":keys[1],"revision":1,"screens":[
                        {"id":keys[1],"name":names[1],"rect":{"x":0,"y":0,"width":240,"height":150}},
                        {"id":keys[0],"name":names[0],"rect":{"x":240,"y":0,"width":240,"height":150}}]}
                    write_config(profiles[1],names[1],ports[1],[(keys[0],f"127.0.0.1:{ports[0]}")],reverse_layout)
                    spawn([str(binary),"--config",str(profiles[1]),"run"],env=peer_envs[1],log=root/"reverse-controller.log")
                    click("controller");click(f"controller:{keys[1]}");click("save")
                    wait(lambda: not state()["dirty"], "receiver role saved through the GUI")
                    approved.clear();click("sharing");wait(ready,"Wayland receiver permissions",110)
                    assert state()["running"],state()["error"]
                    wait(lambda: state()["layout"]["owner"]==keys[1], "Wayland receiver displays its selected controller's layout")
                    gtk_clipboard("focus","text/plain")
                    time.sleep(.3)
                    if gtk_clipboard("active","text/plain")!=b"yes":
                        native.keysym(0xffe9,True);native.keysym(0xff09,True)
                        time.sleep(.1)
                        native.keysym(0xff09,False);native.keysym(0xffe9,False)
                    wait(lambda: gtk_clipboard("active","text/plain")==b"yes", "native receiver widget has keyboard focus")
                    receiver_mouse("mousemove",800,500);time.sleep(.1);receiver_mouse("mousemove",1599,500)
                    wait(lambda: control(profiles[1],"Status")["active_computer"]==keys[0], "X11 controller enters Wayland receiver")
                    receiver_mouse("key","w")
                    wait(lambda: "w" in (clipboard_root/"typed.txt").read_text(), "EI keyboard injection reaches a native Wayland widget")
                    receiver_mouse("mousemove_relative","--",-7000,0)
                    wait(lambda: control(profiles[1],"Status")["active_computer"]==keys[1], "return from Wayland receiver")
                    print("PASS: GUI controller selection, received layout, X11→Wayland EI input and edge return",flush=True)
            print("ALL NATIVE UI CHECKS PASSED", flush=True)
        except Exception:
            if screenshot and 'env' in locals():
                capture(screenshot)
            for log in root.rglob("*.log"):
                print(f"\n{log.name}:\n{log.read_text(errors='replace')[-6000:]}")
            if statefile.exists():
                print(statefile.read_text()[:8000])
            raise
        finally:
            for profile in profiles:
                try:
                    control(profile, "Stop")
                except (OSError, ValueError):
                    pass
            if headless and 'native' in locals():
                try: native.close()
                except Exception: pass
            for proc in reversed(processes):
                if proc.poll() is None:
                    proc.terminate()
            for proc in reversed(processes):
                try:
                    proc.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    proc.kill(); proc.wait(timeout=3)
            for handle in handles:
                handle.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path(__file__).resolve().parents[1]/"target/debug/desklink")
    parser.add_argument("--screenshot", type=Path)
    parser.add_argument("--capture-only", action="store_true")
    parser.add_argument("--wayland", action="store_true")
    parser.add_argument("--sharing", action="store_true")
    parser.add_argument("--lab-env", type=Path, help="Export isolated test environment for permission-dialog automation")
    parser.add_argument("--headless", action="store_true", help="Use GNOME's native headless backend (requires --wayland)")
    parser.add_argument("--approve-gnome", action="store_true", help="Click real GNOME 48 consent dialogs in this private headless desktop")
    args = parser.parse_args()
    assert not args.headless or args.wayland
    assert not args.approve_gnome or args.headless
    run(args.binary.resolve(), args.screenshot, args.capture_only, args.wayland, args.sharing, args.lab_env, args.headless, args.approve_gnome)
