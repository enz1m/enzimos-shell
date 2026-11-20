# Virtual controller vibration gate

`uinput.c` creates two public, disposable virtual controllers. It opens only
`/dev/uinput`; it never reads a physical controller. The target and control
have separate identities. Effect upload, play, stop and erase are logged with
public effect IDs. Any effect on the control fails the fixture. The target
accepts only FF_RUMBLE effects of at most 200 ms, no delay, and both magnitudes
at most 0x3000. It exits after five minutes or SIGTERM and destroys its devices.

Build on disk with the VM off, using the repository budget:

```sh
mkdir -p .build/controller-vibration-gate/tmp
TMPDIR="$PWD/.build/controller-vibration-gate/tmp" scripts/with-build-budget \
  cc -std=c11 -O2 -Wall -Wextra -Werror tools/controller-vibration-gate/uinput.c \
  -o .build/controller-vibration-gate/uinput-qa
```

Run only inside an owned disposable enzimOS guest, from an ordinary account
that already has OS-granted uinput access. The fixture requires
`ENZUI_CONTROLLER_QA=owned-disposable-guest`. Do not chmod input nodes, add an
input group or install shell-owned udev rules to make a test pass.

In real installed Settings → Controllers, select **Test** on **enzUI Vibration
QA**, then **Test vibration**. Check upload → play → stop → erase in its log;
the control must have no effect events. A successful API call is not proof
that physical motors vibrated. Keep that distinction in qualification reports.

`--reject-upload` returns ENOSPC. `--hold-upload` deliberately leaves the target's
upload unanswered; SIGUSR1 releases it. This covers deadline, page-close,
search and seat-loss checks. A kernel upload can defer process exit after
SIGKILL. Verify that Settings keeps the job in Stopping and refuses another
request until the exact worker is reaped; release the fixture before asserting
cleanup. Use the responsive fixture for the parent-death check. Capture exact worker PID/start identity and its selected input FDs;
after cancellation verify that identity is gone and that device descriptors
are closed. Do not kill by process name. Check an inactive/locked session,
device removal, reopening/late callbacks and quiet completed jobs. Inspect the
actual UI in both appearances and at small resolution.

Stop the fixture's recorded managed unit on success and failure. Verify its
recursive cgroup and recorded identities are empty, and its two sysfs devices
are gone. Restore the guest's original settings, sync, then stop and verify the
owned QEMU identity/cgroup. Never stop unrelated sessions or a normal desktop.
