#!/usr/bin/env python3
"""Read Cargo contributions without invoking Cargo, a build script, or an app."""

import argparse
import json
import sys
import tomllib

from contribution import export, flatpak_module, plan


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--crate-root", required=True)
    parser.add_argument("--app-id", required=True, help="App ID from the Flatpak build manifest")
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--files-root", help="Fresh staging directory corresponding to /app")
    parser.add_argument("--flatpak-module", action="store_true",
                        help="Print a Flatpak module JSON (null when opted out)")
    args = parser.parse_args()
    if args.flatpak_module and args.check:
        parser.error("--flatpak-module requires --files-root")
    try:
        files = plan(args.crate_root, args.app_id)
        if not args.check:
            export(files, args.files_root)
        result = (flatpak_module(files, args.files_root) if args.flatpak_module
                  else {"files": [str(path) for path in files]})
        print(json.dumps(result, sort_keys=True))
    except (OSError, ValueError, TypeError, AttributeError, tomllib.TOMLDecodeError) as error:
        print(f"shell-plugin export: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
