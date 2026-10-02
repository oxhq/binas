# Operation, shape, export and runtime support

This matrix describes the unpublished 0.2.0 candidate. A public method remains
available even when a shape is refused. Every mutation below is conditional on
its planner/security/ownership/filter checks and configured resource limits.
Successful parsing, a family capability entry, or a successful reparse does not
establish visual, accessibility, standards or complete preservation support.

The actual retained request is
[`PdfRevisionRequest`](../src/transaction.rs), the coarse
inventory is [`DocumentCapabilityProfile`](../src/profile.rs),
and legacy assembly returns
[`PageOperationReport`](../src/pages.rs), including explicit
`omitted_document_entries`. These are distinct contracts.

## How to use the capability APIs

`DocumentCapabilityProfile` schema version 1 preserves its existing family IDs.
`Supported` describes the named coarse operation with its stated preconditions;
`Conditional` requires operation-specific checks; `Refused` records a known
document/security boundary. `operation(unknown_id)` returns `None`, which a
consumer must treat as unavailable. `reason` is a human diagnostic; consumers
must not branch on that text. The profile does not enumerate every request field,
preflight every shape, choose an export mode, or qualify a target runtime.

The narrower `PdfRevision::text_edit_capability(format, operation, mode)` exposes
the conditional `pdf.text.retained_ascii` contract version 1 with typed refusal,
mode, limits and required checks. Unknown format/operation and Surgical are
refused. Preparation and acceptance still check the actual document/request.
The [text acceptance gate](../tests/text_acceptance.rs) checks these refusals,
required evidence and request/output binding. Other rows below have public
operation-specific APIs and tests, rather than an
equivalent typed capability contract. No generic all-operations receipt is claimed.

## Retained Rust operations

All rows in this table are conditional native Rust routes. `I/C` means explicit
`PdfWriterMode::Incremental` or `Compact` export after a successful commit. The
Rust tests name finite supported and refusal shapes; they do not qualify these
nontext operations in Node or Chromium. Only the raw ASCII row has a public
JavaScript session route.

| Operation / public entry | Admitted shape and effect | Boundaries / unchecked properties | Export | Repository gate |
| --- | --- | --- | --- | --- |
| Raw whole-token text: request `text`; session `prepare_text_edit` | Bounded ASCII token; uniquely owned supported page content; Plain/Flate frozen cohort | No font fallback, paragraph reflow or secure redaction; indirect Length and ambiguous ownership refuse; any selected-read warning means potentially incomplete | I/C; Surgical refused | [retained_ascii_preservation.rs](../tests/retained_ascii_preservation.rs), [session_contract.rs](../../../bindings/binas-pdf-wasm/tests/session_contract.rs), [raw independent oracle](../../../qa/retained-ascii/README.md) |
| Selected/font-aware text: request `page_text` | Existing explicit unambiguous ToUnicode mapping; selected page token; Reject, CloneForPage or AllPageOwners sharing policy | No glyph program/metrics/shaping verification; repeated physical operand or foreign/opaque stream owner refuses; absent/ambiguous mappings refuse; raw JS session refuses fonted inputs | I/C | [shared_revision.rs](../tests/shared_revision.rs), [fonted_text_preservation.rs](../tests/fonted_text_preservation.rs), [fonted oracle](../../../qa/fonted-text/README.md) |
| Annotation text: request `annotations` | Selected uniquely owned Text annotation Contents encoded as a PDF string | Appearance-bearing/shared annotations refuse; no appearance authoring or accessibility claim | I/C | [revision_transactions.rs](../tests/revision_transactions.rs) |
| Checkbox state: request `checkboxes` | Proven field/widget ownership; existing Off/on normal appearances; sets values and appearance state | Does not generate appearances or support arbitrary button/radio behavior | I/C | [revision_checkboxes.rs](../tests/revision_checkboxes.rs) |
| Targeted flatten: request `flatten_fields` | Selected terminal fields with valid existing normal appearances; paints appearances and unregisters selected fields/widgets | Detached source objects remain; no secure data removal; unsupported/conflicting appearance or same-page batches refuse | I/C | [revision_flatten.rs](../tests/revision_flatten.rs) |
| Fixed-template fill: request `text_fields` | Supported text field templates with existing appearance fonts; owner-local appearance copies, including supported multiwidget fields | No font substitution, reflow or arbitrary field template; incompatible combined form operations refuse | I/C | [revision_text_fields.rs](../tests/revision_text_fields.rs) |
| Reorder: request `reorder_pages` | Every current zero-based page exactly once; stable source page identities; explicit labels follow pages | Logical structure-tree order stays unchanged; no reading-order/accessibility assurance | I/C | [preservation_corpus.rs](../tests/preservation_corpus.rs), [revision_geometry.rs](../tests/revision_geometry.rs) |
| Geometry: request `page_transforms` | Absolute rotation in multiples of 90 degrees, MediaBox/CropBox; selections bind to starting revision and compose with reorder | Retained translation/scale unsupported; malformed boxes/overlapping writes refuse | I/C | [revision_geometry.rs](../tests/revision_geometry.rs) |
| Duplicate: request `duplicate_page` | One fresh page after its source; supported Tx fields get independently named identities; original link targets retained | Separate batch required; invalid rename maps, unsupported fields/ownership refuse | I/C | [revision_duplicate.rs](../tests/revision_duplicate.rs) |
| Delete: request `delete_page` | One active page removed; supported form registrations, labels/navigation and explicit excluded-target policy reconciled | Separate batch required; last page refuses; detached source objects retained; no erasure/redaction | I/C | [revision_delete.rs](../tests/revision_delete.rs) |
| Lossless image storage: `plan_lossless_images` / `commit_lossless_images` | Eligible used Flate 8-bit DeviceGray/RGB sample streams; smaller exact-sample predictor encoding wins | Unsupported filters/color spaces/masks/census gaps reported or refused; original wins when not smaller; no JPEG conversion or target-file-size promise | I/C; incremental whole file can grow | [revision_lossless_images.rs](../tests/revision_lossless_images.rs) |

All retained edits refuse encrypted/signed rewrite boundaries, and applicable
form/OCR operations refuse their stated XFA shapes. Failed or cancelled plan or
commit publishes no new revision; an export failure leaves a successful committed
revision available for retry. Snapshot/source-bound plans reject foreign and stale
use. The above suites test these cases, including configured output/work limits.
The writer's final-byte checks are in
[`writer/revision.rs`](../src/writer/revision.rs); Surgical has no
retained object-delta export implementation. Compact does not imply removal of all
unreachable or historically sensitive data.

## Retained legacy public routes

These byte-returning APIs remain usable and independently gated; their output is
not a retained `PdfRevisionExport` or `pdf.text.retained_ascii` receipt. Runtime
qualification below does not automatically extend to them.

| Profile family / API | Conditional shape / refusal boundary | Export / evidence gate |
| --- | --- | --- |
| `page_operations`: `copy_pages`, `extract_pages`, `insert_pages`, `merge_pages`, and `_with_policy` variants | Supported page/resource remapping; default assembly refuses unpreserved catalog/trailer entries; exact explicit omission permission reports input/location/key, including unselected inputs | Byte-returning assembly; [pages.rs](../tests/pages.rs), [preservation_corpus.rs](../tests/preservation_corpus.rs); consult omission report even when verification passes |
| `page_operations`: `transform_pages`, `compose_page` | Only representable transform/composition and supported annotation/resource ownership; appearances, unsupported annotation geometry/actions and security boundaries refuse | Byte-returning output; [pages.rs](../tests/pages.rs), [page_composition.rs](../tests/page_composition.rs) |
| `annotation_mutation`: `create_annotation`, `remove_annotation`, `set_annotation_contents` | Enumerated subtype/geometry/URI and ownership checks; generated appearances only for supported create requests | Byte-returning output; [annotation_lifecycle.rs](../tests/annotation_lifecycle.rs), [interactive_mutation.rs](../tests/interactive_mutation.rs) |
| `form_lifecycle`: `create_form_field`, `remove_form_field`, `flatten_form_fields`; existing value mutations | Supported terminal fields, widgets and verified normal appearances; XFA refuses; operation-specific field/appearance restrictions apply | Byte-returning output; [forms_lifecycle.rs](../tests/forms_lifecycle.rs), [button_form_mutation.rs](../tests/button_form_mutation.rs) |
| `image_replacement`: `replace_image_xobject_encoded`, `replace_image_xobject` | Admitted encoded image, dimensions/color space and explicit mask/ownership policy; replacement is separate from lossless storage optimization | Byte-returning output; [images.rs](../tests/images.rs); no general converter/renderer claim |
| `stream_mutation`, `canonicalize`, legacy surgical/incremental text | Supported filter/DecodeParms and byte-span/ownership rules per operation; surgical legacy path does not enable retained Surgical mode | [streams.rs](../tests/streams.rs), [canonical.rs](../tests/canonical.rs), [surgical_edit.rs](../tests/surgical_edit.rs), [incremental_edit.rs](../tests/incremental_edit.rs) |
| `ocr_text_layer`, `external_signing`, `standard_encrypt`, `decrypt` | Supplied bounded ASCII OCR boxes on unrotated pages, caller-supplied CMS, supported Standard security revisions/passwords or PubSec recipient material respectively; profile reasons state security/XFA refusals | [ocr.rs](../tests/ocr.rs), [signing.rs](../tests/signing.rs), [encryption.rs](../tests/encryption.rs), [public_key_encryption.rs](../tests/public_key_encryption.rs); no recognition/provider/network workflow claim |

## Runtime and proof scope

| Target | Current callable/qualified cohort | Limits and gate |
| --- | --- | --- |
| Native Rust, Windows/MSVC | Public document/revision/legacy APIs; finite operation shapes above | Configured `Limits` plus caller `OperationContext`; repository regression gates and independent corpus gates are distinct |
| Generated JS, Node | Synchronous retained ASCII session, selected extraction, lifecycle and I/C output; existing stateless binding exports retain their own contract | 8 MiB input; 16 MiB decoded-stream/total/cache/live/output ceilings; caller logical live/work budget; [session runtime harness](../../../qa/session-runtime/README.md) |
| Generated JS, single-thread Chromium WASM | Same finite synchronous textual cohort; frozen fonted parity exercises shared Rust API while raw session refuses fonted preparation | Same ceilings; [session_contract.rs](../../../bindings/binas-pdf-wasm/tests/session_contract.rs), [runtime harness](../../../qa/session-runtime/run.mjs); JS input/output copies and synchronous event-loop blocking remain |
| Other OS/browser/worker/C ABI | No additional qualification from this matrix | Hosted multi-OS, worker recovery, mid-call browser cancellation, C ABI and total-process/heap/CPU guarantees remain outside the candidate |

Managed decode/live/work counters exclude object maps, codec/allocator scratch,
returned allocations and JS copies. Cancellation is cooperative. See the
[packaged session contract](../docs/pdf-sessions.md) for lifecycle,
accounting and typed errors. The matrix does not raise any runtime limit.

## Reproducible gates and evidence interpretation

The checkout-only [nontext boundary harness](../../../qa/nontext-boundary/run.py)
stages frozen licensed fixtures, public API outputs and independent pypdf/Poppler
checks outside the checkout. Run `python qa/nontext-boundary/run.py --output
<outside-checkout> --pdftoppm <pinned-executable>` with the versions in
[`requirements.txt`](../../../qa/preservation/requirements.txt). Its manifest and
logs establish the actual outcome of a run; this table specifies coverage, not a
pass declaration.

| Matrix rows | Independent gate scope | Remaining boundary |
| --- | --- | --- |
| Reorder, geometry, duplicate, delete | Frozen structured fixture; exact inventory/navigation/form/page effects and RGB render comparison for I/C | Other document producers/shapes, logical reading order and universal accessibility remain unchecked |
| Checkbox, targeted flatten, fixed-template fill | Frozen field/widget/appearance fixtures; field values, active registrations, unrelated objects and RGB output for I/C | Other field types/templates, font substitution and secure erasure remain unchecked |
| Lossless image storage | Frozen RGB/gray sample/predictor and encoded mask fixture; exact decoded samples/dictionaries plus RGB render for I/C | Image conversion and a target whole-file size remain unqualified |
| Legacy raw RGB image replacement | Existing unmasked 2x2 RGB image in the same frozen fixture; exact requested samples and untouched objects/streams; independent pypdf golden and RGB render; mask/incorrect-sample refusals | Other dimensions, encoded JPEG/JPX replacement and color conversion have repository regressions only, without independent qualification here |
| Retained annotation Contents; legacy annotation/form create/remove | Public nontext consumer and [lifecycle oracle](../../../qa/nontext-boundary/oracle.py); declared field/annotation effects and retained object inventory | Only enumerated consumer cases; no general annotation appearance/accessibility qualification |
| Legacy page import/extract | Frozen Type3 shared/separate resource controls; inventory/text and independent rendering, including missing-font/altered-glyph controls | Unsupported catalog families still refuse or need explicit reported removal; no broad import preservation |
| Other legacy encoded image replacement, stream/canonical/security/OCR operations | Repository regressions listed above only; no independent nontext qualification | Not independently or externally qualified by this cohort; keep conditional preconditions and existing APIs |

Observed peak memory is a separate [packaged probe contract](peak-memory.md)
and [harness](../../../qa/peak-memory/run.py), covering its named native, Node and
Chromium textual cases plus any explicitly recorded nontext case. Observed OS/
runtime peaks, byte parity and hard-limit refusals do not turn logical limits into
a universal process/heap bound or qualify all operation rows in all runtimes.

Run the profile contract with `cargo test -p binas-pdf --test profile`. Retained
nontext regression gates can be run with `cargo test -p binas-pdf --test
revision_transactions --test revision_geometry --test revision_checkboxes --test
revision_flatten --test revision_text_fields --test revision_duplicate --test
revision_delete --test revision_lossless_images`; run legacy gates with `cargo test
-p binas-pdf --test pages --test annotation_lifecycle --test forms_lifecycle --test
images`. These are source-checkout checks, not packaged installation evidence.

An independent gate must name exact source/output hashes, fixture provenance,
requested effect, retained/removed/unchecked properties, reader versions and
negative controls. A fixture count or passing same-engine verification is not an
independent gate. Broader input shapes remain conditional until their own effect,
preservation and runtime gates pass. Publication remains a separate decision in
[release-surface.md](../../../docs/release-surface.md).
