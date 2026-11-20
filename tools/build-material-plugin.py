# Builds the material plugin and generated protocol code under OUT.
from pathlib import Path
import os
import shlex
import subprocess
import sys

root = Path(__file__).resolve().parents[1]
base = Path(sys.argv[1]).resolve()
out = Path(sys.argv[2]).absolute()
if os.environ.get("ENZUI_BUILD_CAPPED") != "1":
    raise SystemExit("Use scripts/build-material-plugin")
if not out.resolve().is_relative_to((root / ".build").resolve()):
    raise SystemExit("Output must remain under this repository's .build directory")
out.mkdir(parents=True, exist_ok=True)
protocol = root / "protocols/enzui-material-v1.xml"
subprocess.run(["wayland-scanner", "server-header", str(protocol), str(out / "enzui-material-v1-protocol.h")], check=True)
subprocess.run(["wayland-scanner", "private-code", str(protocol), str(out / "enzui-material-v1-protocol.c")], check=True)
subprocess.run([str(root / "scripts/build-material-shaders"), str(out)], check=True)
entry = next(item for item in __import__("json").loads((base / "hyprland-current-build/compile_commands.json").read_text())
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
cflags = [f for f in flags if f.startswith(("-I", "-D", "-isystem")) or f == "-isystem"]
subprocess.run(["cc", "-fPIC", "-c", "-O2", str(out / "enzui-material-v1-protocol.c"), "-o", str(out / "protocol.o")], check=True)
temporary = out / f".enzui-material-{os.getpid()}.so"
try:
    subprocess.run([compiler, *flags, f"-I{out}", "-fPIC", "-shared", "-fvisibility=hidden", "-fno-gnu-unique", "-O2", "-g0",
                    str(root / "tools/enzui-material-plugin.cpp"), "-x", "none", str(out / "protocol.o"), "-o", str(temporary)],
                   cwd=entry["directory"], check=True)
    temporary.replace(out / "enzui-material.so")
finally:
    temporary.unlink(missing_ok=True)
print(out / "enzui-material.so")
