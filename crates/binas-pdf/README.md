# binas-pdf

`binas-pdf` is a bounded, fail-closed PDF inspection and verified rewrite engine.

After approved publication, depend on the 0.2 series. For the current local
candidate, use the explicitly extracted artifacts described in the release guide.

```toml
[dependencies]
binas-pdf = "0.2"
```

It provides the native Rust API used by OxPDF. Supported mutations produce typed reports and verification results; unsupported or ambiguous PDF shapes are rejected instead of partially rewritten.

It does not provide pixel rendering, arbitrary layout reflow, bundled OCR recognition, or dynamic XFA rendering.

The local 0.2.0 release candidate also exposes `PdfSession` for retained selected
reads, preparation/acceptance of the conditional raw ASCII text contract, and
Incremental/Compact export. Sessions own one revision and at most one pending
candidate; close/drop releases their logical reservations. These counters are
not whole-process memory limits. This API is absent from the published 0.1.1
crate. See [the session contract](docs/pdf-sessions.md) for
ownership, errors, cancellation and resource-accounting scope.

The [operation support matrix](docs/operation-support-matrix.md) names the
conditional page/form/annotation/image shapes, retained and legacy export routes,
and finite native/JavaScript gates. The coarse capability profile does not
preflight every shape or qualify every Rust method in Node/Chromium.

The crate archive contains production sources and contracts. Repository tests
and their fixture corpora are maintained in the checkout; `cargo test` inside an
extracted archive is not a supported test distribution. See
[0.1.1 migration](docs/migration.md) before upgrading. The new version is a local
candidate until publication is approved.

Licensed under Apache-2.0.
