# binas-pdf

`binas-pdf` is a bounded, fail-closed PDF inspection and verified rewrite engine.

```toml
[dependencies]
binas-pdf = "0.1"
```

It provides the native Rust API used by OxPDF. Supported mutations produce typed reports and verification results; unsupported or ambiguous PDF shapes are rejected instead of partially rewritten.

It does not provide pixel rendering, arbitrary layout reflow, bundled OCR recognition, or dynamic XFA rendering.

Licensed under Apache-2.0.
