from pathlib import Path
import hashlib
import json
import os
import shlex
import subprocess
import sys

root = Path(__file__).resolve().parents[1]
base = Path(sys.argv[1]).resolve()
build = Path(sys.argv[2]).absolute() if len(sys.argv) > 2 else root / ".build/sidebar-scene"
if os.environ.get("ENZUI_BUILD_CAPPED") != "1":
    raise SystemExit("Use scripts/build-sidebar-scene")
if not build.resolve().is_relative_to((root / ".build").resolve()):
    raise SystemExit("Scene output must remain under this repository's .build directory")
if base == build.resolve() or build.is_symlink():
    raise SystemExit("Choose a separate prepared base and scene build directory")
build.mkdir(exist_ok=True)
for name in ("prefix", "source", "hyprland-current-build"):
    link = build / name
    if not link.exists():
        link.symlink_to(base / name, target_is_directory=True)
    if not link.is_symlink() or link.resolve() != (base / name).resolve():
        raise SystemExit("Scene build belongs to a different base")
provenance = build / f".revisions-{os.getpid()}"
try:
    with provenance.open("xb") as file:
        file.write((base / "build-revisions").read_bytes())
    provenance.replace(build / "build-revisions")
finally:
    provenance.unlink(missing_ok=True)
# Keep the compositor receipt alongside its plugin for export validation.
receipt = build / "desktop-compositor.json"
if (base / "desktop-compositor.json").is_file():
    staged = build / f".receipt-{os.getpid()}"
    try:
        with staged.open("xb") as file:
            file.write((base / "desktop-compositor.json").read_bytes())
        staged.replace(receipt)
    finally:
        staged.unlink(missing_ok=True)
elif receipt.is_file() and not receipt.is_symlink():
    receipt.unlink()
entry = next(item for item in json.loads((base / "hyprland-current-build/compile_commands.json").read_text())
             if item["file"].endswith("/LuaEventHandler.cpp"))
args = shlex.split(entry["command"])
compiler = args.pop(0)
flags = []
i = 0
while i < len(args):
    arg = args[i]
    if arg in ("-o", "-include", "-MF", "-MT", "-MQ"):
        i += 2
        continue
    if arg in ("-c", "-MD", "-MMD", "-fmodules-ts", "-Winvalid-pch") or arg.startswith(("-fmodule-mapper=", "-fdeps-format=")) or arg == entry["file"]:
        i += 1
        continue
    flags.append(arg)
    i += 1
sources = [root / name for name in ("tools/enzui-sidebar-events.cpp", "tools/enzui-pointer-events.cpp",
           "compositor/SidebarScene.cpp", "compositor/ShellRegions.inc", "compositor/SidebarViewport.inc", "compositor/SidebarDrag.inc", "compositor/TileDrop.inc", "compositor/IconFeedback.inc", "compositor/SidebarDetachTab.inc", "compositor/WorkspaceTransition.inc", "compositor/OverviewBackdrop.inc", "compositor/PanelShape.inc", "compositor/SidebarLuaApi.inc", "compositor/SidebarLifecycle.inc", "compositor/FloatingChrome.hpp", "compositor/IconDropLease.hpp", "compositor/ContentReadiness.hpp", "compositor/CommittedGeometry.hpp",
           "compositor/SceneTiming.hpp", "compositor/SearchDim.inc", "compositor/RestorePicture.hpp", "compositor/Material.cpp", "compositor/AppearanceTransition.hpp", "compositor/MaterialReadback.hpp", "compositor/NestedKeyboard.cpp", "protocols/enzui-material-v1.xml", "vendor/iced/wgpu/src/shader/material.wgsl", "vendor/iced/wgpu/src/shader/material_filter.wgsl", "tools/wgsl-glsl/src/main.rs", "scripts/build-material-shaders")]
identity = b"".join(path.read_bytes() for path in sources) + Path(__file__).read_bytes()
test_flags = ["-DENZUI_MATERIAL_READBACK_TEST=1"] if os.environ.get("ENZUI_MATERIAL_READBACK_TEST") == "1" else []
identity += entry["command"].encode() + subprocess.check_output([compiler, "--version"]) + " ".join(test_flags).encode()
for header in ("src/version.h", "src/plugins/PluginAPI.hpp", "src/event/EventBus.hpp", "src/render/Renderer.hpp", "src/managers/input/InputManager.hpp",
               "src/desktop/view/window/WindowBackend.hpp", "src/desktop/view/LayerSurface.hpp", "src/protocols/types/SurfaceState.hpp",
               "src/layout/LayoutManager.hpp", "src/layout/supplementary/DragController.hpp",
               "src/layout/space/Space.hpp", "src/layout/algorithm/Algorithm.hpp", "src/layout/algorithm/TiledAlgorithm.hpp", "src/layout/target/Target.hpp",
               "src/desktop/view/focusable/FocusBlockReasons.hpp", "src/desktop/view/Popup.hpp",
               "src/protocols/XDGShell.hpp", "src/protocols/core/Compositor.hpp",
               "src/managers/SeatManager.hpp", "src/managers/fullscreen/FullscreenController.hpp", "src/managers/fullscreen/FullscreenTypes.hpp", "src/output/Monitor.hpp", "src/desktop/view/surfaceTree/PopupOwner.hpp",
               "src/workspace/HLWorkspace.hpp", "src/render/pass/Pass.hpp", "src/desktop/state/FocusState.hpp",
               "src/render/Framebuffer.hpp", "src/render/pass/TexPassElement.hpp", "src/render/types.hpp",
               "src/pointer/cursor/CursorShapeOverrideController.hpp"):
    identity += (base / "source" / header).read_bytes()
fingerprint = hashlib.sha256(identity).hexdigest()
protocol = build / "material-protocol"
protocol.mkdir(exist_ok=True)
xml = root / "protocols/enzui-material-v1.xml"
subprocess.run(["wayland-scanner", "server-header", str(xml), str(protocol / "enzui-material-v1-protocol.h")], check=True)
subprocess.run(["wayland-scanner", "private-code", str(xml), str(protocol / "enzui-material-v1-protocol.c")], check=True)
subprocess.run([str(root / "scripts/build-material-shaders"), str(protocol)], check=True)
subprocess.run(["cc", "-fPIC", "-c", "-O2", str(protocol / "enzui-material-v1-protocol.c"), "-o", str(protocol / "protocol.o")], check=True)
outdir = build / "shell-plugins"
if outdir.is_symlink():
    raise SystemExit("Scene plugin directory must not be a symlink")
outdir.mkdir(exist_ok=True)
artifact = outdir / f"scene-{fingerprint[:16]}.so"
if artifact.is_symlink():
    raise SystemExit("Versioned scene artifact must not be a symlink")
if not artifact.exists():
    temporary = outdir / f".scene-{os.getpid()}.so"
    if temporary.exists() or temporary.is_symlink():
        raise SystemExit("Temporary scene artifact already exists")
    try:
        subprocess.run([compiler, *flags, *test_flags, f"-I{protocol}", "-fPIC", "-shared", "-fvisibility=hidden", "-fno-gnu-unique",
                        "-O1", "-g0", str(sources[0]), "-x", "none", str(protocol / "protocol.o"), "-o", str(temporary)], cwd=entry["directory"], check=True)
        temporary.replace(artifact)
    finally:
        temporary.unlink(missing_ok=True)
link = outdir / f".current-{os.getpid()}"
try:
    link.symlink_to(artifact.name)
    link.replace(outdir / "enzui-pointer-events.so")
finally:
    link.unlink(missing_ok=True)
print("Built isolated native sidebar provider", fingerprint)
