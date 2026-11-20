# Greeter artwork and prototype type assets

`scene.png` is original abstract artwork generated with OpenAI image generation.
It contains no UI and can be selected as a custom background. It is not the
current embedded fallback; that image is `assets/wallpaper.png`.

`type/` preserves the source metadata, numeral outlines and date face for the
Prism, Muse and rounded typography prototypes. Source URLs, hashes and variable
axes permit reproduction with `scripts/generate-greeter-type`; the adjacent SIL
Open Font License notices cover those derivatives.

The current clock and date use selected font outlines from
`shell/src/lock_fonts.rs`, not these prototype outlines. Bundled family sources,
weights and licences are recorded in `shell/assets/lock-fonts/sources.json` and
reproduced with `scripts/generate-lock-fonts`. See
[login personalization](../../docs/login-personalization.md).
