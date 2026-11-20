# Dependency licence notices

`licenses/` preserves upstream notices for evdev and its locked Rust dependencies.
Versions and archive checksums are recorded in `Cargo.lock`; sources are fetched
from crates.io. `export-desktop` includes these notices in the package licence
folder. Framework, compositor, application and font notices accompany their own
exported components.

The shell uses evdev's safe APIs for selected-controller input and bounded rumble
testing. Device access remains seat-authorized by the OS; the controller code
does not change permissions or introduce unsafe Rust. See
[controller integration](../docs/system-services.md#controllers-and-power).
