# Changelog

## v0.1.1

Release hygiene update.

- Use Node 24-native GitHub Actions tags in CI.
- Keep the release surface and install path unchanged from v0.1.0.

## v0.1.0

Initial public release of `binas`.

### Added

- Go-first core tree, selector, adapter, edit plan, report, and verification types.
- PDF inspection, validation, profile, query, and verified selectable-text rewrite CLI.
- Canonical PDF graph rewrite path with object-stream, xref-stream, and hybrid-xref support for supported edits.
- Supported reversible stream filters: Flate, LZW, ASCIIHex, ASCII85, RunLength, and standard abbreviations.
- Supported Flate/LZW `/DecodeParms` predictor handling for the documented fixture-backed shapes.
- `pkg/pdfapi` v0 Go API and fluent DSL for inspect, validate, profile, query, and verified text rewrite.
- AcroForm list/set helpers for supported text, choice, checkbox, button, and narrow radio field shapes.
- Annotation list and supported `/Contents` update helpers.
- XFA packet/dataset inspection and narrow static dataset/packet update helpers.
- Explicit overlay and caller-provided OCR text-layer fallback commands.
- Standard Security RC4/AESV2 password path for supported encrypted graph parse/edit/re-encryption.
- Signature inspect, explicit invalidation, narrow preserve-incremental text edit, and external-signer-backed re-signing helpers.
- Browser/WASM inspect/query/edit helper surface and local editor page.

### Boundaries

- No universal arbitrary-PDF edit guarantee.
- No hidden overlay/OCR fallback inside true text edit.
- No full layout/reflow, full dynamic XFA rendering, full visual fidelity, bundled OCR engine, AESV3/public-key encryption, or legal-grade signature trust validation.
