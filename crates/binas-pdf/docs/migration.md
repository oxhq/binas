# Binas 0.1.1 to 0.2.0

0.2.0 is a local release candidate. Until publication, resolve the exact extracted
candidate archives explicitly; the cached registry 0.1.1 lacks selected reads,
revision identities, operation contexts and retained sessions.

Update `binas-pdf` and any direct `binas-core` dependency together to `"0.2"`.
Applications constructing `Limits` should use `Limits { max_input_bytes: ...,
..Limits::default() }` or initialize the three new decode cache/live/work fields.
`PageOperationReport` adds `omitted_document_entries`; consumers of exhaustive
struct literals must adapt. Exhaustive `PdfErrorCode` matches must handle
`Cancelled`. `OpenOptions { repair: false }` remains unchanged.

Use selected reads on `PdfDocument`, `PdfRevision` or `PdfSession`. Treat warnings
as potentially incomplete text; a warning-bearing no-match result is not an
exhaustive negative search. `PdfSession` admits the raw ASCII contract only;
font-aware edits use `PdfRevision` and their separate conditional contract.
Neither path offers secure redaction or arbitrary reflow.

Persisted receipts are new versioned evidence, not interchangeable with old
boolean `verified` values. Store contract version, engine version, identities,
request and coverage. Reject unknown contract versions and retain the original
source and known-good outputs for rollback. Session handles are process-local
opaque tokens and cannot be saved as durable project history.

Close sessions explicitly, and call generated JavaScript `free()` after `close()`.
Cancellation is cooperative; synchronous JavaScript calls do not yield. Logical
budgets do not bound total heap or CPU. Native Rust and generated WASM bindings
are the implemented interfaces; no stable C ABI, header or C FFI is distributed.

The archive includes production code and this contract. Repository integration
and private module tests need checkout fixtures, some outside the crate directory;
`cargo test` within the extracted archive is not a standalone test distribution.
Consumers build and run from the extracted libraries with explicit sibling
resolution while the new versions are unpublished. Public installation without
patches remains a post-publication gate.
