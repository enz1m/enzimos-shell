# Experimental shell plugin protocol

The wire reference. To write a plugin, start with
[Writing a shell plugin](plugins-authoring.md).

Package format and wire protocol 1 are experimental interfaces. Packages contain
bounded metadata and embedded Python source; installation and grants are
content-bound. Workers require the host's delegated sandbox and budget. The
bundler packages source without approving or executing it. See
[worker isolation and lifecycle](plugins-authoring.md#what-a-plugin-is).

The worker first sends `{"type":"hello","protocol":1,"plugin":"its-id"}` and
reads the host initialization/configuration. Subsequent newline-delimited JSON
frames are bounded to 64 KiB, with existing rate/deadline limits. Answer each
`ping` with a `pong` carrying the same `sequence`. `config` replaces the complete
typed configuration. `snapshot` replaces declarative UI and has a monotonically
increasing positive `revision`. Actions are checked against the current owner,
revision, element and enabled action; an action message contains a host-generated
one-use gesture. UI keys must be stable and unique. Do not log incoming content
or capability references.

## Requests

Each request has an increasing positive `id`:

```json
{"type":"request","id":1,"request":{"operation":"clipboard.subscribe","enabled":true}}
```

A response has `type: "response"`, the same `id`, and
`result: {"ok": true|false, "error": "..."}`. Error text is diagnostic, not a
stable machine-readable error enum. Guest-supplied permissions do not grant
capabilities; the installed approval and active run determine authority.

Implemented operations:

| Operation | Fields besides `operation` | Grant |
|---|---|---|
| `shell.open` | `view`: applications/desktops/controls/settings; `gesture` | `shell.behavior` |
| `shell.layout` | `element`, `section`: start/middle/end, `before`: ID or null, `enabled`: bool or null | `shell.layout` |
| `applications.dock` | `element`: declared local bar element ID, or null to release | `applications.surface` + `shell.bar` |
| `shell.theme` | `colors`: map of up to four opaque `#rrggbb` values | `shell.theme` |
| `notifications.subscribe` | `enabled`: bool | `notifications.read` |
| `clipboard.subscribe` | `enabled`: bool | `clipboard.read` |
| `clipboard.restore` | `blob`: opaque handle, `gesture` | `clipboard.write` |
| `blobs.release` | `blobs`: at most 128 opaque handles | no additional grant; same-owner resources only |

Layout writes are serialized against current placement and preserve peer changes.
Theme claims are exclusive, contrast-checked and revoked with their owner.
Empty theme colors release only the requesting owner's claim. No operation permits a command,
arbitrary filename, host socket, script callback or shell memory access.
No persistent-storage operation is defined.
Unsupported brokers fail. `shell.open` from an open plugin panel queues navigation
after that panel's animated exit. Its accepted request is not proof that a later
window mapped: a newer user intent or forced teardown can cancel the queued action.

## Notification provider

`{"type":"provider","data":{...}}` with `provider: "notifications"` publishes
read-only current previews, not a lossless arrival stream or historical archive:

- `event: "snapshot"`: `available` is boolean; `total` counts unexpired current
  entries; `items` contains at most eight newest entries; `truncated` indicates
  that current entries were omitted by this preview limit. Unavailable means
  empty items and a zero total, without selecting another session's service.
- Each item has an opaque display `id`, read-only replacement `version`, `source`
  (96 Unicode characters), `title` (128), `preview` (256), `urgency`
  (`low`/`normal`/`critical`), `unread`, Unix-millisecond `received_ms` and
  `updated_ms`, and `has_image`. No image bytes, sender/PID/principal, action IDs,
  reply authority or archive are exposed. IDs are not host action capabilities.
- `event: "paused"`: items are empty. Stop treating earlier snapshots as a live
  feed. The demo clears its view and ignores late snapshots while paused.

A successful request registers subscription intent, not proof that the backend
is available. Ordinary changes are sampled every 500 ms, equal projections are
coalesced, and a full channel retries the latest state rather than accumulating
updates. Explicit repeat subscribe refreshes the current snapshot. There is no
guaranteed delivery latency. Unsubscribe does not erase data already delivered
or revoke the grant; disable/remove the plugin or revoke its read grant to end
an untrusted worker's authority.

Ordinary declarative collections allow at most 16 children each and 160 nodes
overall. Group or paginate ordinary previews rather than exceeding that collection
limit. The explicitly opted-in compact `entry_list` below is the only larger root.
Display strings preserve legitimate Unicode joiners, but bidi overrides remain
invalid; display correlation IDs do not relax action or capability validation.

## Compact panels and entry actions

An optional `footer` array on a panel selects compact presentation. It shares the
ordinary 16-child/160-node/depth-6/key-uniqueness limits with `nodes`; an absent
footer retains the original titled panel. Compact panels omit the title/close
header and keep a **host-owned** origin button, labelled with the installed
manifest name and ID, which opens Plugins Settings. Guest footer nodes cannot
replace that origin control. Escape and outside clicks dismiss the panel; inert
interior clicks do not.

Additional node forms (all fields listed are required):

- `icon_action`: `type`, `key`, `id`, `label`, bundled `icon`, `enabled`.
- `icon_toggle`: `type`, `key`, `id`, `label`, bundled `icon`, boolean `value`.
  It renders one framework Switch with a decorative icon in its moving thumb,
  not a second click target. The current compact style uses a 44×24 track,
  16px thumb and 12px glyph; the guest supplies the icon appropriate to its state.
- `entry`: `type`, `key`, `label`, `thumbnail` (same-owner opaque handle or null),
  distinct action IDs `copy`, `delete`, `pin`, and boolean `pinned`. Primary entry
  activation and Copy send `copy` with no value; Delete sends `delete` with no
  value; Pin sends `pin` with `!pinned`. These remain current-revision, same-owner
  gesture-scoped actions. The host does not infer arbitrary clipboard authority
  from a row: restoration still requires the normal grant and valid handle.
  Trailing controls remain mounted and keyboard-accessible while paint is revealed
  on row/action hover or focus. Text is single-line/elided; thumbnails are bounded.
- Text `tone` also accepts `danger` for explicit failure feedback.

A panel may opt into `entry_list: true` only when `footer` is present. Its root
`nodes` must contain only Entry and Text nodes, with at most 128 entries and two
text rows (130 root nodes). Footer and nested ordinary collections retain their
16-child limit; the entire snapshot still has the 160-node, depth and 64KiB frame
bounds. The host renders text headers above a continuous virtual entry viewport
and keeps the footer outside it. Stable keys, unique actions, current revision,
owner and gesture checks are unchanged. Only visible/overscan row widgets and
image handles are realized; the bounded descriptors themselves are not fetched
page by page. Ordinary panels are not implicitly virtualized.

Entry hover surfaces have a 4px outer horizontal gutter, 8px content inset and
4px trailing-action inset. The 38px virtual pitch contains the 36px row surface.

Panel width/height remain bounded logical preferred dimensions. Snapshot size
changes resize the same panel, preserving its bar attachment. Placement is
serialized across cancellation/replacement; native size rules lock its current
size. Input is canceled/re-keyed during reflow and withheld until matching size
confirmation. A failed reflow reports an error and keeps the existing bounded
surface open, without claiming the requested geometry was applied.

These are extensions to experimental protocol1, not backward-compatible messages
for earlier hosts. Clipboard 1.3 requires entry-list support and supplies X/check
icons for paused/collecting state. The panel switch changes runtime collection,
not the persisted startup default. Existing installed packages are not silently
upgraded, restarted or granted new permissions; install an
updated package explicitly or use a new unused profile.

## Clipboard provider

Provider messages have `{"type":"provider","data":{...}}`. Every clipboard
payload has `provider: "clipboard"` and one of these `event` values:

- `status`: `state` is `collecting` or `paused`. This is subscription intent;
  successful subscribe registers the request, not proof that the asynchronous
  Wayland connection is ready. Connection failure arrives as `unavailable`.
- `entry`: `blob` is a 32-character opaque reference; `preview` is at most 240
  sanitized characters; `mime` is `text/plain;charset=utf-8` or `image/png`;
  `bytes` is the accounted original-plus-thumbnail size; `image` is boolean;
  `fresh` says whether this owner received a new handle rather than a duplicate.
  Do not extend the retention age on `fresh: false`.
- `removed`: `blobs` contains evicted/expired references. Remove their UI and
  actions. Old references cannot restore content or resolve images.
- `error`: `message` describes a rejected entry/resource operation. Rejected
  supported-format transfers (invalid UTF-8/PNG, timeout or size/read failure) carry
  no content and reach only readers authorized and collecting when capture began.
  Empty selections and unsupported/sensitive MIME offers remain quiet.
- `unavailable`: `message` describes transport failure. Discard history and
  pending UI state; all host references for that transport were released.

An image entry's same `blob` can be used as a declarative action `thumbnail` or
image-node reference. The host resolves it only for that live owner, without
allowing path/URL/file access. It returns raw bounded RGBA thumbnails to the
renderer; original PNG bytes remain host-owned for restoration. Text originals
also stay host-owned; the guest only needs the preview and restoration handle.

Pause retains history but stops new collection for that subscriber. A late or
resumed subscriber cannot receive an earlier in-flight capture. If another
reader already keeps capture active, a new subscriber starts with subsequent
captures rather than receiving that peer's previous history. Last-reader resume
may receive the current native selection. Primary selection is not collected.

`clipboard.restore` consumes a current gesture and requires a same-owner,
unexpired handle. Success follows a Wayland sync after setting the source, not
a simulated paste or proof that an arbitrary receiving application accepted it.
The owner must remain active. `blobs.release` is idempotent for missing/foreign
references and cannot reveal or release another plugin's content. Clearing
history does not clear the separately held, deliberately restored current
selection; owner revocation does release that owned source.

Clipboard pins an entry **to the top**, without exempting it from retention or
memory limits. Delete releases that entry; Clear releases all history, including
pins, but does not clear the separately restored current selection. Copy reports
failures without routine success chatter or automatic paste. Errors persist until
explicitly dismissed; normal provider updates do not erase them.

The demo enforces its configurable policy and releases references; the host
independently enforces per-owner/aggregate/item/age ceilings. History has no disk
persistence. Host limits and MIME checks are defined by
[clipboard content](../shell/src/clipboard/content.rs) and
[history storage](../shell/src/clipboard/history.rs).

## Application surface broker

```json
{"type":"request","id":1,"request":{"operation":"applications.dock","element":"applications"}}
```

At most one active plugin owns the application surface. Request it on `start`
and after `config`; null releases only the requesting owner's claim. The element
must be declared in the installed manifest. The broker reads the installed typed
configuration; the request cannot override preferences or carry commands, window
IDs, filenames or other application data.

Declare these configuration keys (the bundled example supplies defaults/ranges):

| Kind | Keys |
|---|---|
| toggle | `visible`, `show_running`, `current_desktop` |
| choice | `position` (top/bottom/left/right), `style` (island/panel/minimal), `visibility` (never/always/overlap/fullscreen) |
| integer | `icon_size` (12–64), `spacing` (0–24), `radius` and `highlight_radius` (0–32), `margin` (0–24) |
| applications | `pins` (up to 32 unique valid canonical application IDs) |

`visible` controls the surface within an enabled plugin; disabling the installed
plugin stops its worker and all associated surfaces. Window actions require host
UI gestures and revalidated process/window identity. Hover captures retain the
existing cancellation, 16-window, 650 ms per-window and 3 s batch limits. They are
rendered by the host and never published to the guest.

Configuration edits from Search and dock menus execute an atomic transaction
bound to package digest, enabled activation, live authority and current grant.
Edits read the latest configuration and preserve unrelated keys. Late replies
cannot replace a new owner or a newer configuration revision.

The optional default has an independent initialization marker: it is installed
disabled on an upgrade, preserving legacy appearance/favorites without honoring
legacy `dock.enabled`. Removal remains removal across subsequent starts. The
legacy `builtin.dock` layout item is discarded; the plugin's namespaced element
is governed by normal plugin placement controls.
