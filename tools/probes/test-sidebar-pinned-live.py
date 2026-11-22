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
unit = "enzui-sidebar-pinned-qa-" + secrets.token_hex(5) + ".service"
env = {**os.environ, "XDG_RUNTIME_DIR": config["xdg_runtime"],
       "WAYLAND_DISPLAY": ready["display"]}
env.pop("LD_LIBRARY_PATH", None)
ctl_env = {**env, "LD_LIBRARY_PATH": config["build"] + "/prefix/lib"}
ctl = [config["build"] + "/hyprland-current-build/hyprctl/hyprctl", "-i", ready["instance"]]
artifacts = ROOT / ".build/sidebar-pinned-qa" / RUNTIME.name
artifacts.mkdir(parents=True, exist_ok=True)

def run(*args, environment=None):
    return subprocess.check_output(args, env=environment, text=True, timeout=15).strip()

def hypr(*args):
    return run(*ctl, *args, environment=ctl_env)

def window():
    matches = [w for w in json.loads(hypr("-j", "clients")) if w["class"] == "enzui-sidebar-pinned-qa"]
    assert len(matches) <= 1
    return matches[0] if matches else None

def lua(expression):
    result = hypr("eval", "\n" + expression)
    assert result == "ok", result

def state():
    output = hypr("--batch", "repl return enzui_sidebar_snapshot()")
    return json.loads(output)

def phase():
    return hypr("--batch", "repl return hl.plugin.enzui_sidebar_scene.status(enzui_sidebar.entries[1].window.address)")

def client_resize_disabled(address=None):
    address = address or hypr("--batch", "repl return enzui_sidebar.entries[1].window.address")
    return hypr("--batch", "repl return hl.plugin.enzui_sidebar_scene.client_resize_disabled(" + json.dumps(address) + ")") == "true"

def wait(predicate, explanation):
    for _ in range(100):
        if predicate(): return
        time.sleep(.08)
    backing = [(w["at"], w["size"]) for w in json.loads(hypr("-j", "clients")) if w["class"] == "dev.enzui.SidebarBacking"]
    failure = hypr("--batch", "repl return hl.plugin.enzui_sidebar_scene.pocket_state(enzui_sidebar.owner).failure")
    raise AssertionError(explanation + ": " + str(window()) + " sidebar=" + str(state()) + " backing=" + str(backing) + " native=" + failure)

def pointer(process, command):
    process.stdin.write(command + "\n")
    process.stdin.flush()
    with selectors.DefaultSelector() as selector:
        selector.register(process.stdout, selectors.EVENT_READ)
        assert selector.select(timeout=3), "virtual pointer timed out: " + command
    assert process.stdout.readline().strip() == "ok", command

def shot(label):
    path = artifacts / (label + ".png")
    run("grim", str(path), environment=env)
    return path

assert not window() and not state()["entries"], "QA session already has a pinned app"
assert state()["native"] and state()["scene_version"] == 8
assert (ROOT / ".build/drag-maximize-qa/virtual-pointer-qa").is_file(), "build the QA pointer using scripts/test-drag-maximize"
service_started = False
cgroup = None
process = None
try:
    run("systemd-run", "--user", "--collect", "--unit=" + unit,
        "-p", "KillMode=control-group", "-p", "MemoryMax=256M", "-p", "MemorySwapMax=0",
        "/usr/bin/env", "-i", "HOME=" + str(Path.home()), "PATH=/usr/bin:/bin",
        "XDG_RUNTIME_DIR=" + config["xdg_runtime"], "WAYLAND_DISPLAY=" + ready["display"],
        "LANG=C.UTF-8", "XDG_CURRENT_DESKTOP=Hyprland", "/usr/bin/foot",
        "-a", "enzui-sidebar-pinned-qa", "-T", "Sidebar Pin QA", "-w", "740x490", "/usr/bin/sleep", "120")
    service_started = True
    cgroup = run("systemctl", "--user", "show", unit, "-p", "ControlGroup", "--value")
    assert cgroup.endswith("/" + unit)
    wait(lambda: window() is not None, "QA app not mapped")
    time.sleep(.35)
    original = window()
    assert original["size"] == [740, 490]
    lua('local w=hl.get_active_window();assert(w and w.class=="enzui-sidebar-pinned-qa");assert(enzui_sidebar_pin_window(w).ok);assert(enzui_sidebar_toggle(enzui_sidebar.entries[1].id).ok)')
    wait(lambda: phase() == "open", "Pinned app failed to open")
    initial = window()
    assert initial["size"] == [360, 640] and initial["pinned"]
    assert client_resize_disabled(), "Pinned app still accepts client-owned resize drags"
    x, y = initial["at"]
    image = Image.open(shot("initial-tab")).convert("RGB")
    grip = image.getpixel((x + 352, y + 319))
    assert all(channel > 185 for channel in grip), "Middle-right drag tab did not render"
    process = subprocess.Popen([str(ROOT / ".build/drag-maximize-qa/virtual-pointer-qa")], env=env,
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
    resize_on_right = state()["side"] == "right"
    resize_x = x + initial["size"][0] - 16
    resize_y = y + initial["size"][1] - 16
    indicator_x = x + initial["size"][0] - (22 if resize_on_right else 12)
    indicator_y = y + initial["size"][1] - 12
    indicator = image.getpixel((indicator_x, indicator_y))
    assert indicator[2] > 165 and indicator[2] - indicator[0] > 70, f"Bottom-corner accent border did not render: {indicator}"
    center = image.getpixel((resize_x, resize_y))
    assert center[2] - center[0] < 70, f"Resize grip still paints an icon or solid square: {center}"
    pointer(process, f"m {resize_x} {resize_y}")
    time.sleep(.05)
    shot("resize-hover")
    pointer(process, "b 1")
    pointer(process, "b 0")
    time.sleep(.08)
    assert phase() == "open" and window()["size"] == initial["size"], f"Clicking the resize grip changed the app: phase={phase()} window={window()} sidebar={state()}"
    pointer(process, "b 1")
    pointer(process, f"m {x + (165 if resize_on_right else 515)} {resize_y - 35}")
    time.sleep(.08)
    pointer(process, f"m {x + (120 if resize_on_right else 565)} {resize_y - 60}")
    time.sleep(.06)
    pointer(process, f"m {x + (-45 if resize_on_right else 715)} {resize_y - 75}")
    wait(lambda: phase() == "dragging" and window()["size"][0] > 512 and window()["size"][1] >= 100, "Pinned resize never crossed the previous 512-pixel limit")
    if not os.environ.get("ENZUI_QA_FAST_RELEASE"):
        time.sleep(.12)
    grown = window()["size"]
    if os.environ.get("ENZUI_QA_CAPTURE_RELEASE"):
        ring = (window()["at"][0] - 4, window()["at"][1] + 200)
        held_image = Image.open(shot("resize-held")).convert("RGB")
        held_color = held_image.getpixel(ring)
        if os.environ.get("ENZUI_QA_ASSERT_BACKGROUND"):
            assert held_color[2] - held_color[0] < 14, f"Backing layer vanished while held: {held_color}"
            below = (window()["at"][0] + 40, window()["at"][1] + grown[1] + 10)
            if below[1] < held_image.height:
                color = held_image.getpixel(below)
                assert color[2] - color[0] > 14, f"Old backing protrudes below the resized app: {color}"
    pointer(process, "b 0")
    if os.environ.get("ENZUI_QA_CAPTURE_RELEASE"):
        ring = (window()["at"][0] - 4, window()["at"][1] + 200)
        for frame in range(6):
            image = Image.open(shot(f"resize-release-{frame}")).convert("RGB")
            color = image.getpixel(ring)
            if os.environ.get("ENZUI_QA_TRACE_BACKING"):
                backing_windows = [w for w in json.loads(hypr("-j", "clients")) if w["class"] == "dev.enzui.SidebarBacking"]
                print("release frame", frame, "ring", color, "backing", [(w["at"], w["size"]) for w in backing_windows], "app", (window()["at"], window()["size"]), "phase", phase())
            if os.environ.get("ENZUI_QA_ASSERT_BACKGROUND"):
                assert color[2] - color[0] < 14, f"Backing layer vanished in release frame {frame}: {color}"
    wait(lambda: phase() == "open", "Resized app did not repin")
    opened_from = float(hypr("--batch", "repl return hl.plugin.enzui_sidebar_scene.pocket_state(enzui_sidebar.owner).opened_from"))
    if os.environ.get("ENZUI_QA_ASSERT_NO_REOPEN"):
        assert opened_from >= .99, f"Resizing replayed the sidebar opening animation from {opened_from}"
    print("resized pane opened from:", opened_from)
    resized = window()
    assert resized["size"] == grown and resized["pinned"], (resized, grown)
    lua('assert(enzui_sidebar.entries[1].pinned_size and enzui_sidebar.entries[1].pinned_size.x==' + str(grown[0]) + ')')
    shot("resized")
    if os.environ.get("ENZUI_QA_ASSERT_BACKGROUND"):
        time.sleep(1.05)
        settled_color = Image.open(shot("resized-settled")).convert("RGB").getpixel((resized["at"][0] - 4, resized["at"][1] + 200))
        assert settled_color[2] - settled_color[0] < 14, f"Backing layer vanished after the resize handoff: {settled_color}"
        if os.environ.get("ENZUI_QA_ASSERT_LATE_BACKGROUND"):
            time.sleep(3.2)
            late_color = Image.open(shot("resized-late")).convert("RGB").getpixel((resized["at"][0] - 4, resized["at"][1] + 200))
            assert late_color[2] - late_color[0] < 14, f"Backing layer vanished after the temporary cover ended: {late_color}"
    if os.environ.get("ENZUI_QA_REPEAT_RESIZE") and not resize_on_right:
        x, y = resized["at"]
        grip_x, grip_y = x + grown[0] - 16, y + grown[1] - 16
        pointer(process, f"m {grip_x} {grip_y}")
        pointer(process, "b 1")
        pointer(process, f"m {grip_x - 100} {grip_y - 8}")
        time.sleep(.06)
        pointer(process, f"m {grip_x - 210} {grip_y - 20}")
        time.sleep(.06)
        pointer(process, f"m {grip_x - 270} {grip_y - 30}")
        wait(lambda: phase() == "dragging" and window()["size"][0] < grown[0] - 20, "Second resize did not shrink the pinned app")
        shrunken = window()["size"]
        pointer(process, "b 0")
        if os.environ.get("ENZUI_QA_CAPTURE_RELEASE"):
            ring = (window()["at"][0] - 4, window()["at"][1] + 200)
            for frame in range(6):
                color = Image.open(shot(f"resize-return-release-{frame}")).convert("RGB").getpixel(ring)
                if os.environ.get("ENZUI_QA_ASSERT_BACKGROUND"):
                    assert color[2] - color[0] < 14, f"Backing layer vanished on second release frame {frame}: {color}"
        wait(lambda: phase() == "open", "Second resize did not remain open")
        assert window()["size"] == shrunken and window()["pinned"]
        if os.environ.get("ENZUI_QA_ASSERT_NO_REOPEN"):
            repeat_start = float(hypr("--batch", "repl return hl.plugin.enzui_sidebar_scene.pocket_state(enzui_sidebar.owner).opened_from"))
            assert repeat_start >= .99, f"Second resize reopened from {repeat_start}"
        grown = shrunken
    lua('assert(enzui_sidebar_hide().ok)')
    wait(lambda: state()["shown"] == 0, "Resized app did not hide")
    lua('assert(enzui_sidebar_toggle(enzui_sidebar.entries[1].id).ok)')
    wait(lambda: phase() == "open", "Resized app failed to reopen")
    assert window()["size"] == grown, "Hiding lost pinned size"
    lua('assert(enzui_sidebar_unpin(enzui_sidebar.entries[1].id,false).ok)')
    wait(lambda: not state()["entries"], "Original app failed to unpin")
    failure = hypr("--batch", "repl return hl.plugin.enzui_sidebar_scene.pocket_state(enzui_sidebar.owner).failure")
    assert failure == "none", f"Ordinary restore required recovery: {failure}"
    assert not client_resize_disabled(window()["address"]), "Unpinned app still blocks its own resize"
    assert all(abs(now - before) <= 1 for now, before in zip(window()["size"], original["size"])), "Unpin lost original desktop size"
    print("resized pinned:", grown, "unpin restored:", window()["size"])
    lua('local w;for _,candidate in ipairs(hl.get_windows())do if candidate.class=="enzui-sidebar-pinned-qa" and candidate.pid==' + str(original["pid"]) + ' then w=candidate end end;assert(w);assert(enzui_sidebar_pin_window(w).ok);assert(enzui_sidebar_toggle(enzui_sidebar.entries[1].id).ok)')
    wait(lambda: phase() == "open", "Re-pinned app did not reopen")
    assert window()["size"] == grown, "Re-pinning the same app forgot its pinned size"
    x, y = window()["at"]
    width, height = window()["size"]
    on_right = state()["side"] == "right"
    warm_x = x - 90 if on_right else x + width + 85
    pointer(process, f"m {x + width - 8} {y + height // 2}")
    pointer(process, "b 1")
    pointer(process, "b 0")
    time.sleep(.08)
    assert state()["shown"] != 0 and window()["pinned"], "Clicking tab without moving unpinned app"
    pointer(process, "b 1")
    pointer(process, f"m {warm_x} {y + height // 2 + 12}")
    time.sleep(.08)
    pointer(process, f"m {warm_x + (-25 if on_right else 25)} {y + height // 2 + 14}")
    wait(lambda: phase() in ("dragging", "detached"), "Tab did not detach before a rail drop")
    pointer(process, f"m {state()['rail'][0] + 15} {y + height // 2 + 10}")
    time.sleep(.04)
    pointer(process, "b 0")
    wait(lambda: phase() == "open", "Dropping tab on the rail did not repin")
    ordinary_open = float(hypr("--batch", "repl return hl.plugin.enzui_sidebar_scene.pocket_state(enzui_sidebar.owner).opened_from"))
    if os.environ.get("ENZUI_QA_ASSERT_NO_REOPEN"):
        assert ordinary_open < .99, "Tab drag lost its ordinary opening animation"
    assert window()["size"] == grown and window()["pinned"], "Canceling tab drag lost the pinned size"
    x, y = window()["at"]
    width, height = window()["size"]
    warm_x = x - 90 if on_right else x + width + 85
    farther_x = x - 180 if on_right else x + width + 180
    pointer(process, f"m {x + width - 8} {y + height // 2}")
    pointer(process, "b 1")
    pointer(process, f"m {warm_x} {y + height // 2 + 12}")
    time.sleep(.08)
    pointer(process, f"m {farther_x} {y + height // 2 + 25}")
    wait(lambda: phase() in ("dragging", "detached"), "Tab did not begin the native app drag")
    pointer(process, "b 0")
    wait(lambda: not state()["entries"], "Dragging tab away did not unpin")
    assert all(abs(now - before) <= 1 for now, before in zip(window()["size"], original["size"])) and not window()["pinned"]
    print("Pinned app resized, hid/reopened, retained size after re-pin, tab click/rail return, and tab drag-away unpin passed")
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
        for _ in range(65):
            if not path.exists() or "populated 0" in (path / "cgroup.events").read_text(): break
            time.sleep(.1)
        else: raise RuntimeError("QA app cgroup still populated")
    assert window() is None, "QA app remained mapped after service cleanup"
    print("owned QA app service stopped; cgroup empty")
