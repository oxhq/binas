# binas-pdf

`binas-pdf` is a bounded, fail-closed PDF inspection and verified rewrite engine.

```toml
[dependencies]
binas-pdf = "0.1"
```

It provides the native Rust API used by OxPDF. Supported mutations produce typed reports and verification results; unsupported or ambiguous PDF shapes are rejected instead of partially rewritten.

It does not provide pixel rendering, arbitrary layout reflow, bundled OCR recognition, or dynamic XFA rendering.

The local development candidate also exposes `PdfSession` for retained selected
reads, preparation/acceptance of the conditional raw ASCII text contract, and
Incremental/Compact export. Sessions own one revision and at most one pending
candidate; close/drop releases their logical reservations. These counters are
not whole-process memory limits. This API is absent from the published 0.1.1
crate. See `docs/pdf-sessions.md` in the repository checkout for
ownership, errors, cancellation and resource-accounting scope.

Licensed under Apache-2.0.
