# Passive compositor admission gate

`probe.c` observes all five capture-manager families and both virtual-input
managers; it does not capture pixels or inject input. It requires ordinary UID
>=1000 and `ENZUI_ADMISSION_QA=owned-disposable-compositor`. Run only against an
owned disposable compositor, inside a capped managed unit with its own XDG
runtime and private session bus. Never target a personal desktop.

Build on disk, with the VM inactive and other heavy work stopped:

```sh
mkdir -p .build/admission-gate
wayland-scanner client-header \
  /usr/share/wayland-protocols/staging/ext-session-lock/ext-session-lock-v1.xml \
  .build/admission-gate/ext-session-lock-v1-client-protocol.h
wayland-scanner private-code \
  /usr/share/wayland-protocols/staging/ext-session-lock/ext-session-lock-v1.xml \
  .build/admission-gate/ext-session-lock-v1-protocol.c
scripts/with-build-budget cc -std=c11 -O2 -Wall -Wextra -Werror \
  -I .build/admission-gate tools/admission-gate/probe.c \
  .build/admission-gate/ext-session-lock-v1-protocol.c -lwayland-client \
  -o .build/admission-gate/probe
scripts/with-build-budget cc -std=c11 -O2 -Wall -Wextra -Werror -fPIC -shared \
  tools/admission-gate/loader.c -o .build/admission-gate/loader.so
```

`report` prints observed authority. `hold` keeps that connection open until stdin
closes or receives `q`; server disconnection is reported. `intent HELPER TTL
DELAY COUNT` announces a capture helper and starts one or two direct children,
covering one use and expiry. HELPER must be a separate inode. `control BODY`
exercises a raw admission request. `inherited` forks a socket-retaining child
and exits the original peer; `inherited-control BODY` does the same with a
control connection established by the original peer. Both must be refused after
that original exits. Their owned child identities must also be checked gone.

`lock` requests an actual protocol lock and waits for `u` to unlock. It paints no
lock surfaces and performs no PAM authentication: this is a revocation fault
fixture, never secure-lock certification. A 45-second process deadline does not
replace cleanup of its compositor if the fixture fails. `loader.c` is a harmless
library for explicit preload/audit refusal tests.

Use one managed scope at a time. Record exact PID/start/cgroup identities before
actions. Close fixture stdin, reap owned children, stop the exact compositor and
verify its identity gone. On success and failure, stop only the recorded managed
scope and inspect its recursive cgroup for remaining processes. Preserve the
existing user review. Keep raw identity logs and screenshots private.
