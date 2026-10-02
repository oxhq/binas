# P5: bounded producer compatibility

This additional cohort uses original small documents emitted by installed
ReportLab 4.4.9 and pypdf 6.10.0, independently of the Binas writer. It has seven
intakes, three admitted edits, six exports, and three comparator negatives.
It qualifies these concrete cases; it does not certify general PDF compatibility.

The source/golden generators and text are Apache-2.0. The ReportLab TrueType
fixture embeds its installed Bitstream Vera font. Generation copies the full
installed redistribution notice alongside the PDFs and records the font and
notice SHA-256. No download, installation, private document, remote service or
unverified `testdata/pdf` file is used. PDFs, logs and renders stay outside the
checkout. The Python manifest records provenance, licenses, producer/tool
versions, source/golden hashes, object IDs and predeclared expectations.

## Contract and cases

The font-aware Rust operation is
`PdfRevision::plan(PdfRevisionRequest.page_text)/commit/export`: replace the first
whole `ALPHA` token on page zero with `OMEGA-LONG`, shared policy `Reject`.
`PdfSession::prepare_text_edit/accept_text_edit/export` and the public JS classes
remain the raw P2b v1 operation. Font-aware page edits are not implemented in the
JS API. All refusal tests assert unchanged text and snapshot. Source documents
have two 240-by-200-point pages; page one is untouched.

| Intake | Producer and shape | Rust page-text / raw session | Independent expectation |
| --- | --- | --- | --- |
| `pypdf-fonted` | pypdf, Type1 Helvetica, explicit injective ASCII CMap, plain stream; attachment and unreachable sentinel | Page-text succeeds, raw refuses `unsupported_feature` | One target stream; exact golden text/tokens/render in both modes; all other objects preserved |
| `reportlab-truetype` | ReportLab, embedded Vera TrueType and producer CMap, actual ASCII85+Flate chain | Page-text succeeds, raw refuses `unsupported_feature` | Same checks; embedded font/subset/CMap retained; replacement glyphs already present in original subset |
| `pypdf-raw` | pypdf, no font selection/resources; attachment and unreachable sentinel | Raw retained session succeeds in both modes | Exact golden decoded bytes/tokens/text; all other objects retained; appearance `NotChecked` |
| `pypdf-shared` | pypdf, two pages reference target Contents | Page-text refuses `unsafe_rewrite`; raw refuses `unsupported_feature` | No revision/text publication |
| `pypdf-missing-glyph` | pypdf, explicit CMap omits hyphen required by replacement | Page-text refuses `unsupported_feature`; raw refuses `unsupported_feature` | No revision/text publication |
| `reportlab-standard` | ReportLab, standard Helvetica without ToUnicode | Page-text and raw refuse `unsupported_feature` | Correctly readable source is outside the font-aware edit contract |
| `malformed-header` | Original malformed original text, not PDF | Open refuses `invalid_syntax` | Independent strict pypdf also rejects the header |

The session test config explicitly uses 8 MiB input and 16 MiB output, matching
the named JS session defaults in `bindings/binas-pdf-wasm/src/lib.rs`; its
logical budget is 128 MiB live/work. The JS harness uses 24 MiB live and 128 MiB
work. These logical reservations are not process heap/CPU guarantees.

## Independent checks and their qualifications

`qa/compatibility-p5/oracle.py` imports only the frozen P3a inventory/geometry,
renderer and diagnostic-control helpers. Its P5 generators, expectations and
comparator do not call Binas. Strict pypdf reads every xref object including the
unreachable sentinel. The oracle compares IDs/generations, geometry, trailer
values, all non-target typed dictionaries, encoded/decoded stream hashes and
lengths. The target dictionary must retain everything except `/Length`; its
complete decoded bytes and parsed operations must match the independent golden.
Full extracted page text must match the golden. Incremental preserves the exact
source prefix, two revisions and source startxref `/Prev`; Compact has one
revision, no `/Prev` and the same object set.

Poppler `pdfinfo` 26.07.0 independently parses every valid intake and all six
outputs/goldens with exit zero and empty stderr, comparing two-page geometry.
Poppler renders the four fonted exports/goldens/sources at pinned 96 DPI settings.
Every output page equals the golden's RGB pixels; page one also equals its source.
The target must change more than 50 pixels within the predeclared PDF rectangle
`[35,143,180,170]` (outer pixel rectangle `[46,40,240,76]`). Source and replacement
pixels are expected to differ. No fontless appearance assertion is made.

Source and Compact strict reader phases are warning-free. Incremental may emit
exactly the known nonzero-xref-subsection pypdf diagnostic. For each successful
source an independent append, serialized by pypdf with its source trailer,
reproduces precisely that diagnostic and passes the same inventory/text checks
before the candidate comparison. No permissive parser fallback is used.

The existing blank/Helvetica/Courier renderer controls must pass. Pypdf's
Helvetica sources/outputs/goldens emit precisely the two pinned ambient missing
`Symbol`/`ArialUnicode` display-font messages; ReportLab's embedded-font renders
must have empty stderr. Commands and diagnostics are retained. This is a local
qualified pixel comparison, not a general conformance or font-rendering claim.

Comparator controls prove that a missing target edit fails decoded/tokens/text
and target-pixel comparison, and damage to an unreachable sentinel fails the
non-target inventory check. These are deliberate synthetic controls, not
observed Binas defects. No motor change was required.

## Reproduction

Use the already-installed Python with pypdf/ReportLab/Pillow and Poppler; do not
install or silently substitute tools. On the recorded Windows environment:

```powershell
$p5Python = 'C:/Users/garae/.cache/codex-runtimes/codex-primary-runtime/dependencies/python/python.exe'
$p5Root = 'C:/Users/garae/Documents/Codex/2026-10-01/task-8/p56/p5'
& $p5Python -B qa/compatibility-p5/oracle.py generate "$p5Root/corpus"
$env:BINAS_COMPAT_CORPUS_MANIFEST = "$p5Root/corpus/manifest.json"
$env:BINAS_REQUIRE_COMPAT_CORPUS = '1'
$env:BINAS_P5_CORPUS_MANIFEST = "$p5Root/corpus/manifest.json"
$env:BINAS_REQUIRE_P5_CORPUS = '1'
$env:BINAS_P5_OUTPUT_DIR = "$p5Root/outputs"
$env:CARGO_TARGET_DIR = "$p5Root/target"
cargo test -p binas-pdf --test external_compatibility_corpus --test compatibility_p5 --offline --locked
& $p5Python -B qa/compatibility-p5/oracle.py verify "$p5Root/corpus" "$p5Root/outputs" "$p5Root/oracle-final"
# Artifact root must contain pkg-node and pkg-web generated from the exact candidate.
node qa/compatibility-p5/run.mjs ARTIFACT_ROOT "$p5Root/corpus" "$p5Root/outputs" "$p5Root/runtime"
```

Without their manifests the tests are optional local checks. The release gate sets
both `BINAS_REQUIRE_COMPAT_CORPUS=1` and `BINAS_REQUIRE_P5_CORPUS=1`, so a missing
manifest cannot pass. `BINAS_P5_CORPUS_MANIFEST` has P5-specific kind/target fields;
the general corpus gate keeps its existing manifest schema and environment contract.
Both variables point at the same P5 manifest for this candidate. The existing
corpus gate checks licensing fields, safe paths, hashes and open/validate/refusal
expectations; `compatibility_p5` checks edits, concrete changed IDs and atomic
refusals. `run.mjs` runs the identical public JS contract on Node and real
Chromium: two raw exports must exactly match independently qualified native
bytes; five fonted inputs must refuse the raw operation, and malformed input
must reject. It does not implement font-aware JS or run external readers inside
runtime receipts.

## Scope against the original plan

The planned producer diversity, licensed inputs, independent effect/refusal
checks, inventory, useful negatives and runtime mapping are covered for this
textual cohort. P5's broad form/tag checks and Oxpdf oracle harness remain outside
this agreed local text slice; no new form/tag capability or UI claim is made.
These are synthetic documents, not a representative real-world prevalence
sample. Two producers and finite shapes cannot support universal compatibility,
PDF/UA, secure redaction, reflow, OCR, arbitrary embedded font coverage or worker
recovery. The ReportLab ASCII85+Flate success qualifies that existing serializer
case; it does not broaden negotiated raw-profile filter support.

Evidence and candidate identity are recorded by the final release handoff.
Earlier failed local attempts are preserved: a harness live-budget mismatch
preceded session admission, and the initial oracle incorrectly expected ambient
font diagnostics for an embedded-font source. Both were corrected in the
harness/oracle; neither was a Binas engine failure.
