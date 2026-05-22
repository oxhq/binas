# Release Surface

This document describes the current public CLI surface for the PDF proof slice. It is intentionally narrower than a general PDF editor.

For the semantic edit contract behind these CLI fields, including the difference between fail-closed boundaries and detection-only warnings, see [pdf-semantic-boundaries.md](pdf-semantic-boundaries.md).

## Release Contract

The public surface is a v0 PDF adapter release for `binas`, not a general PDF editor. Supported operations are explicit and fixture-backed; unsupported shapes fail closed or are reported as detection metadata.

- Arbitrary intake is limited to files that parse through the current graph and security boundaries. Unsupported encryption, default signed edits, dynamic/full XFA, malformed xref streams, and unsupported object/filter shapes still fail closed or report detection metadata.
- Form fill is explicit through `form list` and `form set`, with narrow support for text, proven choice options, multi-select values, checkbox/button states, and simple appearance regeneration. Rich widgets, broad field-tree surgery, and broad visual layout remain refusal boundaries.
- Filter/decode tolerance accepts the documented reversible filters and pass-through non-target streams; unsupported filters and unsupported `/DecodeParms` shapes remain precise failures.
- The text editability profile distinguishes directly editable text operands, CMap-backed text, canonical rewrite paths, explicit fallback commands, and non-editable documents. It does not imply full font metrics, visual reflow, or bundled OCR.
- Browser/WASM support exposes the same inspect/query/edit semantics over PDF bytes. PDF.js rendering in the local editor is a visual surface, not a separate correctness proof.

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

For non-blocking high-level PDF markers, JSON mode keeps `valid: true` and reports warnings. AcroForm dictionaries, annotations, and font/CMap markers are detected because their support is deliberately narrower than general PDF semantic editing: field discovery uses `form list`, field updates use `form set`, simple text/choice widget appearance streams require `--regenerate-appearance`, annotation updates write `/Contents` through `annot set-contents`, simple text-like annotation appearances require `--regenerate-appearance`, and CMap support is limited to page font-scoped `/ToUnicode` maps for simple `Tf` flows, CMap-backed hex `TJ` arrays, and one unambiguous fallback map for hex text.

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
      "text_decoding_support": "simple literal operands, ASCII hex operands, literal/hex TJ arrays, page font-scoped ToUnicode CMaps for simple Tf flows, CMap-backed TJ hex arrays, and one unambiguous ToUnicode CMap fallback"
    },
    "xref": {
      "has_object_stream": false,
      "has_hybrid_stream": false,
      "has_stream": false,
      "hybrid_stream_object": null,
      "hybrid_stream_offset": -1,
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
- `has_hybrid_stream`: the last trailer contains `/XRefStm`.
- `hybrid_stream_offset`: the `/XRefStm` byte offset, or `-1` when absent or not an integer.
- `hybrid_stream_object`: xref stream object metadata when the hybrid offset resolves to a known `/Type /XRef` stream object.
- `has_stream`: an xref stream object was detected.
- `has_object_stream`: an object stream was detected.
- `object_count`: count of indirect object headers found by the adapter's xref summary pass.
- `stream_count`: count of xref stream objects detected by the adapter's xref summary pass.
- `object_stream_count`: count of object stream objects detected by the adapter's xref summary pass.
- `objects`: indirect object metadata with object number, generation, and byte offset.
- `stream_objects`: xref stream object metadata with object number, generation, and byte offset.
- `object_stream_objects`: object stream object metadata with object number, generation, and byte offset.

Parseable xref-stream and hybrid `/XRefStm` PDFs route through the graph path for generic parse, query, and supported text rewrites. Malformed or unsupported xref streams still fail closed with a structural xref error; `inspect --json` still emits root metadata and includes `parse_error`, while `validate --json` emits `valid: false`, `errors`, and the same root metadata.

Object streams are detected, inflated into graph objects, and canonical-written back as normal indirect objects for graph rewrites. Surgical mode still refuses object-stream PDFs because it cannot prove byte-local safety inside compressed object containers. Detection is not a broad regex over dictionary bytes: `/ObjStm` in unrelated keys, nested dictionaries, or string literals is ignored to avoid false object-stream reports.

The metadata describes the input shape. `has_object_stream` also implies the graph path parsed object-stream contents when parse succeeds. `has_stream` means xref-stream metadata was detected; `has_hybrid_stream` means a table trailer pointed at a supplemental xref stream. Parseable xref streams use the graph path, while malformed or unsupported xref streams still report a parse error.

Stream nodes expose parse metadata for callers that need to inspect the byte boundary before planning edits. `encoded_length` is set for every detected stream node, `decoded_length` is set only when decoding succeeds or a raw graph stream is known to be unfiltered, and `filter_chain` lists the normalized direct filter names when a direct `/Filter` name or array is present.

The boundary metadata is detection-only for high-level PDF surfaces:

- `has_encrypt`: `/Encrypt` was detected. Encrypted PDFs fail parse with the explicit password-capable-path error unless the caller supplies `--password`. The explicit password path supports Standard Security RC4 R2/R3/R4 and R4 AESV2 files with normal encrypted strings and streams, encrypted object streams inflated into graph objects, encrypted xref streams decoded before graph parsing, plus stream-level `/Crypt` filters for `/Identity` and `/StdCF`, for `inspect`, `validate`, `query`, and canonical text edits. It re-encrypts strings and streams on write and still rejects AESV3+, public-key security, and unsupported crypt filters outside that supported shape.
- `has_signature`: a true PDF name token for `/Sig`, `/ByteRange`, or `/SigFlags` was detected. The scanner skips literal strings, comments, and hex strings; residual boundary scanning skips those same PDF regions for non-signature boundary name markers too. Digitally signed PDFs fail parse by default because rewriting would invalidate signature semantics. `signature inspect` reports byte-range shape, container/digest hints, and, when supported CMS/PKCS#7 signed attributes are parseable, byte-range digest validation plus certificate count/signer subject/issuer metadata. JSON separates `byte_range_digest_validation_status` from `certificate_trust_validation_status`; default inspection does not use system roots or claim trust, revocation, timestamp, or signer public-key validity. `signature re-sign` is explicit and external-signer-backed: it sends the computed byte-range digest to a caller command and verifies the new byte-range digest layer after writing.
- `has_acroform`: `/AcroForm` was detected. Field discovery uses the narrower `form list` command, including inherited field type, `type_flag_names`, alternate `/TU` name, mapping `/TM` name, direct default `/DV` value, inherited/direct `/Ff` flags for `read_only`, `required`, and `no_export`, sorted proven button appearance states, directly decoded choice-field `/Opt` options, and appearance-generation status/blocker metadata for approximate text/choice widget generation versus unsafe rich `/RV` inputs. Field value updates require `form set`; text fields write `/V` and set `/NeedAppearances`; non-editable choice fields with proven direct `/Opt` options reject unlisted values; editable choice fields and choice fields without proven options accept literal values; multi-select choices accept repeatable `--values` or `--value-array`; proven checkbox/button fields write `/V` and `/AS` from `/AP /N`, including inherited `/FT /Btn` and a narrow parent-field radio shape. `form set --regenerate-appearance` creates simple Helvetica widget `/AP /N` streams for text and choice widgets with proven direct `/Rect`, including explicit newlines, approximate wrapping, clipping, and height truncation; it can synthesize basic checkbox normal appearances for one widget with direct `/Rect` and a safe on-state, while radio/group/custom button synthesis remains fail-closed.
- `has_xfa`: `/XFA` was detected. Generic parser/edit paths fail closed; `xfa list` reports directly represented packet metadata, conservative `packet_kind`, XML prolog/root diagnostics when safely detectable, decoded byte/text lengths, stream decode errors when a referenced stream packet cannot be decoded, and JSON semantics metadata for `static_datasets_template`, `dynamic_rendering_required`, `unknown`, or `none`. Dynamic markers include flowed template subforms, repeatable `occur`, pagination/layout nodes, and `dynamicRender` config; dynamic XFA emits a warning and `xfa dataset-set` refuses semantic edits as `unsupported PDF: dynamic XFA requires renderer semantics; xfa dataset-set refuses semantic edits`. `xfa list --packet-kind ... --label ...` filters listed packets exactly. `xfa replace` can update exactly one directly represented XFA packet occurrence, skips literal/hex/name packet labels the same way `xfa list` does, accepts the same selector flags before ambiguity checks, and requires `--match-index` when multiple selected occurrences match.
- `has_annotations`: `/Annots` was detected. `annot list` reports page index/object metadata when a candidate can be proven from page `/Annots`, direct four-number `/Rect` metadata when available, direct `/NM`, `/M`, and `/T` text values when decodable, direct numeric `/F` annotation flags decoded into common flag names, direct numeric color and border metadata, `quad_points_count`, and appearance-generation status/blocker metadata for approximate supported generation versus unsafe `/RC`, JavaScript `/A`, or `/AA` inputs. `annot set-contents` can update dictionary `/Contents`; by default it preserves `/AP`, `--remove-appearance` explicitly removes stale annotation `/AP`, and `--regenerate-appearance` creates simple `/AP /N` Form XObject streams for supported text-like annotations with proven direct `/Rect`, using the same approximate multiline layout as form appearances.
- `has_font_markers`, `has_cmap_markers`, `has_tounicode_cmap`, and `has_cid_font_markers`: font/CMap-related markers were detected. Text extraction supports simple literal operands, simple ASCII hex operands, literal/hex `TJ` arrays, page font-scoped `/ToUnicode` maps for simple `Tf` flows, CMap-backed hex `TJ` arrays, one unambiguous fallback map for hex operands, omitted `/Encoding` inference for standard non-symbol Type1 base fonts, and conservative `width_units` metadata when direct simple-font `/FirstChar` and `/Widths` can be proven; glyph metrics beyond raw width units, visual layout, and reflow are not verified.

These fields do not grant broad semantic edit support. They are guardrails for callers: unsupported encryption and default signature paths stop parsing, XFA requires the explicit packet command, malformed xref streams stop parsing, and object streams require graph/canonical rewrite for edits. AcroForm, annotations, and font/CMap markers are warning-only because the adapter may still rewrite a supported content-stream text operand without updating every higher-level system.

## Current Failure Modes

The current PDF slice is supported for direct literal-string, simple ASCII hex-string, simple literal/ASCII-hex `TJ` array, page font-scoped CMap-backed hex string, and CMap-backed hex `TJ` array edits in raw/uncompressed content streams, direct `/Filter /FlateDecode`, `/Filter /LZWDecode`, `/Filter /ASCIIHexDecode`, `/Filter /ASCII85Decode`, and `/Filter /RunLengthDecode` streams, their standard `/Fl`, `/LZW`, `/AHx`, `/A85`, and `/RL` abbreviations, and filter arrays composed only from those reversible filters after abbreviation normalization with direct or resolved indirect integer `/Length` values and supported direct or resolved indirect `/DecodeParms` values. Known failure modes are explicit:

- Non-PDF input fails as `not a PDF file`.
- Strict PDF parsing fails malformed input without `%%EOF` as `malformed PDF: missing EOF marker`.
- Malformed or unsupported xref-stream parsing fails with a structural xref error; parseable xref streams and valid hybrid `/XRefStm` files use the graph path.
- Surgical object-stream edits fail as `surgical rewrite does not support PDFs with xref streams or object streams; use --rewrite auto or --rewrite canonical`.
- Encrypted PDFs without `--password` fail as `unsupported PDF: encrypted PDFs require an explicit password-capable path; Adapter.Parse does not decrypt`; encrypted PDFs with unsupported handlers, AESV3+, public-key security, or unsupported crypt filters outside supported `/Identity` and `/StdCF` fail with an unsupported encryption error.
- Digitally signed PDFs fail as `unsupported PDF: digital signatures require explicit invalidation mode; Adapter.Parse refuses silent signature invalidation`.
- Generic XFA parser/edit paths fail as `unsupported PDF: XFA forms are not implemented`; `xfa list` and `xfa replace` are the explicit packet-level exceptions. `xfa dataset-set` is limited to directly represented datasets packets classified as `static_datasets_template`; detected dynamic XFA fails as `unsupported PDF: dynamic XFA requires renderer semantics; xfa dataset-set refuses semantic edits`, and unknown/full packet families fail as `unsupported PDF: XFA semantics are not limited to static template/datasets packets; xfa dataset-set refuses semantic edits`.
- AcroForm dictionaries, annotations, and font/CMap markers do not make validation invalid by themselves, but they produce warnings because support is narrower than the full PDF semantics.
- Edits fail when no node matches, more than one node matches without `--match-index`, or `--match-index` is outside the match set.
- Edits fail when the selected node has no editable encoded span or the planned span no longer matches the source bytes.
- Streams without a direct or supported indirect integer `/Length` fail as `unsupported stream: /Length must be an integer or indirect integer reference`.
- Indirect `/Length` is supported only for `/Length N G R` references whose target object body is only a nonnegative integer. Missing or non-integer targets fail as `unsupported stream: /Length reference must resolve to an integer object`.
- Filter arrays containing unsupported filters fail as `unsupported PDF stream filter "..."`.
- Unsupported `TJ` arrays fail closed when they contain non-string/non-number elements, unsupported hex encodings, or CMap-backed array cases outside the current simple page-font or fallback CMap support.
- Unsupported filters fail closed with explicit errors such as `unsupported PDF stream filter "DCTDecode"`.
- Unsupported `/DecodeParms` shapes fail closed with explicit errors such as `unsupported stream: /DecodeParms PNG predictors require /Colors >= 1`, `unsupported stream: /DecodeParms /EarlyChange must be 0 or 1`, `unsupported stream: /DecodeParms TIFF predictor requires /BitsPerComponent <= 32`, or `unsupported stream: /DecodeParms is not implemented`.

Signed PDFs have two explicit edit modes outside the default refusal path.

`--signature-mode invalidate` uses the canonical writer only when the caller deliberately accepts invalidation: `binas edit --rewrite canonical --signature-mode invalidate`, or `--rewrite auto --signature-mode invalidate` when auto selects the canonical path. The legacy `--allow-signature-invalidation` flag remains an alias. JSON output includes `signature_invalidation: "digital signatures invalidated; not preserved or re-signed"` on that explicit path.

`--signature-mode preserve-incremental` uses an append-only incremental update and requires `--rewrite auto`. It supports the same narrow selectable text target only when the target content stream is raw/unfiltered, outside object streams and xref streams, and the signed PDF has parseable `/ByteRange` offset/length pairs. The output preserves the original file bytes as a prefix, appends an updated stream object, xref section, trailer, and `/Prev`, reparses the resulting PDF with signatures allowed, verifies old/new selectable text and page count, and compares every signed byte range byte-for-byte. JSON output includes `signature_preservation` with `incremental_update`, `original_bytes_preserved`, `byte_range_proof`, `byte_ranges_checked`, `signed_byte_ranges_unchanged`, `cryptographic_validation: false`, and a note that re-signing is not performed. Missing, malformed, negative, or out-of-file `/ByteRange` values fail closed.

`signature inspect` is read-only and can validate the PDF byte-range digest against supported CMS/PKCS#7 signed `messageDigest` authenticated attributes. Unsupported or malformed CMS remains explicit with `byte_range_digest_validation_status: "unsupported"`; mismatches report `byte_range_digest_validation_status: "mismatch"`. Certificate trust status is separate. By default it is not performed and no system roots are loaded. Passing repeatable `--trust-root` PEM/DER certificate files, plus optional repeatable `--trust-intermediate` files, validates only the CMS-declared signer certificate chain to those explicit roots. This does not imply revocation, timestamp authority validation, or signer public-key verification over CMS authenticated attributes.

`signature re-sign` is the explicit CLI re-signing path. It accepts a signed PDF, `-o`, `--signer-command`, repeatable `--signer-arg`, repeatable `--signer-env KEY=VALUE`, `--signer-name`, `--external-key-id`, `--digest`, `--container`, `--sub-filter`, and `--reserved-bytes`. `binas` appends a new incremental signature dictionary, computes the draft `/ByteRange` digest, sends JSON containing `digest_base64`, `digest_hex`, algorithm/container/subfilter metadata, byte ranges, and signature metadata to the signer command on stdin, then accepts `signature_base64`, `signature_hex`, or JSON `signature` bytes on stdout. It writes the output only after reparsing and proving `byte_range_digest_validation_status: "valid"`. The CLI refuses raw private-key-looking metadata through the existing signer metadata checks; private key storage, key access, CMS construction policy, timestamping, revocation, and signer public-key validation remain external to `binas`.
- Xref rebuild fails when a table xref is missing or no indirect objects are found. Existing non-zero generation objects are emitted in generation-aware xref rows.
- Verification fails the command if the output does not reparse, the old decoded text remains, or the replacement text is not selectable through the adapter.

Current non-goals:

- Indirect `/Length` references outside `/Length N G R` standalone integer objects are not supported.
- Broader `/DecodeParms` shapes and filters outside ASCIIHex, ASCII85, RunLength, Flate, LZW, and their standard abbreviations are not supported.
- AESV3+/public-key/unsupported-crypt-filter encrypted PDFs, default signed-PDF edits, system-root signature trust, revocation/timestamp validation, signer public-key validation, broad XFA rendering, bundled OCR/raster recognition, and implicit overlay/stamp fallbacks are not supported.
- Appearance generation is intentionally simple: text and choice widgets plus supported text-like annotations only, approximate newlines/wrapping/clipping/truncation, no full layout engine.
- Broad font/CMap-aware text decoding, font layout, visual layout, and page rendering are not verified beyond the current selectable-text adapter parse, standard Type1 default encoding inference, and direct simple-font raw width metadata.
- Browser/WASM support includes the `cmd/binas-wasm` helper surface for inspect, query, and verified text edit over `Uint8Array` bytes. `cmd/binas-wasm/smoke.html` remains a minimal API smoke page. `cmd/binas-wasm/editor.html` is the browser editor surface: it loads Go WASM, loads PDF.js from a pinned CDN or manually selected local files, renders pages to canvas, provides page navigation and zoom, overlays exact text highlights from PDF.js text content, runs the verified WASM edit path, rerenders edited bytes, and downloads the edited PDF. Rendering is still browser/PDF.js rendering, not a second proof of PDF semantic edit correctness.

## Filter Support

Direct `/Filter /ASCIIHexDecode`, `/Filter /ASCII85Decode`, `/Filter /RunLengthDecode`, `/Filter /FlateDecode`, and `/Filter /LZWDecode` streams, their standard abbreviations `/AHx`, `/A85`, `/RL`, `/Fl`, and `/LZW`, and filter arrays composed only of those reversible filters after abbreviation normalization are wired into the parser and edit path for direct and supported indirect integer `/Length` streams. The adapter can:

- Detect supported direct filters and supported reversible filter arrays on the containing stream.
- Decode the stream, parse the decoded content, rewrite the selected literal string, re-encode the stream, and update `/Length` or its referenced integer object.
- Preserve fail-closed span checks across decoded and original byte ranges.
- Rebuild the xref table/trailer after byte-length changes.
- Verify by reparsing the produced PDF and querying the replacement text, not by trusting the edit path.

Remaining filter work is intentionally narrower: broader `/DecodeParms` shapes need separate fixture-backed slices before they are public support. Filters outside ASCIIHex, ASCII85, RunLength, Flate, LZW, and their standard abbreviations remain out of scope.

## DecodeParms

`/DecodeParms` support is limited to FlateDecode and LZWDecode streams with omitted `/Predictor` defaulting to `1`, `/Predictor 1` where direct signed integer `/Columns`, `/Colors`, and `/BitsPerComponent` values are accepted as no-op metadata, TIFF predictor `2` with packed sample rows and `/BitsPerComponent <= 32`, or PNG predictors `10` through `15` with row width computed from `/Columns`, `/Colors`, and `/BitsPerComponent` using PDF's ceil-to-bytes row sizing. Omitted `/Columns`, `/Colors`, and `/BitsPerComponent` default to `1`, `1`, and `8` for predictor shapes that use geometry. LZWDecode also supports `/EarlyChange 0|1`, defaulting to `1`. The stream scanner and graph path resolve `/DecodeParms N G R` when the referenced object is a standalone dictionary, array, or `null`, and resolve array entries such as `[null N G R]` when each reference resolves to a dictionary or `null`. For filter arrays, `/DecodeParms` entries must align one-for-one with the filter chain. All-null arrays are accepted for supported reversible chains. Non-null dictionary entries are supported only for Flate and LZW positions; wrapper filters such as ASCIIHex, ASCII85, and RunLength require `null` parameters.

Everything else under `/DecodeParms` remains unsupported unless a later pass adds focused fixtures and proof for that exact shape. This excludes unknown predictor values, unsupported LZW predictor combinations, unsupported filters, unsupported dictionary keys, scalar references, reference cycles, and non-null DecodeParms dictionaries outside the currently supported Flate/LZW positions.

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

- Filter arrays remain unsupported when their normalized filter chain contains filters outside ASCIIHex, ASCII85, RunLength, Flate, LZW, and their standard abbreviations.
- Object streams have graph/canonical support for the fixture-backed shapes, but preserving original object-stream layout remains out of scope.
- Unresolved, non-integer, compressed, shared, or cyclic indirect `/Length` references remain outside the current `/Length N G R` standalone integer support shape.
- Broader `/DecodeParms` shapes remain unsupported beyond direct or resolved indirect Flate/LZW parameter dictionaries/arrays for `/Predictor 1`, TIFF predictor `2` packed sample rows, PNG predictors `10` through `15`, and LZW `/EarlyChange 0|1`.
- Filters outside ASCIIHex, ASCII85, RunLength, Flate, LZW, and their standard abbreviations, encrypted AESV3+/public-key/unsupported-crypt-filter shapes, default signed-PDF edits, system-root signature trust, revocation/timestamp validation, signer public-key validation, broad XFA rendering, broad appearance/layout support, and broad font/CMap/layout support remain out of scope.
