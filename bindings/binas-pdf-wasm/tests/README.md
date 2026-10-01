# Retained Rust API browser parity

`retained_revision.rs` runs the same fourteen tests natively and as real browser
WASM through the existing wasm-bindgen-test harness. It adds no public JavaScript
exports, session handles or consumer SDK. Missing fixture/candidate bytes fail
compilation; no optional fixture skips exist.

The structured and alphabetic fixtures from `qa/preservation` are reordered with
`PdfRevision::plan/commit` and exported through Incremental and Compact writers.
Tests check source/revision/page identities, exactly one commit, stale-plan
refusal, page labels, Unicode Info/outlines/form values, named destination remap
and attachment bytes after reopening. Both writer outputs must equal the frozen
native outputs independently checked by pypdf/the narrow label oracle. This is
byte parity for these fixtures and pinned code, not a general deterministic
writer promise or an independent browser renderer comparison.

Another original generated fixture exercises UTF-16 page-label prefixes. Failed
planning and cancellation before a plan/commit/export call must leave the current
revision usable; retry with a fresh context succeeds. These tests do not exercise
event-loop cancellation, worker termination, asynchronous bindings or mid-codec
interruption. Alphabetic-label expectations follow Table 159; the separately
recorded pypdf disagreement remains in `qa/preservation`.

The shared-stream test uses a separate original raw-token fixture. It exercises
explicit reject/local-clone/all-owner behavior, two successive commits and both
writer modes. Per-page text, metadata, new object identity, cancellation rollback
and source-prefix preservation must agree across native and browser execution.
It does not establish rendered font or appearance preservation for raw tokens.

The Unicode test uses the original content-sequence fixture, clones its physical
operand stream using an existing CMap and reorders pages in the same commit.
It checks all page/Form text runs, explicit font effects, required requested-text
evidence, both writers, an unavailable mapping and cancellation rollback.

The census test observes committed resource/font identities and verifies that
none of the registry's 27 planned targets can pass conformance with an empty
rule inventory. The image-preflight test checks strict JSON profiles, shared
image placements, exact DPI boundaries, unknown geometry, selected-page scope
and reordered identities. A custom resolution pass is not standards compliance.

The Flate test starts from an already committed text edit, recompresses supported
streams and compares compact output to the frozen independently checked native
fixture. Both writers retain the edit, navigation and attachment; stale and
cancelled plans fail, and an empty subsequent plan advances no revision.

The geometry test applies absolute rotation, MediaBox and CropBox to a stable
source page while reordering the document. It checks the before/after effect,
exactly one commit, cancellation rollback, stale-plan refusal, page labels,
metadata/navigation/forms/attachments and text through both writer modes. Each
result must equal the frozen native geometry output. Viewing geometry changes
intentionally; content, annotation coordinates and appearances are not rewritten.
Native private tests additionally inject self-consistent page-key/tree corruption;
the browser case is supported workflow parity, not a browser fault-injection suite.

The checkbox test changes two existing states, including a nested terminal field
with a separate widget, and reorders pages in one commit. It checks stable
field/widget/page effects, default values, ownership, labels, metadata, attachments,
cancelled/stale plans and a later failed selection. Both writer results must equal
the frozen native outputs independently checked for all 25 typed objects and all
two pages at RGB 144 DPI. The native private suite additionally checks source-key
loss, invalid appearance matrices and unauthorized staged objects. This is existing
appearance selection, not appearance generation, general form support or tag repair.

The flatten test selects only `group.answer`, whose two widgets share an appearance
with a nonidentity Matrix, and reorders pages in one commit. It checks stable
field/widget/page identities and placement effects, cancellation rollback, stale
plans, surviving sibling/checkbox fields and the unrelated link. Both writer
outputs must equal the independently checked native PDFs. A later committed
checkbox change retains the flattened result and the editable sibling.
The independent native gate checks every source object/stream and all two pages
at RGB 144 DPI; this is browser execution and byte parity, not a browser renderer
comparison. The [retained API contract](../../../docs/retained-pdf-revisions.md)
documents the opaque-appearance, explicit-font, Normal-blend, annotation-order and
batch-composition limits.

The text-field test fills a multiwidget terminal and a combined Tx field, using
appearance-local font/CMap resources while preserving a third widget's shared AP.
It checks values/defaults, stable effects, both writer outputs against independently
checked native bytes, cancellation/stale-plan behavior and a later clear. Native
public tests additionally cover refill and a separate later flatten. The oracle
checks exact bytes outside the replaced AP token,
correct glyph codes, original static paint and all-page RGB rendering. Native
private tests cover unsupported templates, metrics/clip fit, corruption and limits.
This is fixed-template filling with declared font metrics; it does not establish
arbitrary font rendering, shaping, reflow or human semantic quality.

The duplication test creates an adjacent page with explicitly named independent
Tx fields and fresh page-owned widgets/links. It checks source/copy identities,
registration/defaults, source-occurrence labels and original destination targets.
Both writers match the independently checked native outputs; later fills on copied
and original fields remain independent. Native public tests additionally cover
copied-field flattening/reorder, current geometry/value copies and failure recovery.
The oracle checks all 34 source objects, five new object roles and all three pages
at RGB 144 DPI. Shared AP/content/font bytes remain immutable. This does not expose
cross-document import, shared-value copies, arbitrary insertion or tag remapping.

The lossless-image test commits a text edit before planning PNG Up 12/Flate storage
for the fixture's 8-bit Gray/RGB images. Both writers must equal the independently
checked native bytes, including the shared image and soft mask. The tiny image
keeps its original encoding, cancelled/stale plans leave the edited revision
usable, and a repeated no-op preserves the revision and bytes. This checks a
[bounded image-storage pass](../../../docs/lossless-images.md); encoded savings
are not a promise about final PDF size or a new codec/quality policy.

Run from the repository root:

```powershell
cargo test -p binas-pdf-wasm --test retained_revision
cargo clippy -p binas-pdf-wasm --test retained_revision -- -D warnings
wasm-pack test --headless --chrome bindings/binas-pdf-wasm --locked
```

Local image-storage checkpoint on 2026-09-07: **659 workspace tests passed,
zero failed and zero ignored, across 76 suites**. All fourteen retained tests passed
natively and in actual single-thread headless Chrome, alongside two legacy browser
exports. The [image iteration receipt](../../../qa/preservation/evidence/lossless-images-iteration.json)
pins 308 file hashes and 235 captured inputs unchanged through the gates. Full
Clippy, locked Rust 1.88 MSRV, API example, fuzz-harness compile, mandatory
preservation and all-page RGB gates passed. No fuzz campaign was run. Browser
execution matches native fixture bytes; the independent rendering gate is native.

The historical duplication checkpoint recorded **630 workspace tests passed,
zero failed and zero ignored, across 75 suites**. All thirteen retained tests passed natively and in
actual headless Chrome; the two existing public-export browser tests also passed.
The [duplication iteration receipt](../../../qa/preservation/evidence/duplicate-iteration.json)
records the exact commands, source/fixture/output/log hashes and runtime details.
Workspace all-target/all-feature Clippy with warnings denied passed on Rust 1.95;
the locked Rust 1.88 MSRV check, public API example, fuzz-harness compile check and
mandatory preservation oracles passed separately. This was not a fuzz campaign.
The receipt is a checkpoint of its pinned source; later changes need fresh gates.

The [text-field iteration receipt](../../../qa/preservation/evidence/text-field-iteration.json)
remains historical: 609 workspace tests across 74 suites, twelve retained
native/Chrome tests and two public-export browser tests. It does not cover duplication.

The [flatten iteration receipt](../../../qa/preservation/evidence/flatten-iteration.json)
remains historical: 589 workspace tests across 73 suites, eleven retained
native/Chrome tests and two public-export browser tests. It does not cover Tx fills.

The [checkbox iteration receipt](../../../qa/preservation/evidence/checkbox-iteration.json)
remains historical: 566 workspace tests across 72 suites, ten retained native/Chrome
tests and two public-export browser tests. It does not cover later flatten or Tx paths.

The [geometry iteration receipt](../../../qa/preservation/evidence/geometry-iteration.json)
remains historical: 529 workspace tests, nine retained native/Chrome tests and
two public-export browser tests. It does not cover later checkbox, flatten or Tx paths.

The earlier resource/preflight/Flate checkpoint remains historical: Rust 1.88.0,
wasm-pack 0.15.0 and Chrome 152.0.7977.76; 508 workspace tests, eight retained
native/Chrome tests and two public-export browser tests passed. Its commands used
`RUSTUP_TOOLCHAIN=1.88.0`, `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=1`,
`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0` and the fresh
`target/validation-resource-188` target directory. The historical
[retained evidence](../../../qa/preservation/evidence/retained-evidence.json) and
[resource iteration receipt](../../../qa/preservation/evidence/resource-iteration.json)
pin that checkpoint's hashes and commands; they do not describe the later geometry,
checkbox, flatten or text-field tests.
This evidence does not establish other browsers, hosted CI, mobile devices,
or the future public retained-session bindings.
