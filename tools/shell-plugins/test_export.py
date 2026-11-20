import copy
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest

import contribution
import manifest

APP_ID = "io.github.enzui.Fixture"
SOURCE = {"format": 1, "id": "fixture.cpu", "name": "CPU", "version": "1.0",
          "description": "CPU counters", "permissions": ["shell.bar"],
          "elements": [{"id": "cpu", "section": "end", "before": "builtin.clock"}],
          "settings": []}
DECLARATION = '''[package.metadata.enzui.shell-plugins]
format = 1
app-id = "io.github.enzui.Fixture"
contributions = [{manifest = "plugin/manifest.json", program = "plugin/main.py"}]
'''
EXPECTED = {"share/enzui/shell-plugins/fixture.cpu.enzplugin",
            "share/enzui-shell/plugins/fixture.cpu.enzplugin",
            "share/enzui/shell-plugin-contributions.json"}


class ExportTests(unittest.TestCase):
    def setUp(self):
        target = Path(os.environ["CARGO_TARGET_DIR"]).resolve(strict=True)
        scratch = target / "test-scratch-shell-plugins"
        scratch.mkdir(mode=0o700, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=scratch)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.crate = self.root / "crate"
        (self.crate / "plugin").mkdir(parents=True)
        self.cargo = self.crate / "Cargo.toml"
        self.header = '[package]\nname = "fixture"\nversion = "0.1.0"\nedition = "2024"\n'
        self.cargo.write_text(self.header + DECLARATION)
        (self.crate / "plugin/manifest.json").write_bytes(manifest.encode(SOURCE))
        (self.crate / "plugin/main.py").write_text("print('worker source')\n")
        self.destination = self.root / "files"

    def planned(self):
        return contribution.plan(self.crate, APP_ID)

    def test_export_identity_paths_and_module(self):
        files = self.planned()
        contribution.export(files, self.destination)
        actual = {str(p.relative_to(self.destination)) for p in self.destination.rglob("*") if p.is_file()}
        self.assertEqual(actual, EXPECTED)
        canonical = self.destination / "share/enzui/shell-plugins/fixture.cpu.enzplugin"
        data = canonical.read_bytes()
        self.assertEqual(data, (self.destination / "share/enzui-shell/plugins/fixture.cpu.enzplugin").read_bytes())
        self.assertEqual(json.loads(data), {**SOURCE, "program": "print('worker source')\n"})
        record = json.loads((self.destination / "share/enzui/shell-plugin-contributions.json").read_bytes())
        self.assertEqual(record["app-id"], APP_ID)
        self.assertEqual(record["contributions"][0]["sha256"], hashlib.sha256(data).hexdigest())
        self.assertIs(record["contributions"][0]["enabled-by-default"], False)
        for path in actual:
            self.assertEqual(stat.S_IMODE((self.destination / path).stat().st_mode), 0o644)
            self.assertEqual((self.destination / path).stat().st_nlink, 1)
        module = contribution.flatpak_module(files, self.destination)
        self.assertEqual(len(module["sources"]), 3)
        self.assertEqual({c.split()[-1] for c in module["build-commands"]}, {"/app/" + p for p in EXPECTED})
        for source, data in zip(module["sources"], files.values()):
            self.assertEqual(Path(source["path"]).read_bytes(), data)
        before = {p: (self.destination / p).stat().st_mtime_ns for p in actual}
        contribution.export(files, self.destination)
        self.assertEqual(before, {p: (self.destination / p).stat().st_mtime_ns for p in actual})

    def test_default_and_explicit_opt_out_do_not_touch_output(self):
        for metadata in ("", '[package.metadata.enzui.shell-plugins]\nformat=1\napp-id="' + APP_ID + '"\ncontributions=[]\n'):
            self.cargo.write_text(self.header + metadata)
            files = self.planned()
            self.assertEqual(files, {})
            contribution.export(files, self.destination)
            self.assertFalse(self.destination.exists())
            self.assertIsNone(contribution.flatpak_module(files, self.destination))
        self.cargo.write_text(self.header + DECLARATION)
        contribution.export(self.planned(), self.destination)
        self.assertTrue((self.destination / "share/enzui/shell-plugins/fixture.cpu.enzplugin").is_file())

    def test_export_never_executes_app_or_worker(self):
        marker = self.root / "executed"
        program = f"from pathlib import Path\nPath({str(marker)!r}).write_text('executed')\n"
        (self.crate / "plugin/main.py").write_text(program)
        (self.crate / "build.rs").write_text('fn main() { panic!("build script must not execute"); }')
        result = subprocess.run([sys.executable, str(Path(__file__).with_name("export.py")),
                                 "--crate-root", str(self.crate), "--app-id", APP_ID,
                                 "--files-root", str(self.destination)], capture_output=True, timeout=3)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(json.loads(result.stdout)["files"]), 3)
        self.assertFalse(marker.exists())
        subprocess.run([sys.executable, "-I", "-S", "-c", program], check=True, timeout=3)
        self.assertEqual(marker.read_text(), "executed")

    def test_schema_refuses_ambiguity_and_unbounded_data(self):
        changes = [("format", True), ("id", "../escape"), ("id", "Upper"),
                   ("name", "hidden\u202e"), ("version", "a/b"),
                   ("permissions", ["host.exec"]), ("permissions", ["shell.bar"] * 2),
                   ("permissions", []), ("elements", SOURCE["elements"] * 2),
                   ("app_id", APP_ID)]
        for key, value in changes:
            with self.subTest(key=key, value=value):
                source = {**SOURCE, key: value}
                with self.assertRaises(manifest.InvalidContribution):
                    manifest.bundle(manifest.encode(source), b"pass")
        for program in (b"", b"\0", b"\xff", b"x" * (512 * 1024 + 1), b"\x01" * 300_000):
            with self.subTest(size=len(program)), self.assertRaises(manifest.InvalidContribution):
                manifest.bundle(manifest.encode(SOURCE), program)
        manifest.bundle(manifest.encode(SOURCE), b"x" * (512 * 1024))
        with self.assertRaises(manifest.InvalidContribution):
            manifest.parse_json(b'{"format":1,"format":2}')
        with self.assertRaises(manifest.InvalidContribution):
            manifest.parse_json(b'{"format":NaN}')

    def test_settings_types_ranges_and_keys(self):
        settings = [{"key": "a", "label": "A", "kind": "toggle", "default": True},
                    {"key": "b", "label": "B", "kind": "integer", "min": 1, "max": 10, "default": 2},
                    {"key": "c", "label": "C", "kind": "choice", "choices": ["a", "b"], "default": "a"}]
        manifest.validate({**SOURCE, "settings": settings})
        for index, key, value in ((0, "default", 1), (1, "default", True), (1, "min", 3),
                                  (1, "max", 1_000_001), (2, "choices", ["a", "a"]),
                                  (2, "default", "unknown"), (0, "key", "../a"),
                                  (0, "extra", False)):
            altered = copy.deepcopy(settings)
            altered[index][key] = value
            with self.subTest(index=index, key=key), self.assertRaises(manifest.InvalidContribution):
                manifest.validate({**SOURCE, "settings": altered})
        with self.assertRaises(manifest.InvalidContribution):
            manifest.validate({**SOURCE, "settings": settings + settings})

    def test_owner_source_paths_and_duplicates(self):
        with self.assertRaisesRegex(ValueError, "owner"):
            contribution.plan(self.crate, "io.github.enzui.Other")
        for path in ("/etc/passwd", "../Cargo.toml", "plugin/../Cargo.toml", "plugin//main.py"):
            with self.subTest(path=path), self.assertRaises(ValueError):
                contribution.read_source(self.crate, path, 100)
        source = self.crate / "plugin/main.py"
        source.unlink()
        source.symlink_to(self.cargo)
        with self.assertRaises(OSError):
            self.planned()
        source.unlink()
        os.mkfifo(source)
        with self.assertRaisesRegex(ValueError, "regular"):
            self.planned()
        source.unlink()
        source.write_text("pass")
        (self.crate / "link").symlink_to(self.crate / "plugin", target_is_directory=True)
        with self.assertRaises(OSError):
            contribution.read_source(self.crate, "link/main.py", 100)
        duplicate = DECLARATION.replace('}]', '}, {manifest="plugin/manifest.json", program="plugin/main.py"}]')
        self.cargo.write_text(self.header + duplicate)
        with self.assertRaisesRegex(ValueError, "duplicate"):
            self.planned()
        self.assertFalse(self.destination.exists())

    def test_destination_does_not_follow_links_or_replace_different_data(self):
        files = self.planned()
        self.destination.mkdir()
        outside = self.root / "outside"
        outside.mkdir()
        (self.destination / "share").symlink_to(outside, target_is_directory=True)
        with self.assertRaises(OSError):
            contribution.export(files, self.destination)
        self.assertEqual(list(outside.iterdir()), [])
        (self.destination / "share").unlink()
        contribution.export(files, self.destination)
        package = self.destination / "share/enzui/shell-plugins/fixture.cpu.enzplugin"
        package.write_bytes(b"unrelated")
        with self.assertRaisesRegex(ValueError, "fresh staging"):
            contribution.export(files, self.destination)
        self.assertEqual(package.read_bytes(), b"unrelated")
        package.unlink()
        package.symlink_to(self.cargo)
        with self.assertRaises(OSError):
            contribution.export(files, self.destination)
        self.assertEqual(self.cargo.read_text(), self.header + DECLARATION)


if __name__ == "__main__":
    unittest.main()
