# Native compositor QA

These tools exercise current geometry, input and lifecycle contracts. They do
not start against the host desktop automatically. Native reviews require an
explicit disposable `ENZUI_QA_RUNTIME`; scripts recheck scope/process identities.

| Entry point | Coverage |
| --- | --- |
| `scripts/test-content-readiness` | Commit readiness, painted geometry, scene timing and retained pictures |
| `scripts/test-floating-chrome` | Floating-window chrome contract |
| `scripts/test-icon-drop-lease` | Icon-drop ownership |
| `scripts/test-drag-maximize` | Lua policy; native input when a QA runtime is supplied |
| `scripts/test-drag-restore-motion` | Native restoration geometry and motion |
| `scripts/test-sidebar-pinned` | Pinned-window placement and input |

Run entry points from the repository root, one at a time. C++ fixtures use
`scripts/with-build-budget`. Native input uses `virtual-pointer-qa.c` and a
prepared protocol definition; it must never target an ordinary desktop.

Preserve existing client documents and prepared compositor roots. Disposable QA
must close its admitted fixtures and verify the owned service and process tree
are empty on success and failure. Captures belong under ignored `.build/` paths.

Use [the shell workflow](../../docs/shell-development.md) for ordinary changes and
[the greeter CU preview](../../docs/greeter-design-preview.md) for login UI work.
Keep fixtures reusable and tied to current contracts. Store run-specific results
with private build artifacts rather than adding per-fix documents.
