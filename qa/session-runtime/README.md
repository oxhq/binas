# Local JavaScript session verification

Uses Node built-ins, generated WASM/JS artifacts and an existing Chromium/driver
pair. No npm packages, installation, downloads or remote services are needed.

Generate separate `pkg-node` and `pkg-web` bindings from the same local release
WASM using the matching wasm-bindgen CLI. Then run:

```powershell
node qa/session-runtime/run.mjs C:\path\artifact-root C:\path\source-root
```

The runner checks actual generated public JS exports in both Node and a headless
browser. Paths for the installed ChromeDriver/Chromium are explicit in run.mjs;
adjust those paths only to a known matching installed pair. A loopback static
server and WebDriver are closed in finally. Results and artifact identity remain
local; this is not npm/crates.io installation proof.

Eight frozen expected PDFs in `bindings/binas-pdf-wasm/tests/fixtures/sessions`
come from the P3a/P3b independently checked native outputs. Their source fixtures
and licensed provenance are in `crates/binas-pdf/tests/fixtures/{retained-ascii,
fonted-text}`; all are original Apache-2.0 fixtures. Runtime tests compare exact
bytes without asserting that an independent parser/renderer ran inside WASM.

The runner also generates an original minimal four-object Flate PDF whose stream
decodes to 16 MiB + 1. It tests a genuine decoded-stream limit, distinct from the
8 MiB + 1 input-size rejection and logical live/work-budget controls. It does not
measure allocator peak memory or concurrent cancellation.
