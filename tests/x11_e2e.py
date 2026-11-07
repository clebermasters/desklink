#!/usr/bin/env python3
"""Real two-desktop integration test. Every input operation targets a private Xvfb display.

Dependencies: Xvfb, xdotool, xclip, xev, Python Pillow. No root access at test runtime.
Usage: cargo build --locked && python3 tests/x11_e2e.py
"""
import argparse
import io
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import tempfile
import time
from PIL import Image


def wait_for(check, label, timeout=12):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            value = check()
            if value:
                return value
        except (OSError, ValueError, subprocess.SubprocessError, AssertionError) as error:
            last = error
        time.sleep(0.08)
    raise AssertionError(f"Timed out: {label}; last error: {last}")


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def run(binary):
    for tool in ("Xvfb", "xdotool", "xclip", "xev", "stdbuf"):
        if not shutil.which(tool):
            raise SystemExit(f"Missing test dependency: {tool}")
    processes, files = [], []
    with tempfile.TemporaryDirectory(prefix="desklink-e2e-") as temp:
        root = Path(temp)

        def spawn(args, env=None, log=None, **kwargs):
            if log:
                handle = open(log, "ab", buffering=0)
                files.append(handle)
                kwargs.update(stdout=handle, stderr=handle)
            proc = subprocess.Popen(args, env=env, **kwargs)
            processes.append(proc)
            return proc

        def display(width, height):
            proc = spawn(["Xvfb", "-displayfd", "1", "-screen", "0", f"{width}x{height}x24", "-nolisten", "tcp", "-ac"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            number = proc.stdout.readline().decode().strip()
            assert number.isdigit(), "Xvfb did not provide a display number"
            env = dict(os.environ, DISPLAY=f":{number}", XDG_SESSION_TYPE="x11", RUST_LOG="desklink=debug,desklink_transport=debug,desklink_clipboard=debug")
            env.pop("WAYLAND_DISPLAY", None)
            wait_for(lambda: subprocess.run(["xdotool", "getmouselocation"], env=env, capture_output=True).returncode == 0, "Xvfb startup")
            return env

        def cli(config, *args, env=None):
            return subprocess.check_output([str(binary), "--config", str(config), *args], env=env, stderr=subprocess.PIPE).decode().strip()

        def mouse(env, *args):
            return subprocess.check_output(["xdotool", *map(str, args)], env=env, stderr=subprocess.PIPE).decode()

        def position(env):
            data = mouse(env, "getmouselocation", "--shell")
            return {key: int(value) for key, value in (line.split("=", 1) for line in data.splitlines())}

        def copy(env, data, kind="UTF8_STRING"):
            subprocess.run(["xclip", "-selection", "clipboard", "-t", kind, "-i"], input=data, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True, timeout=3)

        def paste(env, kind="UTF8_STRING"):
            return subprocess.check_output(["xclip", "-selection", "clipboard", "-t", kind, "-o"], env=env, stderr=subprocess.DEVNULL, timeout=2)

        def png(color):
            output = io.BytesIO()
            Image.new("RGBA", (19, 13), color).save(output, format="PNG")
            return output.getvalue()

        def image_matches(env, expected):
            actual = Image.open(io.BytesIO(paste(env, "image/png"))).convert("RGBA")
            return actual.size == (19, 13) and actual.tobytes() == Image.open(io.BytesIO(expected)).convert("RGBA").tobytes()

        def log_text(path):
            return path.read_text(errors="replace") if path.exists() else ""

        try:
            a, b = display(800, 600), display(900, 700)
            ca, cb = root / "a/config.toml", root / "b/config.toml"
            pa, pb = free_port(), free_port()
            cli(ca, "init", "--name", "Desktop A", "--listen", f"127.0.0.1:{pa}")
            cli(cb, "init", "--name", "Desktop B", "--listen", f"127.0.0.1:{pb}")
            ka, kb = cli(ca, "identity"), cli(cb, "identity")
            cli(ca, "trust", kb, "--name", "B", "--edge", "right", "--address", f"127.0.0.1:{pb}")
            cli(cb, "trust", ka, "--name", "A", "--edge", "left", "--address", f"127.0.0.1:{pa}")
            for label, env in (("A", a), ("B", b)):
                doctor = json.loads(cli(ca if label == "A" else cb, "doctor", env=env))
                assert doctor["platform"]["capture"] and doctor["platform"]["inject"]
                spawn(["stdbuf", "-oL", "xev", "-name", f"Desklink-E2E-{label}", "-geometry", "780x580+0+0"], env=env, log=root / f"events-{label}.log")
                wid = wait_for(lambda: mouse(env, "search", "--name", f"Desklink-E2E-{label}").splitlines()[0], "event window")
                mouse(env, "windowfocus", "--sync", wid)
            la, lb = root / "a.log", root / "b.log"
            ap = spawn([str(binary), "--config", str(ca), "run"], env=a, log=la)
            bp = spawn([str(binary), "--config", str(cb), "run"], env=b, log=lb)
            wait_for(lambda: "peer connected" in log_text(la) and "peer connected" in log_text(lb), "mutual authenticated connection")
            print("PASS: real processes paired and connected over encrypted transport", flush=True)

            for source, destination, text in ((a, b, "Desklink A → B 🧪\nsecond line"), (b, a, "Desklink B → A ✓")):
                copy(source, text.encode())
                wait_for(lambda: paste(destination) == text.encode(), "bidirectional Unicode clipboard")
            print("PASS: Unicode text clipboard A→B and B→A", flush=True)
            for source, destination, color in ((a, b, (220, 10, 30, 128)), (b, a, (0, 150, 210, 240))):
                data = png(color)
                copy(source, data, "image/png")
                wait_for(lambda: image_matches(destination, data), "bidirectional PNG clipboard")
            print("PASS: PNG image clipboard A→B and B→A retains pixels and alpha", flush=True)

            for attempt in range(3):
                choices = [f"concurrent A {attempt}: ".encode() + b"a" * 60000,
                           f"concurrent B {attempt}: ".encode() + b"b" * 60000]
                copy(a, choices[0])
                copy(b, choices[1])
                def converged():
                    left, right = paste(a), paste(b)
                    return left == right and left in choices
                wait_for(converged, "concurrent clipboard copies converge")
                time.sleep(0.6)
                assert converged(), "clipboard reverted after concurrent copies converged"
            print("PASS: concurrent clipboard copies converge without stale-read reverts", flush=True)

            mouse(a, "mousemove", "400", "300")
            time.sleep(0.2)
            mouse(a, "mousemove", "799", "300")
            wait_for(lambda: "receiving input" in log_text(lb), "screen-edge handoff A→B")
            mouse(a, "mousemove_relative", "--", "50", "20")
            wait_for(lambda: position(b)["X"] > 30, "remote pointer motion")
            mouse(a, "key", "a")
            wait_for(lambda: "keysym 0x61, a" in log_text(root / "events-B.log"), "remote key injection")
            mouse(a, "click", "1")
            wait_for(lambda: "ButtonPress event" in log_text(root / "events-B.log"), "remote click")
            mouse(a, "click", "4")
            mouse(a, "click", "7")
            wait_for(lambda: "button 4" in log_text(root / "events-B.log") and "button 7" in log_text(root / "events-B.log"), "vertical and horizontal scrolling")
            mouse(a, "mousemove_relative", "--", "-3000", "0")
            wait_for(lambda: position(a)["X"] >= 795, "return from remote screen")
            print("PASS: pointer, key and button sharing A→B; edge return releases capture", flush=True)

            time.sleep(0.35)
            mouse(b, "mousemove", "450", "300")
            time.sleep(0.15)
            mouse(b, "mousemove", "0", "300")
            wait_for(lambda: "receiving input" in log_text(la), "screen-edge handoff B→A")
            mouse(b, "mousemove_relative", "--", "-70", "0")
            wait_for(lambda: position(a)["X"] < 780, "reverse pointer motion")
            mouse(b, "key", "b")
            wait_for(lambda: "keysym 0x62, b" in log_text(root / "events-A.log"), "reverse keyboard injection")
            mouse(b, "mousemove_relative", "--", "3000", "0")
            wait_for(lambda: position(b)["X"] <= 4, "reverse edge return")
            print("PASS: bidirectional input and normalized coordinates across different display sizes", flush=True)

            time.sleep(0.35)
            mouse(a, "mousemove", "400", "220")
            time.sleep(0.1)
            mouse(a, "mousemove", "799", "220")
            wait_for(lambda: position(a)["X"] == 400, "capture before emergency return")
            mouse(a, "key", "ctrl+alt+shift+Escape")
            mouse(a, "mousemove", "150", "100")
            wait_for(lambda: position(a)["X"] == 150, "emergency chord releases the real pointer grab")
            print("PASS: emergency chord restores local control", flush=True)

            time.sleep(0.35)
            mouse(a, "mousemove", "400", "250")
            time.sleep(0.1)
            mouse(a, "mousemove", "799", "250")
            wait_for(lambda: position(a)["X"] == 400, "capture before loss test")
            mouse(a, "keydown", "Control_L")
            mouse(a, "mousedown", "1")
            wait_for(lambda: "keysym 0xffe3, Control_L" in log_text(root / "events-B.log"), "held remote modifier")
            before = log_text(root / "events-B.log").count("KeyRelease event")
            ap.kill()
            ap.wait(timeout=4)
            wait_for(lambda: "peer disconnected; input released" in log_text(lb), "peer loss detection")
            wait_for(lambda: log_text(root / "events-B.log").count("KeyRelease event") > before, "modifier released after peer crash")
            assert "ButtonRelease event" in log_text(root / "events-B.log")
            connected_before = log_text(lb).count("peer connected")
            ap = spawn([str(binary), "--config", str(ca), "run"], env=a, log=la)
            wait_for(lambda: log_text(lb).count("peer connected") > connected_before, "automatic reconnect")
            assert bp.poll() is None
            print("PASS: peer crash releases held input; identity-preserving restart reconnects", flush=True)

            for proc in (ap, bp):
                proc.terminate()
                assert proc.wait(timeout=4) == 0
            copy(b, b"clipboard stays local")
            count_a, count_b = log_text(la).count("peer connected"), log_text(lb).count("peer connected")
            spawn([str(binary), "--config", str(ca), "run", "--no-clipboard"], env=a, log=la)
            spawn([str(binary), "--config", str(cb), "run"], env=b, log=lb)
            wait_for(lambda: log_text(la).count("peer connected") > count_a and log_text(lb).count("peer connected") > count_b, "clipboard-disabled connection")
            copy(a, b"this must not be transmitted")
            time.sleep(1.2)
            assert paste(b) == b"clipboard stays local"
            assert paste(a) == b"this must not be transmitted"
            print("PASS: --no-clipboard prevents transfer in either direction", flush=True)
            print("ALL X11 END-TO-END CHECKS PASSED", flush=True)
        except Exception:
            for path in root.glob("*.log"):
                print(f"\n--- {path.name} ---\n{log_text(path)[-12000:]}")
            raise
        finally:
            for proc in reversed(processes):
                if proc.poll() is None:
                    proc.send_signal(signal.SIGTERM)
            for proc in reversed(processes):
                try:
                    proc.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(timeout=3)
            for handle in files:
                handle.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path(__file__).resolve().parents[1] / "target/debug/desklink")
    run(parser.parse_args().binary.resolve())
