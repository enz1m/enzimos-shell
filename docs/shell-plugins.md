# Shell-plugin contributions from Flatpak apps

A build-time declaration can ship a small shell worker without running the app,
its build script, or the worker. This is packaging metadata, not an `app_setup`
runtime callback. Python 3.11+ is required by the exporter. Apps with no declaration
produce no files; their native packaging and Rust dependency graphs are unchanged.

## Declare and export

In the application's `Cargo.toml`:

```toml
[package.metadata.enzui.shell-plugins]
format = 1
app-id = "io.github.enzui.Example"
contributions = [{ manifest = "packaging/cpu/manifest.json", program = "packaging/cpu/main.py" }]
```

Paths are relative to the crate, regular files, and cannot traverse symlinks or
`..`. Contributions are limited to 16 unique plugin IDs. An empty list opts out.
The app ID must match the ID supplied from the Flatpak recipe; this claimed
identity is not host authority.

```sh
python3 tools/shell-plugins/export.py --crate-root /path/to/app \
  --app-id io.github.enzui.Example --check
python3 tools/shell-plugins/export.py --crate-root /path/to/app \
  --app-id io.github.enzui.Example --files-root /path/to/new/staging/files \
  --flatpak-module > /path/to/new/staging/plugins.module.json
```

The second command prints a Flatpak `simple` module whose sources reference the
staged data. Add the returned object to the app recipe's `modules`; `null` means
no contribution. Keep the staging sources until `flatpak-builder` finishes. Use a
fresh directory per build: identical exports are idempotent, but different
existing files are refused, never overwritten or cleaned. Opt-out does not delete
files from an old staging tree.

The module installs these paths below `/app`, corresponding to an installed
Flatpak deployment's `files/` directory:

- `share/enzui/shell-plugins/<plugin-id>.enzplugin`: canonical location.
- `share/enzui-shell/plugins/<plugin-id>.enzplugin`: identical data for the native
  host's version-1 discovery path. Both copies are regular mode-0644 files.
- `share/enzui/shell-plugin-contributions.json`: app ID, IDs/versions, package
  paths, SHA-256 content identities, requested permissions, and default-off intent.

The host must inspect the installed deployment. `/app` inside a running app and
XDG desktop exports are not plugin-discovery interfaces. The descriptor is kept
outside the plugin directories so it does not consume their package-entry budget.

## Source and wire formats

A source manifest has exactly `format` (1), `id`, `name`, `version`, `description`,
`permissions`, `elements`, and `settings`. The exporter adds the worker text as
`program` to produce the host's version-1 JSON package. In particular, **do not add
`app_id` to that package**: version-1 hosts reject unknown fields. Deployment
identity belongs to the descriptor and the host's validated Flatpak origin.

Plugin IDs are lowercase ASCII letters/digits with `.`, `_`, or `-`, start with a
letter/digit, and have at most 64 bytes. Element/setting IDs have the same syntax
and a 32-byte limit. There are at most eight elements and sixteen settings.
Elements declare `id`, `section` (`start`, `middle`, `end`), and nullable `before`.
Settings are typed `toggle`, bounded `integer`, finite `choice` values (with an
optional `labels` object naming every choice), or `applications` favourites (at
most 32 unique `app:` IDs). Requested permissions are the shell's own names,
including `applications.surface`. See `tools/shell-plugins/manifest.py` for exact
field and label bounds; unknown fields, duplicate fields/IDs/permissions and
out-of-range defaults are errors.

The shell's parser (`shell/src/plugins/manifest.rs`) is authoritative. Keep the
exporter's permission names, anchors and limits in
`tools/shell-plugins/schema.json` consistent with it. Exporter validation does not
replace host-side package validation.

The source manifest is limited to 128 KiB, UTF-8 worker text to 512 KiB, and the
encoded package to 1 MiB, including JSON escaping. Bundling does not establish that
arbitrary worker code is safe. The host independently validates and isolates it.
The existing runner transfers source using `ENZPY002`; the program itself uses
newline-delimited protocol-1 JSON (`hello`, host `start`, `ready`, `snapshot`,
`ping`/`pong`, and typed `config`). The runner's frame limit is 65,536 bytes.

## Permission and lifecycle boundaries

Requested permissions are **not grants**. Packaging never installs into the
shell's private store, starts a worker, enables a plugin, or changes user choices.
The host discovers/reviews it disabled by default, grants capabilities explicitly,
and revokes them by stopping the worker. No Flatpak host-filesystem, host-bus,
network, or background-app permission is required for the CPU contribution.

Discovery, update and removal are separate from activation: an updated digest
requires host review; a missing app/package is unavailable, not a new enable/disable
choice; rediscovery must not erase a previous disable or explicit removal. The
host owns those transitions and its private registry. The exporter does not
reconcile them or infer trust from the claimed app ID.

A CPU contribution, for example, needs only a standalone read-only `/proc/stat`
worker. The host's PID-namespace `/proc` exposes aggregate CPU counters on Linux;
there is no need to expose the app's filesystem or run its window. If that changes
on a platform, report unavailable or add an explicitly reviewed read-only broker,
not broad host access.

## Manual packages

For a standalone contribution, bundle the manifest and program directly:

```sh
mkdir -p .build/plugin-examples
python3 tools/package-enzui-plugin.py examples/plugins/cpu/manifest.json examples/plugins/cpu/main.py .build/plugin-examples/cpu.enzplugin
```

A Flatpak recipe installs the package under its own
`share/enzui-shell/plugins/` directory. Do not modify an installed deployment from
inside its sandbox. The host discovers app data rather than mounting a Flatpak
extension. Discovery never silently activates or updates an approved package;
changed bytes require a new host review.

## Verification

Run from this repository with an existing local target directory:

```sh
CARGO_TARGET_DIR="$PWD/target" python3 tools/shell-plugins/test_export.py ExportTests -v
scripts/shell-iterate test lib plugins::manifest::tests
```

The exporter checks use private scratch directories under that target. Host
manifest tests cover package admission; they do not regenerate the schema or
assert that arbitrary worker programs are safe.
