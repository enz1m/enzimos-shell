"""Build the pinned RemoteDesktop portal and record its source and binary."""
import argparse
from pathlib import Path
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "vendor/xdg-desktop-portal-hyprland"
PIN = json.loads((ROOT / "sources.lock.json").read_text())["remote_desktop_portal"]
REVISION = PIN["revision"]
UPSTREAM_REVISION = PIN["upstream_revision"]
SUBMODULES = {"subprojects/sdbus-cpp": "b7d85f936d622299ee57966c2b2aefde5e4f3684",
              "subprojects/hyprland-protocols": "bd153e76f751f150a09328dbdeb5e4fab9d23622"}
PROTOCOLS = {"enzui-assistant-input-v1": ROOT / "vendor/Hyprland/protocols/enzui-assistant-input-v1.xml",
             "wlr-virtual-pointer-unstable-v1": ROOT / "vendor/Hyprland/protocols/wlr-virtual-pointer-unstable-v1.xml",
             "virtual-keyboard-unstable-v1": ROOT / "vendor/Hyprland/protocols/virtual-keyboard-unstable-v1.xml"}


def digest(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def run(args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def export(repository, revision, destination):
    if subprocess.check_output(["git", "-C", str(repository), "rev-parse", "HEAD"], text=True).strip() != revision:
        raise SystemExit(f"{repository} is not at the recorded revision {revision}")
    destination.mkdir(parents=True, exist_ok=True)
    archive = subprocess.run(["git", "-C", str(repository), "archive", revision], check=True, capture_output=True).stdout
    run(["tar", "-x", "-C", str(destination)], input=archive)


def main():
    if os.environ.get("ENZUI_BUILD_CAPPED") != "1":
        raise SystemExit("Use scripts/build-remote-desktop-portal")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, nargs="?")
    args = parser.parse_args()
    protocols = {name: digest(xml) for name, xml in PROTOCOLS.items()}
    builder = digest(Path(__file__))
    key = hashlib.sha256(json.dumps([REVISION, protocols, builder], sort_keys=True).encode()).hexdigest()[:16]
    output = args.output.absolute() if args.output else ROOT / ".build/remote-desktop-portal" / key
    if not output.parent.resolve().is_relative_to((ROOT / ".build").resolve()):
        raise SystemExit("Output must stay in this repository's build directory")
    if output.exists() or output.is_symlink():
        if args.output:
            raise SystemExit("Explicit output must be new")
        receipt = json.loads((output / "remote-desktop-portal.json").read_text())
        if (receipt.get("fork_revision"), receipt.get("protocols_sha256"), receipt.get("builder_sha256"),
                receipt.get("binary_sha256"), receipt.get("patch_sha256")) != (
                REVISION, protocols, builder, digest(output / "xdg-desktop-portal-enzui-remote"),
                digest(output / "remote-desktop.patch")):
            raise SystemExit("Completed portal build changed. Choose a fresh output directory.")
        print(output)
        return
    output.parent.mkdir(parents=True, exist_ok=True)
    work = ROOT / ".build/xdph-work"
    work.mkdir(exist_ok=True)
    scratch = Path(tempfile.mkdtemp(prefix="xdph-", dir=work))
    source = scratch / "source"
    export(SOURCE, REVISION, source)
    for path, revision in SUBMODULES.items():
        export(SOURCE / path, revision, source / path)
    generated = source / "src/enzui-protocols"
    generated.mkdir()
    for name, xml in PROTOCOLS.items():
        run(["wayland-scanner", "client-header", str(xml), str(generated / f"{name}-client-protocol.h")])
        run(["wayland-scanner", "private-code", str(xml), str(generated / f"{name}-protocol.c")])
    build = scratch / "build"
    run(["cmake", "-S", str(source), "-B", str(build), "-G", "Ninja", "-DCMAKE_BUILD_TYPE=Release", "-DSYSTEMD_SERVICES=OFF"], stdout=sys.stderr)
    run(["ninja", "-C", str(build), "-j1", "xdg-desktop-portal-hyprland"], stdout=sys.stderr)
    staged = scratch / "output"
    staged.mkdir()
    binary = staged / "xdg-desktop-portal-enzui-remote"
    shutil.copy2(build / "xdg-desktop-portal-hyprland", binary)
    # The distro packager includes the upstream-to-fork diff with the binary.
    if subprocess.run(["git", "-C", str(SOURCE), "cat-file", "-e", f"{UPSTREAM_REVISION}^{{commit}}"],
                      stderr=subprocess.DEVNULL).returncode:
        run(["git", "-C", str(SOURCE), "fetch", "--no-tags", PIN["url"], UPSTREAM_REVISION])
    patch = staged / "remote-desktop.patch"
    with patch.open("wb") as stream:
        run(["git", "-C", str(SOURCE), "diff", "--binary", UPSTREAM_REVISION, REVISION, "--"], stdout=stream)
    receipt = {"source": PIN["url"], "revision": UPSTREAM_REVISION, "fork_revision": REVISION,
               "submodules": SUBMODULES, "patch_sha256": digest(patch),
               "protocols_sha256": protocols, "builder_sha256": builder, "binary_sha256": digest(binary)}
    (staged / "remote-desktop-portal.json").write_text(json.dumps(receipt, indent=2) + "\n")
    staged.rename(output)
    print(output)


if __name__ == "__main__":
    main()
