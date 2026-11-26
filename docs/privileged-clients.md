# Who may capture the screen, send input or control the compositor

With `ENZUI_PRIVILEGED_CLIENTS=enforce`, the compositor filters screen-capture,
virtual-input, input-method and input-capture globals, restricts compositor
control requests and reserves `dev.enzui.*` application ids. The implementation
lives in the [Hyprland fork](https://github.com/enz1m/enzim-hyprland).

## Threat model

An ordinary same-user app may start arbitrary binaries, copy or rename tools,
try loader injection, retain an inherited connection, or connect after an
approved process exits. These actions must not turn it into the desktop shell.
Root, ptracing a trusted process, and writing approved installed executables
are outside this boundary. Development previews trust their prepared source
and binary tree. Portal service-instance attestation is separate from executable admission.

## Routes

| Route | Authority | Identity |
|---|---|---|
| Shell | Input | Exact live child of the verified supervisor; approved executable at registration, then retained process/image identity |
| Shell-announced helper | Announced capture/input, once, within 100–10,000 ms | Original authenticated shell control peer; exact direct child and announced executable inode |
| Portal executables | Capture and input capture | Startup-approved executable inode |
| Remote-desktop portal | Input | Startup-approved executable inode; its own consent flow still governs sessions |
| Portal helpers | Capture | Approved helper inode and an uninjected approved portal parent |
| Input method | Input method | Startup-approved executable inode (`ENZUI_IME_EXES`: Fcitx5, and dictation to type its text) |
| Root grants | Named capture/input | Matching UID and exact/subtree `/system.slice/…` cgroup from a protected public grant |
| Explicit preview dev tools | Capture, input, input method, input capture and control | Startup-approved executable inode; QA only |

The capture family includes `zwlr_screencopy_manager_v1`,
`hyprland_toplevel_export_manager_v1`, `ext_image_copy_capture_manager_v1`,
`ext_output_image_capture_source_manager_v1` and
`ext_foreign_toplevel_image_capture_source_manager_v1`. Input includes
`zwlr_virtual_pointer_manager_v1` and `zwp_virtual_keyboard_manager_v1`.
Input method is `zwp_input_method_manager_v2`; input capture is
`hyprland_input_capture_manager_v1`.

## Compositor control

Control requests change compositor code, configuration or outputs: `eval`,
`repl`, `dispatch`, `plugin` (except `plugin list`), `reload`, `output`,
`reloadshaders` and `seterror`, alone or inside a batch. They are answered for
the registered shell; the supervisor and its descendants up to four levels
down, outside the shell's and the compositor's process trees; uninjected
executables named in `ENZUI_CONTROL_EXES` (Settings, Welcome and the Phone Link
host, which opens desktop apps for a paired phone); and preview dev tools.
Anyone else receives an `error:` reply. An admitted executable sends its
requests on the compositor socket itself; a `hyprctl` child it starts is
identified as `hyprctl`. Read-only queries stay open to every same-user client.
In-process requests from plugins carry no socket peer and are not filtered.

## Shell application ids

Window rules, hit testing and first-configure placement single out
`dev.enzui.*` application ids. The compositor keeps such an id only for the
registered shell or an uninjected executable installed in the same directory
as `ENZUI_SHELL_EXE`, and otherwise leaves the window without an application
id. X11 windows never keep one.

## Supervisor contract

The per-user supervisor derives its own PID and `/proc/self/stat` field-22 start
ticks, opens a pidfd for itself, and passes a dedicated inherited descriptor
(number at least 3) to the compositor. The startup environment names them as
`ENZUI_SHELL_LAUNCHER_PID`, `ENZUI_SHELL_LAUNCHER_START` and
`ENZUI_SHELL_LAUNCHER_FD`. The compositor validates the pidfd target, liveness,
UID and start identity, duplicates it CLOEXEC, closes the supplied descriptor
and removes that environment entry. A missing, stale or dead capability never
falls back to the first matching executable.

The shell stays the supervisor's direct child. The supervisor must not adopt
unrelated orphan processes: the Python launcher checks `PR_GET_CHILD_SUBREAPER`
and refuses that configuration. An adopting Rust supervisor needs a
non-adopting launch leaf or an explicit child-registration extension; it must
not treat adopted children as launch authority. Every OS supervisor must satisfy the same registration contract.

Registration binds the live process and the image actually executing. Replacing
the approved binary on disk does not revoke that already registered image;
a different process must qualify against the current approved path. Loader
checks still apply. Normal previews and installed supervision relaunch only
the selected child, preserving the compositor and other clients. Repeated
preview failures pause restarts and retain those clients. `--exit-with-shell`
reviews still end with the shell. The managed `enzui-hypr app shell` path waits
for its supervisor rather than starting an unbound second shell.

## Peer and policy lifetime

Wayland and privileged control requests use `SO_PEERPIDFD` to pin the original
socket peer, rather than interpreting a possibly reused numeric PID. Admitted
Wayland connections have a pidfd event source: original-process exit disconnects
the client even when another process retains its socket. This adds no idle timer.
The Linux API is described in the
[kernel maintainers' pidfd presentation](https://lpc.events/event/18/contributions/1691/attachments/1499/3166/-LPC-2024-%20PID%20FDs-%20where%20we%20were%2C%20where%20we%20are%20and%20were%20we%20would%20like%20to%20go.pdf).

Admission allowlists, the supervisor capability and approved loader path are
snapshotted before external IPC dispatch. A later `hl.env` call cannot grant
capture/input by rewriting an allowlist. On executable routes, nonempty
`LD_PRELOAD`/`LD_AUDIT` and a differing nonempty `LD_LIBRARY_PATH` refuse
admission. Root grants continue to trust root's unit environment.

Locking disconnects admitted clients except the registered shell and input
methods. New portal and ordinary helper clients are refused while locked;
unlocking requires a fresh connection. While locked, the input method receives
no keys and no lock-screen field state, and active input capture is released and
cannot start again until unlock. Root grants stay in
`/run/enzimos-compositor-grants`; the private `/run/enzimos` directory is
unchanged. The grant directory is root-owned and not group/world writable. Each
grant is a regular, root-owned, singly linked, non-group/world-writable file of
at most 4 KiB.

## Acceptance boundaries

An installed supervisor must provide the verified launcher capability above.
Executable/parent matching on portal routes does not establish service-unit or
instance attestation. Root controls cgroup entry for system-unit grants.

Qualify the exact staged graph with portal consent, recording, lock revocation,
PAM, hardware/render-node and remote-login checks before deployment. See
[validation](validation.md). A generic app/Flatpak capture override cannot replace
these boundaries.
