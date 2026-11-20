# Existing-window mobile handoff (opt-in)

The separately built shared-desktop compositor can loan a running native Wayland
window to a density-aware headless output. It does not reopen the document or
start another app/compositor. This is an experimental integration; ordinary
`enzui-hypr start` still preserves the prepared desktop stack.

## Build and isolation

1. Prepare the normal pinned desktop stack and apps.
2. Run `scripts/build-shared-compositor` under the build budget. This uses
   `.build/shared-desktop`, not `.build/desktop` or a running user's libraries.
3. Build `cargo build -p enzui-shell --bin enzui-remote-placeholder` under the
   budget. The image/controls use the pinned framework and its accessibility
   runtime.
4. Start a separately selected runtime with
   `ENZUI_HYPR_BUILDROOT=$PWD/.build/shared-desktop` and
   `ENZREMOTE_SHARED_DESKTOP=1`.

Prepare a separate stack and disposable runtime. Do not restart an existing
desktop or replace its compositor while applications are open.

## Transaction

The compositor's same-UID, mode-0600 `.enzremote.sock` owns the transaction.
Messages are newline-delimited JSON with `v: 1`. A persistent controller connection
owns the loan. The protocol is local, not an unauthenticated network API.

- `list` returns stable window IDs and bounded metadata; X11 and shell surfaces
  are excluded.
- `prepare` takes `window_id` and a physical `viewport` (`width`, `height`,
  `scale`, `refresh`). It returns a private snapshot path, capture handle and
  separate placeholder/input capabilities. The app is still in its desktop slot.
- A UID/PID-checked placeholder client uses `ui_attach`. It decodes the captured
  PNG before opening its first window on an inactive staging workspace.
- `activate` requires a mapped desktop-sized placeholder frame. The placeholder
  takes the desktop layout/stacking slot, while the actual app moves to
  `ENZREMOTE-APP` in the same compositor. Mobile decorations are scoped to the
  leased window. The remote seat is enabled after matching app buffers and the
  commit/settling barrier; hidden staging continues frame callbacks.
- `input_bind` authorizes a separate media input connection. It cannot select
  another window. Physical-seat keyboard/pointer focus is blocked from the
  leased surface tree. Verified same-client, parented native transients stay on
  its output, including eligible already-open dialogs, work-area fitting and
  input routing. Remote input does not invoke desktop shortcuts.
- `restore` revokes remote input, configures the same app on a desktop-scale
  staging workspace, waits for its desktop-sized buffer, then uncovers it in the
  placeholder's slot. The placeholder has an accessible **Resume here** action.
- Closing the owner or placeholder connection also restores. Input teardown
  sends Escape before releasing a held pointer button, so revocation can cancel
  a drag before release commits it.

`status` exposes phases and readiness, not capability secrets. `repaint` with an
explicit output name schedules a repaint without moving a cursor or changing
window geometry. A pending prepare
is bounded to eight seconds; resize/restore frame waits are bounded to four.
An unresponsive client ultimately gets its desktop geometry back even if a new
buffer did not arrive: status reports a frame-timeout reason. That degraded case
is not claimed visually seamless. Snapshot files and the temporary output are
cleaned after completion.

## Recovery and limits

Controller disconnect, placeholder loss and private-output loss initiate
restoration. Input-channel replacement releases held keys/buttons before binding
the new channel. A new connection cannot redirect a capability to another window.
The placeholder image must be decoded before exposure, and its frozen content
never accepts application input.

Slot replacement refreshes pointer hit testing without warping an unrelated
physical cursor. Tiling policy preserves saved floating geometry through the
placeholder. Changes across a responsive-layout breakpoint can still affect the
application's own widget focus; do not conceal that with a synthetic click.

The implementation permits one app loan per compositor. Source identity belongs
to that compositor process; server failure cannot preserve its Wayland clients.
The loaned app keeps its existing filesystem and desktop-service permissions:
this is not an application sandbox. Clipboard mirroring remains off.

Same-client parented transients can participate; unrelated portal dialogs are
not adopted. Multi-monitor hotplug, broader dialog patterns, cross-surface drag,
physical remote delivery and frame-by-frame visual continuity need explicit
[acceptance](validation.md). A geometry or protocol success alone does not
qualify phone input, latency or every dialog pattern.
