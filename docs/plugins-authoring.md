# Writing a shell plugin

A plugin adds an element to the bar (an icon and a short label) and, when it is
clicked, a panel of simple controls. It can also read the clipboard or
notifications, restore clipboard entries, recolour the shell or move bar items,
each only if the person installing it grants that permission.

Package format 1 is an experimental interface. Execution requires a delegated
desktop session with enforced worker limits. This guide covers development;
[the protocol reference](plugin-protocol.md) defines wire messages.

## What a plugin is

One package file: a JSON manifest with a Python 3 program embedded in it. The
shell never runs it in its own process. Each plugin runs in its own sandbox
(bubblewrap: no network, no view of other processes, a private filesystem, a
seccomp filter, memory and CPU limits).
The program talks to the shell over stdin/stdout, one JSON object per line.

It cannot run commands, open host files or sockets, or reach the shell's memory.
Everything it can do goes through the operations listed under
[Permissions](#permissions).

## Disable lifecycle

The Plugins enable switch has the same lifecycle for every plugin. Disabling
revokes its live authority, stops its sandbox worker and all descendants through
its owned cgroup, and removes its live UI and broker subscriptions. Host caches
and queued panel targets owned by that activation are released. Re-enabling
starts a fresh worker; saved configuration and placement remain available. The
shell also attaches the plugin-store file watch when that directory is created
after startup, so fresh profiles do not wait for the fallback poll to disable.

Cleanup does not depend on the guest accepting a shutdown message or SIGTERM.
The service cancels its actor and bounds shutdown/reaping; retired cgroups retain
their admission slot until they can be removed. Clipboard and notification
brokers release each owner's subscription and stop plugin-only watches/timers
when their last subscriber leaves. Shared desktop services remain available to
other users of those services.

Turning off a feature within an enabled plugin (for example, clipboard
collection or dock visibility) is a configuration change. To stop the plugin's
processes, use its Plugins enable switch. Ordinary desktop apps opened through a
host UI action retain their own lifetime when the plugin is disabled.

## Quick start

1. Copy an example: `cp -r examples/plugins/cpu my-plugin`.
2. Edit `my-plugin/manifest.json`: give it your own `id` and `name`.
3. Edit `my-plugin/main.py`.
4. Bundle it (this only combines the two files; it runs nothing):

   ```sh
   scripts/package-plugin my-plugin ~/my-plugin.enzplugin
   ```

5. Install it: Settings › Shell › Plugins › **Local package**, enter the
   absolute path, **Review package**, check the permissions it asks for, then
   **Install**. The element appears in the bar.
6. To update it, bundle again and review/install again; the installed copy is
   bound to the package's content, so an edited file needs a new review.

Settings › Shell › Plugins lists installed plugins with a switch to turn each
on or off. **Details** opens a dialog with its settings, **Review permissions**
and **Remove**.

## The manifest

```json
{
  "format": 1,
  "id": "cpu-meter",
  "name": "CPU meter",
  "version": "1.0.1",
  "description": "Total CPU use sampled from /proc/stat.",
  "permissions": ["shell.bar", "shell.panels"],
  "elements": [{"id": "usage", "section": "end", "before": "builtin.controls"}],
  "settings": [
    {"key": "interval_seconds", "label": "Sample every (seconds)",
     "kind": "integer", "default": 2, "min": 1, "max": 10}
  ]
}
```

- `id`: a short identifier like `cpu-meter`, unique among installed
  plugins; the hello message names it too.
- `elements`: the bar elements the plugin owns. `section` is `start`,
  `middle` or `end`; `before` is the id of the item it goes in front of
  (`builtin.applications`, `builtin.desktops`, `builtin.clock`,
  `builtin.notifications`, `builtin.controls`, …) or `null` for the end of
  the section.
- `settings`: shown in the plugin's Details dialog. Kinds:
  - `toggle`: `default` true/false;
  - `integer`: `default`, `min`, `max` (shown as a stepper);
  - `choice`: `default` and `choices` (up to 16 strings); optional `labels` maps
    every stored choice to a unique friendly display label.
  - `applications`: `default`, an array of at most 32 unique canonical `app:` IDs;
    displayed as favorites and edited through host application context menus.

  The current values arrive in `start` and in every `config` message.

## The program

The program's life is a loop over lines on stdin. The examples show the whole
pattern in under 150 lines; [`examples/plugins/cpu/main.py`](../examples/plugins/cpu/main.py)
is the simplest.

1. **Say hello** first: `{"type":"hello","protocol":1,"plugin":"<your id>"}`.
2. **Read `start`**: `{"type":"start","protocol":1,"config":{...},"grants":[...]}`.
   `grants` are the permissions the person actually granted.
3. **Send a snapshot** whenever what you show changes (see below). Each one
   has a `revision` larger than the last.
4. **Handle messages**:
   - `ping` → answer `{"type":"pong","sequence":<same sequence>}` promptly;
     a plugin that stops answering is stopped.
   - `config` → the complete new settings.
   - `action` → a click: `{"type":"action","element":"usage","revision":7,
     "action":"<node id>","value":<toggle value or null>,"gesture":"…"}`.
     The `gesture` is a one-use token; pass it with requests that need a
     user action (`clipboard.restore`, `shell.open`).
   - `provider` → data you subscribed to (clipboard, notifications).
   - `response` → the answer to a request you sent.

Write each message as one line and flush (`print(json.dumps(m), flush=True)`).
Lines are limited to 64 KiB. Never log what you receive: it may be clipboard or
notification content.

### Snapshots: what the bar and panel show

```json
{"type": "snapshot", "revision": 3, "elements": [{
  "id": "usage", "icon": "cpu", "label": "12% CPU",
  "tooltip": "Total CPU utilisation",
  "panel": {"title": "CPU meter", "width": 320, "height": 180, "nodes": [
    {"type": "text", "key": "now", "text": "12% CPU", "tone": "title"},
    {"type": "action", "key": "reset", "id": "reset", "label": "Reset",
     "detail": null, "icon": "rotate-ccw", "thumbnail": null, "enabled": true}
  ]}
}]}
```

`icon` names come from the shell's bundled icon set (Lucide names such as
`cpu`, `clipboard`, `bell`). Node types (every listed field is required):

| Node | Fields | Shows |
|---|---|---|
| `text` | `key`, `text`, `tone`: normal/muted/title/danger | a line of text (`danger` is a red banner) |
| `action` | `key`, `id`, `label`, `detail`, `icon`, `thumbnail`, `enabled` | a clickable row; a click sends `action` with this `id` |
| `toggle` | `key`, `id`, `label`, `value` | a switch; a flip sends `action` with the new `value` |
| `icon_action` | `key`, `id`, `label`, `icon`, `enabled` | a small icon button (footers) |
| `icon_toggle` | `key`, `id`, `label`, `icon`, `value` | a switch with an icon in its thumb (footers) |
| `image` | `key`, `blob`, `alt`, `width`, `height` | a picture you received as a blob (clipboard images) |
| `row`, `column` | `key`, `children` | grouping |
| `separator` | `key` | a divider |
| `entry` | see [the reference](plugin-protocol.md#compact-panels-and-entry-actions) | a clipboard-style list entry with copy/pin/delete |

Keys must be unique within a snapshot and stable across snapshots (they keep
focus and scroll position). Limits: 16 children per collection, 160 nodes, a
depth of 6. A `footer` array on a panel switches to the compact panel style
the clipboard example uses.

### Requests

Send `{"type":"request","id":<increasing>,"request":{"operation":...}}` and
wait for the `response` with the same `id`:

| Operation | Needs | Does |
|---|---|---|
| `clipboard.subscribe` (`enabled`) | `clipboard.read` | start/stop `provider` clipboard messages |
| `clipboard.restore` (`blob`, `gesture`) | `clipboard.write` | put an entry back on the clipboard after a click |
| `notifications.subscribe` (`enabled`) | `notifications.read` | start/stop notification snapshots |
| `shell.open` (`view`, `gesture`) | `shell.behavior` | open Applications, Desktops, Controls or Settings after a click |
| `shell.layout` (`element`, `section`, `before`, `enabled`) | `shell.layout` | move or hide a bar item |
| `shell.theme` (`colors`) | `shell.theme` | recolour the shell (up to four `#rrggbb`) |
| `blobs.release` (`blobs`) | — | free image handles you no longer show |

## Permissions

| Permission | Shown to the person as |
|---|---|
| `shell.bar` | Add bar elements |
| `shell.panels` | Show plugin panels |
| `shell.theme` | Change shell appearance |
| `shell.layout` | Change shell element placement and visibility |
| `shell.behavior` | Open shell views after a user action |
| `clipboard.read` | Read copied text and images in this desktop |
| `clipboard.write` | Restore clipboard content after a user action |
| `storage.persistent` | Keep private plugin data on disk |
| `notifications.read` | Read notifications in this desktop |
| `applications.surface` | Show application launchers, window controls and hover previews |

Ask only for what the plugin needs: the review screen lists every permission,
and a request without its grant fails.

## Testing and debugging

- `scripts/test-plugin-worker` checks that the sandbox works on this machine.
- A plugin that faults shows **Stopped:** and the reason under its name in
  Settings › Shell › Plugins. Turn it off and on to restart it.
- The examples cover the main patterns: `cpu` (a timer and a panel),
  `clipboard` (a provider, blobs, a compact panel with a footer, actions with
  gestures) and `notifications` (a provider and snapshots).

Plugins installed as Flatpak apps are described in [app-contributed packaging](shell-plugins.md).

## Optional application surfaces

[`examples/plugins/app-dock`](../examples/plugins/app-dock/) is bundled disabled.
It uses `applications.surface` together with `shell.bar` to request the host's
application dock renderer. Its position, size, appearance, visibility, running-app
filter and favorites are typed plugin settings. Choosing the main bar's edge
embeds its declared element in the bar; other edges use a transparent native
xdg-toplevel surface. These are xdg-toplevel surfaces, not layer-shell.

Any plugin can use this broker with the same configuration schema; its package
identity is not special. The host owns application discovery, icon resolution,
user gestures, focus/minimize/close, grouping, cycling and bounded hover captures.
The guest receives no window titles, screenshots, native handles or executable
commands. Disable, removal, fault or grant revocation ends the surface authority.
See [the wire contract](plugin-protocol.md#application-surface-broker).

Application surfaces also show `com.canonical.Unity.LauncherEntry` count and
urgency updates for exact `application://ID.desktop` identities. Counts use a
compact static badge (`99+` above 99); urgency uses a static outline, without a
frame loop. Multiple live publishers use the largest visible count and any
urgency flag. Sender disconnect removes that sender's state. Progress continues
through the OS activity bridge rather than a second progress presentation.

This listener exists only while an authorized application surface is enabled.
Disabling or losing its owner cancels the listener and clears metadata; pending
updates from that generation cannot repopulate it. Re-enabling waits for fresh
application updates. Position and icon-size changes retain live badge state.
The host does not forward badge contents or session-bus access to plugin guests.
