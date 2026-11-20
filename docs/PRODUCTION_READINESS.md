# Release readiness

The Shell is in development. Use the exact dependency and submodule revisions
from the release candidate for validation. [Source pins](../sources.lock.json)

## Source and native checks

Run `scripts/shell-iterate gate` for strict Clippy and the Shell and greeter test
suites. Run the launcher and staging checks when changing the export. Prepare
the pinned compositor and portal together and check the compositor and shader
paths in a disposable Wayland session. [Validation guide](validation.md)

## Installed checks

- Qualify first-login password changes and account/avatar lifecycle in the native
  greeter, including user switching, lock recovery and logout.
- Test automatic backup retention, history, and file and folder restore through
  the installed Settings UI.
- Exercise remote viewer counts and disconnection on lock or logout using a
  paired OS runtime.
- Check both appearances, output changes, GPU rendering, suspend and resume,
  keyboard focus and assistive technology on an installed system.
- Verify the export's dependency and licence closure and build receipts before
  assembling an installation image.

Check the workflows that apply to the build being shipped. A mock greeter or
nested compositor is suitable for development review. Installed authentication,
hardware behavior and recovery need their own acceptance checks.
