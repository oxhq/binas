# binas

`binas` is a bounded PDF inspection and verified rewrite engine. The active implementation is a four-crate Rust workspace; the Go v0.2 implementation remains the frozen oracle and rollback surface during the Rust consumer cutover.

The engine parses PDF object/content structure, exposes queryable nodes, and applies fail-closed rewrites that are verified by reparsing the output. It is not a renderer or general visual PDF editor.

## Rust workspace

For a published Rust release, add the engine or install the CLI from crates.io:

```powershell
cargo add binas-pdf
cargo install binas-cli
```

To build and verify the current checkout instead:

```powershell
cargo test --workspace
cargo run -p binas-cli -- inspect C:\path\file.pdf --json
```

- `binas-pdf`: public PDF engine and consumer API.
- `binas-core`: shared checked spans, diagnostics, and verification vocabulary.
- `binas-cli`: installs the `binas` executable.
- `binas-pdf-wasm`: browser binding built from this workspace; it is not a crates.io package.

Rust releases and their proof boundaries are recorded in [CHANGELOG.md](CHANGELOG.md).

## Install Go v0.2

```powershell
go install github.com/oxhq/binas/cmd/binas@latest
```

Or build from a checkout:

```powershell
go test ./...
go build -o tmp\binas.exe ./cmd/binas
```

## Quick Start

```powershell
binas inspect C:\path\file.pdf --format pdf --json
binas validate C:\path\file.pdf --format pdf --json
binas query C:\path\file.pdf --format pdf --kind pdf.content.text_show --text "Invoice #1234" --json
binas edit C:\path\file.pdf --format pdf --kind pdf.content.text_show --text "Invoice #1234" --replace "Invoice #5678" --verify reparse,old-gone,new-selectable,page-count-unchanged,no-fallback -o C:\path\out.pdf --json
```

Go consumers can use the format-neutral v0 API without shelling out:

```go
import (
	"github.com/oxhq/binas"
	"github.com/oxhq/binas/pkg/core"
)

doc := binas.Open(input, binas.WithFormat(binas.PDF))
output, report, verification, err := doc.
	Kind("pdf.content.text_show").
	Text("Invoice #1234").
	Replace("Invoice #5678").
	Verify(core.InvariantReparse, core.InvariantOldGone, core.InvariantNewSelectable).
	Bytes()
```

That same fluent layer is backed by `core.Adapter`, so future adapters such as
ELF can use the same `Open` / `Inspect` / `Query` / `Edit` flow. PDF-specific
helpers remain in `pkg/pdfapi`:

```go
graph, err := pdfapi.ParseGraph(input, pdfapi.GraphOptions{})
pages, err := graph.PageTree()
output, report, verification, err := pdfapi.Merge([][]byte{left, right})
```

Canonical page operations preserve and reconcile catalog structures such as name
trees, page labels, outlines, and AcroForm fields across merged/inserted sources
when their referenced objects can be cloned safely. Metadata and viewer/open
action entries remain primary-document settings. Page transforms include
rotate/crop plus a conservative scale operation that scales page boxes and wraps
unfiltered content streams with a graphics-state matrix.

## CLI Surface

- `inspect`: parse an input and report root metadata, xref/object-stream shape, semantic boundary markers, and stream metadata.
- `validate`: report parse validity, errors, warnings, and partial root metadata where available.
- `profile`: summarize whether a PDF is currently editable/fillable, which rewrite mode is recommended, and which boundaries are present.
- `query`: select parsed nodes, including `pdf.content.text_show`, by decoded text and exact metadata filters.
- `edit`: rewrite supported selectable text operands and verify by reparsing.
- `form list` / `form set`: list AcroForm fields and update supported text, choice, checkbox, button, and narrow radio field shapes.
- `annot list` / `annot set-contents`: list annotations and update supported `/Contents` values, with explicit appearance preservation/removal/regeneration modes.
- `xfa list`, `xfa datasets`, `xfa mappings`, `xfa dataset-set`, `xfa replace`: inspect and narrowly update directly represented XFA packets and static datasets.
- `overlay text`: explicitly stamp visible text as a fallback; this is not a true selectable-text rewrite.
- `ocr text-layer-plan` / `ocr text-layer`: plan or embed caller-provided OCR text-layer data; `binas` does not run OCR.
- `signature inspect` / `signature re-sign`: inspect supported signature byte ranges/CMS metadata and append a new external-signer-backed signature layer.

See [docs/release-surface.md](docs/release-surface.md) for the JSON contracts and failure modes, and [docs/pdf-semantic-boundaries.md](docs/pdf-semantic-boundaries.md) for the semantic guardrails around encryption, signatures, XFA, AcroForm, annotations, and font/CMap markers.

## Supported PDF Rewrite Shape

`edit` rewrites direct literal-string text, supported hex-string text, and literal/hex `TJ` array text inside raw content streams and supported reversible filtered streams:

- `/FlateDecode`, `/LZWDecode`, `/ASCIIHexDecode`, `/ASCII85Decode`, `/RunLengthDecode`
- standard abbreviations `/Fl`, `/LZW`, `/AHx`, `/A85`, `/RL`
- filter arrays composed only from those reversible filters
- supported Flate/LZW `/DecodeParms` predictors and `/EarlyChange`
- direct or resolved standalone-indirect integer `/Length`
- graph/canonical rewrites for object-stream, xref-stream, and hybrid-xref PDFs when supported

Every rewrite remains fail-closed: ambiguous matches, unsupported encodings, unsupported streams, unsafe semantic surfaces, stale source spans, and failed verification stop the command instead of producing a partial edit.

## Boundaries

Not supported as broad/default behavior:

- universal arbitrary-PDF text replacement or layout reflow
- filters outside ASCIIHex, ASCII85, RunLength, Flate, LZW, and their abbreviations
- broader `/DecodeParms` shapes beyond the documented Flate/LZW support
- AESV3, public-key encryption, and unsupported crypt filters
- default edits to signed PDFs without explicit invalidation/preserve/re-sign modes
- legal-grade signature trust, revocation, timestamp, or system-root validation
- full dynamic XFA rendering
- full AcroForm/widget/annotation visual fidelity
- bundled OCR or hidden overlay fallback

Overlay and OCR text-layer output are available only through explicit fallback commands and are reported as fallback usage.

## Packages

- `pkg/core`: span-preserving tree, selector, adapter, edit plan, report, and verification types.
- root package `github.com/oxhq/binas`: fluent, format-neutral API over `core.Adapter`.
- `pkg/adapters/pdf`: PDF parser, object graph, stream/text/form/annotation/XFA/signature helpers, canonical writer, and verification logic.
- `pkg/pdfapi`: PDF-specific graph traversal, canonical page copy/insert/extract/merge/transform, narrow form/annotation/XFA/stream/image mutation, explicit overlay/OCR fallback helpers, security/signature inspection and signing helpers, Standard Security password encryption/decrypt-to-plain helpers, and direct PDF text rewrite helpers.
- `cmd/binas`: CLI for PDF inspection, validation, query, edit, forms, annotations, XFA, overlay/OCR fallback, and signature flows.
- `cmd/binas-wasm`: browser/WASM entrypoint for inspect, query, and verified text edit over `Uint8Array` PDF bytes.

## Browser/WASM

```powershell
$env:GOOS = "js"; $env:GOARCH = "wasm"
go build -o tmp\binas.wasm ./cmd/binas-wasm
Remove-Item Env:\GOOS, Env:\GOARCH
Copy-Item "$(go env GOROOT)\lib\wasm\wasm_exec.js" tmp\wasm_exec.js
python -m http.server 8765
# Open http://127.0.0.1:8765/cmd/binas-wasm/editor.html
```

The browser editor uses PDF.js for rendering and the Go WASM module for semantic inspect/query/edit operations. Rendering is not a second proof of PDF semantic correctness; the rewrite proof still comes from `binas`.

## Verification

```powershell
go test ./... -count=1
go build -o tmp\binas.exe ./cmd/binas
$env:GOOS = "js"; $env:GOARCH = "wasm"; go build -o tmp\binas.wasm ./cmd/binas-wasm; Remove-Item Env:\GOOS, Env:\GOARCH
```
