# Iterating on the login screen

```sh
scripts/preview-greeter-cu
```

This opens the production greeter view inside a GPU-backed CU desktop window.
Nothing is installed. Alex Morgan and Sam Rivera are mock accounts; the password
is **`preview`**. Sign-in, desktop launch, sleep, restart and shutdown are
simulated inside the greeter preview. Host sessions are preserved.

- **F1** animates back to idle; **F2** wakes the sign-in form.
- **F3** shows an error; **F4** plays the success fade; **F5** resets the fixture.
- **F6** captures the greeter window to `.build/greeter-preview/preview.png`.
- **F7** cycles Glass, Frosted glass and Simple in the isolated review profile.
- **F8** hides/shows a comparison pill beneath an idle glass clock at 600×700 or
  larger. It uses the clock shader and material recipe; it is preview-only.
- **F9** switches between the wallpaper and a synthetic comparison backdrop,
  making inward/outward distortion easier to compare without reflective artwork.
- **F10** cycles plain dark, plain light and the previous backdrop, for checking
  the glass contours without a busy image. F9 also exits the solid comparison.
- Tab/Shift+Tab move focus. Arrow keys move among power actions; Escape closes
  the popover while preserving the password. Outside clicks also dismiss it.
- Typing or clicking wakes the form. The keyboard button opens the on-screen
  keyboard with the account avatar retained beside the name.

The default clock uses rounded Fredoka numerals with adaptive glass. Material,
font and layout can be selected independently in Settings. See
[login personalization](login-personalization.md) for the appearance and package contracts.

Time and date travel together: the idle composition contracts and moves up as
sign-in wakes, then expands back into the scene on returning to idle. The form
arrives after the clock has made room. Opening the keyboard moves the clock to
a compact header; very short, narrow windows prioritize the input controls.
The power popover opens above its button with a short fade and upward motion,
and retains an inert surface while closing. No perpetual animation runs at rest.
The backdrop follows the selected desktop or custom wallpaper, including solid colours.

## Editing and live preferences

```sh
scripts/preview-greeter-cu reload
scripts/preview-greeter-cu settings
scripts/preview-greeter-cu settings --page users
scripts/preview-greeter-cu cu shot .build/greeter-preview/review.png
scripts/preview-greeter-cu reload --no-build --state keyboard --size 800x600
scripts/preview-greeter-cu stop
```

Settings → Date & Time → Login screen selects material, font and weight, layout,
background, size and separate time/date toggles. The preview follows the isolated profile at
`.build/greeter-preview/settings.json` live. Settings is built if missing; after
editing its code, rebuild with `scripts/shell-iterate build settings`.
Installed account Settings use the same controls for that account's lock screen.
Pre-login uses the shared default; these controls do not configure all accounts.
The ordinary `scripts/preview-greeter` window remains available as well.

Set `ENZUI_CU` to the executable computer-use harness. The launcher uses a
private runtime and
configuration directory, loopback VNC port 5917, and a 1400 MiB service cap for
the nested compositor, VNC server, greeter and optional Settings. CU input and
captures stay inside this desktop. The launcher does not edit host configuration.

Closing/reloading this review also closes its Settings app. Stop validates the
recorded worker PID/start identity and verifies recorded processes and the
recursive cgroup are empty. Startup failure also cleans up its owned parent.
The CU pass-through accepts capture, input and status commands; app launches stay
inside the service. The greeter launches directly there because CU's generic app
launcher has a 64 MiB file limit that affects Vulkan's memory-backed driver files
on this machine.

## Design assets and extension

`lock/src/clock_widgets.rs` owns presentation; the appearance model is shared in
`shell/src/session_preferences.rs`. `shell/src/lock_fonts.rs` loads bundled OFL
families and reads installed outlines, keeping digit cells fixed. Font provenance,
licences and deterministic generation live in `shell/assets/lock-fonts/` and
`scripts/generate-lock-fonts`. No dynamic scene code is loaded into the greeter.

The preview binary and its development controls require the `greeter-preview`
Cargo feature. Both preview launchers enable it explicitly; release builds omit
it. The nested Settings window uses sample accounts and a private theme directory.

Reference reading: [Apple lock-screen customization](https://support.apple.com/en-ie/guide/iphone/iph4d0e6c351/ios)
and [Material 3 Expressive](https://blog.google/products-and-platforms/platforms/android/material-3-expressive-android-wearos-launch/).
No Apple/Google UI code or proprietary artwork is copied.

Use [validation](validation.md) for native and installed acceptance.
