"""Opt-in Cargo metadata to data-only Flatpak deployment files."""

import hashlib
import os
from pathlib import Path, PurePosixPath
import secrets
import stat
import tomllib

from manifest import (MAX_MANIFEST, MAX_PROGRAM, app_id, bundle, encode, fields,
                      require, sequence)

CANONICAL = PurePosixPath("share/enzui/shell-plugins")
COMPATIBILITY = PurePosixPath("share/enzui-shell/plugins")
DEPLOYMENT = PurePosixPath("share/enzui/shell-plugin-contributions.json")


def relative_parts(value):
    require(type(value) is str and value and not value.startswith("/")
            and "\\" not in value and "\0" not in value, "source must be relative")
    parts = value.split("/")
    require(all(part not in ("", ".", "..") for part in parts), "invalid relative source path")
    return parts


def read_source(root, relative, maximum):
    parts = relative_parts(relative)
    directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for part in parts[:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory)
            os.close(directory)
            directory = child
        fd = os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
        with os.fdopen(fd, "rb") as source:
            require(stat.S_ISREG(os.fstat(source.fileno()).st_mode), "source must be regular")
            data = source.read(maximum + 1)
            require(len(data) <= maximum, "source exceeds its byte limit")
            return data
    finally:
        os.close(directory)


def plan(crate_root, expected_app_id):
    root = Path(crate_root).resolve(strict=True)
    cargo = tomllib.loads(read_source(root, "Cargo.toml", 256 * 1024).decode("utf-8"))
    metadata = cargo.get("package", {}).get("metadata", {}).get("enzui", {}).get("shell-plugins")
    if metadata is None:
        return {}
    fields(metadata, ("format", "app-id", "contributions"))
    require(type(metadata["format"]) is int and metadata["format"] == 1,
            "unsupported contribution metadata")
    identity = app_id(metadata["app-id"])
    require(identity == app_id(expected_app_id), "Flatpak app-id does not match contribution owner")
    rows = sequence(metadata["contributions"], 16)
    if not rows:
        return {}
    files, records, seen = {}, [], set()
    for row in rows:
        fields(row, ("manifest", "program"))
        manifest, package = bundle(read_source(root, row["manifest"], MAX_MANIFEST),
                                   read_source(root, row["program"], MAX_PROGRAM))
        plugin_id = manifest["id"]
        require(plugin_id not in seen, "duplicate contributed plugin identity")
        seen.add(plugin_id)
        path = CANONICAL / (plugin_id + ".enzplugin")
        compatible = COMPATIBILITY / path.name
        files[path] = package
        files[compatible] = package
        records.append({"id": plugin_id, "app-id": identity, "version": manifest["version"],
                        "sha256": hashlib.sha256(package).hexdigest(), "package": str(path),
                        "host-v1-package": str(compatible), "permissions": manifest["permissions"],
                        "enabled-by-default": False})
    files[DEPLOYMENT] = encode({"format": 1, "app-id": identity, "contributions": records})
    return files


def export(files, files_root):
    if not files:
        return
    root = Path(files_root)
    root.mkdir(parents=True, exist_ok=True)
    root_fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    prepared = []
    try:
        for relative, data in files.items():
            parts = relative_parts(str(relative))
            directory = os.dup(root_fd)
            try:
                for part in parts[:-1]:
                    try:
                        os.mkdir(part, mode=0o755, dir_fd=directory)
                    except FileExistsError:
                        pass
                    child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory)
                    os.close(directory)
                    directory = child
                name = parts[-1]
                try:
                    fd = os.open(name, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW, dir_fd=directory)
                except FileNotFoundError:
                    prepared.append((directory, name, data))
                    directory = None
                else:
                    with os.fdopen(fd, "rb") as existing:
                        mode = os.fstat(existing.fileno()).st_mode
                        require(stat.S_ISREG(mode) and stat.S_IMODE(mode) == 0o644,
                                "existing export is not regular mode 0644 data")
                        require(existing.read(len(data) + 1) == data,
                                "export differs: use a fresh staging directory")
            finally:
                if directory is not None:
                    os.close(directory)
        for directory, name, data in prepared:
            temporary = ".enzui-export-" + secrets.token_hex(16)
            fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                         0o600, dir_fd=directory)
            try:
                with os.fdopen(fd, "wb") as output:
                    output.write(data)
                    output.flush()
                    os.fchmod(output.fileno(), 0o644)
                    os.fsync(output.fileno())
                os.link(temporary, name, src_dir_fd=directory, dst_dir_fd=directory,
                        follow_symlinks=False)
            finally:
                os.unlink(temporary, dir_fd=directory)
            os.fsync(directory)
    finally:
        for directory, _, _ in prepared:
            os.close(directory)
        os.close(root_fd)


def flatpak_module(files, files_root):
    if not files:
        return None
    commands, sources = [], []
    root = Path(files_root).resolve(strict=True)
    for index, relative in enumerate(files):
        name = f"contribution-{index}.data"
        sources.append({"type": "file", "path": str(root / relative), "dest-filename": name})
        commands.append(f"/usr/bin/install -Dm644 {name} /app/{relative}")
    return {"name": "enzui-shell-contributions", "buildsystem": "simple",
            "build-commands": commands, "sources": sources}
