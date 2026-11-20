# Validation and release acceptance

Source tests, native visual review and installed acceptance establish different
facts. A passing fixture or older binary does not qualify a new build, complete
installation image or physical machine.

## Development checks

Follow [shell development](shell-development.md) for scoped iteration. With the
QA VM inactive and heavy QA stopped, run the precommit gate:

```sh
scripts/shell-iterate gate
```

The gate requires the QA VM to be inactive and serializes subprocess fixtures.
Run relevant launcher/staging scripts for packaging changes. Use
[specific native fixtures](../tools/probes/README.md) for compositor contracts and
[the mock greeter](greeter-design-preview.md) for login UI changes. Do not replace
native rendering/input checks with synthetic pixel fixtures or constant tests.

## Native review

Inspect actual screenshots, keyboard focus, pointer actions and real workspace
changes in both appearances. Check small outputs, scaling, window fitting,
reduced motion, interruption/reversal and relevant GPU/software rendering paths.
Screenshots establish visual behavior, not physical presentation latency or a
frame-rate guarantee.

QA must use explicit private runtime/configuration roots and verify PID/start,
executable, compositor and cgroup identities. Close only admitted fixture apps.
On success and failure, verify owned services, recursive cgroups and any headless
parent are empty. Preserve normal sessions, personal files and prepared binaries.

## Installed acceptance

Use a disposable installation with the exact staged graph and dependencies.
Verify the features actually being shipped:

- Greeter idle does not start PAM; submitted authentication, additional prompts,
  lock recovery and user switching retain their security boundaries.
- Account creation, initial/replacement passwords, avatar propagation and account
  removal use real authorization and preserve homes where requested.
- Search cold activation is tested only when the OS advertises its separate
  qualified capability, with revocation and service-owner replacement.
- Backup enrollment, backup completion, restore, retention and recovery-copy
  confirmation are separate checks; uncertain jobs never count as success.
- Remote view/control consent, capture/input admission, lock revocation, listener
  shutdown and fresh post-unlock authorization are exercised independently.
- Output fitting, primary-output changes, floating/tiling restoration, input and
  idle return are checked on the target rendering and hardware paths.

Missing capabilities remain unavailable. Installing a dependency into a QA overlay
does not qualify the unmodified image. A software rasterizer does not establish
physical-GPU acceptance; successful initialization does not prove recovery from a
later driver failure. Follow [login integration requirements](login-personalization.md)
for greeter dependencies and publication boundaries.

## Evidence and staging

Keep private screenshots, serial logs, VM disks and transient results in ignored
build/runtime directories. Store build receipts beside staged binaries and app
milestones; never relabel old bytes as a new source build. Exported source, pins,
locks, dependency licences and binary hashes must agree, with no unreleased
exceptions in a production manifest.

Keep reusable commands and durable contracts in these guides. Do not commit dated
run reports, issue-sweep ledgers, per-fix receipts or machine-local capture paths.
