# Shell languages

Settings → System → Date & Time → Language offers English, Español and Follow
system. The choice is stored as `language` (`en-US`, `es` or `auto`) in the shell
settings file. Settings, the shell and the greeter preview refresh it without a
restart. Lock screens and helper applications read the same profile on startup.
The installed greeter uses its own profile or its system language preferences;
it does not read private files from unauthenticated accounts.

Follow system uses the framework's `LANGUAGE`, `LC_ALL`, `LC_MESSAGES` and `LANG`
negotiation. A C/POSIX locale disables `LANGUAGE`. Missing translations fall back
to English. External application names, account names, file names and previously
received notifications remain data supplied by their owners. Desktop entry names
and comments use the selected locale's translated fields when available.

## Maintaining translations

`shell/src/i18n.rs` uses `enzui_core::localization`: Fluent catalogues,
`Localizer::view` for root views and overlays, and CLDR date/time formatting.
`shell/src/locales/en-US.ftl` and `es.ftl` contain only the shell's messages.
The framework supplies its own Spanish widget messages and language fallback;
do not copy its catalogue into the shell.
These resources are embedded, so deployment needs no translation download or
installed Spanish libc locale.

Keep message IDs and argument names stable. Pass inserted values as Fluent
arguments; keep plural forms in the catalogue. Cached display content must
include the localizer generation or be rebuilt when the language changes.
Static metadata borrows from bounded immutable catalogue caches; no strings are
leaked on language changes. Controls retain owned `Localizer::text` results;
input values remain borrowed. Give translated controls explicit, stable IDs.

Control IDs, settings keys, shortcuts, D-Bus methods and compositor commands
are independent of display labels. Never use a translated string as a command,
file path or serialized value. Installed agent declarations use a scoped English
localizer so their runtime metadata matches the packaged declaration. Catalogue tests exercise both languages,
formatting arguments, Spanish dates and canonical shortcut round trips.

The standalone wallpaper renderer uses the same framework locale formatters
for named date fields in wallpaper templates. Its parent supplies `ENZUI_LOCALE`
and restarts the renderer when the selected language changes. Greeter rendering
keeps its existing sandbox and public-data restrictions. The renderer patches
only `iced_core`, `iced_futures` and `iced_runtime`, as documented in the
[framework's standalone consumer guide](https://github.com/enz1m/enzui/blob/a53869d67a11171b8c7ee0fa1dad164af76087d8/docs/localization.md#standalone-core-consumers).
