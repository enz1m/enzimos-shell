# Login and lock screen appearance

Settings → Date & Time → Login screen keeps material, typeface, layout, size,
visibility and background independent. Settings draws a small layout illustration,
not a second greeter or an authentication surface.

- **Glass** uses the approved clear adaptive lens; **Frosted glass** uses the same
  lens with diffusion. **Simple** uses flat numerals and opaque, dark accent-tinted
  controls. White control ink remains readable even with a light desktop theme.
- **Centered** is the large horizontal clock; **Duet** places stacked hours and
  minutes on the right and sign-in on the left; **Portrait** uses a centered
  stacked clock; **Horizon** balances a left clock and right sign-in. Narrow
  screens center sign-in, and keyboard presentation takes priority over a tall
  clock. Clock and date travel together on wake and return to idle.
- Rounded favourites are Fredoka, Nunito, Enz Pebble and Rubik. The font dropdown
  starts with these favourites, followed by all system families; each name previews
  its own typeface. Weight choices reflect the face's actual weights, including
  variable weight ranges. Selecting “Fredoka (default)” restores Fredoka 600.
  Numerals use fixed digit cells to avoid movement as the minute changes.
- Background follows desktop wallpaper unless an image is chosen. Images pass
  through the existing bounded importer and become private canonical PNGs.
  The image picker also serves avatar selection; cancelling it does not change
  the desktop wallpaper.

Old Prism/Muse preferences migrate to Simple; old Glass/Frosted Glass choices
retain their material. Retired identifiers remain deserialization aliases only.
Changing one appearance axis preserves the others and unrelated settings.

## Accounts and profile pictures

Settings → System → Users uses AccountsService over the system bus. Users can
edit their own name and picture. Administrators unlock other-account controls
through polkit; service calls request interactive authorization again whenever
policy requires it. Account operations retain their system-bus authorization subject, and closing
Settings waits for a
pending account change to finish.
New accounts receive an initial password chosen by the administrator. The two
secure fields stay masked by default, keep accessibility nodes valueless, and use
shared wipe-on-drop storage with redacted formatting. Leaving Users, cancelling a
form or submitting a change clears its secure drafts. No password is written to
settings, command arguments or environment variables.

An unprivileged `enzui-password-hash` helper receives the password only on stdin,
uses the system libxcrypt default algorithm with fresh OS entropy, disables core
dumps and wipes its buffers. Only the salted encrypted hash goes to AccountsService
`SetPassword`. The async owner bounds output, times out and reaps the helper.
Missing password preparation fails before creating an account. Administrators can
set another local user's password (this explicitly enables sign-in), change account
type, disable sign-in, or remove an inactive account while preserving home files.
The current session and the last unlocked administrator are protected.
Organization-managed accounts are read-only.

Forced first-login/reset-only password changes are unavailable in this UI.
Supporting PAM `NEW_AUTHTOK_REQD` requires a matching greetd password-change
conversation; normal initial and replacement passwords remain supported. The greeter client supports empty starts and multiple PAM
prompts without supplying an unseen new-password challenge. Pending imported
accounts must receive a regular password before their sign-in availability can
be toggled; this avoids AccountsService's empty-password unlock failure.

The greeter reads bounded cached pictures from
`/var/lib/AccountsService/icons/<username>` rather than reading user homes.
Absent pictures use Nunito initials centered from their actual outline bounds,
with a deterministic muted gradient. Unknown glyphs use a person silhouette.
Photos use bounded, cached square thumbnails. Native image radius clips them to
a circle on both GPU and software renderers.
Account operations are isolated sample operations when
`ENZUI_SYSTEM_SETTINGS_DEMO=1`; the nested review sets this automatically.

A signed-in user's lock uses their session preferences. A pre-login greeter runs
as a separate account and uses its own shared defaults. Public per-user
appearance/media publication requires a trusted OS integration contract.
Do not make private home directories readable to solve that boundary.

## Animated wallpapers and package metadata

A package is declarative `scene.toml` plus bounded assets, with built-in renderer
primitives. It never supplies an executable, plugin, shader or a login input
surface. Tag any clock/date component, including decorative backing shapes:

```toml
[[layer]]
kind = "text"
role = "clock"
text = "{time:%H:%M}"

[[layer]]
kind = "text"
role = "date"
text = "{date:%A, %-d %B}"
```

Roles are `decoration` (default), `clock`, `date`, or `clock_date` (combined content
hidden when either role is hidden). Legacy time/date templates infer their role;
shipped packages also carry explicit metadata. Settings → Appearance → Wallpaper
can hide clock/date on the desktop. Login-only switches leave the desktop alone.
The greeter hides its own clock/date independently when the visible scene supplies
them, avoiding duplicate clocks. Reduced motion freezes scene motion while wall-clock text can still update.

`enzwall run` accepts `--hide-clock` and `--hide-date`. `enzwall stream` emits the
`ENZWALL1` header, little-endian width/height/role flags, then fixed-size opaque
RGBA frames. Greeter output is at most 960×540 and 15 fps (1 fps with reduced
motion); output sizes are checked before allocation. No scene variables reveal
host/user/resource readings in public mode.

`lock/src/live_background.rs` owns the child and reader lifetimes. It launches the
trusted renderer through bubblewrap and prlimit, with a new PID/user/network
namespace, read-only system libraries/fonts, device metadata and one scene, a private temporary
filesystem, and GPU render nodes. It gives the child no home, session bus or
Wayland socket. Limits include 512 MiB data, 8 GiB virtual address space (GPU
reservation), 128 descriptors, scene text 256 KiB, fonts 16 MiB each, and scene
images 64 MiB total decoded, 4 megapixels per text texture and 32 MiB total text textures. The enclosing desktop/greeter service must also cap
memory and tasks. Failure or absence of the sandbox falls back to the embedded
still; it never starts an unsandboxed package process. Dropping the feed kills and
reaps its owned child and joins its reader.

Install trusted packages under the prepared platform's `enzwall/scenes/<id>`.
IDs are bounded simple names; greeter scene resolution rejects traversal and
scene-directory symlink escape. User package import/publication is an OS-managed
boundary, not arbitrary greeter access to a user-writable home directory.

## Installed integration

The OS must bundle AccountsService, polkit and its session agent, Python Pillow
for the bounded image importer, bubblewrap, util-linux `prlimit`, and the
`enzui-password-hash` executable linked against libxcrypt. A dependency installed
only in a disposable QA overlay does not qualify the installation image.

Verify GPU and software rendering independently, including Settings card expansion,
scrolling and cached text/images, and native circular photo clipping. Account authorization,
password sign-in, avatar publication, removal and preserved homes need real
installed acceptance. Public per-user appearance must not expose private homes.
See [validation](validation.md) for the release process.

## Fonts and release builds

`shell/assets/lock-fonts/sources.json` records pinned Google Fonts source revisions,
SHA256 hashes, available weights and OFL notices. `scripts/generate-lock-fonts`
rebuilds deterministic static Latin/punctuation subsets using fontTools. The
Quicksand derivative is named **Enz Pebble** to honor its Reserved Font Name.
Full installed fonts supply other scripts and additional families.

The mock greeter binary requires the explicit Cargo feature `greeter-preview`.
Normal `scripts/build-release` builds omit it. F-key scene/background probes,
mock passwords, mock sessions and comparison pills live only in that feature.
The native greeter has no development control strip. Nested PAM/greetd
password fixtures also require this compile feature; installed production binaries
do not read `ENZUI_LOCK_TEST_PASSWORD`.

`shell/build.rs` builds the unprivileged password helper for native Linux.
`scripts/build-release` stages it beside the app binaries and records its hash.

The clock uses the framework’s `adaptive_glass::enz_adaptive_glass` with its
`Clear` and `Frosted` presets. `lock/src/clock_widgets.rs` supplies cached font
silhouettes, animation geometry, fallback colors and accessibility labels. Simple
material uses the native image widget with a bounded, cached raster so SVG
resize rounding cannot shift numeral cells during animation. Framework and ICED
revisions are pinned in `sources.lock.json`.
