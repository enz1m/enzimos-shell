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
assert config["exit_with_shell"] and ready["runtime"] == str(RUNTIME)
assert len(json.loads((RUNTIME / "ready.json").read_text())["display"]) > 0
unit = "enzui-restore-motion-qa-" + secrets.token_hex(5) + ".service"
env = {**os.environ, "XDG_RUNTIME_DIR": config["xdg_runtime"],
       "LD_LIBRARY_PATH": config["build"] + "/prefix/lib"}
ctl = [config["build"] + "/hyprland-current-build/hyprctl/hyprctl", "-i", ready["instance"]]
artifacts = ROOT / ".build/drag-restore" / ("qa-" + RUNTIME.name)
artifacts.mkdir(parents=True, exist_ok=True)

def run(*args, environment=None):
    return subprocess.check_output(args, env=environment, text=True, timeout=15).strip()

def hypr(*args):
    return run(*ctl, *args, environment=env)

def app():
    matches = [w for w in json.loads(hypr("-j", "clients")) if w["class"] == "dev.enzui.Settings"]
    assert len(matches) <= 1, "multiple Settings windows in QA runtime"
    return matches[0] if matches else None

def pointer(process, command):
    process.stdin.write(command + "\n")
    process.stdin.flush()
    with selectors.DefaultSelector() as selector:
        selector.register(process.stdout, selectors.EVENT_READ)
        assert selector.select(timeout=3), "virtual pointer timed out: " + command
    assert process.stdout.readline().strip() == "ok", "virtual pointer rejected: " + command

def shot(label):
    path = artifacts / (label + ".png")
    run("grim", str(path), environment={**env, "WAYLAND_DISPLAY": ready["display"]})
    return path

def settings_span(path):
    image = Image.open(path).convert("RGB")
    assert image.size == (1440, 900)
    visible = []
    for x in range(20, 1420):
        red, green, blue = image.getpixel((x, 500))
        if red < 70 and abs(red - green) < 8 and abs(green - blue) < 10:
            visible.append(x)
    assert visible, "Settings not visible in screenshot: " + str(path)
    return max(visible) - min(visible) + 1

def blue_preview(path):
    image = Image.open(path).convert("RGB")
    return any((lambda red, green, blue: blue > 145 and blue - red > 110 and green > 90)(*image.getpixel((x, 400)))
               for x in range(24))

assert not app(), "QA runtime already has a Settings window"
monitor = next(w for w in json.loads(hypr("-j", "monitors")) if w["name"] == ready["output"])
assert (monitor["width"], monitor["height"], monitor["scale"]) == (1440, 900, 1), "QA virtual pointer needs a 1440x900 output"
settings = ROOT / "target/debug/enzui-settings"
preview = ROOT / "target/debug/enzui-maximize-preview"
assert settings.is_file() and preview.is_file(), "build the pinned Settings and preview binaries first"
process = None
service_started = False
binding_installed = False
cgroup = None
try:
    command = "exec " + json.dumps(str(preview)) + " " + json.dumps(ready["output"])
    lua = "\n" + (ROOT / "config/drag-maximize.lua").read_text() + "\nassert(enzui_drag_maximize_install(" + json.dumps(command) + "))"
    assert hypr("eval", lua) == "ok", "could not install maximize-only gesture"
    assert hypr("eval", "hl.config({binds={drag_threshold=6}})") == "ok"
    run("systemd-run", "--user", "--collect", "--unit=" + unit,
        "-p", "KillMode=control-group", "-p", "MemoryMax=400M", "-p", "MemorySwapMax=0",
        "/usr/bin/env", "-i", "HOME=" + str(Path.home()), "PATH=/usr/bin:/bin",
        "XDG_RUNTIME_DIR=" + config["xdg_runtime"], "WAYLAND_DISPLAY=" + ready["display"],
        "LD_LIBRARY_PATH=" + env["LD_LIBRARY_PATH"], "LANG=C.UTF-8",
        "ENZUI_SETTINGS_WIDTH=1024", "ENZUI_SETTINGS_HEIGHT=768", "ENZUI_SETTINGS_PAGE=shell",
        "ENZUI_DESKTOP_ROOT=" + str(ROOT), "ENZUI_SHELL_SETTINGS=" + config["settings"],
        str(settings))
    service_started = True
    cgroup = run("systemctl", "--user", "show", unit, "-p", "ControlGroup", "--value")
    assert cgroup.endswith("/" + unit), "unexpected QA service cgroup"
    for _ in range(100):
        if app(): break
        time.sleep(.1)
    assert app(), "QA Settings did not open"
    time.sleep(.5)
    initial = app()
    assert initial["size"] == [1024, 768] and initial["fullscreen"] == 0
    assert hypr("eval", 'hl.unbind("mouse:272");hl.bind("mouse:272",function() return hl.dispatch(hl.dsp.window.drag()) end,{mouse=true})') == "ok"
    binding_installed = True
    process = subprocess.Popen([str(ROOT / ".build/drag-maximize-qa/virtual-pointer-qa")],
        env={**env, "WAYLAND_DISPLAY": ready["display"]}, stdin=subprocess.PIPE,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
    pointer(process, "m 950 105")
    time.sleep(.05)
    pointer(process, "b 1")
    time.sleep(.08)
    pointer(process, "m 960 110")
    time.sleep(.12)
    pointer(process, "m 970 44")
    time.sleep(.12)
    assert app()["at"] != initial["at"], "virtual input did not really drag Settings"
    assert app()["fullscreen"] == 0
    assert blue_preview(shot("maximize-preview")), "maximize-only border missing while dragging up"
    pointer(process, "b 0")
    time.sleep(.4)
    assert app()["fullscreen"] == 1, "upward drop did not maximize"
    pointer(process, "m 950 55")
    time.sleep(.04)
    pointer(process, "b 1")
    time.sleep(.04)
    pointer(process, "m 960 60")
    pointer(process, "m 970 72")
    early = shot("restore-early")
    pointer(process, "m 990 170")
    middle = shot("restore-middle")
    time.sleep(.1)
    assert app()["fullscreen"] == 0, "real drag did not restore maximized Settings"
    assert app()["size"] == initial["size"], "restore lost original size"
    pointer(process, "b 0")
    time.sleep(.3)
    final = shot("restore-final")
    goal = settings_span(final)
    first = settings_span(early)
    next_width = settings_span(middle)
    print("real native drag widths (first, next, settled):", first, next_width, goal)
    assert not blue_preview(early) and not blue_preview(middle), "restore painted a blue overlay"
    if os.environ.get("ENZUI_EXPECT_SMOOTH", "1") == "1":
        assert first >= goal + 200, "restore snapped on its first frame"
        assert next_width <= first - 40, "restore did not shrink through intermediate frames"
    else:
        assert first <= goal + 70, "baseline did not snap: comparison fixture changed"
    pointer(process, "m 950 164")
    pointer(process, "b 1")
    pointer(process, "b 0")
    time.sleep(.08)
    assert app()["fullscreen"] == 0, "plain click unexpectedly maximized"
    print("real drag: maximizing preview, uncovered animated restore and click OK")
finally:
    if process:
        process.stdin.close()
        try: process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.wait(timeout=3)
    if binding_installed:
        assert hypr("eval", 'hl.unbind("mouse:272")') == "ok", "temporary QA drag binding remained installed"
    if service_started:
        run("systemctl", "--user", "stop", unit)
    if cgroup:
        path = Path("/sys/fs/cgroup") / cgroup.lstrip("/")
        for _ in range(60):
            if not path.exists() or "populated 0" in (path / "cgroup.events").read_text(): break
            time.sleep(.1)
        else: raise RuntimeError("QA Settings service cgroup still populated")
    assert not app(), "QA Settings still mapped after cleanup"
    print("owned QA Settings service stopped; cgroup empty")
