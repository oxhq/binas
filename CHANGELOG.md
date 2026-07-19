# Changelog

## Rust 0.1.1

- Report the Apache-2.0 SPDX license correctly on crates.io and include the canonical license in every published crate archive.

## Rust 0.1.0

- Introduces the four-crate Rust workspace: `binas-core`, `binas-pdf`, `binas-cli`, and the non-crates.io `binas-pdf-wasm` binding.
- Provides bounded PDF parsing, inspection, text extraction, verified rewrites, page/document operations, forms, annotations, XFA, images, encryption, signatures, CLI, and browser bindings within their documented fail-closed boundaries.
- Configures dependency-ordered publication (`binas-core`, `binas-pdf`, then `binas-cli`) from a version-matched `rust-v0.1.0` tag.

This source entry does not claim crates.io publication, installed-consumer proof, hosted multi-OS CI, sustained fuzzing, performance baselines, or independent interoperability results. Those remain release-time evidence gates.

## v0.2.0 - 2026-06-17

- Add exported PDF graph traversal primitives for catalog, page tree, name tree,
  object resolution, stream access, and object-stream provenance.
- Add canonical page copy, extract, and merge helpers with reparse, page-count,
  text, resource, and dangling-reference verification.
- Add page insert and rotate/crop transform helpers on the canonical page writer
  path.
- Preserve and reconcile catalog structures across canonical page operations,
  including name trees, page labels, outlines, and AcroForm fields from
  merged/inserted sources when referenced objects can be cloned safely. Metadata
  and viewer/open-action entries remain primary-document settings.
- Add conservative page scale transforms for unfiltered page content streams and
  expose richer graph provenance/filter metadata.
- Add narrow form field create/remove APIs, raw stream mutation, simple
  unfiltered image XObject replacement, and structured unsupported errors for
  form flattening and inline-image replacement.
- Add dynamic XFA boundary inspection and Standard Security R2 password
  encryption/change-password/decrypt-to-plain helpers, with public-key
  encryption still returning a structured unsupported error.
- Add a root `github.com/oxhq/binas` fluent API over `core.Adapter` for the
  shared PDF-now, future-format-later public layer.
- Re-export the graph and page-operation surface through `pkg/pdfapi` for
  Go consumers.
- Re-export form field list/edit, annotation contents edit, XFA semantic update,
  overlay/OCR fallback, security metadata, and signature preservation/re-signing
  helpers through `pkg/pdfapi`, keeping `pkg/adapters/pdf` internal-facing for
  higher-level Go consumers such as OxPDF.
- Remove the fluent `pdfapi.Document` DSL; direct `pkg/pdfapi` functions remain
  the PDF-specific helper surface.

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
