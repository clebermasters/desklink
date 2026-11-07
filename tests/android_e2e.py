#!/usr/bin/env python3
"""End-to-end test of the Android receiver on an emulator or a test phone.

A headless controller (crates/android/examples/controller.rs) connects through `adb forward`,
so this computer's real keyboard and mouse are never captured. Both input modes are exercised:

* Accessibility: remote click focuses a text field, typing, Ctrl+A/Ctrl+V, wheel scrolling.
* ADB developer mode: the same through the shell-user helper, plus wrong-token rejection.
* Clipboard both ways (computer -> phone paste, phone copy -> computer).
* With --x11-agent: a real `desklink run` on a private Xvfb display hands its pointer to the
  phone across its right edge, types with captured X11 keys, and syncs the X11 clipboard both ways.
  Needs Xvfb, xdotool and xclip; nothing touches your own desktop session.

Usage:  tests/android_e2e.py [--serial emulator-5554] [--apk path/to/app-debug.apk] [--x11-agent]
Needs: adb, a booted device with nothing important on it (the test changes accessibility
settings and the app's pairing), and `cargo build -p desklink-android --example controller`.
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import xml.etree.ElementTree as ET

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PACKAGE = "com.desklink.android"
SERVICE = f"{PACKAGE}/{PACKAGE}.input.DesklinkAccessibilityService"
FORWARD_PORT = 24890


class Device:
    def __init__(self, adb, serial):
        self.base = [adb] + (["-s", serial] if serial else [])

    def run(self, *args, check=True):
        result = subprocess.run(self.base + list(args), capture_output=True, text=True)
        if check and result.returncode != 0:
            raise RuntimeError(f"adb {' '.join(args)} failed: {result.stderr or result.stdout}")
        return result.stdout

    def shell(self, command, check=True):
        return self.run("shell", command, check=check)

    def nodes(self) -> list:
        # uiautomator suspends accessibility services while it runs; see settle().
        self.shell("uiautomator dump /sdcard/desklink-ui.xml >/dev/null")
        xml = self.shell("cat /sdcard/desklink-ui.xml")
        return list(ET.fromstring(xml[xml.index("<"):]).iter("node"))

    def bounds(self, resource: str) -> tuple | None:
        for node in self.nodes():
            if node.get("resource-id") == f"{PACKAGE}:id/{resource}":
                return tuple(map(int, re.findall(r"\d+", node.get("bounds") or "")))
        return None

    def settle(self) -> None:
        """Give Android time to rebind accessibility services after a uiautomator dump."""
        time.sleep(4)

    def text(self, resource):
        for node in self.nodes():
            if node.get("resource-id") == f"{PACKAGE}:id/{resource}":
                return node.get("text")
        return None

    def tap(self, resource):
        x1, y1, x2, y2 = self.bounds(resource)
        self.shell(f"input tap {(x1 + x2) // 2} {(y1 + y2) // 2}")


class Controller:
    def __init__(self, identity, phone_code):
        self.binary = os.path.join(ROOT, "target", "debug", "examples", "controller")
        self.identity = identity
        self.phone_code = phone_code

    def pairing_code(self):
        return subprocess.check_output(
            [self.binary, "--identity", self.identity, "--print-code", "--address", "10.0.2.2:9"], text=True
        ).strip()

    def command(self, script: str) -> list:
        return [self.binary, "--identity", self.identity, "--phone-code", self.phone_code,
                "--address", f"127.0.0.1:{FORWARD_PORT}", "--script", script]

    def start(self, script: str) -> subprocess.Popen:
        return subprocess.Popen(self.command(script), stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)

    def run(self, script: str) -> str:
        command = self.command(script)
        result = subprocess.run(command, capture_output=True, text=True, timeout=90)
        if result.returncode != 0:
            raise AssertionError(f"controller failed:\n{result.stdout}\n{result.stderr}")
        return result.stdout


def check(condition, message):
    if not condition:
        raise AssertionError(message)
    print(f"PASS  {message}")


def wait_for(predicate, timeout=15, interval=0.5):
    deadline = time.time() + timeout
    while time.time() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(interval)
    return None


def scroll_to_top(device) -> None:
    for _ in range(4):
        device.shell("input swipe 540 700 540 2000 120")
    time.sleep(1.5)


def logical(bounds: tuple, scale: float) -> tuple:
    x1, y1, x2, y2 = bounds
    return round((x1 + x2) / 2 / scale), round((y1 + min(y2, y1 + 120)) / 2 / scale)


def type_into_test_field(device, controller, scale, text):
    scroll_to_top(device)
    time.sleep(1)
    bounds = device.bounds("test_input")
    if bounds is None:
        return None
    x, y = logical(bounds, scale)
    device.settle()
    controller.run(f"enter left 0.5; moveto {x} {y}; click; wait 1000; chord 29 30; key 14; "
                   f"type {text}; wait 600; leave left")
    return wait_for(lambda: device.text("test_input") == text and text, timeout=5)


def ensure_sharing(device):
    device.run("shell", "am", "start", "-n", f"{PACKAGE}/.MainActivity")
    time.sleep(1.5)
    scroll_to_top(device)
    if "Off" in (device.text("status") or "Off"):
        device.tap("toggle")
    check(wait_for(lambda: (device.text("status") or "").startswith("On")), "sharing service started")


def x11_agent_stage(device: Device, scale: float) -> None:
    """Interoperates with the real desktop agent. Runs last: it becomes the phone's controller."""
    binary = os.path.join(ROOT, "target", "debug", "desklink")
    # Unix control sockets need a short path, so the profile lives directly under /tmp.
    profile = tempfile.mkdtemp(prefix="dl.", dir="/tmp")
    config = os.path.join(profile, "config.toml")
    desklink = f"{binary} --config {config}"
    xvfb = subprocess.Popen(["Xvfb", "-displayfd", "1", "-screen", "0", "1280x800x24", "-nolisten", "tcp"],
                            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    display = ":" + xvfb.stdout.readline().strip()
    env = {**os.environ, "DISPLAY": display, "XDG_SESSION_TYPE": "x11"}
    agent = None
    try:
        subprocess.run(desklink.split() + ["init", "--name", "Xvfb desktop", "--listen", "0.0.0.0:24881"],
                       check=True, capture_output=True)
        paired = subprocess.run([os.path.join(ROOT, "scripts/android-pair.sh"), "--edge", "right",
                                 "--address", "10.0.2.2:24881"],
                                env={**os.environ, "ADB": " ".join(device.base), "DESKLINK": desklink},
                                capture_output=True, text=True)
        check(paired.returncode == 0, "android-pair.sh pairs a real desktop profile both ways")
        with open(config) as f:
            text = re.sub(r'address = "[^"]+:24880"', f'address = "127.0.0.1:{FORWARD_PORT}"', f.read())
        with open(config, "w") as f:
            f.write(text)
        log = open(os.path.join(profile, "agent.log"), "w")
        agent = subprocess.Popen(desklink.split() + ["run"], env=env, stdout=log, stderr=log)
        ensure_sharing(device)
        check(wait_for(lambda: "peer connected" in open(log.name).read(), timeout=30),
              "real desktop agent connects to the phone")

        def xdotool(*args):
            subprocess.run(["xdotool", *args], env=env, check=True)

        def visit(actions):
            bounds = device.bounds("test_input")
            x, y = logical(bounds, scale)
            device.settle()
            xdotool("mousemove", "1200", "400")
            for _ in range(20):
                xdotool("mousemove_relative", "--", "10", "0")
                time.sleep(0.02)
            time.sleep(0.5)
            dx, dy = x - 1, y - round(0.5 * 913)
            for _ in range(30):
                xdotool("mousemove_relative", "--", str(dx // 30), str(dy // 30))
                time.sleep(0.015)
            time.sleep(0.4)
            xdotool("click", "1")
            time.sleep(1)
            actions()
            for _ in range(60):
                xdotool("mousemove_relative", "--", "-20", "0")
                time.sleep(0.01)
            time.sleep(1)

        scroll_to_top(device)
        visit(lambda: (xdotool("key", "ctrl+a"), xdotool("type", "--delay", "60", "Real desktop"), time.sleep(1)))
        check(device.text("test_input") == "Real desktop", "real agent: X11 pointer and keys control the phone")
        subprocess.run(["xclip", "-selection", "clipboard"], input="copied on X11", text=True, env=env, check=True)
        time.sleep(1.5)
        visit(lambda: (xdotool("key", "ctrl+a"), xdotool("key", "ctrl+v"), time.sleep(1)))
        check(device.text("test_input") == "copied on X11", "real agent: X11 clipboard pastes on the phone")
        visit(lambda: (xdotool("key", "ctrl+a"), xdotool("type", "--delay", "50", "phone copy 42"),
                       xdotool("key", "ctrl+a"), xdotool("key", "ctrl+c"), time.sleep(2)))
        copied = subprocess.run(["xclip", "-selection", "clipboard", "-o"], env=env, capture_output=True, text=True)
        check(copied.stdout == "phone copy 42", "real agent: a phone copy reaches the X11 clipboard")
    finally:
        subprocess.run(desklink.split() + ["stop"], capture_output=True)
        if agent:
            agent.wait(timeout=10)
        xvfb.terminate()
        subprocess.run(["rm", "-rf", profile])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--serial")
    parser.add_argument("--adb", default=os.environ.get("ADB", "adb"))
    parser.add_argument("--apk", default=os.path.join(ROOT, "android/app/build/outputs/apk/debug/app-debug.apk"))
    parser.add_argument("--x11-agent", action="store_true", help="also test against a real desklink run on Xvfb")
    args = parser.parse_args()
    device = Device(args.adb, args.serial)
    work = tempfile.mkdtemp(prefix="desklink-android-e2e-")
    try:
        run_checks(args, device, work)
    finally:
        shutil.rmtree(work, ignore_errors=True)  # holds the test controller's private key


def run_checks(args: argparse.Namespace, device: Device, work: str) -> None:

    device.run("install", "-r", args.apk)
    device.shell(f"pm clear {PACKAGE}")  # hermetic: fresh identity and pairing every run
    device.shell(f"pm grant {PACKAGE} android.permission.POST_NOTIFICATIONS", check=False)
    controller = Controller(os.path.join(work, "controller.json"), "")
    paired = device.shell(f"content call --uri content://{PACKAGE}.shell --method pair "
                          f"--arg '{controller.pairing_code()}'")
    check("paired" in paired, "shell-only provider pairs the controller")
    raw = device.shell(f"content call --uri content://{PACKAGE}.shell --method pairing-code")
    match = re.search(r"result=(desklink://pair/[A-Za-z0-9_-]+)", raw)
    check(match is not None, "shell-only provider prints the phone's pairing code")
    controller.phone_code = match.group(1) if match else ""
    device.run("forward", f"tcp:{FORWARD_PORT}", "tcp:24880")
    density = int(re.findall(r"\d+", device.shell("wm density"))[-1])
    scale = density / 160

    # Accessibility mode.
    device.shell(f"settings put secure enabled_accessibility_services {SERVICE}")
    device.shell("settings put secure accessibility_enabled 1")
    ensure_sharing(device)
    scroll_to_top(device)
    device.tap("mode_accessibility")
    check(type_into_test_field(device, controller, scale, "Hello from accessibility"),
          "accessibility: remote click focuses a field and typing arrives with capitals")
    device.settle()
    controller.run("enter left 0.5; copy pasted from the computer; wait 800; chord 29 30; chord 29 47; "
                   "wait 800; leave left")
    check(wait_for(lambda: device.text("test_input") == "pasted from the computer", timeout=5),
          "clipboard: a computer copy pastes on the phone with Ctrl+V")
    before = device.bounds("toggle")
    device.settle()
    controller.run("enter left 0.5; moveto 200 260; scroll -12; wait 1500; leave left")
    after = device.bounds("toggle")
    check(after is None or (before is not None and after[1] < before[1]), "accessibility: the wheel scrolls the page")

    scroll_to_top(device)
    time.sleep(1)
    x, y = logical(device.bounds("test_input"), scale)
    device.settle()
    controller.run(f"enter left 0.5; moveto {x} {y}; click; wait 1000; chord 29 30; key 14; type copied on the phone; "
                   "chord 29 30; chord 29 46; expect-copy copied on the phone; leave left")
    check(True, "clipboard: a copy made on the phone (Ctrl+C) reaches the computer")

    # ADB developer mode, with accessibility off so only the shell helper can inject.
    device.shell("settings put secure enabled_accessibility_services '\"\"'")
    device.shell("settings put secure accessibility_enabled 0")
    device.shell(f"appops set {PACKAGE} SYSTEM_ALERT_WINDOW allow")
    helper = subprocess.run([os.path.join(ROOT, "scripts/android-input-helper.sh")],
                            env={**os.environ, "ADB": " ".join(device.base)}, capture_output=True, text=True)
    check(helper.returncode == 0, "ADB helper starts as the shell user")
    check("shell" in device.shell("ps -A -o USER,ARGS | grep InputHelper | grep -v grep"),
          "ADB helper runs without root")
    scroll_to_top(device)
    device.tap("mode_shell")
    check(wait_for(lambda: "app connected" in device.shell("cat /data/local/tmp/desklink-helper.log"), timeout=15),
          "app authenticates to the helper")
    check(type_into_test_field(device, controller, scale, "Hello from ADB mode"),
          "ADB mode: injected mouse click and keyboard typing")
    port = re.findall(r"127\.0\.0\.1:(\d+)", device.shell("cat /data/local/tmp/desklink-helper.log"))[-1]
    reply = device.shell(f"printf '%032d' 0 | nc -w 2 127.0.0.1 {port} | od -An -tx1; true")
    time.sleep(1)
    check(reply.strip() == "" and "wrong token" in device.shell("cat /data/local/tmp/desklink-helper.log"),
          "helper rejects a client without the token")

    subprocess.run([os.path.join(ROOT, "scripts/android-input-helper.sh"), "--stop"],
                   env={**os.environ, "ADB": " ".join(device.base)}, capture_output=True)
    if args.x11_agent:
        scroll_to_top(device)
        device.tap("mode_accessibility")
        device.shell(f"settings put secure enabled_accessibility_services {SERVICE}")
        device.shell("settings put secure accessibility_enabled 1")
        x11_agent_stage(device, scale)
    device.run("forward", "--remove", f"tcp:{FORWARD_PORT}", check=False)
    print("All Android end-to-end checks passed.")


if __name__ == "__main__":
    try:
        main()
    except AssertionError as error:
        print(f"FAIL  {error}", file=sys.stderr)
        sys.exit(1)
