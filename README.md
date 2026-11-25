# EnzimOS Shell

EnzimOS Shell provides the Wayland desktop, Settings, lock screen and greeter for
enzimOS. The desktop runs on a pinned Hyprland stack and includes Search, floating
and tiling modes, a window overview, a plugin sidebar, notifications and Control
Center. Shell and its companion programs use [enzUI](https://github.com/enz1m/enzui).

## Build and preview

From a Wayland session, initialize the pinned sources and build the nested desktop.

```sh
scripts/setup-sources
./enzui-hypr build
./enzui-hypr build-shell
./enzui-hypr shell
```

The repository selects Rust 1.99.1 and pins the client from the public
[Phone Link repository](https://github.com/enz1m/enzimos-phone-link). Building
the full desktop also requires a C++26 compiler, CMake, Meson, Ninja, Lua, libsystemd,
systemd user services and the pinned components' native libraries. The Cargo lockfile pins the
Rust dependencies. The source lock and Git submodules pin the compositor, shader
sources, wallpaper engine and other components used for staging. [Source pins](sources.lock.json)

The default nested build root is `.build/desktop`. Runtime state is under
`$XDG_RUNTIME_DIR/enzui-desktop`. Set `ENZUI_HYPR_BUILDROOT`,
`ENZUI_HYPR_RUNROOT` and `ENZUI_HYPR_CONFIG` to run an isolated preview alongside
another session. Use a fresh prepared root when compositor or shader pins change.

```sh
./enzui-hypr status
./enzui-hypr gpu
./enzui-hypr config-check
./enzui-hypr stop
```

A managed review runs the pinned desktop compositor, the scene plugin and the
shell's helpers under a private profile, as an installed login does. Prepare its
root once per set of pins:

```sh
ENZUI_HYPR_BUILDROOT=$PWD/.build/compositor scripts/build-desktop-compositor
ENZUI_HYPR_BUILDROOT=$PWD/.build/compositor \
    ENZUI_SIDEBAR_BUILDROOT=$PWD/.build/scene scripts/build-sidebar-scene
scripts/build-window-snapshot
scripts/build-enzwall target/debug
cargo build --locked --workspace
scripts/start-shell-platform --experimental --profile review \
    --build-root $PWD/.build/scene
```

The window snapshot helper draws window previews, and the staged Enzwall binary
runs live wallpapers; the preview starts without either. Add `--exit-with-shell`
when closing the review should close the apps it launched.
[Installed and nested session options](docs/installed-session.md)

For login UI work, `scripts/preview-greeter-cu` opens an isolated mock greeter.
[Greeter preview controls](docs/greeter-design-preview.md)

## Where to make changes

| Area | Source |
| --- | --- |
| Desktop runtime, bar, dock, Search, Control Center and sidebar | [shell/src](shell/src) |
| Settings entry point and page UI | [enzui-settings](shell/src/bin/enzui-settings.rs), [system pages](shell/src/system_pages), [settings catalog](shell/src/settings_catalog) |
| Lock screen, login and greeter | [lock/src](lock/src) |
| Compositor integration, native material and private protocols | [compositor](compositor), [protocols](protocols) |
| Default compositor configuration | [config](config) |
| Wallpaper scenes and rendering | [Enzwall](https://github.com/enz1m/enzwall) |

## Develop and validate

```sh
scripts/shell-iterate check shell
scripts/shell-iterate check settings
scripts/shell-iterate build shell
scripts/shell-iterate gate
```

Use scoped builds for iteration and the workspace gate before merging. Validate
UI changes in an isolated preview on the relevant renderer. See the
[development workflow](docs/shell-development.md), [script reference](scripts/README.md)
and [documentation index](docs/README.md).

The shell's original code is licensed under MIT. Dependencies, fonts and
wallpapers carry their own notices. [MIT license](LICENSE) · [Third-party notices](third-party/README.md)
