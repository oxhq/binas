# W-8BEN Variable-Length Rewrite

This document tracks the current proof slice for rewriting selectable W-8BEN PDF text without painting an overlay on top of the page.

For the CLI output contract, `validate` behavior, inspect metadata, and current failure modes, see [release-surface.md](release-surface.md). For the semantic guardrails around detection-only and fail-closed PDF surfaces, see [pdf-semantic-boundaries.md](pdf-semantic-boundaries.md).

## Current Support Boundary

Supported today:

- PDF files with a raw/uncompressed content stream.
- PDF files with direct `/Filter /FlateDecode`, `/Filter /LZWDecode`, `/Filter /ASCIIHexDecode`, `/Filter /ASCII85Decode`, or `/Filter /RunLengthDecode` content streams and direct or resolved indirect integer `/Length`.
- PDF files using the standard filter abbreviations `/Fl`, `/LZW`, `/AHx`, `/A85`, and `/RL` where the normalized filter or filter chain is otherwise listed here.
- Supported filter arrays composed only of FlateDecode, LZWDecode, ASCIIHexDecode, ASCII85Decode, RunLengthDecode, and their standard abbreviations.
- PDF files with FlateDecode or LZWDecode `/DecodeParms` limited to direct or resolved indirect dictionaries/arrays, omitted `/Predictor` defaulting to `1`, `/Predictor 1` with any direct signed integer geometry keys treated as no-op metadata, TIFF predictor `2` packed sample rows, or PNG predictors `10` through `15` with row width computed from `/Columns`, `/Colors`, and `/BitsPerComponent`.
- PDF files with LZWDecode `/DecodeParms` `/EarlyChange 0|1`, defaulting to `1`.
- `/DecodeParms` arrays that align one-for-one with the filter array; all-null arrays are accepted for supported reversible chains, while direct or resolved indirect non-null dictionaries are accepted only for Flate and LZW positions.
- A text-show node parsed from a direct PDF literal string, for example `(08\05515\0552024) Tj`.
- A text-show node parsed from a simple ASCII hex string, page font-scoped `/ToUnicode` CMap-backed hex string for simple `Tf` flows, or one unambiguous fallback `/ToUnicode` CMap.
- A text-show node parsed from a simple `TJ` array made of literal strings, simple ASCII hex strings, CMap-backed hex strings, and numeric spacing entries.
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
- Xref-stream and hybrid `/XRefStm` detection in generic parse, with graph/canonical edit support for supported content-stream text.
- Detection of `/Encrypt`, true signature marker names `/Sig`, `/ByteRange`, and `/SigFlags`, `/AcroForm`, `/XFA`, `/Annots`, and font/CMap markers. Encrypted PDFs fail closed unless `--password` selects the supported Standard Security RC4 R2/R3/R4 or R4 AESV2 path; digitally signed PDFs fail closed by default; residual boundary scanning skips literal strings, comments, and hex strings for non-signature boundary name markers too; XFA fails closed in generic parse/edit but has explicit packet listing and replacement commands; AcroForm, annotations, and font/CMap markers are reported as metadata and validation warnings because support is narrower than full PDF semantics.
- AcroForm field discovery through `form list`, including alternate `/TU` names, mapping `/TM` names, direct default `/DV` values, inherited/direct `/Ff` flags, `type_flag_names`, proven button appearance states, and directly decoded choice-field options; exact AcroForm text field `/V` updates, fully qualified parent/child field matching, non-editable choice-field value checks against proven direct `/Opt` options, editable choice-field literal updates, proven checkbox/button `/V` plus `/AS` updates, inherited `/FT /Btn` support, narrow parent-field radio updates through `form set`, and simple `--regenerate-appearance` widget streams for text/choice widgets with proven direct `/Rect`, explicit newlines, approximate wrapping, clipping, and height truncation; annotation discovery through `annot list`, including page metadata, direct common text metadata, direct numeric `/F` flags, direct numeric color and border metadata, direct four-number `/Rect` values, and `quad_points_count` when proven; annotation `/Contents` updates through `annot set-contents`, with explicit stale `/AP` removal available via `--remove-appearance` and simple text-like `/AP /N` generation available via `--regenerate-appearance` using the same approximate multiline layout; and directly represented XFA packet discovery/replacement through `xfa list` and `xfa replace --match-index`, including exact `--packet-kind`/`--label` selector filters, replacement that skips literal/hex/name packet labels, XML prolog/root diagnostics, decoded byte/text lengths, conservative packet kind, and XFA stream decode-error diagnostics in the list path.

Not supported yet:

- Broader `/DecodeParms` shapes, including scalar references, reference cycles, unsupported keys, and unknown predictors, or filters outside ASCIIHex, ASCII85, RunLength, Flate, LZW, and their standard abbreviations.
- Indirect `/Length` references with missing targets, non-integer object bodies, compressed targets, or shared/cyclic semantics beyond the resolved standalone integer object shape.
- Preserving original object-stream/xref-stream layout after canonical rewrite.
- Broad AcroForm field tree surgery beyond `form list` and supported `form set`, XFA packet-family semantics beyond `xfa list` diagnostics/kind/root classification and exact packet replacement, annotations beyond discovery/page/flags/rect/color/border/quad metadata, `/Contents`, and explicit stale `/AP` removal or regeneration, or full widget/annotation layout fidelity.
- Encrypted AESV3+/public-key/unsupported-crypt-filter shapes. The public password path supports Standard Security RC4 R2/R3/R4 and R4 AESV2 graph parse, query, canonical text edit, and re-encryption for normal encrypted strings, normal encrypted streams, encrypted object streams inflated into graph objects, encrypted xref streams decoded before graph parsing, plus stream-level `/Crypt` `/Identity` or `/StdCF`.
- Digital signatures beyond the explicit modes. `--signature-mode invalidate` canonical-writes only when invalidation is accepted. `--signature-mode preserve-incremental` is public for supported raw content-stream text edits in table-xref PDFs with parseable `/ByteRange`: it appends updated objects/xref/trailer with `/Prev`, preserves original bytes as a prefix, and compares signed byte ranges byte-for-byte. It does not cryptographically validate, re-sign, or support object-stream/xref-stream/filtered-stream targets.
- Broad font/CMap-aware text decoding, glyph substitution, layout reflow, or width checks beyond the current selectable-text parse and conservative raw simple-font width metadata.
- Overlay, stamp, raster, OCR, or visual fallback modes.
- Browser editor gaps beyond the current surface. `cmd/binas-wasm/editor.html` now renders PDF pages with PDF.js, navigates pages, zooms, overlays exact text highlights from PDF.js text content, runs verified WASM text edits, rerenders edited bytes, and downloads the edited PDF. It does not add a second PDF semantic verifier, broad layout editing, OCR, annotation drawing, or form authoring.

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
  --replace "05-05-2026" `
  --verify reparse,old-gone,new-selectable `
  -o $OutputPdf `
  --json

go run ./cmd/binas query $OutputPdf `
  --format pdf `
  --kind pdf.content.text_show `
  --text "05-05-2026" `
  --json
```

Expected proof:

- `go test ./...` passes.
- `validate` returns `valid: true` with empty `errors`. The W-8BEN fixture may report warnings for AcroForm, annotation, and font/CMap markers because those surfaces have narrower explicit commands or limited verification.
- `inspect` returns root metadata including `xref.has_table`, `xref.table_offset`, `xref.has_stream`, and `xref.object_count`.
- The first query returns one match for `08-15-2024`.
- The edit command returns JSON with `old_text_removed: true` and `new_text_selectable: true`.
- The output query returns one match for `05-05-2026`.
- The edit report shows `fallback_used: false`.

If `BINAS_W8BEN_PDF` is not set, `TestW8BENDateRewriteIsSelectableText` skips that real-file proof. The synthetic variable-length test still proves the byte-length mechanics by rewriting `08-15-2024` to `05-05-2026`, updating `/Length`, rebuilding `xref`, and verifying selectable text.

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
   - Treat unknown predictors, unsupported `/DecodeParms` keys, and unsupported filter names as separate residual boundaries until fixture-backed parser, edit, length-update, xref-rebuild, and verification proof lands.
   - Keep broader `/DecodeParms` shapes out of scope until fixture-backed.
   - Keep object-stream and xref-stream rewrites canonical; preserving original compressed structural layout is not a first contract.
   - Keep verification based on reparsing the produced PDF, not on trusting the edit path.

4. Add higher-level PDF surfaces incrementally.
   - Keep AcroForm fields, XFA, annotations, encryption, signatures, and font/CMap-aware decoding as separate commands or sub-flows.
   - Do not imply broad support for those surfaces until fixture-backed commands prove the exact semantics.

## Next Residual Boundaries

- Broader `/DecodeParms` shapes remain unsupported beyond direct or resolved indirect Flate/LZW parameter dictionaries/arrays for `/Predictor 1`, TIFF predictor `2` packed sample rows, PNG predictors `10` through `15`, and LZW `/EarlyChange 0|1`.
- Filter chains remain unsupported when they contain filters outside ASCIIHex, ASCII85, RunLength, Flate, LZW, and their standard abbreviations.
- Object-stream/xref-stream/hybrid-xref original layout preservation remains out of scope; canonical rewrite is the supported path.
- Other `/DecodeParms` shapes, including scalar references, reference cycles, unsupported keys, and unknown predictors, filters outside ASCIIHex, ASCII85, RunLength, Flate, LZW, and their standard abbreviations, encrypted AESV3+/public-key/unsupported-crypt-filter shapes, default signed-PDF edits, cryptographic signature validation/re-signing, broad XFA editing beyond exact packet selectors, broad appearance/layout fidelity, and broad font/CMap/layout support remain out of scope.
