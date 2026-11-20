# OS service integration

Settings treats capability, observed state and permission as separate facts.
Unavailable services remain unavailable; unknown values are not invented zeros
or successful operations. Preview mutations require explicit opt-in and must
never target host services. [Installed startup](installed-session.md) provides
the session identity and authorization agents.

## Reviewed mutations

A review is bound to the original service owner, request, target and digest.
Replacement owners, navigation and expired reviews retire pending authority.
The UI sends an explicit mutation once, then observes its result. Lost replies
and uncertain outcomes require a fresh read/review, never automatic replay.
Closing a window or losing a connection cannot undo a committed operation.
Secrets use redacted storage and protected input channels, not arguments,
environment variables, settings files or public JSON status.

| Consumer | Service or source | Code |
| --- | --- | --- |
| Backups, recovery and memory policy | `org.enzimos.System1` | [System backend](../shell/src/system_settings/enzimos.rs) |
| Developer setup | `org.enzimos.Developer1` | [Developer client](../shell/src/developer_tools.rs) |
| Ports, processes and containers | `org.enzimos.DevMonitor1`, version 2 | [Dev Monitor](../shell/src/dev_monitor.rs) |
| Background app policy | `org.enzimos.Background1` | [Background client](../shell/src/background_apps/client.rs) |
| Flatpak overrides | Shared permissions worker | [Sandbox policy](../shell/src/sandbox_policy.rs) |
| Account administration | AccountsService and polkit | [Account client](../shell/src/user_accounts.rs) |
| Controller testing | Seat-authorized kernel event nodes | [Controller reader](../shell/src/controllers.rs) |
| Desktop game request | `com.feralinteractive.GameMode` | [GameMode client](../shell/src/game_mode.rs) |
| Device firmware | fwupd (`org.freedesktop.fwupd`, `fwupdmgr`) | [Firmware client](../shell/src/firmware.rs) |
| Location | GeoClue agent and `dev.enzui.Location` | [Location agent](../shell/src/location.rs) |
| Thunderbolt approval | bolt (`org.freedesktop.bolt`) | [Thunderbolt client](../shell/src/thunderbolt.rs) |
| Disks and partitions | UDisks2 | [Disks client](../shell/src/disks.rs) |
| Fingerprints | fprintd (`net.reactivated.Fprint`); lock screen PAM `enzui-fingerprint` | [Fingerprint client](../shell/src/fingerprint.rs) |
| VPNs added by hand | NetworkManager (`nmcli`) | [VPN forms](../shell/src/vpn_manual.rs) |

## Backups and recovery

Destination enrollment requires the advertised protected-input capability and an
opaque eligible candidate. Selecting a drive does not authorize formatting or
replacement. A read-only administrator review precedes password entry; enrollment
is a separate approved operation. New repositories require matching confirmation.
Enrollment never starts a backup, enables scheduling or confirms a recovery copy.

Protected input uses a private sealed memfd, with bounded single-line UTF-8 and
no trimming of passwords. Service responses must retain request, mode, owner and
plan-digest identity. Only explicit documented refusal/retry states permit another
manually submitted password. Leaving the page clears drafts and retires queued
input. See [the protected-input client](../shell/src/system_settings/backup_setup.rs).

Passive recovery-point reads can contend with mutations. Reads back off, pause
during forms/jobs, and an action requested during an existing read waits for it
before submitting once. Unsupported queries and failed supported queries remain
distinct. A backend `uncertain` result does not become a completed backup.

Retention writes use schema 31 and all six fields: `automatic`, `last`, `daily`,
`weekly`, `monthly`, `yearly`. `last` is 1–1000; calendar tiers are 0–1000.
Presets preserve the automatic-cleanup switch. Saving retention does not change
backup scheduling or claim files were removed. Unknown policies are not replaced
with editable guesses; legacy keep-last metadata remains display-only.

Recovery save/print uses `/usr/lib/enzimos-native/enzimos-policy backup-recovery`.
Password input is inherited as descriptor 3; printing receives a separate sealed
document descriptor 4. Parent descriptors retain CLOEXEC across concurrent app
launches. Password/document bytes never use public status channels. Saving or
printing a recovery key and confirming the user's copy are separate operations.

Memory protection displays configured policy separately from confirmed running
policy. Missing running-preset evidence is unconfirmed, not a mismatch or stopped
guard. Hardware and installed backend behavior require independent acceptance.

## Developer tools

Setup validates offered profiles, package lists and operation capabilities before
review. Transactions retain the reviewing owner's dedicated connection. Service
replacement requires a new review; removal preserves personal data. Downloading
packages uses a separate cache contract and never implies installation. Stopping
observation does not claim rollback after commit.

Dev Monitor performs visible-only bounded reads. Closing or changing views retires
old results. Process actions retain PID/start identity; container actions retain
full container and endpoint identity. Force stop has a separate confirmation and
cannot replace a stale target with a newly observed process. Logs are explicit
reads, unavailable metrics stay unknown, and suggested URLs are not reachability
probes. Read-only observations grant no mutation authority.

## Apps and permissions

Background-app observations distinguish native units from exact portal instances.
Minimized windows still count as windows; missing work observations remain
unknown. Shutdown and future-activation policy use separate reviews. Force quit
requires its own confirmation, and acknowledgements are followed by readback.
Search activation requires the independent `search_policy_adapter` capability and
per-app search permission; it grants neither agent nor tool consent.

Flatpak static access comes from the permissions worker's computed policy and
attributed layers. Settings offers only supported user-override changes and never
reimplements merge policy. Saved portal choices are separate: no saved entry is
not proof of denial or of a request never having happened. Forget removes the
stored decision; portal defaults determine later behavior.

Startup changes create user XDG autostart overrides for the next enzUI login.
System entries and unrelated groups stay untouched. Displayed review digests,
regular-file bounds and no-clobber publication guard writes. Adding an entry never
executes it. The installed session starts the systemd-generated target; no second
launcher is introduced. Session restoration remains a separate opt-in feature.

## Controllers and power

Controller discovery is event-driven; input is read only for the selected open
test. Seat access belongs to logind/udev. Vibration requires a verified active
seat and the exact device identity captured by that test. It is one bounded
rumble effect, not a persistent preference or calibration change. Potentially
blocking driver work stays in a cancellable child whose lease survives until
actual reaping; device loss, navigation and seat loss retire old completions.

GameMode distinguishes the desktop's own registration from game-owned activity.
Turning the desktop request off never unregisters another client and does not
activate an absent daemon. Changes require readback. User configuration updates
only selected general keys, preserving unrelated options and scripts.

Remote sharing warns from the protected per-account permission grant. This says
sharing is enabled, not that a viewer is connected. Locking revokes the session;
unlock does not resume sharing automatically. Listener, firewall, polkit and
per-account grant policy remain OS-owned. See [portal routing](installed-session.md#portals-and-remote-control)
and [compositor admission](privileged-clients.md).

## Firmware, location, Thunderbolt, disks and fingerprints

- **Device firmware** (Settings > Updates): devices and offered releases come
  from fwupd's D-Bus interface; "Check now" runs `fwupdmgr refresh` and Install
  runs `fwupdmgr update <device>`, which download, verify and ask polkit.
  Nothing is checked automatically.
- **Location** (Settings > Apps > Location): `enzui-location-agent`, supervised
  by the shell like the polkit agent, is the only GeoClue agent the enzimOS
  policy accepts. It registers only once GeoClue runs (it never starts it) and
  caps apps at city level (Approximate) or allows nearby Wi-Fi (Precise); Off
  keeps it unregistered, so GeoClue refuses every app. Settings changes the
  choice through `dev.enzui.Location` on the session bus, or saves it in
  `enzui-shell/location.json` when the agent is not running.
- **Thunderbolt** (Settings > Thunderbolt, only with a Thunderbolt controller):
  bolt's devices and security level; Allow enrolls a device (remembered), Forget
  removes it; bolt asks polkit.
- **Disks** (Settings > Disks): UDisks2 objects as disks, partitions and free
  space; Format, Create, Delete and Erase first unmount and lock what is open
  on the target themselves, because UDisks' `tear-down` option also edits
  fstab/crypttab and always needs an administrator. Disks holding the running
  system are read-only here; internal disks ask polkit for an administrator.
- **Fingerprints** (Settings > Users): enrollment through fprintd (polkit asks
  for the user's own password). The lock screen runs the `enzui-fingerprint` PAM
  service (pam_fprintd only) in a thread beside the password and shows the
  reader's messages; three wrong fingers end it until the next lock. Login after
  a restart, sudo and administrator approval keep the password.
- **VPNs**: hand-added L2TP/IPsec, IKEv2, Cisco IPsec, PPTP and OpenConnect
  profiles are built from checked fields (no separators or quotes) and added
  with `nmcli`; passwords are asked at connect time. The secret agent runs
  OpenConnect's own sign-in program (NetworkManager's auth-dialog protocol).


## Backups and previous versions (D-123)

- Settings › History & Backups shows "What is backed up" (skip rebuildable
  folders, a person's exclusions, removals still waiting for the backup drive)
  and "Space" (backup drive use and limit, earlier versions and system
  snapshots on this computer, the free-space floor, where the home's space
  goes and what is not backed up). It reads `org.enzimos.History1`
  (`history_client`) and the System1 `backups` query; a drive that is not
  connected shows "Connect … to back up" instead of a failure.
- The shell checks the backup status two minutes after login and every half
  hour (`backup_notices`): a missing drive is announced once per episode, a
  failure once, a week without a backup at most weekly, a too-small space
  limit once; success is silent. What was said is kept in
  `~/.local/state/enzui/backup-notices.json`.
