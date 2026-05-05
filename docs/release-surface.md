# Release Surface

This document describes the current public CLI surface for the PDF proof slice. It is intentionally narrower than a general PDF editor.

For the semantic edit contract behind these CLI fields, including the difference between fail-closed boundaries and detection-only warnings, see [pdf-semantic-boundaries.md](pdf-semantic-boundaries.md).

## Validate

```powershell
go run ./cmd/binas validate C:\path\file.pdf --format pdf --json
```

JSON mode returns:

```json
{
  "format": "pdf",
  "valid": true,
  "errors": [],
  "warnings": []
}
```

`validate` reads one input file, creates the requested adapter, and runs a strict parse. Parser-level failures are returned as `valid: false` with an error string in `errors`. The current PDF parser reports malformed input such as a missing `%%EOF` marker this way.

For structural PDF failures where the adapter can still build the document root, JSON mode includes `root` metadata alongside `valid: false`. This is used for malformed or unsupported xref streams so callers can see the structural boundary that caused validation to fail.

For non-blocking high-level PDF markers, JSON mode keeps `valid: true` and reports warnings. AcroForm dictionaries, annotations, and font/CMap markers are detected because their support is deliberately narrower than general PDF semantic editing: field discovery uses `form list`, field updates use `form set`, annotation updates only write `/Contents`, and CMap support is limited to page font-scoped `/ToUnicode` maps for simple `Tf` flows plus one unambiguous fallback map for hex text.

## Query

```powershell
go run ./cmd/binas query C:\path\file.pdf --format pdf --kind pdf.content.text_show --text "08-15-2024" --meta operator=Tj --json
```

`query` selects parsed nodes by kind and decoded text. Repeatable `--meta key=value` filters match node metadata exactly using the core selector rules. This is useful for distinguishing `Tj` from `TJ` text-show nodes without introducing a separate query language.

Command-level failures are not wrapped into the JSON validation envelope. Unsupported formats, missing files, unreadable paths, bad flags, or the wrong number of input files return command errors.

By default, invalid PDFs are reported in the validation envelope and the command exits successfully. Add `--fail-on-invalid` when parser-level invalid results should still print the same JSON envelope but also return a non-zero exit code. Command-level failures continue to be normal command errors.

## Inspect

```powershell
go run ./cmd/binas inspect C:\path\file.pdf --format pdf --json
```

JSON mode returns the adapter format, total parsed node count, and root metadata:

```json
{
  "format": "pdf",
  "nodes": 276,
  "root": {
    "header": "%PDF-1.3",
    "pages": 1,
    "size": 79881,
    "boundaries": {
      "has_acroform": true,
      "has_annotations": true,
      "has_cid_font_markers": false,
      "has_cmap_markers": true,
      "has_encrypt": false,
      "has_font_markers": true,
      "has_signature": false,
      "has_tounicode_cmap": true,
      "has_xfa": false,
      "text_decoding_support": "simple literal, ASCII hex operands, page font-scoped ToUnicode CMaps for simple Tf flows, and one unambiguous ToUnicode CMap fallback"
    },
    "xref": {
      "has_object_stream": false,
      "has_stream": false,
      "has_table": true,
      "object_count": 66,
      "object_stream_count": 0,
      "table_offset": 78462
    }
  }
}
```

The xref metadata is a summary of the current file shape:

- `has_table`: a literal xref table marker was found.
- `table_offset`: byte offset for the table marker, or `-1` when no table marker is found.
- `has_stream`: an xref stream object was detected.
- `has_object_stream`: an object stream was detected.
- `object_count`: count of indirect object headers found by the adapter's xref summary pass.
- `stream_count`: count of xref stream objects detected by the adapter's xref summary pass.
- `object_stream_count`: count of object stream objects detected by the adapter's xref summary pass.
- `objects`: indirect object metadata with object number, generation, and byte offset.
- `stream_objects`: xref stream object metadata with object number, generation, and byte offset.
- `object_stream_objects`: object stream object metadata with object number, generation, and byte offset.

Parseable xref-stream PDFs route through the graph path for generic parse, query, and supported text rewrites. Malformed or unsupported xref streams still fail closed with `unsupported PDF: xref streams are not implemented`; `inspect --json` still emits root metadata and includes `parse_error`, while `validate --json` emits `valid: false`, `errors`, and the same root metadata.

Object streams are detected, inflated into graph objects, and canonical-written back as normal indirect objects for graph rewrites. Surgical mode still refuses object-stream PDFs because it cannot prove byte-local safety inside compressed object containers. Detection is not a broad regex over dictionary bytes: `/ObjStm` in unrelated keys, nested dictionaries, or string literals is ignored to avoid false object-stream reports.

The metadata describes the input shape. `has_object_stream` also implies the graph path parsed object-stream contents when parse succeeds. `has_stream` means xref-stream metadata was detected; parseable xref streams use the graph path, while malformed or unsupported xref streams still report a parse error.

Stream nodes expose parse metadata for callers that need to inspect the byte boundary before planning edits. `encoded_length` is set for every detected stream node, `decoded_length` is set only when decoding succeeds or a raw graph stream is known to be unfiltered, and `filter_chain` lists the normalized direct filter names when a direct `/Filter` name or array is present.

The boundary metadata is detection-only for high-level PDF surfaces:

- `has_encrypt`: `/Encrypt` was detected. Encrypted PDFs fail parse with the explicit password-capable-path error because `Adapter.Parse` does not decrypt.
- `has_signature`: a true PDF name token for `/Sig`, `/ByteRange`, or `/SigFlags` was detected. The scanner skips literal strings, comments, and hex strings. Digitally signed PDFs fail parse by default because rewriting would invalidate signature semantics.
- `has_acroform`: `/AcroForm` was detected. Field discovery uses the narrower `form list` command, including inherited field type, inherited/direct `/Ff` flags for `read_only`, `required`, and `no_export`, sorted proven button appearance states, and directly decoded choice-field `/Opt` options. Field value updates require `form set`; text fields write `/V` and set `/NeedAppearances`, while proven checkbox/button fields write `/V` and `/AS` from `/AP /N`, including inherited `/FT /Btn` and a narrow parent-field radio shape. Widget appearance regeneration is not implemented.
- `has_xfa`: `/XFA` was detected. Generic parser/edit paths fail closed; `xfa list` reports directly represented packet metadata, conservative `packet_kind`, XML prolog/root diagnostics when safely detectable, and stream decode errors when a referenced stream packet cannot be decoded, and `xfa replace` can update exactly one directly represented XFA packet occurrence with `--match-index` required when multiple occurrences match.
- `has_annotations`: `/Annots` was detected. `annot list` reports page index/object metadata when a candidate can be proven from page `/Annots`, direct four-number `/Rect` metadata when available, and direct numeric `/F` annotation flags decoded into common flag names. `annot set-contents` can update dictionary `/Contents`; by default it preserves `/AP`, and `--remove-appearance` explicitly removes stale annotation `/AP` while reporting appearance invalidation/removal. Annotation/widget appearance regeneration is not implemented.
- `has_font_markers`, `has_cmap_markers`, `has_tounicode_cmap`, and `has_cid_font_markers`: font/CMap-related markers were detected. Text extraction supports simple literal operands, simple ASCII hex operands, page font-scoped `/ToUnicode` maps for simple `Tf` flows, and one unambiguous fallback map for hex operands; glyph metrics, widths, and layout are not verified.

These fields do not grant broad semantic edit support. They are guardrails for callers: encryption and default signature paths stop parsing, XFA requires the explicit packet command, malformed xref streams stop parsing, and object streams require graph/canonical rewrite for edits. AcroForm, annotations, and font/CMap markers are warning-only because the adapter may still rewrite a supported content-stream text operand without updating every higher-level system.

## Current Failure Modes

The current PDF slice is supported for direct literal-string, simple ASCII hex-string, and simple literal/ASCII-hex `TJ` array edits in raw/uncompressed content streams, direct `/Filter /FlateDecode`, `/Filter /ASCIIHexDecode`, `/Filter /ASCII85Decode`, and `/Filter /RunLengthDecode` streams, single-item `/Filter [/FlateDecode]` streams, and exact `/Filter [/ASCII85Decode /FlateDecode]`, `/Filter [/ASCIIHexDecode /FlateDecode]`, `/Filter [/RunLengthDecode /FlateDecode]`, `/Filter [/ASCII85Decode /RunLengthDecode /FlateDecode]`, `/Filter [/ASCIIHexDecode /RunLengthDecode /FlateDecode]`, `/Filter [/RunLengthDecode /ASCII85Decode /FlateDecode]`, or `/Filter [/RunLengthDecode /ASCIIHexDecode /FlateDecode]` streams with direct or resolved indirect integer `/Length` values. Known failure modes are explicit:

- Non-PDF input fails as `not a PDF file`.
- Strict PDF parsing fails malformed input without `%%EOF` as `malformed PDF: missing EOF marker`.
- Malformed or unsupported xref-stream parsing fails as `unsupported PDF: xref streams are not implemented`; parseable xref streams use the graph path.
- Surgical object-stream edits fail as `surgical rewrite does not support PDFs with xref streams or object streams; use --rewrite auto or --rewrite canonical`.
- Encrypted PDFs fail as `unsupported PDF: encrypted PDFs require an explicit password-capable path; Adapter.Parse does not decrypt`.
- Digitally signed PDFs fail as `unsupported PDF: digital signatures require explicit invalidation mode; Adapter.Parse refuses silent signature invalidation`.
- Generic XFA parser/edit paths fail as `unsupported PDF: XFA forms are not implemented`; `xfa list` and `xfa replace` are the explicit packet-level exceptions.
- AcroForm dictionaries, annotations, and font/CMap markers do not make validation invalid by themselves, but they produce warnings because support is narrower than the full PDF semantics.
- Edits fail when no node matches, more than one node matches without `--match-index`, or `--match-index` is outside the match set.
- Edits fail when the selected node has no editable encoded span or the planned span no longer matches the source bytes.
- Streams without a direct or supported indirect integer `/Length` fail as `unsupported stream: /Length must be an integer or indirect integer reference`.
- Indirect `/Length` is supported only for `/Length N G R` references whose target object body is only a nonnegative integer. Missing or non-integer targets fail as `unsupported stream: /Length reference must resolve to an integer object`.
- Filter arrays outside the supported exact chains fail as `unsupported stream: /Filter arrays are not implemented`.
- Unsupported `TJ` arrays fail closed when they contain non-string/non-number elements, unsupported hex encodings, or CMap-backed array cases outside the simple literal/ASCII-hex support.
- Unsupported filters fail closed with explicit errors such as `unsupported PDF stream filter "LZWDecode"`.
- Unsupported `/DecodeParms` shapes fail closed with explicit errors such as `unsupported stream: /DecodeParms PNG predictors require /Colors 1`, `unsupported stream: /DecodeParms PNG predictor row width must be byte-aligned`, or `unsupported stream: /DecodeParms is not implemented`.

Signed PDFs have one explicit invalidation helper for canonical writes: `ApplyCanonicalEditInvalidatingSignatures`. It is not used by the default adapter parse/edit path. CLI callers must opt in with `binas edit --rewrite canonical --allow-signature-invalidation`, or `--rewrite auto --allow-signature-invalidation` when auto selects the canonical path. JSON output includes `signature_invalidation: "digital signatures invalidated; not preserved or re-signed"` on that explicit path. The helper verifies only structural/text invariants under the same invalidation mode and does not verify, preserve, or re-sign cryptographic signatures.
- Xref rebuild fails when a table xref is missing or no indirect objects are found. Existing non-zero generation objects are emitted in generation-aware xref rows.
- Verification fails the command if the output does not reparse, the old decoded text remains, or the replacement text is not selectable through the adapter.

Current non-goals:

- Indirect `/Length` references outside `/Length N G R` standalone integer objects are not supported.
- Broader filter arrays, broader `/DecodeParms` shapes, and filters outside standalone ASCIIHex, ASCII85, RunLength, Flate, and the exact two- and three-stage chains listed above are not supported.
- Encrypted PDFs, default signed-PDF edits, widget/annotation appearance regeneration, broad XFA editing, OCR, raster, overlay, and stamp fallbacks are not supported.
- Broad font/CMap-aware text decoding, glyph widths, visual layout, and page rendering are not verified beyond the current selectable-text adapter parse.
- Browser and WASM runtimes are out of scope for the current Go-first CLI surface.

## Flate Roadmap

Direct `/Filter /ASCIIHexDecode`, `/Filter /ASCII85Decode`, `/Filter /RunLengthDecode`, and `/Filter /FlateDecode` streams, single-item `/Filter [/FlateDecode]` arrays, and exact `/Filter [/ASCII85Decode /FlateDecode]`, `/Filter [/ASCIIHexDecode /FlateDecode]`, `/Filter [/RunLengthDecode /FlateDecode]`, `/Filter [/ASCII85Decode /RunLengthDecode /FlateDecode]`, `/Filter [/ASCIIHexDecode /RunLengthDecode /FlateDecode]`, `/Filter [/RunLengthDecode /ASCII85Decode /FlateDecode]`, and `/Filter [/RunLengthDecode /ASCIIHexDecode /FlateDecode]` arrays are wired into the parser and edit path for direct and supported indirect integer `/Length` streams. The adapter can:

- Detect supported direct filters and supported exact filter arrays on the containing stream.
- Decode the stream, parse the decoded content, rewrite the selected literal string, re-encode the stream, and update `/Length` or its referenced integer object.
- Preserve fail-closed span checks across decoded and original byte ranges.
- Rebuild the xref table/trailer after byte-length changes.
- Verify by reparsing the produced PDF and querying the replacement text, not by trusting the edit path.

Remaining filter work is intentionally narrower: broader filter arrays and broader `/DecodeParms` shapes need separate fixture-backed slices before they are public support. Filters outside standalone ASCIIHex, ASCII85, RunLength, Flate, and the exact two- and three-stage chains listed above remain out of scope.

## DecodeParms

`/DecodeParms` support is limited to FlateDecode streams with `/Predictor 1` and either omitted geometry keys or explicit default geometry keys (`/Columns 1`, `/Colors 1`, `/BitsPerComponent 8`), byte-aligned TIFF predictor `2`, or PNG predictors `10` through `15` constrained to direct `/Columns >= 1`, or omitted `/Columns` defaulting to 1 when `/Colors` is also 1/default, and byte-aligned predictor rows. The current fixture-backed PNG predictor lanes are `/BitsPerComponent 8 /Colors 1`, `/BitsPerComponent 8 /Colors 3`, and `/BitsPerComponent 16 /Colors 2`. Omitted `/Colors` defaults to 1 and omitted `/BitsPerComponent` defaults to 8. For filter arrays, `/DecodeParms` entries must align one-for-one with the exact supported filter chain. The supported shapes are `/Filter [/FlateDecode] /DecodeParms [<<...>>]`; `/Filter [/ASCII85Decode /FlateDecode] /DecodeParms [null <<...>>]`, `/Filter [/ASCIIHexDecode /FlateDecode] /DecodeParms [null <<...>>]`, and `/Filter [/RunLengthDecode /FlateDecode] /DecodeParms [null <<...>>]`; and any exact supported three-stage chain with `/DecodeParms [null null <<...>>]`, where `null` entries belong to wrapper filters and the dictionary entry belongs to `/FlateDecode`.

Everything else under `/DecodeParms` remains unsupported unless a later pass adds focused fixtures and proof for that exact shape. This excludes non-default geometry keys with `/Predictor 1`, omitted PNG `/Columns` when `/Colors` is not 1/default, unknown predictor values, non-byte-aligned predictor rows, unfixture-backed bit-depth/color lanes, unsupported filters, and mixed filter chains beyond the currently supported exact chains.

## Residual Boundary Fixtures

Synthetic corpus fixtures live under `testdata/pdf`. They are small PDFs for parser and edit-boundary checks, not examples of broad PDF compatibility.

- `uncompressed-direct-length.pdf`: supported direct `/Length` baseline for variable-length literal-string rewrites.
- `multiple-streams.pdf`: supported multiple-stream baseline for query selection and selected-match edits.
- `malformed-missing-eof.pdf`: strict-parse invalid input for `validate`.
- `indirect-length-uncompressed.pdf`: supported indirect `/Length` boundary fixture for `/Length N G R` references that resolve to a standalone integer object.
- `asciihex-content-stream.pdf`: supported standalone `/Filter /ASCIIHexDecode` content-stream boundary.
- `asciihex-flate-content-stream.pdf`: supported exact `/Filter [/ASCIIHexDecode /FlateDecode]` content-stream boundary.
- `ascii85-flate-content-stream.pdf`: supported exact `/Filter [/ASCII85Decode /FlateDecode]` content-stream boundary.
- `flate-decodeparms-predictor12-columns1.pdf`: supported narrow `/DecodeParms` PNG predictor boundary fixture.
- `flate-decodeparms-predictor12-columns4.pdf`: supported multi-column `/DecodeParms` PNG predictor boundary fixture.
- `flate-decodeparms-predictor12-rgb.pdf`: supported `/DecodeParms` PNG predictor fixture for `/Columns 1 /Colors 3 /BitsPerComponent 8`.
- `flate-decodeparms-predictor12-bpc16.pdf`: supported `/DecodeParms` PNG predictor fixture for `/Columns 1 /Colors 2 /BitsPerComponent 16`.
- `xref-stream.pdf`: generic-parse unsupported xref-stream detection fixture.
- `object-stream.pdf`: object-stream graph parse fixture.

## Next Residual Boundaries

- Broader filter arrays remain unsupported unless explicitly listed above.
- Object streams have graph/canonical support for the fixture-backed shapes, but preserving original object-stream layout remains out of scope.
- Unresolved, non-integer, compressed, shared, or cyclic indirect `/Length` references remain outside the current `/Length N G R` standalone integer support shape.
- Broader `/DecodeParms` shapes remain unsupported beyond FlateDecode with `/Predictor 1` plus omitted/default geometry keys or PNG predictors `10` through `15` constrained to direct `/Columns >= 1`, or omitted `/Columns` with `/Colors` 1/default, byte-aligned predictor rows, and the fixture-backed bit-depth/color lanes.
- Filters outside standalone ASCIIHex, ASCII85, RunLength, Flate, and the exact two- and three-stage chains listed above, default signed-PDF edits, encryption, appearance regeneration, broad XFA editing, broad font/CMap/layout support, browser, and WASM surfaces remain out of scope.
