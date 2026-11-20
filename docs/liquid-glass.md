# Liquid glass

In-app material samples the application's own painted backdrop. Material that
shows the desktop or another window requires compositor sampling. The material
protocol never returns another client's pixels to the application.

## Client protocol

[enzui-material-v1.xml](../protocols/enzui-material-v1.xml) exposes one material
object per Wayland surface. `set_regions` declares up to 32 rounded rectangles
in surface-local logical pixels, framework material tokens and tint. Regions are
double-buffered and apply on the next surface commit; an empty list removes them.
The client keeps those regions transparent and draws its own content above them.

Version 2 reports per-region luminance after asynchronous backdrop measurement.
The framework probe samples the inner filtered backdrop. `set_measure_only`
allows a client to select readable ink before displaying glass. Clients receive
brightness metadata, not captured images.

Readback ownership includes material lifetime and committed region/size revision.
Resize, replacement and destruction invalidate old samples. The compositor bounds
pending work globally and per material, limits spare buffers and expires stalled
readbacks. Probe failure stops measurement without stopping material painting.
See [the readback policy](../compositor/MaterialReadback.hpp).

## Rendering and ink

[Material.cpp](../compositor/Material.cpp) runs before window content. It copies
the padded backdrop, applies framework filtering and material composition, then
measures luminance. Padding covers filter reach, refraction, dispersion and
shadow. Damage intersecting retained glass repaints its backdrop to avoid sampling
last frame's composite. The shadow is drawn after the source copy.

`scripts/build-material-shaders` translates pinned framework WGSL to GLSL ES with
naga. Uniform blocks match the framework and ENCODED is enabled for the
compositor's sRGB-encoded framebuffer. Fading windows composite glass at window
opacity. Hyprland's own blur is disabled for material windows to avoid filtering
the backdrop twice.

[shell/src/glass_ink.rs](../shell/src/glass_ink.rs) applies the framework's
hysteresis, dwell, initial settling and crossfade rules. Adaptive styles choose
light ink over dark backdrops and dark ink over light ones. Content ink and glass
tint must change together; styles with fixed or theme-owned ink retain that policy.
Reduced motion settles optical displacement through the framework material policy.

Settings > Shell > Placement offers a Shell material choice: Solid or Frosted
glass. Floating panels, bar islands and the sidebar share this preference;
panels attached to the continuous frame remain solid. Frosted glass uses a
stronger dark tint, a lighter translucent fill in light mode and a broad blur
with a quiet outline. Unsupported surfaces paint a solid fallback.
Application window material and the greeter's
[clock/control material](login-personalization.md) are independent preferences.

## Application integration

[desktop_glass](../shell/src/desktop_glass.rs) demonstrates the client path:

1. Create a transparent window and bind its reachable Wayland surface.
2. Wrap the view with `window_glass` and mark pane bounds with `glass_pane`.
3. Send regions during drawing and request another frame when material availability
   or ink differs from the last drawn state.
4. Paint a solid fallback when the protocol or chosen style is unavailable.

The compositor can sample only content below the window. A pane over the app's
own content uses in-app material instead. `bleed` extends material under an opaque
neighbour, hiding the optical rim along their shared seam.

Build the sample explicitly under the normal budget:

```sh
scripts/with-build-budget cargo build --offline --locked -j1 -p enzui-shell --bin enzui-glass-sample
scripts/build-glass-demo
```

`enzui-glass-sample` switches style/appearance live. `glass-demo pattern` and
`glass-demo ticker` supply static and changing backdrops. Run these only inside
an owned preview. `scripts/test-liquid-glass` and
`scripts/test-material-readback-live` are native gates requiring a prepared
material-enabled runtime; read their environment requirements before running.
Never load a newly built plugin into an ordinary user desktop.

## Glyph material and limits

The clock uses the framework’s
[`adaptive_glass`](https://github.com/enz1m/enzui/blob/a53869d67a11171b8c7ee0fa1dad164af76087d8/crates/enzui-widgets/src/adaptive_glass.rs)
widget with `GlassSilhouette` font outlines and the `Clear` or `Frosted` preset.
Each connected component has its own lens center, smooth optical tails and shared
ambient adaptation. Frost changes diffusion while preserving the lens. The widget
owns backdrop capture, multi-instance GPU resources and material-policy fallback;
[clock_widgets.rs](../lock/src/clock_widgets.rs) owns layout and clock semantics.

Layer surfaces have no equivalent per-layer material pass here. Subsurfaces and
popups do not receive independent regions. Translated scene redraws retain their
glass; unsupported scale/rotation redraws omit it. Frame-joined shell panels stay
solid. Software rasterizers may fall back to opaque panels rather than run costly
filters. Qualify actual rendering using [the validation guide](validation.md);
configuration flags alone do not establish blur or optical parity.
