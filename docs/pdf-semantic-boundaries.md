# PDF Semantic Boundaries

The PDF object graph is the foundation for `binas`: it lets the adapter detect file shape, expose root metadata, find supported content-stream text nodes, and verify byte rewrites by reparsing the output. It is not a blanket guarantee that every object reachable from the graph is semantically editable.

The current edit contract is narrow: rewrite supported selectable `pdf.content.text_show` operands in supported content streams, update supported stream lengths, rebuild a basic xref table, and verify `reparse`, `old-gone`, `new-selectable`, `page-unchanged`, and `no-fallback-used`.

## Fail-Closed Boundaries

These surfaces stop parsing before edit planning:

- Encryption: `/Encrypt` means encrypted object and stream bytes may need password-derived decryption. The adapter returns `unsupported PDF: encrypted PDFs require an explicit password-capable path; Adapter.Parse does not decrypt`. This is an explicit password-path contract, not a generic parse refusal; `binas` does not fake decryption or inspect encrypted object bytes as if they were plaintext.
- Digital signatures: true PDF name tokens for `/Sig`, `/ByteRange`, or `/SigFlags` mean byte-range and digest semantics can be invalidated by ordinary rewrites. The signature boundary scanner skips literal strings, comments, and hex strings, then returns `unsupported PDF: digital signatures require explicit invalidation mode; Adapter.Parse refuses silent signature invalidation` for true markers.
- XFA in generic parse/edit: `/XFA` means form data may live in XML packets instead of normal AcroForm fields or page content streams. The default adapter returns `unsupported PDF: XFA forms are not implemented`; the explicit `xfa list` and `xfa replace` commands are the narrow packet-level exceptions. `xfa list` can report referenced stream packets with decode errors instead of silently omitting them, classify obvious packet kinds from labels or XML roots, and expose conservative XML prolog/root diagnostics when safely detectable.
- Xref streams: these compressed xref carriers are detected in xref metadata. Parseable xref streams route through the object graph for generic parse, query, and supported rewrites; malformed or unsupported xref streams still fail closed with partial metadata when available.

Fail-closed means callers get a parser error instead of a partial edit plan. For malformed xref streams, `inspect --json` and `validate --json` can still include root metadata because the adapter can summarize that structural shape before rejecting it. Object streams are inflated into graph objects and canonical-written as normal indirect objects for the supported graph path.

Signature invalidation is an explicit helper-only mode. `ApplyCanonicalEditInvalidatingSignatures` may canonical-write a signed PDF only when the caller deliberately chooses to invalidate signatures. Its verification remains honest but narrow: it reparses the produced object graph under the same invalidation mode and checks the text/page invariants; it does not claim that an existing cryptographic signature remains valid. The default adapter parse and default canonical edit path still fail closed for `/Sig`, `/ByteRange`, and `/SigFlags`.

## Detection-Only Boundaries

These markers do not make the file invalid by themselves, but they do not become broad editable features:

- AcroForm: `/AcroForm` is reported as `has_acroform`. `form list` reports field indexes, fully qualified names, object ids, inherited field types, inherited/direct `/Ff` field flags for `read_only`, `required`, and `no_export`, decoded `/V` values, kid counts, proven button widget appearance state, sorted proven button appearance states, and directly decoded choice-field `/Opt` options. `form set` updates exactly one matching field dictionary by exact or fully qualified parent/child field name. Text fields set a literal `/V` value and `/NeedAppearances true`; proven checkbox/button fields set `/V` and `/AS` to `/Off` or the proven on-state from `/AP /N`, including inherited `/FT /Btn` and a narrow parent-field radio shape. Field tree rewriting beyond matching/inherited type detection, default appearance generation, and broad widget synchronization are not implemented.
- Annotations: `/Annots` is reported as `has_annotations`. `annot list` exposes candidate indexes, subtype, contents, indirect object id when present, `/AP` presence, page index/object metadata when the candidate can be proven from a page `/Annots` array, direct four-number `/Rect` values when available, and direct numeric `/F` annotation flags decoded into common flag names. `annot set-contents` updates exactly one annotation dictionary `/Contents` value by index. It preserves `/AP` by default; `--remove-appearance` explicitly removes stale annotation `/AP` and reports appearance invalidation/removal. Widget appearance streams, popup state, highlight geometry editing, and annotation appearance regeneration are not implemented.
- Font/CMap markers: `/Font`, `/CMap`, `/ToUnicode`, `/CIDFontType0`, `/CIDFontType2`, and `/CIDToGIDMap` markers are summarized through the font/CMap boundary fields. Text decoding supports simple literal operands, simple ASCII hex operands, page font-scoped `/ToUnicode` maps for simple `Tf` flows, and one unambiguous fallback map for hex operands; glyph substitution beyond those maps, character widths, and layout reflow are not implemented.

For these limited-support boundaries, `validate --json` keeps `valid: true` and emits warnings. `inspect --json` exposes the boundary fields under `root.boundaries`. A supported content-stream rewrite may still run if the selected text node is one of the supported operands, but that command is not updating the corresponding high-level form, annotation, font, or visual semantics.

## pypdf Comparison Notes

A local pypdf checkout was sampled for comparison. It is a general Python PDF library with documented support for reading and writing encryption, decrypting encrypted files, adding annotations, reading AcroForm field structures, parsing `/ToUnicode` CMaps, and carrying font metrics for text extraction and appearance work.

That comparison should not be read as a requirement that `binas` mirror pypdf's scope. It clarifies the opposite boundary:

- pypdf-style encryption/decryption would require password handling, crypt filters, object and stream decryption, and re-encryption decisions. `binas` currently rejects `/Encrypt` with the explicit password-capable-path error above.
- pypdf-style annotation support includes creating and writing annotation dictionaries. `binas` can list annotations with page metadata, direct `/Rect` values, and decoded `/F` flags when proven from page `/Annots` arrays and update `/Contents`, but does not regenerate annotation or widget appearances.
- pypdf-style AcroForm support reads form trees and has XFA-specific checks. `binas` can list AcroForm fields, inherited/direct `/Ff` flags, proven button states, and directly decoded choice options, update exact text field `/V` values, update proven checkbox/button or narrow radio `/V` and `/AS` values, list directly represented XFA packets plus stream decode diagnostics, XML prolog/root diagnostics, and conservative kind classification, and update exact directly represented XFA packets, but does not implement broad field appearance or XFA package semantics.
- pypdf-style CMap/font support parses `/ToUnicode`, encodings, CID widths, and font metrics. `binas` supports page font-scoped `/ToUnicode` maps for simple `Tf` flows and one unambiguous fallback map for hex text matching/replacement, but does not verify widths, visual layout, or full font semantics.

The useful near-term `binas` moat is therefore not broad PDF feature parity. It is fail-closed, span-preserving, verified rewrites for the subset that has explicit fixtures and command proof.

## Documentation Rule

Do not document a surface as editable until all of these exist:

- A focused fixture or real-file proof for that exact shape.
- Parser metadata that distinguishes supported from unsupported cases without broad string false positives.
- Edit planning that refuses ambiguous semantic ownership.
- Apply-time span and source-byte precondition checks.
- Verification that reparses the produced PDF and proves the user-visible invariant claimed by the feature.

Until then, keep the surface either fail-closed or detection-only with an explicit warning.
