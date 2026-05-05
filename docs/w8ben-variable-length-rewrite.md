# W-8BEN Variable-Length Rewrite

This document tracks the current proof slice for rewriting selectable W-8BEN PDF text without painting an overlay on top of the page.

For the CLI output contract, `validate` behavior, inspect metadata, and current failure modes, see [release-surface.md](release-surface.md). For the semantic guardrails around detection-only and fail-closed PDF surfaces, see [pdf-semantic-boundaries.md](pdf-semantic-boundaries.md).

## Current Support Boundary

Supported today:

- PDF files with a raw/uncompressed content stream.
- PDF files with a direct `/Filter /FlateDecode` content stream and direct or resolved indirect integer `/Length`.
- PDF files with a direct `/Filter /ASCIIHexDecode` content stream and direct or resolved indirect integer `/Length`.
- PDF files with a direct `/Filter /ASCII85Decode` content stream and direct or resolved indirect integer `/Length`.
- PDF files with a direct `/Filter /RunLengthDecode` content stream and direct or resolved indirect integer `/Length`.
- PDF files with a single-item `/Filter [/FlateDecode]` content stream and direct or resolved indirect integer `/Length`.
- PDF files with the exact `/Filter [/ASCII85Decode /FlateDecode]` content stream filter chain and direct or resolved indirect integer `/Length`.
- PDF files with the exact `/Filter [/ASCIIHexDecode /FlateDecode]` content stream filter chain and direct or resolved indirect integer `/Length`.
- PDF files with the exact `/Filter [/RunLengthDecode /FlateDecode]` content stream filter chain and direct or resolved indirect integer `/Length`.
- PDF files with the exact `/Filter [/ASCII85Decode /RunLengthDecode /FlateDecode]` content stream filter chain and direct or resolved indirect integer `/Length`.
- PDF files with the exact `/Filter [/ASCIIHexDecode /RunLengthDecode /FlateDecode]` content stream filter chain and direct or resolved indirect integer `/Length`.
- PDF files with the exact `/Filter [/RunLengthDecode /ASCII85Decode /FlateDecode]` content stream filter chain and direct or resolved indirect integer `/Length`.
- PDF files with the exact `/Filter [/RunLengthDecode /ASCIIHexDecode /FlateDecode]` content stream filter chain and direct or resolved indirect integer `/Length`.
- PDF files with FlateDecode `/DecodeParms` limited to `/Predictor 1` plus omitted/default geometry keys, or PNG predictors `10` through `15` constrained to direct `/Columns >= 1`, or omitted `/Columns` defaulting to 1 when `/Colors` is also 1/default, byte-aligned predictor rows, and the fixture-backed `/BitsPerComponent 8 /Colors 1`, `/BitsPerComponent 8 /Colors 3`, and `/BitsPerComponent 16 /Colors 2` lanes.
- Supported `/DecodeParms` arrays that align exactly with supported filter chains: `/Filter [/FlateDecode] /DecodeParms [<<...>>]`; `/Filter [/ASCII85Decode /FlateDecode]`, `/Filter [/ASCIIHexDecode /FlateDecode]`, and `/Filter [/RunLengthDecode /FlateDecode]` with `/DecodeParms [null <<...>>]`; or the exact three-stage chains above with `/DecodeParms [null null <<...>>]`.
- A text-show node parsed from a direct PDF literal string, for example `(08\05515\0552024) Tj`.
- A text-show node parsed from a simple ASCII hex string, page font-scoped `/ToUnicode` CMap-backed hex string for simple `Tf` flows, or one unambiguous fallback `/ToUnicode` CMap.
- A text-show node parsed from a simple `TJ` array made of literal strings, simple ASCII hex strings, and numeric spacing entries.
- `pdf.content.text_show` queries by decoded text value and exact metadata filters such as `--meta operator=TJ`.
- Replacement text whose encoded literal-string length may be shorter or longer than the original.
- Direct integer stream lengths, for example `/Length 27`.
- Indirect stream lengths of the form `/Length N G R` when the referenced object body is only a nonnegative integer.
- Rebuilding a basic xref table and trailer after the byte-length change.
- Verification that the output reparses, the old decoded text is gone, and the new decoded text is selectable through the adapter.
- Fail-closed behavior when the selected span no longer matches the planned source bytes.
- `validate` as a strict parser smoke check for this support boundary.
- `inspect` xref metadata for table xref files: `has_table`, `table_offset`, `has_stream`, `has_object_stream`, `object_count`, and `object_stream_count`.
- Object-stream graph parsing and canonical rewrite as normal indirect objects for supported content-stream text.
- Xref-stream detection in generic parse, with graph/canonical edit support for supported content-stream text.
- Detection of `/Encrypt`, true signature marker names `/Sig`, `/ByteRange`, and `/SigFlags`, `/AcroForm`, `/XFA`, `/Annots`, and font/CMap markers. Encrypted and digitally signed PDFs fail closed by default; signature marker scanning skips literal strings, comments, and hex strings; XFA fails closed in generic parse/edit but has explicit packet listing and replacement commands; AcroForm, annotations, and font/CMap markers are reported as metadata and validation warnings because support is narrower than full PDF semantics.
- AcroForm field discovery through `form list`, including inherited/direct `/Ff` flags, proven button appearance states, and directly decoded choice-field options; exact AcroForm text field `/V` updates, fully qualified parent/child field matching, proven checkbox/button `/V` plus `/AS` updates, inherited `/FT /Btn` support, and narrow parent-field radio updates through `form set`; annotation discovery through `annot list`, including page metadata, direct numeric `/F` flags, and direct four-number `/Rect` values when proven; annotation `/Contents` updates through `annot set-contents`, with explicit stale `/AP` removal available via `--remove-appearance`; and directly represented XFA packet discovery/replacement through `xfa list` and `xfa replace --match-index`, including XML prolog/root diagnostics, conservative packet kind, and XFA stream decode-error diagnostics in the list path.

Not supported yet:

- Other multi-filter arrays, broader `/DecodeParms` shapes, or filters outside standalone ASCIIHex, ASCII85, RunLength, Flate, and the exact two- and three-stage chains listed in the support boundary.
- Indirect `/Length` references with missing targets, non-integer object bodies, compressed targets, or shared/cyclic semantics beyond the resolved standalone integer object shape.
- Preserving original object-stream/xref-stream layout after canonical rewrite.
- Broad AcroForm field tree surgery beyond `form list` and supported `form set`, XFA packet-family semantics beyond `xfa list` diagnostics/kind/root classification and exact packet replacement, annotations beyond discovery/page/flags/rect metadata, `/Contents`, and explicit stale `/AP` removal, or widget/annotation appearance regeneration.
- Encrypted PDFs.
- Digital signatures, signature preservation, or incremental signed-document updates.
- Broad font/CMap-aware text decoding, glyph substitution, layout reflow, or width checks beyond the current selectable-text parse.
- Overlay, stamp, raster, OCR, or visual fallback modes.
- Browser or WASM runtime support.

## Exact W-8BEN Proof Commands

Run these from the repository root in PowerShell. The commands assume the source fixture path is provided through `BINAS_W8BEN_PDF`; the fixture itself is intentionally not committed.

```powershell
$InputPdf = $env:BINAS_W8BEN_PDF
if (-not $InputPdf) { throw "BINAS_W8BEN_PDF must point to a local W-8BEN PDF fixture" }
$OutputPdf = "$PWD\tmp\w8ben-binas-output.pdf"

go test ./...

go run ./cmd/binas validate $InputPdf --format pdf --json

go run ./cmd/binas inspect $InputPdf --format pdf --json

go run ./cmd/binas query $InputPdf `
  --format pdf `
  --kind pdf.content.text_show `
  --text "08-15-2024" `
  --json

go run ./cmd/binas edit $InputPdf `
  --format pdf `
  --kind pdf.content.text_show `
  --text "08-15-2024" `
  --replace "May 5, 2026" `
  --verify reparse,old-gone,new-selectable `
  -o $OutputPdf `
  --json

go run ./cmd/binas query $OutputPdf `
  --format pdf `
  --kind pdf.content.text_show `
  --text "May 5, 2026" `
  --json
```

Expected proof:

- `go test ./...` passes.
- `validate` returns `valid: true` with empty `errors`. The W-8BEN fixture may report warnings for AcroForm, annotation, and font/CMap markers because those surfaces have narrower explicit commands or limited verification.
- `inspect` returns root metadata including `xref.has_table`, `xref.table_offset`, `xref.has_stream`, and `xref.object_count`.
- The first query returns one match for `08-15-2024`.
- The edit command returns JSON with `old_text_removed: true` and `new_text_selectable: true`.
- The output query returns one match for `May 5, 2026`.
- The edit report shows `fallback_used: false`.

If `BINAS_W8BEN_PDF` is not set, `TestW8BENDateRewriteIsSelectableText` skips that real-file proof. The synthetic variable-length test still proves the byte-length mechanics by rewriting `08-15-2024` to `May 5, 2026`, updating `/Length`, rebuilding `xref`, and verifying selectable text.

## Incremental Roadmap

1. Keep the direct-length literal-string path tight.
   - Preserve fail-closed source-span checks.
   - Keep the CLI proof command stable for `inspect`, `query`, and `edit`.
   - Add regression fixtures for shorter and longer literal-string replacements.

2. Expand direct PDF stream coverage only after fixtures prove it.
   - Keep indirect `/Length` support limited to the documented resolved-object shape, including updating the referenced length object.
   - Keep unresolved, non-integer, cyclic, shared, or compressed length objects fail-closed until each shape has fixtures.
   - Add focused tests for multiple streams and multiple matching text nodes.
   - Make unsupported stream dictionaries produce explicit errors.

3. Expand compressed stream integration.
   - Treat omitted PNG `/Columns` when `/Colors` is not 1/default, non-byte-aligned predictor rows, unknown predictors, and new filter chains beyond the exact supported set as separate residual boundaries until fixture-backed parser, edit, length-update, xref-rebuild, and verification proof lands.
   - Keep broader filter chains and broader `/DecodeParms` shapes out of scope until fixture-backed.
   - Keep object-stream and xref-stream rewrites canonical; preserving original compressed structural layout is not a first contract.
   - Keep verification based on reparsing the produced PDF, not on trusting the edit path.

4. Add higher-level PDF surfaces later.
   - Keep AcroForm fields, XFA, annotations, encryption, signatures, and font/CMap-aware decoding as separate commands or sub-flows.
   - Do not imply broad support for those surfaces until fixture-backed commands prove the exact semantics.

## Next Residual Boundaries

- Broader `/DecodeParms` shapes remain unsupported beyond FlateDecode with `/Predictor 1` plus omitted/default geometry keys, or PNG predictors `10` through `15` constrained to direct `/Columns >= 1` or omitted `/Columns` with `/Colors` 1/default, byte-aligned predictor rows, and the fixture-backed bit-depth/color lanes.
- Broader filter chains remain unsupported unless they are one of the exact supported chains listed in the current support boundary.
- Object-stream/xref-stream original layout preservation remains out of scope; canonical rewrite is the supported path.
- Other `/DecodeParms` shapes, filters outside standalone ASCIIHex, ASCII85, RunLength, Flate, and the exact two- and three-stage chains listed in the support boundary, default signed-PDF edits, encryption, appearance regeneration, broad XFA editing, broad font/CMap/layout support, browser, and WASM support remain out of scope.
