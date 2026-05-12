package pdf

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestLiteralStringDecodingAndEncoding(t *testing.T) {
	got := decodeLiteralString(`08\05515\0552024`)
	if got != "08-15-2024" {
		t.Fatalf("decoded literal = %q", got)
	}
	encoded := encodeLiteralString("05-04-2026")
	if encoded != `05\05504\0552026` {
		t.Fatalf("encoded literal = %q", encoded)
	}
}

func TestParseFindsTextShowNodes(t *testing.T) {
	input := []byte("%PDF-1.3\n1 0 obj\n<<>>\nstream\nBT\n(08\\05515\\0552024) Tj\nET\nendstream\nendobj\n%%EOF\n")
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].Span.Len() != int64(len(`08\05515\0552024`)) {
		t.Fatalf("span len = %d", matches[0].Span.Len())
	}
}

func TestParseAddsUnfilteredStreamLengthMetadata(t *testing.T) {
	content := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	streams := tree.Query(core.Match{Kind: KindStream})
	if len(streams) != 1 {
		t.Fatalf("stream nodes = %d, want 1", len(streams))
	}
	meta := streams[0].Meta
	if meta["encoded_length"] != len(content) {
		t.Fatalf("encoded_length = %v, want %d", meta["encoded_length"], len(content))
	}
	if meta["decoded_length"] != len(content) {
		t.Fatalf("decoded_length = %v, want %d", meta["decoded_length"], len(content))
	}
	if _, ok := meta["filter_chain"]; ok {
		t.Fatalf("filter_chain present for unfiltered stream: %+v", meta)
	}
}

func TestParseAddsFilteredStreamLengthAndFilterMetadata(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	encoded, err := encodeFlateDecode(decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode >>\nstream\n%sendstream", len(encoded), encoded),
	)
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	streams := tree.Query(core.Match{Kind: KindStream})
	if len(streams) != 1 {
		t.Fatalf("stream nodes = %d, want 1", len(streams))
	}
	meta := streams[0].Meta
	if meta["encoded_length"] != len(encoded) {
		t.Fatalf("encoded_length = %v, want %d", meta["encoded_length"], len(encoded))
	}
	if meta["decoded_length"] != len(decoded) {
		t.Fatalf("decoded_length = %v, want %d", meta["decoded_length"], len(decoded))
	}
	if meta["filter"] != "FlateDecode" {
		t.Fatalf("filter = %v, want FlateDecode", meta["filter"])
	}
	assertStringSliceMeta(t, meta, "filter_chain", []string{"FlateDecode"})
}

func TestParseKeepsUnsupportedFilteredStreamMetadataWithoutDecodedLength(t *testing.T) {
	encoded := []byte("not-jpeg")
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /DCTDecode >>\nstream\n%sendstream", len(encoded), encoded),
	)
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	streams := tree.Query(core.Match{Kind: KindStream})
	if len(streams) != 1 {
		t.Fatalf("stream nodes = %d, want 1", len(streams))
	}
	meta := streams[0].Meta
	if meta["encoded_length"] != len(encoded) {
		t.Fatalf("encoded_length = %v, want %d", meta["encoded_length"], len(encoded))
	}
	if _, ok := meta["decoded_length"]; ok {
		t.Fatalf("decoded_length present for unsupported filtered stream: %+v", meta)
	}
	if meta["filter"] != "DCTDecode" {
		t.Fatalf("filter = %v, want DCTDecode", meta["filter"])
	}
	assertStringSliceMeta(t, meta, "filter_chain", []string{"DCTDecode"})
}

func TestParseFindsHexStringTextShowNodes(t *testing.T) {
	content := []byte("BT\n<30382D31352D32303234> Tj\nET\n")
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].Span.Len() != int64(len("30382D31352D32303234")) {
		t.Fatalf("span len = %d", matches[0].Span.Len())
	}
	if matches[0].Meta["encoding"] != "hex" {
		t.Fatalf("encoding meta = %v, want hex", matches[0].Meta["encoding"])
	}
}

func TestPlanEditRewritesHexStringTextShowOperand(t *testing.T) {
	content := []byte("BT\n<30382D31352D32303234> Tj\nET\n")
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 5, 2026"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(output, []byte("30382D31352D32303234")) {
		t.Fatal("old hex text remains")
	}
	if !bytes.Contains(output, []byte("<4D617920352C2032303236> Tj")) {
		t.Fatalf("new hex text missing:\n%s", output)
	}
	if !bytes.Contains(output, []byte("/Length 34")) {
		t.Fatalf("stream length was not updated:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
}

func TestHexStringTextShowMalformedHexFailsClosed(t *testing.T) {
	for _, encoded := range []string{"303", "303G"} {
		t.Run(encoded, func(t *testing.T) {
			content := []byte(fmt.Sprintf("BT\n<%s> Tj\nET\n", encoded))
			input := testPDF(
				"<< /Type /Page >>",
				fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
			)
			tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
			if err != nil {
				t.Fatal(err)
			}
			if matches := tree.Query(core.Match{Kind: KindTextShow}); len(matches) != 0 {
				t.Fatalf("text matches = %d, want 0", len(matches))
			}
			_, err = NewAdapter().PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "05-04-2026"})
			if err == nil {
				t.Fatal("expected edit planning to fail closed")
			}
		})
	}
}

func TestPlanEditHexStringRejectsNonASCIIReplacement(t *testing.T) {
	content := []byte("BT\n<30382D31352D32303234> Tj\nET\n")
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	_, err = NewAdapter().PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "mañana"})
	if err == nil {
		t.Fatal("expected non-ASCII hex replacement to fail closed")
	}
	if err.Error() != "replacement for hex text show operand must be ASCII" {
		t.Fatalf("error = %q", err)
	}
}

func TestParseAddsXrefSummaryToDocumentRoot(t *testing.T) {
	input := []byte("%PDF-1.3\n1 0 obj\n<< /Type /Page /Length 27 >>\nstream\nBT\n(08\\05515\\0552024) Tj\nET\nendstream\nendobj\nxref\n0 2\n0000000000 65535 f \n0000000009 00000 n \ntrailer\n<< /Size 2 /Root 1 0 R >>\nstartxref\n100\n%%EOF\n")
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	root, ok := tree.Node(tree.Root)
	if !ok {
		t.Fatal("missing root node")
	}
	value := root.Value.(map[string]any)
	xref := value["xref"].(map[string]any)
	if xref["has_table"] != true {
		t.Fatalf("xref has_table = %v", xref["has_table"])
	}
	if xref["object_count"] != 1 {
		t.Fatalf("xref object_count = %v", xref["object_count"])
	}
	if xref["has_object_stream"] != false {
		t.Fatalf("xref has_object_stream = %v", xref["has_object_stream"])
	}
	if xref["object_stream_count"] != 0 {
		t.Fatalf("xref object_stream_count = %v", xref["object_stream_count"])
	}
}

func TestParseSupportsParseableXrefStreamThroughGraph(t *testing.T) {
	input := testXrefStreamPDF(t)
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	root := tree.Nodes[tree.Root]
	value := root.Value.(map[string]any)
	xref := value["xref"].(map[string]any)
	if xref["has_stream"] != true {
		t.Fatalf("xref has_stream = %v, want true", xref["has_stream"])
	}
	objectGraph := value["object_graph"].(map[string]any)
	if objectGraph["parsed"] != true {
		t.Fatalf("object graph parsed = %v, want true", objectGraph["parsed"])
	}
	if objectGraph["xref_stream_entries"] != 5 {
		t.Fatalf("xref stream entries = %v, want 5", objectGraph["xref_stream_entries"])
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	streams := tree.Query(core.Match{Kind: KindStream, Meta: map[string]any{"canonical_graph": true}})
	if len(streams) != 1 {
		t.Fatalf("canonical stream nodes = %d, want 1", len(streams))
	}
	if streams[0].Meta["encoded_length"] != int(streams[0].Span.Len()) {
		t.Fatalf("canonical stream encoded_length = %v, want %d", streams[0].Meta["encoded_length"], streams[0].Span.Len())
	}
	if streams[0].Meta["decoded_length"] != int(streams[0].Span.Len()) {
		t.Fatalf("canonical stream decoded_length = %v, want %d", streams[0].Meta["decoded_length"], streams[0].Span.Len())
	}
}

func TestParseRejectsMalformedXrefStreamsClearly(t *testing.T) {
	input := []byte("%PDF-1.5\n1 0 obj\n<< /Type /Catalog >>\nendobj\n8 0 obj\n<< /Type /XRef /Length 4 /Size 9 /W [1 2 1] >>\nstream\nabcd\nendstream\nendobj\nstartxref\n45\n%%EOF\n")
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err == nil {
		t.Fatal("expected xref stream error")
	}
	if err.Error() != "unsupported PDF: xref streams are not implemented" {
		t.Fatalf("error = %q", err)
	}
	assertUnsupportedParseXrefMetadata(t, tree, "xref stream", map[string]any{
		"has_stream":        true,
		"stream_count":      1,
		"has_object_stream": false,
	})
}

func TestParseSupportsObjectStreams(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /ObjStm /N 1 /First 4 /Length 8 >>\nstream\n0 0 <<>>\nendstream",
	)
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	xref := tree.Nodes[tree.Root].Value.(map[string]any)["xref"].(map[string]any)
	if xref["has_object_stream"] != true {
		t.Fatalf("xref has_object_stream = %v", xref["has_object_stream"])
	}
	if xref["object_stream_count"] != 1 {
		t.Fatalf("xref object_stream_count = %v", xref["object_stream_count"])
	}
}

func TestParseRejectsEncryptClearly(t *testing.T) {
	input := testPDF("<< /Type /Catalog /Encrypt 2 0 R >>", "<<>>")
	_, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err == nil {
		t.Fatal("expected encrypted PDF error")
	}
	if err.Error() != "unsupported PDF: encrypted PDFs require an explicit password-capable path; Adapter.Parse does not decrypt" {
		t.Fatalf("error = %q", err)
	}
}

func TestParseRejectsSignatureClearly(t *testing.T) {
	input := testPDF("<< /Type /Catalog /SigFlags 3 >>", "<< /FT /Sig >>")
	_, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err == nil {
		t.Fatal("expected signature PDF error")
	}
	if err.Error() != "unsupported PDF: digital signatures require explicit invalidation mode; Adapter.Parse refuses silent signature invalidation" {
		t.Fatalf("error = %q", err)
	}
}

func TestParseRejectsXFAClearly(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA 2 0 R >> >>", "<<>>")
	_, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err == nil {
		t.Fatal("expected XFA PDF error")
	}
	if err.Error() != "unsupported PDF: XFA forms are not implemented" {
		t.Fatalf("error = %q", err)
	}
}

func TestParseReportsAcroFormAnnotAndCMapBoundaries(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Type /Page /Annots [ ] /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /ToUnicode 4 0 R /DescendantFonts [5 0 R] >>",
		"<< /Length 9 >>\nstream\nbegincmap\nendstream",
		"<< /Type /Font /Subtype /CIDFontType2 >>",
	)
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	root, ok := tree.Node(tree.Root)
	if !ok {
		t.Fatal("missing root node")
	}
	value := root.Value.(map[string]any)
	boundaries := value["boundaries"].(map[string]any)
	for _, key := range []string{
		"has_acroform",
		"has_annotations",
		"has_font_markers",
		"has_cmap_markers",
		"has_tounicode_cmap",
		"has_cid_font_markers",
	} {
		if boundaries[key] != true {
			t.Fatalf("%s = %v, want true", key, boundaries[key])
		}
	}
	for _, key := range []string{"has_encrypt", "has_signature", "has_xfa"} {
		if boundaries[key] != false {
			t.Fatalf("%s = %v, want false", key, boundaries[key])
		}
	}
}

func assertUnsupportedParseXrefMetadata(t *testing.T, tree *core.Tree, name string, want map[string]any) {
	t.Helper()
	if tree == nil {
		t.Fatalf("%s unsupported parse returned nil tree, want partial metadata", name)
	}
	root, ok := tree.Node(tree.Root)
	if !ok {
		t.Fatalf("%s unsupported parse returned no root node", name)
	}
	value := root.Value.(map[string]any)
	xref := value["xref"].(map[string]any)
	for key, expected := range want {
		if xref[key] != expected {
			t.Fatalf("%s xref %s = %v, want %v", name, key, xref[key], expected)
		}
	}
	objects, ok := xref["objects"].([]xrefObjectOffset)
	if !ok || len(objects) == 0 {
		t.Fatalf("%s xref objects = %#v, want object offset metadata", name, xref["objects"])
	}
}

func TestW8BENDateRewriteIsSelectableText(t *testing.T) {
	path := os.Getenv("BINAS_W8BEN_PDF")
	if path == "" {
		t.Skip("BINAS_W8BEN_PDF is not set")
	}
	input, err := os.ReadFile(path)
	if err != nil {
		t.Skipf("fixture unavailable: %v", err)
	}
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "05-04-2026"})
	if err != nil {
		t.Fatal(err)
	}
	output, report, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if report.FallbackUsed {
		t.Fatal("unexpected fallback")
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
	outPath := filepath.Join(t.TempDir(), "updated.pdf")
	if err := os.WriteFile(outPath, output, 0644); err != nil {
		t.Fatal(err)
	}
	reparsed, err := adapter.Parse(output, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if got := reparsed.Query(core.Match{Kind: KindTextShow, Text: "05-04-2026"}); len(got) != 1 {
		t.Fatalf("new selectable matches = %d, want 1", len(got))
	}
}

func TestVariableLengthRewriteUpdatesStreamLengthAndXref(t *testing.T) {
	input := []byte("%PDF-1.3\n1 0 obj\n<< /Type /Page >>\nendobj\n2 0 obj\n<<\n/Length 27\n>>\nstream\nBT\n(08\\05515\\0552024) Tj\nET\nendstream\nendobj\nxref\n0 3\n0000000000 65535 f \n0000000009 00000 n \n0000000045 00000 n \ntrailer\n<< /Size 3 /Root 1 0 R >>\nstartxref\n122\n%%EOF\n")
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 5, 2026"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(output, []byte(`08\05515\0552024`)) {
		t.Fatal("old encoded text remains")
	}
	if !bytes.Contains(output, []byte(`May 5, 2026`)) {
		t.Fatal("new variable-length text missing")
	}
	if !bytes.Contains(output, []byte("/Length 22")) {
		t.Fatalf("stream length was not updated:\n%s", output)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable {
		t.Fatalf("verification failed: %+v", verification)
	}
}

func TestIndirectLengthRewriteUpdatesReferencedObjectAndXref(t *testing.T) {
	input := testPDF(
		"<< /Type /Page >>",
		"<<\n/Length 3 0 R\n>>\nstream\nBT\n(08\\05515\\0552024) Tj\nET\nendstream",
		"27",
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})
	if len(matches) != 1 {
		t.Fatalf("old selectable matches = %d, want 1", len(matches))
	}
	if refStreams := tree.Query(core.Match{Kind: KindStream, Meta: map[string]any{"length_ref": true}}); len(refStreams) != 1 {
		t.Fatalf("indirect-length streams = %d, want 1", len(refStreams))
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 5, 2026"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(output, []byte("/Length 3 0 R")) {
		t.Fatalf("stream no longer references indirect length object:\n%s", output)
	}
	if !bytes.Contains(output, []byte("3 0 obj\n22\nendobj")) {
		t.Fatalf("referenced length object was not updated:\n%s", output)
	}
	if bytes.Contains(output, []byte("/Length 22")) {
		t.Fatalf("stream dictionary was rewritten directly instead of preserving reference:\n%s", output)
	}
	if !bytes.Contains(output, []byte("xref\n0 4\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
}

func TestIndirectLengthRewriteUpdatesNonZeroGenerationReferencedObjectAndXref(t *testing.T) {
	input := testPDFWithObjects(
		testPDFObject{number: 1, body: "<< /Type /Page >>"},
		testPDFObject{number: 2, body: "<<\n/Length 3 2 R\n>>\nstream\nBT\n(08\\05515\\0552024) Tj\nET\nendstream"},
		testPDFObject{number: 3, generation: 2, body: "27"},
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"})
	if len(matches) != 1 {
		t.Fatalf("old selectable matches = %d, want 1", len(matches))
	}
	if refStreams := tree.Query(core.Match{Kind: KindStream, Meta: map[string]any{"length_ref": true}}); len(refStreams) != 1 {
		t.Fatalf("indirect-length streams = %d, want 1", len(refStreams))
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 5, 2026"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(output, []byte("/Length 3 2 R")) {
		t.Fatalf("stream no longer references non-zero generation indirect length object:\n%s", output)
	}
	if !bytes.Contains(output, []byte("3 2 obj\n22\nendobj")) {
		t.Fatalf("referenced non-zero generation length object was not updated:\n%s", output)
	}
	if bytes.Contains(output, []byte("/Length 22")) {
		t.Fatalf("stream dictionary was rewritten directly instead of preserving reference:\n%s", output)
	}
	if !bytes.Contains(output, []byte("xref\n0 4\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
	if !bytes.Contains(output, []byte("00002 n \ntrailer")) {
		t.Fatalf("xref table does not contain generation-aware entry:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
}

func TestFlateDecodeRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	encoded, err := encodeFlateDecode(decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode >>\nstream\n%sendstream", len(encoded), encoded),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"}); len(matches) != 1 {
		t.Fatalf("old selectable matches = %d, want 1", len(matches))
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 5, 2026"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
	streamAt := bytes.Index(output, []byte("stream\n"))
	if streamAt == -1 {
		t.Fatal("missing stream marker")
	}
	dataStart := streamAt + len("stream\n")
	objStart := findObjectStart(output, streamAt)
	lengthSpan, lengthValue, err := findDirectLength(output, objStart, streamAt)
	if err != nil {
		t.Fatal(err)
	}
	if lengthSpan.Start < 0 {
		t.Fatal("missing direct stream length")
	}
	if dataStart+lengthValue > len(output) {
		t.Fatalf("stream length %d extends past output", lengthValue)
	}
	updatedDecoded, err := decodeFlateDecode(output[dataStart : dataStart+lengthValue])
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if lengthValue == len(encoded) {
		t.Fatal("compressed stream length was not updated")
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestRunLengthDecodeRewriteReencodesLengthAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	encoded, err := encodeStreamFilter("/RunLengthDecode", decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /RunLengthDecode >>\nstream\n%sendstream", len(encoded), encoded),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"}); len(matches) != 1 {
		t.Fatalf("old selectable matches = %d, want 1", len(matches))
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 5, 2026"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
	stream, ok, err := findNextStream(output, 0)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("missing stream")
	}
	if stream.lengthIndirect {
		t.Fatal("stream length unexpectedly became indirect")
	}
	if stream.lengthValue == len(encoded) {
		t.Fatal("encoded stream length was not updated")
	}
	updatedDecoded, err := decodeStreamFilter("/RunLengthDecode", output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !bytes.Contains(output, []byte("xref\n0 3\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
}

func TestFlateDecodeIndirectLengthRewriteReencodesReferencedObjectAndXref(t *testing.T) {
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	encoded, err := encodeFlateDecode(decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length 3 0 R /Filter /FlateDecode >>\nstream\n%sendstream", encoded),
		fmt.Sprintf("%d", len(encoded)),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"}); len(matches) != 1 {
		t.Fatalf("old selectable matches = %d, want 1", len(matches))
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 5, 2026"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	stream, ok, err := findNextStream(output, 0)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("missing stream")
	}
	if !stream.lengthIndirect {
		t.Fatal("stream length is no longer indirect")
	}
	if stream.lengthValue == len(encoded) {
		t.Fatal("referenced compressed stream length was not updated")
	}
	updatedDecoded, err := decodeFlateDecode(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("decoded stream = %q", updatedDecoded)
	}
	if !bytes.Contains(output, []byte("/Length 3 0 R")) {
		t.Fatalf("stream no longer references indirect length object:\n%s", output)
	}
	wantLengthObject := []byte(fmt.Sprintf("3 0 obj\n%d\nendobj", stream.lengthValue))
	if !bytes.Contains(output, wantLengthObject) {
		t.Fatalf("referenced length object was not updated to %d:\n%s", stream.lengthValue, output)
	}
	if !bytes.Contains(output, []byte("xref\n0 4\n")) {
		t.Fatalf("xref table was not rebuilt:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
}

func TestIndirectLengthMissingOrNonIntegerFailsClosed(t *testing.T) {
	encoded, err := encodeFlateDecode([]byte("BT\n(08\\05515\\0552024) Tj\nET\n"))
	if err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		name    string
		objects []string
	}{
		{
			name: "raw missing object",
			objects: []string{
				"<< /Type /Page >>",
				"<< /Length 3 0 R >>\nstream\nBT\n(08\\05515\\0552024) Tj\nET\nendstream",
			},
		},
		{
			name: "raw non-integer object",
			objects: []string{
				"<< /Type /Page >>",
				"<< /Length 3 0 R >>\nstream\nBT\n(08\\05515\\0552024) Tj\nET\nendstream",
				"<< /Length 27 >>",
			},
		},
		{
			name: "flate missing object",
			objects: []string{
				"<< /Type /Page >>",
				fmt.Sprintf("<< /Length 3 0 R /Filter /FlateDecode >>\nstream\n%sendstream", encoded),
			},
		},
		{
			name: "flate non-integer object",
			objects: []string{
				"<< /Type /Page >>",
				fmt.Sprintf("<< /Length 3 0 R /Filter /FlateDecode >>\nstream\n%sendstream", encoded),
				"(27)",
			},
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			tree, err := NewAdapter().Parse(testPDF(tc.objects...), core.ParseOptions{Strict: true})
			if err != nil {
				t.Fatal(err)
			}
			if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"}); len(matches) != 0 {
				t.Fatalf("text matches = %d, want 0", len(matches))
			}
			streams := tree.Query(core.Match{Kind: KindStream, Meta: map[string]any{
				"unsupported": "unsupported stream: /Length reference must resolve to an integer object",
			}})
			if len(streams) != 1 {
				t.Fatalf("unsupported stream matches = %d, want 1", len(streams))
			}
			_, err = NewAdapter().PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "05-04-2026"})
			if err == nil {
				t.Fatal("expected edit planning to fail closed")
			}
		})
	}
}

func TestFlateDecodeUnsupportedParametersFailClosed(t *testing.T) {
	encoded, err := encodeFlateDecode([]byte("BT\n(08\\05515\\0552024) Tj\nET\n"))
	if err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		name string
		dict string
		want string
	}{
		{
			name: "unsupported filter in multi-filter array",
			dict: fmt.Sprintf("<< /Length %d /Filter [/DCTDecode /FlateDecode] >>", len(encoded)),
			want: `unsupported PDF stream filter "DCTDecode FlateDecode"`,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			input := testPDF("<< /Type /Page >>", fmt.Sprintf("%s\nstream\n%sendstream", tc.dict, encoded))
			tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
			if err != nil {
				t.Fatal(err)
			}
			if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"}); len(matches) != 0 {
				t.Fatalf("text matches = %d, want 0", len(matches))
			}
			streams := tree.Query(core.Match{Kind: KindStream, Meta: map[string]any{"unsupported": tc.want}})
			if len(streams) != 1 {
				t.Fatalf("unsupported stream matches = %d, want 1", len(streams))
			}
			_, err = NewAdapter().PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "05-04-2026"})
			if err == nil {
				t.Fatal("expected edit planning to fail closed")
			}
		})
	}
}

func TestDecodeParmsUnsupportedParametersFailClosed(t *testing.T) {
	encoded, err := encodeFlateDecode([]byte("BT\n(08\\05515\\0552024) Tj\nET\n"))
	if err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		name string
		dict string
		want string
	}{
		{
			name: "tiff predictor columns zero",
			dict: fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms << /Predictor 2 /Columns 0 >> >>", len(encoded)),
			want: "unsupported stream: /DecodeParms TIFF predictor requires /Columns >= 1",
		},
		{
			name: "png predictor columns zero",
			dict: fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 0 >> >>", len(encoded)),
			want: "unsupported stream: /DecodeParms PNG predictors require /Columns >= 1",
		},
		{
			name: "missing predictor",
			dict: fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms << /Columns 1 >> >>", len(encoded)),
			want: "unsupported stream: /DecodeParms /Predictor is missing",
		},
		{
			name: "malformed scalar decode parms",
			dict: fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms 12 >>", len(encoded)),
			want: "unsupported stream: /DecodeParms must be a dictionary, array, or null",
		},
		{
			name: "malformed decode parms array entry",
			dict: fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms [12] >>", len(encoded)),
			want: "unsupported stream: /DecodeParms array entries must be null or direct dictionaries",
		},
		{
			name: "single filter decode parms array has too many entries",
			dict: fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms [<< /Predictor 12 >> << /Predictor 12 >>] >>", len(encoded)),
			want: "unsupported stream: /DecodeParms array length must match /Filter array length",
		},
		{
			name: "filter array decode parms missing null alignment",
			dict: fmt.Sprintf("<< /Length %d /Filter [/ASCII85Decode /FlateDecode] /DecodeParms [<< /Predictor 12 >> << /Predictor 12 >>] >>", len(encoded)),
			want: "unsupported stream: /DecodeParms for /ASCII85Decode must be null",
		},
		{
			name: "filter array decode parms too short",
			dict: fmt.Sprintf("<< /Length %d /Filter [/ASCII85Decode /FlateDecode] /DecodeParms [null] >>", len(encoded)),
			want: "unsupported stream: /DecodeParms array length must match /Filter array length",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			assertPDFDecodeParmsUnsupportedParametersFailClosed(t, tc.dict, encoded, tc.want)
		})
	}
}

func TestFlateDecodeTIFFBitPackedPredictorRewriteReencodesRowsAndXref(t *testing.T) {
	decoded := []byte("BT\n(A1) Tj\nET\n")
	decodeParms := "<< /Predictor 2 /Columns 7 /Colors 1 /BitsPerComponent 1 >>"
	encoded, err := encodeStreamFilterWithDecodeParms("/FlateDecode", decodeParms, decoded)
	if err != nil {
		t.Fatal(err)
	}
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms %s >>\nstream\n%sendstream", len(encoded), decodeParms, encoded),
	)

	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "A1"}); len(matches) != 1 {
		t.Fatalf("text matches = %d, want 1", len(matches))
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "A1"}, core.Mutation{Replace: "B2"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
	stream, ok, err := findNextStream(output, 0)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("missing stream")
	}
	updatedDecoded, err := decodeStreamFilterWithDecodeParms("/FlateDecode", decodeParms, output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(updatedDecoded, []byte("(B2) Tj")) {
		t.Fatalf("updated decoded stream = %q, want replacement", updatedDecoded)
	}
}

func TestDecodeParmsDecodedRowsFailClosed(t *testing.T) {
	predictedWithPartialRow := []byte{0, 'B', 0}
	encoded, err := encodeFlateDecode(predictedWithPartialRow)
	if err != nil {
		t.Fatal(err)
	}
	dict := fmt.Sprintf("<< /Length %d /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 1 /Colors 1 /BitsPerComponent 8 >> >>", len(encoded))
	assertPDFDecodeParmsUnsupportedParametersFailClosed(t, dict, encoded, "decode PNG predictor stream: partial row")
}

func assertPDFDecodeParmsUnsupportedParametersFailClosed(t *testing.T, dict string, encoded []byte, wantUnsupported string) {
	t.Helper()

	input := testPDF("<< /Type /Page >>", fmt.Sprintf("%s\nstream\n%sendstream", dict, encoded))
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"}); len(matches) != 0 {
		t.Fatalf("text matches = %d, want 0", len(matches))
	}
	streams := tree.Query(core.Match{Kind: KindStream})
	if len(streams) != 1 {
		t.Fatalf("stream nodes = %d, want 1", len(streams))
	}
	gotUnsupported, ok := streams[0].Meta["unsupported"]
	if !ok {
		t.Fatalf("stream unsupported metadata missing: %+v", streams[0].Meta)
	}
	if gotUnsupported != wantUnsupported {
		t.Fatalf("unsupported metadata = %q, want %q", gotUnsupported, wantUnsupported)
	}
	_, err = NewAdapter().PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "05-04-2026"})
	if err == nil {
		t.Fatal("expected edit planning to fail closed")
	}
}

func assertStringSliceMeta(t *testing.T, meta map[string]any, key string, want []string) {
	t.Helper()
	got, ok := meta[key].([]string)
	if !ok {
		t.Fatalf("%s = %#v, want []string", key, meta[key])
	}
	if len(got) != len(want) {
		t.Fatalf("%s length = %d, want %d (%v)", key, len(got), len(want), got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("%s[%d] = %q, want %q", key, i, got[i], want[i])
		}
	}
}

func testPDF(objects ...string) []byte {
	var out bytes.Buffer
	out.WriteString("%PDF-1.3\n")
	offsets := make([]int, 0, len(objects))
	for i, object := range objects {
		offsets = append(offsets, out.Len())
		fmt.Fprintf(&out, "%d 0 obj\n%s\nendobj\n", i+1, object)
	}
	xrefOffset := out.Len()
	fmt.Fprintf(&out, "xref\n0 %d\n", len(objects)+1)
	out.WriteString("0000000000 65535 f \n")
	for _, offset := range offsets {
		fmt.Fprintf(&out, "%010d 00000 n \n", offset)
	}
	fmt.Fprintf(&out, "trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n", len(objects)+1, xrefOffset)
	return out.Bytes()
}

type testPDFObject struct {
	number     int
	generation int
	body       string
}

func testPDFWithObjects(objects ...testPDFObject) []byte {
	var out bytes.Buffer
	out.WriteString("%PDF-1.3\n")
	offsets := make([]xrefObjectOffset, 0, len(objects))
	maxObject := 0
	for _, object := range objects {
		offsets = append(offsets, xrefObjectOffset{
			Number:     object.number,
			Generation: object.generation,
			Offset:     out.Len(),
		})
		if object.number > maxObject {
			maxObject = object.number
		}
		fmt.Fprintf(&out, "%d %d obj\n%s\nendobj\n", object.number, object.generation, object.body)
	}
	xrefOffset := out.Len()
	offsetByNumber := make(map[int]xrefObjectOffset, len(offsets))
	for _, offset := range offsets {
		offsetByNumber[offset.Number] = offset
	}
	fmt.Fprintf(&out, "xref\n0 %d\n", maxObject+1)
	out.WriteString("0000000000 65535 f \n")
	for number := 1; number <= maxObject; number++ {
		offset, ok := offsetByNumber[number]
		if !ok {
			out.WriteString("0000000000 65535 f \n")
			continue
		}
		fmt.Fprintf(&out, "%010d %05d n \n", offset.Offset, offset.Generation)
	}
	fmt.Fprintf(&out, "trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n", maxObject+1, xrefOffset)
	return out.Bytes()
}

func testXrefStreamPDF(t *testing.T) []byte {
	t.Helper()
	var out bytes.Buffer
	out.WriteString("%PDF-1.5\n")
	offsets := make(map[int]int)
	writeObject := func(number int, body []byte) {
		offsets[number] = out.Len()
		fmt.Fprintf(&out, "%d 0 obj\n", number)
		out.Write(body)
		if !bytes.HasSuffix(body, []byte("\n")) {
			out.WriteByte('\n')
		}
		out.WriteString("endobj\n")
	}
	content := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	writeObject(1, []byte("<< /Type /Catalog /Pages 2 0 R >>\n"))
	writeObject(2, []byte("<< /Type /Page /Contents 3 0 R >>\n"))
	writeObject(3, []byte(fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream\n", len(content), content)))

	xrefOffset := out.Len()
	var entries bytes.Buffer
	writeXrefEntry := func(entryType, offset, generation int) {
		entries.WriteByte(byte(entryType))
		entries.WriteByte(byte(offset >> 8))
		entries.WriteByte(byte(offset))
		entries.WriteByte(byte(generation))
	}
	writeXrefEntry(0, 0, 255)
	writeXrefEntry(1, offsets[1], 0)
	writeXrefEntry(1, offsets[2], 0)
	writeXrefEntry(1, offsets[3], 0)
	writeXrefEntry(1, xrefOffset, 0)

	offsets[4] = xrefOffset
	fmt.Fprintf(&out, "4 0 obj\n<< /Type /XRef /Size 5 /Root 1 0 R /W [1 2 1] /Length %d >>\nstream\n", entries.Len())
	out.Write(entries.Bytes())
	out.WriteString("\nendstream\nendobj\n")
	fmt.Fprintf(&out, "startxref\n%d\n%%%%EOF\n", xrefOffset)
	return out.Bytes()
}
