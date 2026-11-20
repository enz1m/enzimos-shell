# Documentation

Start with the [project overview](../README.md) and [script entry points](../scripts/README.md).
These guides describe supported interfaces and current workflows; implementation
history belongs in Git rather than dated checkpoint documents.

| Topic | Guide |
| --- | --- |
| Build, iteration and tests | [Shell development](shell-development.md) |
| Release and native acceptance | [Validation](validation.md) |
| Installed startup, lifetime and packaging | [Installed session](installed-session.md) |
| OS service consumers and authorization | [System services](system-services.md) |
| Login appearance, accounts and wallpaper packages | [Login personalization](login-personalization.md) |
| Languages and translation catalogues | [Localisation](localization.md) |
| Mock login UI review | [Greeter preview](greeter-design-preview.md) |
| Capture and virtual-input admission | [Privileged clients](privileged-clients.md) |
| Existing-window remote handoff | [Shared desktop](shared-desktop.md) |
| Compositor and glyph glass | [Liquid glass](liquid-glass.md) |
| App search integration | [Search providers](search-providers.md) |
| Plugin development | [Authoring](plugins-authoring.md), [protocol](plugin-protocol.md) |
| App-contributed plugin packaging | [Shell plugins](shell-plugins.md) |

Keep one canonical guide per contract. Update it when behavior changes, check its
commands against retained tools, and document limits without implying that a past
fixture run qualifies a new binary or installation image. Private captures and
run logs stay under ignored build/runtime directories. Build receipts travel with
the binaries they describe.
