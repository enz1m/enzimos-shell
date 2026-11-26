#!/usr/bin/env python3
import json
import os
from pathlib import Path
import secrets
import selectors
import subprocess
import time
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
RUNTIME = Path(os.environ["ENZUI_QA_RUNTIME"])
config = json.loads((RUNTIME / "launch.json").read_text())
ready = json.loads((RUNTIME / "ready.json").read_text())
unit = "enzui-gesture-qa-" + secrets.token_hex(5) + ".service"
env = {**os.environ, "XDG_RUNTIME_DIR": config["xdg_runtime"],
       "LD_LIBRARY_PATH": config["build"] + "/prefix/lib"}
control = Path(config["build"]) / "hyprland-current-build/hyprctl/enz-hyprctl"
if not control.is_file():
    control = control.with_name("hyprctl")
ctl = [str(control), "-i", ready["instance"]]

def run(*args, environment=None):
    return subprocess.check_output(args, env=environment, text=True, timeout=12).strip()

def hypr(*args):
    return run(*ctl, *args, environment=env)

def client():
    clients = json.loads(hypr("-j", "clients"))
    windows = [w for w in clients if w["class"] == "enzui-gesture-qa"]
    return windows[0] if windows else None

def state():
    return hypr("--batch", "repl return enzui_drag_maximize.armed, enzui_drag_maximize.preview, enzui_drag_maximize.morph ~= nil")

def check(predicate, explanation):
    if not predicate:
        raise AssertionError(explanation + ": " + str(client()) + " state=" + state())

def pointer(process, command):
    process.stdin.write(command + "\n")
    process.stdin.flush()
    with selectors.DefaultSelector() as selector:
        selector.register(process.stdout, selectors.EVENT_READ)
        check(bool(selector.select(timeout=3)), "virtual pointer timed out on " + command)
    check(process.stdout.readline().strip() == "ok", "virtual pointer rejected " + command)

def shot(label):
    path = ROOT / ".build" / "drag-maximize-qa" / (label + ".png")
    run("grim", str(path), environment={**env, "WAYLAND_DISPLAY": ready["display"]})
    return path

def visible_border(path):
    image = Image.open(path).convert("RGB")
    for x in range(24):
        red, green, blue = image.getpixel((x, 400))
        if blue > 145 and blue - red > 110 and green > 90:
            return True
    return False

process = None
cgroup = None
service_started = False
try:
    run("systemd-run", "--user", "--collect", "--unit=" + unit,
        "-p", "KillMode=control-group", "-p", "MemoryMax=256M", "-p", "MemorySwapMax=0",
        "/usr/bin/env", "-i", "HOME=" + str(Path.home()), "PATH=/usr/bin:/bin",
        "XDG_RUNTIME_DIR=" + config["xdg_runtime"], "WAYLAND_DISPLAY=" + ready["display"],
        "LANG=C.UTF-8",
        "XDG_CURRENT_DESKTOP=Hyprland", "/usr/bin/foot", "-a", "enzui-gesture-qa",
        "-T", "Gesture QA", "-w", "800x560", "/usr/bin/sleep", "120")
    service_started = True
    cgroup = run("systemctl", "--user", "show", unit, "-p", "ControlGroup", "--value")
    for _ in range(100):
        if client(): break
        time.sleep(.1)
    check(client() is not None, "QA window did not open")
    process = subprocess.Popen([str(ROOT / ".build/drag-maximize-qa/virtual-pointer-qa")],
        env={**env, "WAYLAND_DISPLAY": ready["display"]},
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        text=True, bufsize=1)
    time.sleep(.15)
    start = client()
    sx, sy = start["at"]
    pointer(process, f"m {sx + 120} {sy + 110}")
    pointer(process, "b 1")
    # The compositor's move dispatcher starts a drag while the pointer is held.
    check(hypr("eval", "hl.dispatch(hl.dsp.window.drag())") == "ok", "move dispatch rejected")
    pointer(process, "m 380 44")
    time.sleep(.18)
    check("true" in state().lower(), "first trip did not arm")
    check(client()["fullscreen"] == 0, "window maximized before release")
    check(visible_border(shot("first-preview")), "first preview border was not rendered")
    pointer(process, "b 0")
    time.sleep(.35)
    check(client()["fullscreen"] == 1, "first drop did not maximize")
    pointer(process, "m 400 140")
    pointer(process, "b 1")
    check(hypr("eval", "hl.dispatch(hl.dsp.window.drag())") == "ok", "restore drag rejected")
    pointer(process, "m 420 180")
    time.sleep(.07)
    restore = client()
    check(restore["fullscreen"] == 0, "restore drag did not unmaximize")
    check(restore["size"] == start["size"], "restored size differs from original")
    restore_image = Image.open(shot("restore-motion")).convert("RGB")
    middle = (restore["at"][0] + restore["size"][0] // 2,
              restore["at"][1] + restore["size"][1] // 2)
    pixel = restore_image.getpixel(middle)
    check(max(pixel) - min(pixel) < 20,
          "restore still painted a blue cover over the terminal")
    pointer(process, "b 0")
    time.sleep(.1)
    pointer(process, "m 460 230")
    pointer(process, "b 1")
    check(hypr("eval", "hl.dispatch(hl.dsp.window.drag())") == "ok", "second move rejected")
    pointer(process, "m 490 43")
    time.sleep(.13)
    check("true" in state().lower(), "second trip did not arm")
    check(visible_border(shot("second-preview")), "second preview border was not rendered")
    pointer(process, "b 0")
    time.sleep(.3)
    check(client()["fullscreen"] == 1, "second drop did not maximize")
    pointer(process, "m 400 140")
    pointer(process, "b 1")
    time.sleep(.05)
    pointer(process, "b 0")
    check(client()["fullscreen"] == 1, "plain click unmaximized the window")
    pointer(process, "b 1")
    check(hypr("eval", "hl.dispatch(hl.dsp.window.drag())") == "ok", "fast reverse drag rejected")
    pointer(process, "m 530 200")
    time.sleep(.03)
    check(client()["fullscreen"] == 0, "fast reverse did not restore")
    pointer(process, "m 530 43")
    pointer(process, "b 0")
    time.sleep(.25)
    check(client()["fullscreen"] == 1, "dropping in the band during collapse did not re-maximize")
    pointer(process, "m 420 180")
    pointer(process, "b 1")
    check(hypr("eval", "hl.dispatch(hl.dsp.window.drag())") == "ok", "cancel drag rejected")
    pointer(process, "m 540 230")
    time.sleep(.23)
    pointer(process, "m 540 42")
    time.sleep(.06)
    pointer(process, "m 540 210")
    pointer(process, "b 0")
    time.sleep(.14)
    check(client()["fullscreen"] == 0, "leaving the band did not cancel the drop")
    print("live drag-maximize: maximize-only preview, uncovered restore, quick reverse, cancel, click OK")
finally:
    if process:
        process.stdin.close()
        try: process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.wait(timeout=3)
    if service_started:
        run("systemctl", "--user", "stop", unit)
    if cgroup:
        path = Path("/sys/fs/cgroup") / cgroup.lstrip("/")
        for _ in range(60):
            if not path.exists() or (path / "cgroup.events").read_text().splitlines()[0] == "populated 0": break
            time.sleep(.1)
        else: raise RuntimeError("owned QA service still has processes")
    print("owned QA service stopped; cgroup empty")
