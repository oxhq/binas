# Binas Rust Rewrite Specification

Status: implementation specification

Branch: `rewrite/rust-engine-spec`

Branch-point commit: `b67029e109f4a1b6fb28248eafdb7264542cc6c3`

Audience: Binas engine, OXPDF, CLI, WASM, security, and release owners

## 1. Decision

Binas will be rewritten in Rust as a small workspace with one public crate per
format. The PDF implementation and its public API will live in the same crate.
Parser, object graph, xref, filters, content streams, forms, annotations,
signatures, encryption, and writers remain private modules until an actual
independent consumer or target constraint proves that a separate crate is
needed.

```text
binas/
├── Cargo.toml
├── crates/
│   ├── binas-core/
│   ├── binas-pdf/
│   └── binas-cli/
└── bindings/
    └── binas-pdf-wasm/
```

This accepts the external reviewer's main recommendation and makes these
decisions explicit:

- `pkg/core` maps to a deliberately small `binas-core`.
- Go's internal PDF engine and public PDF façade merge into `binas-pdf`.
- The CLI becomes `binas-cli` but continues to install a `binas` executable.
- The generic JavaScript API becomes `binas-pdf-wasm`.
- There is no universal `BinaryDocument` trait in v1.
- There are no parser, writer, xref, forms, or signature sub-crates in v1.
- Rust exposes one explicit plan/apply/verify API, not parallel fluent and
  mutable-document APIs.

The rewrite is complete only when the Rust implementation can replace the Go
engine for OXPDF, the CLI, and WASM under the gates in this document. Merely
parsing PDFs or matching the current happy-path tests is not completion.

## 2. Goals and non-goals

### 2.1 Goals

1. Preserve Binas's differentiating behavior: bounded parsing, explicit edit
   plans, fail-closed mutations, structural rewrite choices, and post-write
   verification.
2. Preserve every supported Go PDF operation and every public CLI/WASM contract
   unless this specification explicitly replaces it.
3. Close the product-critical gaps listed in section 10 so that Rust does not
   become a line-for-line port of known limitations.
4. Give OXPDF a stable native engine API without requiring it to spawn the CLI.
5. Make malformed and hostile input a first-class test domain: no panic, no
   unchecked allocation growth, no unbounded recursion, and deterministic
   resource-limit errors.
6. Support native Windows, Linux, and macOS builds plus browser WASM.
7. Permit later crates such as `binas-png` or `binas-elf` without forcing PDF
   concepts into a generic document interface.

### 2.2 Non-goals for the initial cutover

- Rendering arbitrary dynamic XFA layouts.
- Bundling an OCR recognition engine. Binas consumes explicit OCR results and
  writes a text layer; recognition belongs to the caller.
- Key storage, certificate lifecycle, and recipient discovery for public-key
  PDF encryption. Callers supply the certificate/key material explicitly.
- Pixel rendering, PDF-to-image conversion, or a full layout engine.
- Reproducing deprecated Go package aliases as Rust API design.
- Byte-for-byte output equality for canonical rewrites. Semantic equality and
  declared invariants are the contract there.
- A `no_std` core at cutover. It can be considered after a real consumer needs
  it; the initial workspace targets `std` and WASM.

## 3. Architecture

### 3.1 Workspace

```toml
[workspace]
resolver = "2"
members = [
    "crates/binas-core",
    "crates/binas-pdf",
    "crates/binas-cli",
    "bindings/binas-pdf-wasm",
]
```

Use the repository's pinned stable Rust toolchain and edition 2024. All direct
dependencies are pinned in `Cargo.lock`; published library crates use compatible
semver requirements. CI must test the minimum supported Rust version once one
is declared.

### 3.2 `binas-core`

`binas-core` contains only types that are already useful to at least two format
implementations or that form the stable edit/diagnostic vocabulary:

- `Span` and checked source ranges;
- byte-source abstraction when non-contiguous input is actually required;
- node identifiers and format-neutral inspection nodes;
- selectors and match indices;
- edit-plan identifiers and common plan metadata;
- reports, diagnostics, warnings, and verification results;
- invariant identifiers shared by public surfaces.

It must not contain PDF objects, dictionaries, streams, pages, xrefs, filters,
forms, annotations, encryption, or signatures. It must not contain a generic
adapter trait merely to mirror Go's `core.Adapter`.

Start with immutable in-memory input (`Arc<[u8]>` natively and owned bytes in
WASM). Add a general `ByteSource` only when a measured large-file or streaming
use case needs random access. Do not pay the trait and error-genericity cost in
every parser function speculatively.

### 3.3 `binas-pdf`

Proposed module ownership:

```text
src/
├── lib.rs                 # curated exports only
├── document.rs            # open document and public read surface
├── query.rs               # selectors and text/object queries
├── edit.rs                # public requests, plans, outcomes
├── report.rs              # PDF-specific reports and verification
├── limits.rs              # EngineConfig and Limits
├── error.rs               # stable typed error taxonomy
├── parser/                # lexer, values, indirect objects
├── objects/               # graph, resolution, object streams
├── xref/                  # tables, streams, hybrid chains
├── filters/               # bounded stream decoding/encoding
├── content/               # operators, text state, inline images
├── fonts/                 # encodings, widths, CMaps
├── writer/                # surgical, canonical, preserved, incremental
├── pages/                 # page tree and composition
├── forms/                 # AcroForm and appearances
├── annotations/           # annotation operations and appearances
├── xfa/                   # bounded XML semantics
├── security/              # standard security handlers
├── signatures/            # ByteRange, CMS metadata, signing plans
├── images/                # image XObject/inline-image contracts
├── overlay.rs
└── ocr.rs
```

Modules are private by default. `lib.rs` re-exports only consumer contracts.
Low-level PDF values may be publicly inspectable through a stable read-only
graph API, but parser and writer internals are not public extension points.

### 3.4 `binas-cli`

The crate depends on `binas-pdf` and contains argument parsing, filesystem I/O,
JSON presentation, and external signer process integration. Engine behavior
does not live in the CLI. `clap` is the intended argument parser.

### 3.5 `binas-pdf-wasm`

The binding is a thin serialization layer over `binas-pdf`. It owns JavaScript
conversion and exported names, but no PDF logic. It uses `wasm-bindgen` and
builds with the same operation vectors as native Rust.

### 3.6 OXPDF integration

OXPDF will be rewritten in Rust after the Binas engine is complete and will
depend directly on `binas-pdf`. It must not spawn `binas`, and no temporary C
ABI is part of the cutover. The existing Go OXPDF remains a reference and
rollback implementation until the Rust consumer passes its engine-facing suite.

## 4. Public Rust contract

### 4.1 Design rules

- Operations are explicit: open, inspect/query, plan, apply, verify, serialize.
- Documents are not silently mutated after a failed operation.
- Every mutation produces a plan before output bytes are committed.
- An applied edit returns bytes plus report and verification in one outcome.
- Verification failure is an error; unverified bytes are not returned through
  the normal convenience path.
- Callers can request a plan-only result for preview, UI, or external signing.
- Public DTOs are serializable for CLI, WASM, vectors, and direct Rust consumers.
- Public errors have stable machine-readable codes and contextual diagnostics.
- Unknown enum values are rejected, not silently mapped to defaults.

### 4.2 Core types

The exact field layout may evolve during Gate 1, but the semantic contract is:

```rust
pub struct PdfEngine {
    config: EngineConfig,
}

pub struct PdfDocument {
    // immutable input, parsed indexes, bounded decode cache
}

pub struct EditPlan {
    pub id: PlanId,
    pub operation: OperationSummary,
    pub rewrite: RewriteMode,
    pub predicted_effects: Vec<Effect>,
    pub warnings: Vec<Diagnostic>,
}

pub struct EditOutcome {
    pub bytes: Vec<u8>,
    pub report: RewriteReport,
    pub verification: VerificationReport,
}

pub enum RewriteMode {
    Auto,
    Surgical,
    Canonical,
    PreserveStructure,
    Incremental,
}
```

Typical use:

```rust
let engine = PdfEngine::new(EngineConfig::default());
let document = engine.open(input, OpenOptions::default())?;
let plan = document.plan(EditRequest::ReplaceText(request))?;
let outcome = document.apply_and_verify(plan, invariants)?;
```

There is no second builder/fluent API at cutover. Convenience functions may
wrap this contract without creating separate behavior.

### 4.3 Input ownership and concurrency

- `PdfDocument` is safe to inspect concurrently when its input and configuration
  are immutable.
- Mutations create independent plans and outputs; they do not modify shared
  document state.
- Decoded-stream caches are bounded, keyed by object reference plus decode
  parameters, and never bypass cumulative budgets.
- The cumulative decoded-byte budget is charged exactly once per newly cached
  decode and remains enforced under concurrent access.
- No public API accepts or stores ambient global configuration.

### 4.4 Error taxonomy

Every error has a stable code, human message, optional source span/object
reference, and causal detail. Required top-level classes:

- `invalid_syntax`
- `unsupported_feature`
- `encrypted_document`
- `invalid_password`
- `permission_denied`
- `resource_limit`
- `selection_not_found`
- `selection_ambiguous`
- `unsafe_rewrite`
- `signature_policy`
- `verification_failed`
- `external_signer`
- `io`
- `internal`

Malformed input never produces `internal`. Panics are bugs and may not cross a
Rust or WASM boundary.

## 5. Resource and security contract

`EngineConfig` owns a `Limits` value supplied to `PdfEngine`; individual calls
may only lower limits unless an explicit trusted configuration creates another
engine. Defaults must be safe for server use and configurable for OXPDF desktop.

Required checked budgets:

| Budget | Applies to |
| --- | --- |
| input bytes | complete source |
| object count | indirect and compressed objects |
| xref entries and revisions | tables, streams, `/Prev` chains |
| parser depth | arrays, dictionaries, references, object streams |
| resolved-reference depth | recursive graph walkers |
| value/container items | arrays, dictionaries, name trees, page trees |
| stream encoded bytes | source slice before decode |
| stream decoded bytes | each decoded stream |
| cumulative decoded bytes | whole document/session, including cache |
| filter-chain length | nested filters and decode parameters |
| content operations | each content stream and whole operation |
| string/name/token length | lexer allocations |
| CMap entries and code width | ToUnicode and encoding maps |
| XML bytes, depth, nodes, attributes | XFA and ALTO input |
| page/form/annotation counts | specialized walkers |
| signature ranges/certificates | signature inspection and validation |
| incremental revisions/output growth | incremental edits and signing |
| output bytes | every writer |

Implementation requirements:

- All offset, length, allocation, range, and object-number arithmetic is checked.
- Every recursive or graph traversal has both a depth limit and a visited set
  where cycles are legal.
- Filter expansion is charged while decoding, not after allocating output.
- XML parsing disables external entities and network/file resolution.
- Cryptographic comparisons use constant-time operations where secrets apply.
- Passwords, keys, decrypted bytes, signer environment, and certificates are
  never written to ordinary logs.
- Unsupported algorithms fail before mutation begins.
- FFI entrypoints catch unwinds and translate them to `internal` errors.
- Unsafe Rust is forbidden by default; any exception requires a documented
  safety invariant, focused tests, and reviewer approval.

## 6. Go-to-Rust migration inventory

The current working tree has 133 Go files, 68 Go test files, and 22 checked-in PDF
fixtures. The small corpus means passing translated unit tests alone is weak
evidence; sections 11 and 12 require broader differential and adversarial proof.

| Current Go area | Rust owner | Required behavior |
| --- | --- | --- |
| `pkg/core` | `binas-core` | spans, nodes, selectors, plans, reports, invariants, verification |
| `binas.go` adapter façade | `binas-pdf` API | PDF detection/open plus explicit plan/apply/verify replacement |
| `internal/pdf/graph*.go`, `xref.go` | parser, objects, xref | classic xref, xref stream, hybrid refs, object streams, trailer/revision chains |
| `adapter.go` | query, content, edit | inspection tree, text selection, edit planning, invariant enforcement |
| `filter*.go` | filters | Flate/predictors, LZW/EarlyChange, ASCIIHex, ASCII85, RunLength, Crypt, CCITT G3/G4, DCT/image passthrough |
| `cmap.go`, `font_*.go`, `cid_metrics.go` | fonts | ToUnicode, simple encodings, widths, CID metrics, bounded reverse encoding |
| `text_state.go`, `text_layout.go` | content, fonts | Tj/TJ handling, text state, position/width proof, layout modes |
| `structure_plan.go`, `packed_writer.go` | writer | canonical object serialization and graph rewrite |
| `preserve_structure_writer.go` | writer | structural preservation contract |
| `incremental*.go` | writer, signatures | append-only revisions, prefix preservation, signature-aware edits |
| `page_ops.go`, `page_transform.go` | pages | copy, extract, insert, merge, transform, catalog preservation |
| `page_verification.go` | pages, report | page count/order/closure/dangling-ref verification |
| `form*.go`, `appearance_da.go` | forms | field discovery/edit/create/remove, DA and appearance handling |
| `annotation.go` | annotations | list/edit contents and safe appearance regeneration |
| `stream_mutation.go`, `inline_image.go` | images, filters | explicit stream/image replacement and verification |
| `security.go`, `encryption*.go` | security | password handling, decrypt, encrypt, password change, policy metadata |
| `signature_info.go` | signatures | ByteRange, digest, CMS/cert/timestamp metadata, explicit trust stores |
| `signature_signing.go`, `signature_timestamp.go` | signatures | signing plan, reservation, external CMS application, verification |
| `semantic.go`, `xfa*.go` | xfa | current Rust support: packet listing, simple static `datasets/data` leaf fields, raw packet replacement, bounded dataset updates, dynamic detection, and read-only template-to-dataset mappings for exact/simple named-subform paths; XFA-specific selector/match-index semantics remain planned |
| `ocr.go` | ocr | JSON/ALTO plan and explicit text-layer application |
| `overlay*.go` | overlay | explicit overlay plus fallback-policy reporting |
| `resource_limits.go` | limits | all budgets in section 5, with no global mutable state |
| `cmd/binas` | `binas-cli` | command, flag, JSON, exit-status, and signer-process compatibility |
| `cmd/binas-wasm` | `binas-pdf-wasm` | JavaScript export and payload compatibility |

## 7. PDF capability requirements

### 7.1 Syntax, graph, and xref

Must support:

- PDF header and binary marker handling;
- direct and indirect null, Boolean, numeric, name, string, hex string, array,
  dictionary, reference, and stream values;
- escaped literal strings, nested parentheses, odd-length hex strings, and
  token-boundary rules;
- indirect stream lengths;
- generations and free/in-use xref entries;
- classic xref tables, xref streams, hybrid files, object streams, `/Prev`, and
  `/XRefStm` revision chains;
- catalog, page tree, name tree, AcroForm, annotation, and signature traversal;
- deterministic duplicate-object/revision precedence;
- read-only public graph inspection without exposing parser implementation.

Repair heuristics must be opt-in, reported, and never used by a surgical or
signature-preserving write without an explicit caller policy.

### 7.2 Streams and filters

The decoder and encoder must preserve ordered filter chains and aligned
`DecodeParms`. Supported decode behavior at Go parity:

- Flate with TIFF and PNG predictors;
- LZW with `EarlyChange` and predictors;
- ASCIIHex;
- ASCII85;
- RunLength;
- Crypt filter dispatch used by supported encryption revisions;
- CCITT Fax Group 3 and Group 4;
- DCT and supported image filters as declared passthrough when pixels are not
  required.

Unknown filters, invalid parameters, truncated streams, and expansion-limit
failures return typed errors including object reference and filter position.

### 7.3 Text and fonts

Must preserve and then improve:

- content tokenization, graphics/text state stacks, `BT`/`ET`, `Tf`, text
  matrices, spacing, scaling, leading, `Tj`, `'`, `"`, and `TJ`;
- simple font encodings and Differences;
- ToUnicode CMaps including variable-width codes;
- Type 0/CID text decoding needed by current fixtures;
- deterministic match ordering and zero-based `match_index`;
- reverse encoding proof before true text replacement;
- `preserve-width`, `allow-width-change`, and `reflow-line` layout policies;
- verification that old text is gone and replacement remains selectable.

Rust completion also requires a document/page text extraction API returning
text spans with page, content object, font, source operator, and best-known
geometry. Extraction must make uncertainty explicit; it must not invent reading
order confidence.

### 7.4 Writers and edits

Required writer modes:

- `Surgical`: replace only a proven byte span; preserve all other bytes; reject
  length/encoding/layout changes it cannot prove safe.
- `PreserveStructure`: update affected objects while preserving supported xref,
  object-stream, and catalog structure; report every structural fallback.
- `Canonical`: serialize a complete, valid graph deterministically and verify
  semantic invariants after reparse.
- `Incremental`: append a valid revision, preserve the original prefix exactly,
  maintain revision chains, and apply signature policy.
- `Auto`: choose by documented safety rules and report the selected mode. It may
  not silently downgrade an invariant.

All modes must preserve or explicitly report catalog entries, metadata,
outlines, destinations, attachments, page labels, AcroForm, XFA, annotations,
encryption, and signatures. Unknown reachable objects are preserved as graph
values rather than discarded.

### 7.5 Pages

Parity operations: copy, extract, insert, merge, select, translate, rotate,
scale, and verify. Completion adds:

- inherited page attributes and resource resolution;
- resource-name collision detection and deterministic renaming during merge;
- correct copying of transitive resources and annotations;
- filtered content-stream transforms by decode/modify/re-encode when supported;
- page boxes, rotation, user units, and catalog/page-label preservation;
- explicit refusal when a safe transform cannot be proven.

### 7.6 Forms and annotations

AcroForm requirements:

- enumerate fully qualified field names and widget relationships;
- edit field values with type validation;
- create and remove text, checkbox, radio, choice, and signature fields needed
  by OXPDF;
- regenerate standard appearances deterministically or report why it cannot;
- flatten supported fields into page content, remove widget/field references,
  and verify visual-content/resource closure;
- preserve calculation order, default resources, flags, and unrelated fields.

Annotation requirements:

- list all supported subtypes and page ownership;
- edit contents with safe appearance behavior;
- create and remove common OXPDF annotations;
- preserve actions, flags, border/color, QuadPoints, and unrelated appearance
  states;
- reject unsafe JavaScript/rich-content rewrites unless explicitly supported.

### 7.7 Images, overlays, and OCR layers

- Replace image XObjects and inline images with explicit dimensions, color
  space, bits, filters, and decode parameters.
- Add a normalized image-input path for common encoded images without making
  callers manually construct PDF stream dictionaries.
- Preserve transparency masks and resource references when supported; otherwise
  reject with a typed capability error.
- Preserve explicit overlay fallback policy and report whether output is a true
  text edit, overlay, or OCR layer.
- Parse bounded JSON and ALTO XML OCR results, plan per-page boxes, add selectable
  text layers, and verify page count and text selection.

### 7.8 XFA

- List XFA packets and detect dynamic-XFA markers.
- Parse bounded static `datasets/data` XML for simple leaf paths. Read-only
  template-to-dataset mapping accepts only exact field paths or enclosing
  simple named-subform paths that identify one dataset leaf; ambiguous and
  unmatched fields are omitted.
- List simple dataset fields, and apply raw packet text or simple dataset-field
  updates with verification. XFA-specific selector and match-index selection
  remain unsupported/planned.
- Preserve packet ordering and unrelated XML.
- Dynamic rendering remains a non-goal; report it as a capability boundary.

### 7.9 Encryption

Go-parity cutover requires standard security handler read/write behavior already
proved by the Go suite, including R2, R4, R5, and R6 output interoperability,
decrypt-to-plain, and password change. Gate 6 must inventory the exact algorithms
and crypt filters accepted for reading before claiming parity.

Completion additionally requires:

- owner/user password distinction and permission-bit reporting;
- correct metadata-encryption handling;
- AES/RC4 algorithm selection only where the PDF revision permits it;
- authentication and key derivation vectors from authoritative specifications;
- supported permission/options writing rather than hard-coded defaults;
- preservation or intentional removal of encryption during every writer mode;
- Adobe.PubSec s3/s4/s5 public-key encryption for the implemented
  RC4/AESV2/AESV3 shapes, with typed failure for unsupported variants.

Cryptographic primitives must come from maintained libraries; Binas implements
PDF policy and byte layout, not AES, RC4, hashing, ASN.1, or X.509 primitives.

### 7.10 Signatures

- Discover signature fields and dictionaries.
- Validate ByteRange shape, ordering, file bounds, and covered revisions.
- Compute and report digest match.
- Parse CMS/certificate/timestamp metadata within budgets.
- Accept explicit trust roots and intermediates; do not use ambient OS trust
  unless a caller explicitly selects it.
- Distinguish cryptographic validity, trust, time validity, modification after
  signing, and unsupported algorithms.
- Plan incremental re-signing with reserved contents, produce bytes-to-sign,
  apply external CMS bytes, and verify final ranges/digest.
- Never claim long-term validation, revocation, or legal validity without the
  required evidence.

## 8. Compatibility surfaces

### 8.1 CLI commands

Rust must initially preserve these command paths:

```text
inspect
query
edit
overlay text
ocr text-layer
ocr text-layer-plan
form list
form set
annot list
annot set-contents
xfa list
xfa datasets
xfa mappings
xfa dataset-set
xfa replace
signature inspect
signature re-sign
validate
profile
```

`xfa mappings` remains a planned CLI compatibility command. The Rust core
provides the bounded read-only mapping API, but not XFA-specific selector or
match-index operations.

Compatibility includes flag names, defaults, stdin/stdout behavior, output file
rules, JSON field names/types, zero-based indices, warnings, and exit classes.
Important enums remain:

- rewrite: `auto`, `surgical`, `canonical`, `preserve-structure`;
- layout: `preserve-width`, `allow-width-change`, `reflow-line`;
- signature: `refuse`, `invalidate`, `preserve-incremental`.

The deprecated `--index` alias may remain only for one documented compatibility
window and must map exactly to `--match-index`; conflicts are errors. New native
operations added by this spec may introduce commands after compatibility is
locked.

`signature re-sign` keeps the external signer JSON protocol, repeated argument
and environment flags, digest/container/sub-filter/reserved-byte options, and
base64/hex/byte response forms. Environment values must be redacted in errors.

### 8.2 WASM

The first Rust release preserves these global compatibility functions:

- `binasInspectPDF(Uint8Array)`
- `binasQueryPDF(Uint8Array, string)`
- `binasEditPDFText(Uint8Array, string, string)`

Their current JSON and byte-result schemas become golden vectors before Rust
implementation. A new namespaced module API may be added, but the compatibility
functions are removed only in a major release after OXPDF web migrates.

WASM uses the same limits and typed errors, with a lower configurable input and
decode ceiling where browser memory requires it. WASM must not depend on native
filesystem, process, or OS trust-store behavior.

### 8.3 Go package transition

The Go packages remain the frozen oracle and rollback source while Binas is
completed. They are not re-created as a second Rust API and are not bridged into
the future Rust OXPDF process.

## 9. External dependency policy

Dependencies reduce risk only when their boundary matches Binas's contract.
Every dependency needs an owner, license review, advisory scan, WASM check, and
reason it is preferable to a small local implementation.

Current candidates, to be confirmed by a short proof before Gate 1 closes:

- [`clap`](https://docs.rs/clap/latest/clap/) for CLI parsing.
- [`wasm-bindgen`](https://docs.rs/wasm-bindgen/latest/wasm_bindgen/) for the JS
  boundary.
- [`quick-xml`](https://docs.rs/quick-xml/latest/quick_xml/) for bounded streaming
  XFA/ALTO parsing, with Binas-owned depth/node/entity policy.
- RustCrypto [`der`](https://docs.rs/der/latest/der/) and
  [`x509-cert`](https://docs.rs/x509-cert/latest/x509_cert/) for ASN.1/X.509
  primitives where their supported profiles fit.
- Maintained RustCrypto primitives for hashes, AES, and legacy algorithms only
  where PDF compatibility requires them. Low-level APIs such as
  [`aes`](https://docs.rs/aes/latest/aes/) are hazardous primitives, not a PDF
  security implementation.

PDF libraries are references and differential peers, not automatic foundations:

- [`lopdf`](https://docs.rs/lopdf/latest/lopdf/) may help with differential
  parsing and corpus comparison, but Binas must retain its own bounded parsing,
  preserve/surgical/incremental writers, edit plans, and verification semantics.
- [`pdf-writer`](https://docs.rs/pdf-writer/latest/pdf_writer/) may be evaluated
  as a low-level canonical serializer. It does not replace object allocation,
  validation, preservation, or incremental planning.

Do not adopt a dependency if wrapping it requires duplicating its entire object
model or if it cannot enforce Binas budgets before allocation.

## 10. Required gap closure

The rewrite has two milestones: Go parity and Rust engine completion. These gaps
are not allowed to disappear into an unspecified post-rewrite backlog.

| Gap in current Go surface | Cutover treatment |
| --- | --- |
| `FlattenFormFields` is fail-closed | implement before Rust engine completion |
| public-key encryption is fail-closed | Adobe.PubSec RSA/CMS s3/s4/s5 recipients with RC4/AESV2/AESV3 are implemented; invalid handlers fail closed |
| filtered page-content scaling is rejected | implement safe decode/transform/re-encode |
| form creation is narrow | implement OXPDF-required field types and appearances |
| text extraction/layout is narrower than PyPDF-class expectations | add structured extraction and broader text-state coverage |
| page merging lacks a general compositor/resource-collision layer | implement deterministic resource remapping |
| annotation API lacks general create/remove | implement common OXPDF subtypes |
| metadata/XMP/outlines/destinations/attachments have limited mutation APIs | supported-filter XMP reads, verified unfiltered XMP updates, and exact inventory-bound attachment byte reads are implemented; broader tree mutation shapes remain |
| image replacement requires low-level PDF knowledge | add normalized encoded-image input |
| encryption options/permissions are incomplete | Standard Security R2/R3/R4/R5/R6 and exact claimed crypt-filter policies are implemented and vector-tested |
| repeated operations reparse/rewrite independently | add a bounded batch edit request that creates one plan and one output |
| corpus is only 22 checked-in PDFs | establish licensed, manifested compatibility/adversarial corpora |

“Feature richness versus PyPDF” is not defined as cloning every convenience
method. The completion target is the feature set OXPDF needs for editing and
document security, with Binas's stronger fail-closed and verification contracts.
PyPDF is used as an interoperability peer for overlapping operations.

## 11. Migration method

### Gate 0 — freeze contracts and decide the OXPDF bridge

Deliverables:

- tag or immutable commit for the Go oracle;
- machine-readable operation manifest covering every public PDF function,
  CLI command, WASM export, option enum, error class, and JSON schema;
- fixture license/provenance manifest;
- ADR recording direct Rust OXPDF integration;
- benchmark baseline for representative parse, query, canonical edit,
  preserve-structure edit, page merge, encryption, and signature inspection;
- explicit list of current accepted and rejected encryption algorithms.

Exit: Go behavior can be replayed without relying on undocumented knowledge.

### Gate 1 — workspace and stable DTO contract

Deliverables:

- four-crate workspace only;
- error, limits, selector, request, plan, report, and verification DTOs;
- JSON schemas/golden snapshots shared by CLI and WASM;
- dependency and license decisions;
- CI matrix and formatting/lint policy.

Exit: no PDF parser claim yet; API review prevents later parallel APIs.

### Gate 2 — bounded syntax, object graph, and xref

Deliverables:

- lexer/value parser;
- indirect objects and streams;
- classic/xref-stream/hybrid/revision/object-stream support;
- catalog/page/name-tree graph traversal;
- all applicable resource budgets;
- Go-oracle and malformed-input differential vectors.

Exit: all supported corpus files inspect with normalized graph parity, and all
malformed/fuzz seeds fail without panic or budget escape.

### Gate 3 — filters, content, fonts, and query

Deliverables:

- complete parity filter matrix;
- content operators and inline images;
- text/font/CMap decoding and match ordering;
- structured extraction API;
- selector and query parity.

Exit: Go/PyPDF differential text/filter corpus passes within declared semantic
differences.

### Gate 4 — edit planning, writers, and verification

Deliverables:

- surgical, preserve-structure, canonical, incremental, and auto modes;
- text edit/layout policies;
- invariant engine and post-write reparse;
- batch edit request;
- output-growth limits and deterministic canonical serialization.

Exit: every writer has positive, refusal, malformed, and verification-failure
tests. Incremental output preserves the entire original prefix byte-for-byte.

### Gate 5 — pages, forms, annotations, images, XFA, overlays, OCR

Deliverables:

- current Go operation parity;
- completion items in sections 7.5 through 7.8 and section 10;
- resource collision and transitive-closure verification;
- bounded XML behavior.

Exit: OXPDF editing workflows for these domains pass end to end.

### Gate 6 — encryption and signatures

Deliverables:

- security handler vectors for every claimed revision/algorithm;
- permissions and password-change behavior;
- signature inspection/trust/time/modification reporting;
- two-phase signing plan and external signer integration;
- security review and secret-redaction tests.

Exit: interoperability peers open Rust output and Rust opens their supported
output; signature claims are separated and accurately reported.

### Gate 7 — CLI, WASM, and direct Rust consumer contract

Deliverables:

- all command paths and compatibility flags;
- JSON/exit/golden parity;
- WASM globals and browser memory-limit tests;
- direct `binas-pdf` consumer examples and ownership/lifetime tests.

Exit: consumers use Rust behind unchanged compatibility tests.

### Gate 8 — shadow operation and cutover

Deliverables:

- optional shadow comparison in OXPDF test/pre-release environments;
- full corpus and fuzz regression;
- performance/memory comparison;
- migration and rollback release notes;
- no Go-only public operation.

Exit: Rust becomes default while a documented rollback artifact remains.

### Gate 9 — remove the Go engine

Delete the Go PDF implementation only after the rollback window closes, no
supported consumer uses it, and release gates remain green. Preserve operation
vectors permanently; they become the compatibility contract rather than legacy
tests.

## 12. Verification strategy

### 12.1 Operation vectors

Each vector contains:

```text
case-id/
├── input.pdf (or generated fixture recipe)
├── request.json
├── expected.normalized.json
├── expected.error.json (for refusal cases)
├── invariants.json
└── provenance.toml
```

Normalize volatile data such as object numbers only when the operation contract
does not expose it. Never normalize away a broken reference, changed page order,
signature coverage, encryption state, or fallback.

Comparison rules:

- surgical edits compare all unaffected bytes and the exact declared span;
- incremental edits compare the complete original prefix and revision semantics;
- canonical output compares reparsed graph, visible/selectable semantics,
  security state, and invariants rather than raw bytes;
- errors compare stable code plus relevant object/span/context, not prose alone.

### 12.2 Test layers

- unit tests for syntax, filters, state machines, crypto vectors, and writers;
- port or represent every Go test in a Rust test or operation vector;
- property tests for parse/serialize/reparse, checked ranges, filter round trips,
  edit-plan determinism, and page-tree closure;
- differential tests against the frozen Go oracle;
- interoperability tests against current PyPDF for overlapping reads/writes;
- command-line validation with `qpdf` and `mutool` where available;
- PDF/A validation with veraPDF only for profiles Binas explicitly claims;
- fuzzing with [`cargo-fuzz`](https://rust-fuzz.github.io/book/cargo-fuzz.html).

Required fuzz targets:

- lexer/value parser;
- xref table and xref stream;
- object stream;
- each filter and filter chain;
- content stream and inline image;
- CMap/font encoding;
- page/name/form/annotation graph walkers;
- XFA and ALTO XML;
- encryption dictionary/authentication inputs;
- ByteRange, CMS, certificate, and timestamp metadata;
- every public byte-taking operation via a reduced limit configuration.

Fuzzing success means no panic, hang, out-of-budget allocation, integer wrap,
or output returned after failed verification. It does not mean every random file
must parse.

### 12.3 Corpus policy

Maintain separate manifested corpora:

- small redistributable unit fixtures;
- real-world licensed compatibility files;
- generated format-feature matrix;
- malformed/adversarial seeds;
- encryption/signature vectors with known credentials and trust chains;
- regression files tied to issue IDs.

CI uses the redistributable subset. Scheduled/private jobs may use corpora whose
licenses prevent publication. Every file records origin, license, expected
capabilities, and whether secrets are synthetic.

## 13. CI and release gates

Required on each pull request, scoped as appropriate:

- `cargo fmt --check`;
- `cargo clippy --all-targets --all-features -- -D warnings`;
- unit, integration, operation-vector, CLI snapshot, and WASM tests;
- native builds/tests on Windows, Linux, and macOS;
- browser WASM build and smoke execution;
- dependency license/advisory scan;
- Go oracle comparison while the Go engine exists;
- changed fuzz-target smoke runs;
- no public API change without schema/snapshot review.

Scheduled/release jobs add full corpus, longer fuzzing, interoperability tools,
performance/memory baselines, and OXPDF end-to-end suites.

Performance is a gate, not the primary design objective. Representative native
operations must not regress materially from the frozen Go baseline without a
documented reason. Peak memory must respect configured budgets. WASM bundle size
and peak heap are recorded per release.

## 14. Definition of done

The Rust rewrite is finished only when all statements below are true:

- [x] The workspace contains the four approved crates; any additional crate has
  a recorded independent-consumer or target-boundary justification.
- [ ] Every current public Go PDF operation is mapped to a Rust operation or an
  explicitly accepted non-goal.
- [ ] Every Go test is ported or represented by a named operation vector.
- [ ] All CLI commands, JSON schemas, meaningful exit behavior, and compatibility
  flags pass golden comparison.
- [ ] All three existing WASM globals and payload schemas pass browser tests.
- [ ] Rust OXPDF's complete engine-facing suite passes against `binas-pdf`.
- [ ] Syntax, xref, object streams, filters, fonts/CMaps, writers, pages, forms,
  annotations, XFA, overlays, OCR layers, encryption, and signatures meet their
  gates above.
- [ ] Required section 10 gap-closure items are implemented, not left as stubs.
- [ ] Unsupported public-key encryption variants and dynamic XFA return stable
  capability errors without mutation or misleading success claims.
- [ ] No untrusted-input path panics, performs unchecked range arithmetic, or
  bypasses configured depth/allocation/output limits.
- [ ] Fuzz targets have run for the release duration without an unresolved crash,
  hang, or budget escape.
- [ ] Canonical output reparses and satisfies semantic invariants; surgical and
  incremental modes satisfy their stronger byte-preservation contracts.
- [ ] Encryption and signature claims pass independent interoperability vectors.
- [ ] Windows, Linux, macOS, and browser WASM release artifacts build from a
  clean checkout.
- [ ] Dependency licenses/advisories and cryptographic/security review are clear.
- [ ] Release documentation describes capabilities, refusals, resource defaults,
  migration, and rollback truthfully.
- [ ] The rollback window has closed before the Go engine is removed.

## 15. Immediate implementation sequence

1. Commit this specification and the current Go refactor separately; do not mix
   Rust scaffolding with unrelated Go changes.
2. Freeze the Go oracle and generate the operation manifest/golden schemas.
3. Decide the OXPDF bridge with a small compile-and-call spike, then discard the
   spike or promote only the selected boundary.
4. Scaffold exactly the four approved crates and implement Gate 1 DTOs/limits.
5. Migrate vertically: open one PDF, parse its graph, query its text, plan one
   edit, write it, and verify it before broadening each subsystem.
6. Expand by the gates in section 11; do not delete Go behavior until its Rust
   replacement passes the shared operation vectors.

The vertical slice is sequencing, not a reduced finish line. Sections 7, 8, 10,
13, and 14 remain the release contract.
