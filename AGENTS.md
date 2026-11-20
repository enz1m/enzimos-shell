# EnzimOS Shell

This is the desktop, Settings, lock screen, greeter and Hyprland integration
checkout. Start with README.md, docs/shell-development.md and TESTING_RULES.md.
Keep improvements to shared widgets in enzUI. Shell apps build and package from
their own repositories and are launched through their installed desktop entries.

Use the submodule and Cargo pins in sources.lock.json and Cargo.lock. Run
scripts/setup-sources after updating a pin. Iced's checked-out source also
supplies material WGSL and font notices. Update wallpaper scenes in Enzwall and
advance its gitlink after the engine change is tested.

Build the affected target with scripts/shell-iterate. Use a private prepared
compositor root and XDG runtime for native checks. Keep any prepared roots used by
an active session. Verify file ownership and process identity before cleaning up
native fixtures. Preserve ordinary user sessions and their configuration. Use
--exit-with-shell for a disposable managed review after explaining that it also
closes the apps launched by that review.

Run the full serial fixture gate before merging coherent work. Prefer one
durable regression test per failure boundary and check visual changes in the
native preview. Use comments for durable contracts, preserve licensing, and keep
credentials, private captures and user data out of Git.
