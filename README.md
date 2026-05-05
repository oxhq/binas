# binas

`binas` is a Go-first binary AST and verified rewrite engine. The first adapter is PDF, focused on fail-closed content-stream text surgery instead of overlaying pixels.

This is an early v0 proof. The public core API is intentionally small and unstable while the PDF adapter teaches the shape of the engine.

## Current Proof

```powershell
go run ./cmd/binas inspect C:\path\file.pdf --format pdf --json
go run ./cmd/binas validate C:\path\file.pdf --format pdf --json
go run ./cmd/binas query C:\path\file.pdf --format pdf --kind pdf.content.text_show --text "08-15-2024" --meta operator=Tj --json
go run ./cmd/binas edit C:\path\file.pdf --format pdf --kind pdf.content.text_show --text "08-15-2024" --replace "May 5, 2026" --verify reparse,old-gone,new-selectable -o C:\path\out.pdf --json
go run ./cmd/binas form list C:\path\file.pdf --format pdf --json
go run ./cmd/binas form set C:\path\file.pdf --format pdf --field payer.name --value "New Name" -o C:\path\form-out.pdf --json
go run ./cmd/binas annot list C:\path\file.pdf --format pdf --json
go run ./cmd/binas annot set-contents C:\path\file.pdf --format pdf --index 0 --contents "Updated note" --remove-appearance -o C:\path\annot-out.pdf --json
go run ./cmd/binas xfa list C:\path\file.pdf --format pdf --json
go run ./cmd/binas xfa replace C:\path\file.pdf --format pdf --text "<template>old</template>" --replace "<template>new</template>" --match-index 0 -o C:\path\xfa-out.pdf --json
```

The main `edit` mode rewrites direct PDF literal-string text, supported hex-string text, and simple literal/ASCII-hex `TJ` array text inside raw/uncompressed content streams, direct `/Filter /FlateDecode`, `/Filter /ASCIIHexDecode`, `/Filter /ASCII85Decode`, and `/Filter /RunLengthDecode` streams, single-item `/Filter [/FlateDecode]` streams, exact `/Filter [/ASCII85Decode /FlateDecode]`, `/Filter [/ASCIIHexDecode /FlateDecode]`, `/Filter [/RunLengthDecode /FlateDecode]`, `/Filter [/ASCII85Decode /RunLengthDecode /FlateDecode]`, `/Filter [/ASCIIHexDecode /RunLengthDecode /FlateDecode]`, `/Filter [/RunLengthDecode /ASCII85Decode /FlateDecode]`, and `/Filter [/RunLengthDecode /ASCIIHexDecode /FlateDecode]` chains, and narrowly supported Flate `/DecodeParms` streams with a direct or resolved indirect integer `/Length`. It can rewrite to a different encoded length, updates the direct length or referenced length object, rebuilds a basic xref table/trailer, and reparses the output for verification. It does not use overlay/stamp fallback.

See [docs/release-surface.md](docs/release-surface.md) for the current CLI output contract, validation behavior, inspect metadata, and failure modes. See [docs/pdf-semantic-boundaries.md](docs/pdf-semantic-boundaries.md) for the semantic guardrails around encryption, signatures, XFA, AcroForm, annotations, and font/CMap markers. See [docs/w8ben-variable-length-rewrite.md](docs/w8ben-variable-length-rewrite.md) for the W-8BEN proof commands, current support boundaries, and incremental roadmap.

## Current CLI Surface

- `validate` performs a strict adapter parse and reports `{ "format": "pdf", "valid": true|false, "errors": [], "warnings": [] }` in JSON mode. Parse failures are reported as `valid: false`; unsupported formats and file I/O errors are command errors. Add `--fail-on-invalid` when invalid parser results should also return a non-zero exit code. For fail-closed structural PDF shapes that can still be summarized, JSON mode also includes partial root metadata. Non-blocking detected boundaries such as AcroForm, annotations, and font/CMap markers are reported as warnings when their semantics require a narrower command or have limited verification.
- `inspect` reports the parsed tree size and root metadata. For PDFs this includes `header`, byte `size`, estimated `pages`, `boundaries` fields for detected high-level PDF surfaces, and `xref` fields: `has_table`, `table_offset`, `has_stream`, `stream_count`, `has_object_stream`, `object_count`, `object_stream_count`, `objects`, `stream_objects`, and `object_stream_objects`. Parseable xref-stream and object-stream PDFs parse through the graph path; malformed xref streams can still return inspect metadata with `parse_error`.
- `query` currently targets parsed `pdf.content.text_show` nodes by decoded text value and repeatable exact metadata filters such as `--meta operator=TJ`.
- `edit` currently rewrites direct literal-string text operands, simple ASCII hex-string text operands, simple literal/ASCII-hex `TJ` array operands, page font-scoped `/ToUnicode` CMap-backed hex text for simple `Tf` flows, and one unambiguous `/ToUnicode` CMap fallback in raw or supported filtered content streams, then verifies `reparse`, `old-gone`, and `new-selectable` by default. `--rewrite auto` uses canonical graph rewriting for object-stream and xref-stream PDFs when the surgical path cannot prove safety.
- `form list` reports AcroForm field indexes, fully qualified names, object ids, inherited field types, decoded `/V` values, kid counts, proven button widget appearance state, sorted proven button appearance states, and directly decoded choice-field `/Opt` options. `form set` updates one AcroForm field `/V` value by exact field name or fully qualified parent/child field name and fails closed on zero or multiple matches unless `--match-index` selects one. Text fields use a literal `/V` value and set `/NeedAppearances true`; proven checkbox/button fields set `/V` and `/AS` to `/Off` or the proven on-state from `/AP /N`, including inherited `/FT /Btn` and a narrow parent-field radio shape. It does not regenerate widget appearance streams.
- `annot list` reports zero-based annotation indexes, subtype, contents, object id when indirect, whether `/AP` is present, proven page index/object metadata when the annotation is reached through a page `/Annots` array, and direct four-number `/Rect` metadata when available. `annot set-contents` updates one annotation dictionary `/Contents` value by index. It preserves `/AP` by default; `--remove-appearance` explicitly removes stale annotation `/AP` and reports appearance invalidation/removal. It does not regenerate annotation appearances.
- `xfa list` reports directly represented XFA packet indexes, labels, conservative packet kind, object ids, stream flags, decoded text length, preview text, filter/decode-params metadata, and decode errors for referenced stream packets that cannot be decoded. `xfa replace` updates exactly one directly represented XFA packet occurrence in a literal/hex value, referenced string/stream object, or XFA packet array. Multiple matches fail closed unless `--match-index` selects the zero-based occurrence. Generic `inspect`, `query`, `validate`, and `edit` still reject XFA by default.

Current support is intentionally narrow: direct PDF literal strings, simple ASCII hex strings, simple literal/ASCII-hex `TJ` arrays, page font-scoped `/ToUnicode` CMaps for simple `Tf` flows plus one unambiguous fallback CMap, direct or resolved indirect integer `/Length`, raw streams, direct `/Filter /FlateDecode`, `/Filter /ASCIIHexDecode`, `/Filter /ASCII85Decode`, and `/Filter /RunLengthDecode` streams, single-item `/Filter [/FlateDecode]` arrays, exact `/Filter [/ASCII85Decode /FlateDecode]`, `/Filter [/ASCIIHexDecode /FlateDecode]`, `/Filter [/RunLengthDecode /FlateDecode]`, `/Filter [/ASCII85Decode /RunLengthDecode /FlateDecode]`, `/Filter [/ASCIIHexDecode /RunLengthDecode /FlateDecode]`, `/Filter [/RunLengthDecode /ASCII85Decode /FlateDecode]`, and `/Filter [/RunLengthDecode /ASCIIHexDecode /FlateDecode]` chains, narrowly supported Flate `/DecodeParms`, canonical object graph rewrites for object-stream/xref-stream content, and basic xref table rebuilds. Indirect length support is limited to `/Length N G R` references that resolve to a standalone integer object. Fixture-backed `/DecodeParms` support is limited to FlateDecode with `/Predictor 1` and omitted or explicit default geometry keys, byte-aligned TIFF predictor `2`, or PNG predictors `10` through `15` constrained to direct `/Columns >= 1` or omitted `/Columns` defaulting to 1 when `/Colors` is also 1/default, byte-aligned predictor rows, and the current `/BitsPerComponent 8` plus `/Colors 1`, `/BitsPerComponent 8` plus `/Colors 3`, and `/BitsPerComponent 16` plus `/Colors 2` lanes. Supported `/DecodeParms` arrays must align exactly with the supported filter chain: one entry for `[/FlateDecode]`, `[null <<...>>]` for `[/ASCII85Decode /FlateDecode]`, `[/ASCIIHexDecode /FlateDecode]`, and `[/RunLengthDecode /FlateDecode]`, or `[null null <<...>>]` for the four exact three-stage chains listed above.

Other multi-filter arrays, broader `/DecodeParms` shapes, filters outside standalone ASCIIHex, ASCII85, RunLength, Flate, and the exact two- and three-stage chains listed above, encryption, default signed-PDF edits, XFA outside the explicit `xfa list` and `xfa replace` packet paths, AcroForm widget appearance regeneration, annotation appearance regeneration, full font/CMap/layout support, browser/WASM runtimes, overlays, and OCR/raster fallbacks are not supported.

Object streams are parsed into the internal graph and canonical-written back as normal indirect objects. Parseable xref-stream PDFs also route through the graph path for generic parse and supported text queries/rewrites; malformed xref streams still fail closed with partial metadata. Detection is intentionally limited to stream dictionaries whose top-level `/Type` is `/ObjStm` or `/XRef`; unrelated `/ObjStm` mentions in other keys, nested dictionaries, or string literals are not treated as object streams.

Encrypted PDFs, digitally signed PDFs, and XFA PDFs fail closed during generic parse. Encryption failures use an explicit password-capable-path error because `Adapter.Parse` does not decrypt. Signed-PDF markers include true PDF name tokens for `/Sig`, `/ByteRange`, and `/SigFlags`; the scanner skips literal strings, comments, and hex strings. Signed PDFs require `binas edit --rewrite canonical --allow-signature-invalidation` or `--rewrite auto --allow-signature-invalidation` when auto selects the canonical path. Defaults refuse silent signature invalidation. This invalidates signatures and does not preserve or re-sign them. AcroForm dictionaries, annotations, and font/CMap markers are exposed through `inspect` metadata and `validate` warnings because their supported behavior is narrower than general PDF semantic editing.

The object graph is the foundation for detection, selection, and verified rewrites. It is not a promise that every PDF semantic surface is editable. Widget appearances, annotation appearances, signature byte ranges, encrypted object graphs, XFA packet families outside the explicit list/replace helpers, full CMap/font decoding, font widths, and visual layout remain separate semantic systems.

## Next Boundary

Current residual boundaries:

- Broader `/DecodeParms` shapes, including omitted PNG `/Columns` when `/Colors` is not 1/default, non-byte-aligned predictor rows, unknown predictor values, and bit-depth/color combinations beyond the fixture-backed lanes, stay out of scope until each one has fixture-backed parser, edit, length-update, xref-rebuild, and verification proof.
- Broader filter chains remain unsupported except for the exact supported chains listed above.
- Filters outside standalone ASCIIHex, ASCII85, RunLength, Flate, and the exact two- and three-stage chains listed above, encrypted PDFs, default signed-PDF edits, XFA outside exact packet listing/replacement, appearance regeneration, broad font/CMap/layout support, browser, and WASM surfaces remain out of scope.

## Packages

- `pkg/core`: span-preserving tree, selector, adapter, edit plan, report, and verification types.
- `pkg/adapters/pdf`: PDF detection, object graph parsing, xref summary helpers, stream scanning with encoded/decoded length metadata, text-show node parsing, simple `TJ` array parsing/rewrite, variable-length literal-string rewrite, direct and indirect `/Length` update, generation-aware xref rebuild, supported ASCIIHexDecode, ASCII85Decode, RunLengthDecode, FlateDecode, exact filter-chain stream edits, narrow `/DecodeParms` handling including `/Predictor 1` default geometry keys, omitted `/Columns` default, and a 16-bit fixture lane, AcroForm/XFA/annotation semantic helpers with field/annotation flags and XFA XML diagnostics, page font-scoped ToUnicode CMap support, and verification.
- `cmd/binas`: CLI for `inspect`, `validate`, `query`, `edit`, `form`, `annot`, and `xfa`.

## Verification

```powershell
go test ./...
go build -o tmp\binas.exe ./cmd/binas
```
