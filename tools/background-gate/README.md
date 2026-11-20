# Native background shutdown fixture

`lifecycle-client.c` is a public timer implementing
`org.enzimos.ApplicationLifecycle1` for a disposable enzimOS guest. It owns
`org.enzui.BackgroundQA` and accepts calls only from the current unique owner
of `org.enzimos.Background1`. It requires UID >= 1000 and
`ENZUI_BACKGROUND_QA=owned-disposable-guest`; it refuses Flatpak execution.

Build on disk, with the VM inactive and the normal build budget:

```sh
mkdir -p .build/background-gate/tmp
TMPDIR="$PWD/.build/background-gate/tmp" scripts/with-build-budget sh -c \
  'cc -std=c11 -O2 -Wall -Wextra tools/background-gate/lifecycle-client.c -o .build/background-gate/enzui-background-qa $(pkg-config --cflags --libs gio-2.0)'
```

Install only in an owned disposable guest, as a root-owned executable and
app-specific systemd user unit with `Restart=no`, `KillMode=control-group`,
`RuntimeMaxSec=1200`, `MemoryMax=32M` and `TasksMax=8`. A root-owned Background1
manifest must declare that exact executable and
`graceful_stop: application-lifecycle-v1`. The lifecycle object is
`/org/enzui/BackgroundQA`. Finish waits five seconds; cancel waits half a
second. Every accepted request logs its public mode. `--idle-control` runs the
same executable without registering a lifecycle interface; give it a different
unit and no manifest to check that another process is untouched.

Use the real installed Settings and Background1 service, without substituting
a bus provider or bypassing the service's trusted-client checks. Record PID,
start ticks, executable digest and cgroup before each action; an acknowledgement
alone does not prove exit. Check that the original process and recursive cgroup
are gone, and that the control process kept its identity. Stop both owned units
on every exit path, remove their installed fixtures, restore any saved settings
and policy bytes, and sync before stopping the VM. Never target a process by
name alone.

This fixture qualifies the public protocol and desktop controls. It does not
qualify the framework lifecycle SDK, sandbox admission, file-copy cancellation,
work inventory, or tray reporting; those need their own native integrations.
