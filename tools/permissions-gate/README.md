# Public native sandbox permission probe

This fixture exercises the real Notification and Background desktop portals.
It refuses to run outside its own `org.enzui.PermissionsQA` Flatpak. Notification
IDs are short public labels; it never reads user files. Background mode prints
the portal's actual answer and, when allowed, stays alive for 120 seconds.
It does not claim that a successful request involved an interactive prompt.

Build on disk, with the disposable VM inactive, through the repository budget
wrapper. The host needs the GLib/GIO headers; the installed guest needs the
Freedesktop Platform 25.08 runtime already available. No SDK download is needed.

```sh
mkdir -p .build/permissions-gate/tmp
TMPDIR="$PWD/.build/permissions-gate/tmp" scripts/with-build-budget cc -O2 -Wall -Wextra -Werror \
  tools/permissions-gate/portal-client.c -o .build/permissions-gate/permissions-qa \
  $(pkg-config --cflags --libs gio-2.0)
```

In an **owned disposable installed guest**, mount that binary and `prepare.sh`
read-only at `/run/enzui-permissions-qa/`, then run `bash
/run/enzui-permissions-qa/prepare.sh` from its normal graphical admin Terminal.
The installer refuses another account/session or an existing fixture work tree.
It creates only a user app and explicitly unsigned **local file** remote. It
uses real Flatpak build/export/install commands, disables dependency downloads,
and requests network, PulseAudio and read-only Documents access for review.
An unknown metadata group is included to check that unsupported policy stays
visible. Do not use this fixture in a personal installation.

```sh
flatpak run --user org.enzui.PermissionsQA notify initial
flatpak run --user org.enzui.PermissionsQA background
```

Review it in Settings → Apps → App permissions. Check Flatpak's computed
permissions after changes; removing and restoring Documents must retain `:ro`.
Reset must remove only this app's user overrides. With Notifications disabled,
a new probe must report the portal's refusal. With it enabled, the notification
must appear. Forget should remove the saved entry, not assert denial or that
no request ever happened. Inspect the real PermissionStore without inserting
synthetic grants. Background availability, default policy and prompting depend
on the installed portal; record the returned answer and stored choice separately.

After any live probe has exited, remove only this owned user installation and
remote, restore the guest's saved idle policy, and **sync the guest filesystem
before stopping QEMU**. Verify the recorded QEMU PID/start identity and recursive
cgroup are empty. A forced QEMU stop without sync can lose recent cleanup writes.

```sh
flatpak --user uninstall --assumeyes --noninteractive org.enzui.PermissionsQA
flatpak --user remote-delete enzui-permissions-qa
sync
```

Keep raw screenshots, journals, per-account metadata and VM disks out of git.
