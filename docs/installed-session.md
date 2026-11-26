# Installed desktop session

The installed launcher runs a desktop on its logind seat. Nested previews use
private configuration and runtime roots; they must not change host services.
[System services](system-services.md) describes feature-specific OS contracts.

## Startup and identity

greetd selects `/usr/share/wayland-sessions/enzui.desktop`, whose `enzui-session`
entry runs `/usr/share/enzui/scripts/start-shell-platform --installed`.
The launcher refuses an inherited Wayland desktop or missing logind session.
Installed component paths come from the exported `installed.json`.

One per-account session lock prevents a second login from merging with an
existing desktop. Each login creates a fresh nonce under
`$XDG_RUNTIME_DIR/enzui-shell-platform/desktop-<nonce>` and a delegated
`enzui-platform-desktop-<nonce>.service`. The unit uses a supervisor leaf,
`Delegate=yes` and `KillMode=control-group`.

Startup proceeds in this order:

1. Start the selected compositor and its ABI-matched pointer/scene plugin.
2. Publish the Wayland display and compositor identity to D-Bus activation and
   the systemd user manager, using `XDG_CURRENT_DESKTOP=enzUI:Hyprland`.
3. Adopt the PAM-unlocked keyring when available and apply the account's input
   method policy. Optional services remain unavailable when not installed.
4. Start wallpaper and shell. The shell owns notifications, clipboard services,
   plugin workers and the session's authorization dialogs.
5. Publish `ready.json` after the shell frame covers its selected output, then
   start `enzui-session-ready.target`. The stock `graphical-session.target`
   and `xdg-desktop-autostart.target` refuse manual starts; the ready target
   binds the first and wants the second, so generated autostart units start
   only after the frame is ready. Logout stops it with `enzui-session.target`,
   and both stock targets stop once no desktop needs them.

Private receipts include compositor PID/start identity, instance, display,
`launch.json`, session-bus address/peer and shell PID/start identity. The installed
bus is the account's fixed runtime `bus`, owned by its user manager; it is not a
private daemon in the desktop cgroup. Receipt readers still verify the selected
compositor and shell. Operations requiring desktop membership enforce that
separately from read-only Settings observation.

## Environment and lifetime

Identity, locale, logind seat, valid XDG directories and input-method variables
survive the launcher's clean-environment boundary. The PAM keyring path is kept
only when it belongs to the account's private runtime directory.

| Setting | Installed behavior |
| --- | --- |
| `ENZUI_DESKTOP_SESSION=installed` | Enables installed-session behavior rather than nested host isolation |
| `ENZUI_ALLOW_SYSTEM_POWER=1` | Allows explicit system power actions |
| `ENZUI_PLUGIN_ISOLATION=delegated` | Requires owned, bounded plugin cgroups |
| `ENZUI_TERMINAL` | Uses the exported Terminal unless the login explicitly overrides it |
| `ENZUI_HYPR_*`, `ENZUI_SHELL_SETTINGS`, `ENZUI_SHELL_BIN_DIR` | Derived from the selected installed layout |

Production binaries omit the `greeter-preview` feature and do not consume mock
passwords. Installed sessions never set the preview credential flag.

Logout or compositor exit ends the desktop unit and returns control to greetd.
The supervisor relaunches a failed shell while preserving the compositor and
applications; more than three crash restarts in a minute ends an installed login.
A primary-output relocation is a planned restart and does not consume that budget.

The per-login runtime directory and its logs are removed when the desktop ends;
`$XDG_STATE_HOME/enzui/last-desktop.log` keeps the startup log. When the
compositor crashes or is killed (it left its crash report or instance lock), the
shell keeps exiting, or startup fails, the launcher also keeps bounded excerpts
of the startup, compositor, compositor-internal and shell logs and the crash
report's head under `$XDG_STATE_HOME/enzui/desktop-failures/`. Bundles are owner-only, stay on
the device and only the newest five are kept. An orderly logout keeps none.
Normal previews preserve apps after shell exit; a review started with
`--exit-with-shell` closes its own apps with the shell.

Locking uses `ext-session-lock`. The compositor keeps outputs locked after a lock
client failure, while supervision attempts bounded prompt recovery. `LockedHint`
follows compositor acknowledgement and clears only after authenticated unlock.
logind Unlock signals cannot bypass authentication. A verified inactive-to-active
return restores only outputs the idle worker requested off and re-arms timers.

## Outputs and windows

The shell chooses the top-left physical output: lowest y, then x, then compositor
ID. `ENZREMOTE-*` loan outputs are excluded from shell placement and wallpaper.
Other desktop outputs receive a static backdrop without shell reservations.
The live wallpaper renderer follows the primary output.

Startup uses conservative pixel-based scaling rather than physical-size/PPI
metadata. Saved settings apply afterwards. Logical geometry accounts for scale
and rotation. Output-management changes are persisted as monitor rules so they
survive client disconnect and later rule refreshes.

New floating clients fit within their output's work area after their first
committed geometry. Existing clients retain their geometry unless an output change
strands them outside it. Tiling restores saved floating geometry when disabled.

When an output disconnects, its workspaces move to a surviving output. If the
disconnected output had focus, its active workspace is shown on the survivor.
Reconnecting an output restores its remembered workspaces and active workspace.
Pinned windows stay on the survivor. Windows held by a removed sidebar return to
the active desktop of their surviving output.

Output removal, resolution, rotation or scale changes trigger shell relocation without
restarting the compositor or application sessions. Lock surfaces cover every
output, including outputs connected while locked; new surfaces remain black until
painted. The greeter reads system keyboard configuration before password entry.
Desktop layout overrides preserve unrelated system XKB options.
On first login the wallpaper starts before broker enrollment, so enrollment does
not extend the blank compositor handoff.

## Settings and agent activation

Settings is a single-instance app with a windowless mode. D-Bus activation starts
`enzui-settings --agent-background` through
`enzui-agent-dev.enzui.Settings.service`; it is on demand, not enabled at login.
Desktop launches wake the same model using `enzui-settings:ROUTE[?plugin=ID]`.
An idle background instance exits when it has no retained work.
Opening the window starts a fresh versioned settings read. A late snapshot from
the windowless start cannot replace it or clear its pending state, so navigation
and polling resume from the current persisted settings.
Installed activation also carries the current logind session hint; applications
validate its account and seat before using it to refresh state after unlock.

Its user unit needs desktop rights because launched applications inherit the
unit's restrictions. Installed startup publishes the selected desktop environment
to activation; previews leave the host user manager untouched. Packaging files
under `config/agent/` come from the live Settings registry. Regenerate them with
`ENZUI_REGENERATE_AGENT_PACKAGING=1` when that registry changes.

The MCP Center remains the permission engine. Its trusted control credential is
used only by consent/permission UI, never exposed as an agent capability.
Unanswered requests expire as denied. Settings exposes a fixed allowlist through
`settings.list`, `settings.get`, `settings.set` and `settings.set_system`; the
source catalogue in `shell/src/settings_agent.rs` defines supported keys.

User changes complete after persistence. Computer changes require polkit before
mutation, on the original bus connection, followed by non-interactive timedated
calls and readback. Cancellation before dispatch withdraws approval; cancellation
after dispatch cannot undo a committed change. Broker consent is per function,
not per setting key. Windowless Settings can serve agent requests.

Each account receives its own enrolled app credentials when the broker is
available. Existing credentials are not replaced to bypass revocation. The
framework's provisioning overrides select credentials and broker sockets for
isolated development.

## First-login Welcome

`enzui-welcome` sets the desktop layout, appearance and optional AI once per
account. `/etc/xdg/autostart/dev.enzui.Welcome.desktop` starts it with
`--first-login` through `enzui-session-ready.target`. It opens only in a local
installed session and only while `$XDG_CONFIG_HOME/enzimos/welcome.json` is
absent. systemd's generator ignores the entry's `X-GNOME-Autostart-Condition`,
so the app's own check decides. Finishing or
cancelling writes that record atomically; its presence alone ends first-login
launches. The Welcome app entry reopens it on demand.

Layout and appearance choices go through the shell settings store and shared
theme file. A scale change uses the display path from Settings and returns to
the previous layout after 15 seconds unless kept. On-device AI defaults to
off when completing setup for the first time. Cancelling preserves the
current AI preference. Providers use the gateway configuration that Settings writes.
Chosen models are passed to `enzui-settings --queue-models ID...`. Settings
owns those downloads and their bar progress. Welcome records completion only
after Settings accepts the job.

## Portals and remote control

`config/portal/enzui-portals.conf` selects the installed routing:

| Interface | Preferred backend |
| --- | --- |
| FileChooser, Access, Background | enzUI |
| Settings color-scheme | enzUI; other settings fall through |
| ScreenCast | enzUI remote, then Hyprland |
| RemoteDesktop | enzUI remote |
| Screenshot, GlobalShortcuts | Hyprland |
| Print | GTK |
| Other interfaces | Hyprland, then GTK |

The frontend owns document grants; the chooser returns selected files without
opening them. Cancel, window close and Request.Close cancel selection. Nested
previews register their own backend on their private session bus.

The separately named remote portal builds from the pinned [portal fork](https://github.com/enz1m/enzim-xdg-desktop-portal-hyprland).
Screen selection precedes an explicit view/control decision. Dismissal, errors
or unanswered control consent grant no input devices. `ConnectToEIS` is a
one-client, per-session descriptor with only the approved device capabilities;
using it disables the session's alternate Notify input path.

Closing or locking removes devices and connections; unlock requires a fresh
request. Portal shutdown drains callbacks before destroying session targets,
then destroys Wayland proxies and joins workers. Capture/input globals also
require [compositor admission](privileged-clients.md); consent alone does not
bypass process identity checks.

## User state and optional services

Applications use the account's valid XDG directories in installed sessions.
Previews use private disposable app roots while retaining read access to shared
theme configuration. Package upgrades never own or remove user settings,
credentials, homes or per-login runtime data.

Startup apps, background-app activation and session restoration are separate:
startup edits affect the next login; background policy changes require reviewed
service plans; session restoration is opt-in and stores installed desktop IDs and
placement rather than command lines or window contents.

Standard ScreenSaver inhibition runs in a tracked helper for installed local
sessions. Cookies are caller-owned and disappear on disconnect. The shell observes
the service without activating an absent provider. Explicit Lock/Suspend remain
available. Remote sessions and previews do not install host inhibitors.

Other OS-backed features, including backups, developer tools, account management,
permissions, controller tests and GameMode, follow the contracts in
[system services](system-services.md) and [login personalization](login-personalization.md).

## Packaging

`scripts/build-release --bin-dir .build/NEW/bin` stages an explicit shipped binary
set. Compilation, installation and receipt generation share that set; development
probes are built separately. The enzUI apps are not part of this export: each is
built from its own repository and installed by its own OS package into
`/usr/lib/enzui/apps/`, which `installed.json` names for the shell's catalog.
The export only sets the desktop's default handlers for folders and text.

`scripts/export-desktop` writes an empty DESTDIR from prepared components. Exported
paths must not point into the checkout. ELF library paths are relocatable, and
exported symlinks must be relative, contained and resolvable.

`PREFIX/share/enzui/manifest.json` is the package ownership interface:

- `files`, `generated.path` and `owned_directories` describe the complete owned
  tree, including modes, hashes and symlink targets. The OS marks greetd/PAM files
  as configuration and preserves account-local data.
- `runtime_dependencies` distinguishes required libraries/tools from optional
  feature dependencies. Missing optional services disable their features.
- `installed.json` identifies component/app roots. Exported desktop entries use
  installed binaries; account MIME defaults retain precedence.
- `source_revision`, dependency pins, licences and build receipts
  identify the actual shipped graph. Each binary must match its receipt.

The exporter refuses mismatched bytes, pins, source revisions and dirty source.
`--allow-unreleased` records development exceptions in `unreleased`; a release
manifest must leave that list empty. A compositor and its ABI-matched plugin ship
together. Never overwrite artifacts used by a live session to prepare a new
release. See [validation](validation.md) for acceptance requirements.

## Window decoration negotiation

The pinned compositor honors explicit XDG client-side decoration requests.
Applications using the framework’s shell titlebar therefore retain their own
Close and window actions. Server-side requests and an unset preference keep
the existing server-side default. The compositor fork changes negotiation only;
it does not add a second titlebar or change the scene-plugin ABI.

Resident authentication prompts reload the stored desktop language when a new
request becomes active. An already active prompt keeps its password and
authorization transaction.

## Desktop fonts

Settings stores UI, document and monospace font choices and the weight preset
in `enzui-shell/settings.json`, separately from themes. The Settings portal
publishes these choices as Pango descriptions under
`org.gnome.desktop.interface` (`font-name`, `document-font-name`,
`monospace-font-name`) and emits `SettingChanged` after a saved change.
This compatibility namespace requires no GNOME libraries. Accessibility text
scaling is published separately and applied once.

The first settings read migrates font families and recognized weight presets
from the user's shared `enzui/theme.toml`. It persists the replacement before
removing those keys, preserves unrelated tokens and comments, and leaves
`enzui/<application-id>.toml` overrides untouched. Linked themes are imported
without rewriting their files. An existing font preferences
object, including an explicit reset, takes precedence over legacy values.
Apps using the framework's default desktop portal features follow the session's
fonts at startup and while running; on other desktops they use that desktop's
portal, with GTK/KDE files as fallback.

## Shared appearance editing

Settings follows the framework’s live theme integration for file, desktop, font
and text-scale changes. Its simplified appearance editor retains the parsed
theme document and changes only the selected token family. An external edit
refreshes the editor; a stale read cannot replace a newer local choice. Unknown
keys, future values, comments and unrelated custom overrides survive a save.

Window controls and focus outline width are shared theme preferences. Corner
presets update the framework window radius and compositor preference together
after a successful theme save. The standard rounded preset leaves the framework
window radius unpinned so later default refinements can reach it.
