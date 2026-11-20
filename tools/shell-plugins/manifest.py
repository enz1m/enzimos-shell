"""Version-1 plugin data; validating and bundling never execute worker source.

The shell's plugin parser (`shell/src/plugins/manifest.rs`) is the authority.
`schema.json` carries its permission names, bar anchors and limits; a shell
test regenerates it and runs one corpus through both validators, so this copy
cannot drift from what the shell accepts.
"""

import json
from pathlib import Path
import re
import sys
import unicodedata

SCHEMA = json.loads((Path(__file__).resolve().parent / "schema.json").read_text())
# The exporter's own input bound; the shell only limits the encoded package.
MAX_MANIFEST = 128 * 1024
MAX_PROGRAM = SCHEMA["max_program_bytes"]
MAX_PACKAGE = SCHEMA["max_package_bytes"]
PERMISSIONS = frozenset(SCHEMA["permissions"])
BUILTINS = frozenset(SCHEMA["builtin_anchors"])


class InvalidContribution(ValueError):
    pass


def require(condition, reason):
    if not condition:
        raise InvalidContribution(reason)


def fields(value, expected):
    require(type(value) is dict and set(value) == set(expected),
            "unexpected or missing fields")


def sequence(value, maximum):
    require(type(value) is list and len(value) <= maximum, "invalid or excessive array")
    return value


def slug(value, maximum):
    require(type(value) is str and len(value) <= maximum
            and re.fullmatch(r"[a-z0-9][a-z0-9._-]*", value) is not None,
            "invalid plugin or element identity")
    return value


def application(value):
    require(type(value) is str, "invalid application favorite")
    if value.startswith("app:desktop/"):
        name = value[len("app:desktop/"):]
        require(len(name.encode("utf-8")) <= 220 and name.endswith(".desktop")
                and not any(unicodedata.category(ch) == "Cc" or ch in "/\\" for ch in name),
                "invalid application favorite")
        return value
    require(value.startswith("app:") and 5 <= len(value.encode("utf-8")) <= 255
            and re.fullmatch(r"[A-Za-z0-9._-]+", value[4:]) is not None, "invalid application favorite")
    return value


def app_id(value):
    require(type(value) is str and len(value) <= 128
            and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_-]*){2,}", value) is not None,
            "invalid Flatpak app-id")
    return value


def label(value, maximum):
    require(type(value) is str and value.strip() and len(value) <= maximum,
            "invalid label length")
    require(not any(unicodedata.category(ch) == "Cc" or 0x200B <= ord(ch) <= 0x200F
                    or 0x202A <= ord(ch) <= 0x202E or 0x2060 <= ord(ch) <= 0x206F
                    or ord(ch) == 0xFEFF for ch in value), "unsafe label characters")
    return value


def bar_id(value):
    require(type(value) is str, "invalid placement anchor")
    if value in BUILTINS:
        return
    require(value.startswith("plugin:") and len(value) <= 104, "invalid placement anchor")
    parts = value[7:].split("/")
    require(len(parts) == 2, "invalid placement anchor")
    slug(parts[0], 64)
    slug(parts[1], 32)


def unique_pairs(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate JSON field")
        result[key] = value
    return result


def parse_json(data):
    def invalid_number(_):
        raise InvalidContribution("non-finite JSON number")
    try:
        return json.loads(data.decode("utf-8"), object_pairs_hook=unique_pairs,
                          parse_constant=invalid_number)
    except (UnicodeError, json.JSONDecodeError) as error:
        raise InvalidContribution("invalid UTF-8 JSON") from error


def encode(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"),
                      allow_nan=False).encode("utf-8") + b"\n"


def bundle(manifest_bytes, program_bytes):
    require(len(manifest_bytes) <= MAX_MANIFEST, "manifest too large")
    require(len(program_bytes) <= MAX_PROGRAM, "worker source too large")
    manifest = parse_json(manifest_bytes)
    validate(manifest)
    try:
        program = program_bytes.decode("utf-8")
    except UnicodeError as error:
        raise InvalidContribution("worker source is not UTF-8") from error
    require(program.strip() and "\0" not in program, "invalid worker source")
    package = encode({**manifest, "program": program})
    require(len(package) <= MAX_PACKAGE, "encoded package too large")
    return manifest, package


def validate(manifest):
    fields(manifest, ("format", "id", "name", "version", "description", "permissions",
                      "elements", "settings"))
    require(type(manifest["format"]) is int and manifest["format"] == 1, "unsupported package format")
    slug(manifest["id"], 64)
    label(manifest["name"], 64)
    label(manifest["description"], 512)
    version = manifest["version"]
    require(type(version) is str and 1 <= len(version) <= 32
            and re.fullmatch(r"[A-Za-z0-9._+\-]+", version) is not None, "invalid plugin version")
    permissions = sequence(manifest["permissions"], len(PERMISSIONS))
    require(all(type(value) is str and value in PERMISSIONS for value in permissions),
            "unsupported permission")
    require(len(set(permissions)) == len(permissions), "duplicate permission")
    elements = sequence(manifest["elements"], SCHEMA["max_elements"])
    require(not elements or "shell.bar" in permissions, "bar elements require shell.bar")
    ids = set()
    for element in elements:
        fields(element, ("id", "section", "before"))
        identity = slug(element["id"], 32)
        require(identity not in ids, "duplicate element")
        ids.add(identity)
        require(element["section"] in ("start", "middle", "end"), "invalid bar section")
        if element["before"] is not None:
            bar_id(element["before"])
    keys = set()
    for setting in sequence(manifest["settings"], SCHEMA["max_settings"]):
        require(type(setting) is dict, "invalid setting")
        key = slug(setting.get("key"), 32)
        require(key not in keys, "duplicate setting")
        keys.add(key)
        label(setting.get("label"), 80)
        kind = setting.get("kind")
        if kind == "toggle":
            fields(setting, ("key", "label", "kind", "default"))
            require(type(setting["default"]) is bool, "invalid toggle default")
        elif kind == "integer":
            fields(setting, ("key", "label", "kind", "default", "min", "max"))
            require(all(type(setting[key]) is int for key in ("min", "max", "default")),
                    "invalid integer setting")
            require(-1_000_000 <= setting["min"] <= setting["default"] <= setting["max"] <= 1_000_000,
                    "invalid integer range")
        elif kind == "choice":
            fields(setting, ("key", "label", "kind", "default", "choices", "labels") if "labels" in setting
                   else ("key", "label", "kind", "default", "choices"))
            choices = [label(choice, 64) for choice in sequence(setting["choices"], 16)]
            require(choices and len(choices) == len(set(choices)), "invalid choices")
            require(label(setting["default"], 64) in choices, "invalid choice default")
            if "labels" in setting:
                names = setting["labels"]
                require(type(names) is dict and set(names) == set(choices), "choice labels must name every choice")
                shown = [label(names[choice], 64) for choice in choices]
                require(len(set(shown)) == len(shown), "choice labels must be unique")
        elif kind == "applications":
            fields(setting, ("key", "label", "kind", "default"))
            pins = setting["default"]
            require(type(pins) is list and len(pins) <= 32, "invalid application favorites")
            require(len(set(application(pin) for pin in pins)) == len(pins), "duplicate application favorite")
        else:
            raise InvalidContribution("unsupported setting kind")


def conformance(path):
    """Prints whether each source manifest in the JSON list at `path` bundles."""
    verdicts = []
    for source in json.loads(Path(path).read_text()):
        try:
            bundle(encode(source), b"pass\n")
            verdicts.append(True)
        except InvalidContribution:
            verdicts.append(False)
    print(json.dumps(verdicts))


if __name__ == "__main__":
    conformance(sys.argv[1])
