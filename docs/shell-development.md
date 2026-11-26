# Fast, scoped shell development

Use a batch of related changes before native builds. The shell includes many
executables; rebuilding all of them for a Settings-only change wastes time.

## Compositor and portal forks

The Hyprland, Aquamarine and portal submodules pin commits from the enzimOS
forks. The other compositor variants use the revisions in
[sources.lock.json](../sources.lock.json). Build scripts prepare clean
worktrees at those revisions.

To update an existing checkout, refresh its submodule URLs and pins.

```sh
git submodule sync
git submodule update --init
scripts/setup-sources
```

| Build | Hyprland branch | Aquamarine branch |
| --- | --- | --- |
| Installed desktop | `enzim` | `nested-wayland` |
| Input preview | `scoped-eis` | `nested-wayland` |
| Unrenamed desktop | `desktop-unrenamed` | `nested-wayland` |
| Shared desktop | `shared-desktop` | `nested-wayland` |
| Remote session | `desktop-unrenamed` | `headless-render-node` |

Use a fresh build root when changing compositor pins. For local builds set
`CMAKE_BUILD_PARALLEL_LEVEL` to the number of concurrent compiler jobs the
machine can support. The default is one.

The portal builds from the `enzim` branch of the
[portal fork](https://github.com/enz1m/enzim-xdg-desktop-portal-hyprland).
`scripts/build-remote-desktop-portal` prints the output directory. It contains
the binary, the receipt, and the upstream-to-fork diff used by the package builder.
A preview whose `--shell-bin-dir` holds that binary as
`xdg-desktop-portal-enzui-remote` beside `enzui-portal` registers it as its
Hyprland portal backend, so screenshots and screen casts work on a host without
`/usr/lib/xdg-desktop-portal-hyprland`. The portal asks with `enzui-share-picker`
when that binary sits in the same directory as the portal's resolved executable,
and with `hyprland-share-picker` otherwise; a `screencopy:custom_picker_binary`
in `xdph.conf` overrides both. Copy or hard-link both binaries into the
preview's binary directory rather than symlinking the portal.

## Iteration commands

Run `scripts/setup-sources` before the first build or after a pin changes.
The iteration helper uses the lockfile and the fetched sources. It builds one
target at a time and runs subprocess fixtures serially. Use `CARGO_TARGET_DIR`
to choose a dedicated cache. To cap memory on a machine with systemd user
services, set `ENZUI_BUILD_MEMORY_MAX` to a value such as `4G`.

```sh
scripts/shell-iterate check settings
scripts/shell-iterate test settings system_pages::developer::tests
scripts/shell-iterate test lib developer_tools::tests
scripts/shell-iterate build settings
scripts/shell-iterate lint
```

Targets are `lib`, `settings` and `shell`. `check` performs type checking without
code generation/linking. Focused tests require a test-name filter and always use
`--test-threads=1` because fixtures run subprocesses and private buses. Inspect
the result count: a mistyped filter passing zero tests is not qualification.
`build settings` compiles only `enzui-settings` and its dependencies. There is
no `--bins` or release build hidden in the helper.

The lint policy matches the local framework policy in the workspace manifests.
`lint` checks the desktop workspace and the WGSL translation tool in
sequence; vendor snapshots remain dependency inputs. Test panic/print exceptions
live in `clippy.toml`. Application-internal library records are exempt from API
documentation lints so comments remain about contracts rather than field names.
Other exceptions require a local reason, including native FFI boundaries and
intentional CLI/protocol output.

Workflow:

1. Read and implement related fixes and regression tests as one batch.
2. Type-check the affected target; run its focused tests.
3. Build only the binary actually needed for native QA. For a shell-only change,
   select `shell`; do not rebuild Settings, lock or the app fleet automatically.
4. Before committing a coherent milestone, run the full required gate once:
   `scripts/shell-iterate gate`. It refuses unless `enzui-enzimos-vm.service` is
   inactive; it never stops the VM. It first denies Clippy warnings across all
   owned Rust targets and features, then runs the Shell and greeter tests with serial test fixtures.
   Desktop linting covers both production defaults and preview/report features.
5. Build/export a fresh release only at a qualified release milestone. Existing
   exported binaries, app snapshots and their receipts are not replaced or
   relabeled during iteration.

Do not batch dependent shell binaries into one native review without checking
which exact binaries changed. Keep owned QA and builds serialized. The helper
reports actual wall time and exit code; a timed-out outer invocation is not a
successful build, even if earlier tests completed.

## Expandable desktop panels

Size a popup's native window for its usable content before revealing it.
Use the fitted popup renderer for disclosure animations. Layout, paint and
the Wayland input region follow the visible card inside the reserved window.
On small outputs, cap the window at the available height and scroll the body.

## Test scope

Keep tests for distinct product behavior: persisted settings and migrations,
authentication and account guards, protocol/input bounds, asynchronous ownership,
resource cleanup, and representative layout or rendering invariants. Prefer
extending an existing scenario over adding a test for each issue or branch.
Keep a small set of representative subsystem scenarios; do not retain a separate
case for every animation phase, navigation event or stale-callback permutation.

Do not add tests that repeat implementation constants, inspect source text or icon
spellings, merely construct a widget, or check a synthetic screenshot analyzer.
Review cosmetic changes in the native preview. Keep native QA probes for actual
rendering, input and compositor behavior; do not replace them with mock pixels.
A reversible UI adjustment does not automatically need a new test.

## Compilation profiles

Only the local `enzui-shell` package enables incremental compilation in dev and
test; dependency profile settings remain unchanged. Production-like dev/native
QA keeps shell `opt-level=2`, debug=0 and the existing codegen-units. The compositor
maximize-preview composition cost is the reason not to lower native optimization
merely to make builds faster.

Shell test-only code uses `opt-level=0`, debug=0 and incremental=true. These are
correctness test executables, not visual/performance benchmark binaries. Runtime
acceptance still uses the optimized dev or release app. Use the committed Cargo lockfile for reproducible builds.

Incremental cache stays on disk in this repository's `target/debug/incremental`;
never delete a shared target wholesale. A profile change has an initial cache
cost, and an edit affecting the shell library can still rebuild many dependents
at the full gate. Dependency upgrades and release builds remain expensive.

Enzwall is an independent pinned dependency. Run its `scripts/check` in the
[Enzwall checkout](https://github.com/enz1m/enzwall) before updating the shell submodule; the shell gate does not duplicate
that workspace suite. The owned greeter preview uses a separate service/runtime
and VNC port 5929 (override with `ENZUI_GREETER_PREVIEW_VNC`), preserving the original checkout's running preview.

## Fullscreen policy

True native fullscreen fills the output and hides the top bar and Assistant
trigger on the affected output, including their input regions. Keyboard shortcuts
and explicitly opened shell controls remain available. The passive display frame
uses the top layer so native fullscreen occludes it. Custom input regions commit
independently of pixel buffer changes, allowing explicit reveals when the renderer
reuses a buffer.
Shell work-area spacing applies to maximized windows, including the preference serialized as `fullscreen_padding`.
Passive sidebar and dock edge-hover reveal is suppressed while a mapped fullscreen
window occupies the selected output and active workspace. Other outputs and
workspaces keep their own behavior. The sidebar shortcut remains an explicit
reveal. Sidebar errors wait for fullscreen to end. Hiding the idle sidebar
backing lets a fullscreen window qualify for scanout. Fullscreen and minimize
events wake sidebar policy immediately.

A fullscreen app on a visible desktop inhibits idle. Notifications holds new
banners by default while a fullscreen app occupies the active desktop on the
shell's output. Held alerts stay in the center, and critical alerts follow the
normal delivery policy. Direct scanout auto mode admits fullscreen game content.
The VRR content-type mode covers fullscreen game and video content.
