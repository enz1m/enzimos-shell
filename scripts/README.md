# Script entry points

Run from the repository root. Build commands serialize shared output and
accept an optional `ENZUI_BUILD_MEMORY_MAX` cap. Use a separate build root for
a runtime needed alongside a live desktop.

| Task | Entry points |
| --- | --- |
| Source setup | `setup-sources` |
| Nested desktop | `../enzui-hypr`, `start-shell-platform` |
| Shell iteration | `shell-iterate check/test/build`, `shell-iterate gate` |
| Login UI review | `preview-greeter`, `preview-greeter-cu` |
| Production staging | `build-release --bin-dir .build/NEW/bin`, `export-desktop` |
| Pinned compositor | `build-desktop-compositor`, `build-compositor`, `build-pinned-desktop-compositor`, `build-sidebar-scene` |
| Remote stack | `build-remote-session-runtime`, `build-remote-desktop-portal` |
| Live wallpaper | `build-enzwall` |
| Native support | `build-pointer-events`, `build-window-snapshot`, `build-shell-recorder` |
| Material development | `build-material-plugin`, `build-material-shaders`, `build-glass-demo` |
| Asset generation | `generate-lock-fonts`, `generate-greeter-type` |
| Wallpaper management | `wallpaper-gallery`, `import-wallpaper`, `install-wallpapers`, `run-wallpaper` |
| Disposable OS QA | `enzimos-vm install ISO`, `settings-vm` |

`launch-desktop-app` and `build-receipt` support launch and provenance.
`build-shared-compositor` prepares the compositor for shared-desktop previews.

`test-*` commands cover policy, launch/staging admission, protocol boundaries and
native lifecycle behavior. Some need a separately prepared disposable runtime;
read their arguments before running them. Native tests must verify exact process
identities and clean up only their owned scope on success and failure.

Use [shell development](../docs/shell-development.md) for the normal gate,
[greeter review](../docs/greeter-design-preview.md) for UI iteration and
[native QA](../tools/probes/README.md) for compositor-specific checks.
