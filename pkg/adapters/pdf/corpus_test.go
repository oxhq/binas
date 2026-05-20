package pdf

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestCorpusUncompressedDirectLengthRewrite(t *testing.T) {
	input := readCorpusPDF(t, "uncompressed-direct-length.pdf")
	adapter := NewAdapter()

	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, tree, "08-15-2024", 1)

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
	if bytes.Contains(output, []byte(`08\05515\0552024`)) {
		t.Fatal("old encoded date remains")
	}
	if !bytes.Contains(output, []byte(`05\05504\0552026`)) {
		t.Fatal("new encoded date missing")
	}
	if !bytes.Contains(output, []byte("/Length 27")) {
		t.Fatalf("stream length changed unexpectedly:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
}

func TestCorpusMultipleStreamsQueriesAndEditsSelectedMatch(t *testing.T) {
	input := readCorpusPDF(t, "multiple-streams.pdf")
	adapter := NewAdapter()

	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if streams := tree.Query(core.Match{Kind: KindStream}); len(streams) != 2 {
		t.Fatalf("stream nodes = %d, want 2", len(streams))
	}
	assertCorpusTextMatches(t, tree, "first", 1)
	assertCorpusTextMatches(t, tree, "second", 1)

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "second"}, core.Mutation{Replace: "second-updated"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(output, []byte("/Length 28")) {
		t.Fatalf("second stream length was not updated:\n%s", output)
	}
	if bytes.Contains(output, []byte("(second) Tj")) {
		t.Fatal("old second stream text remains")
	}

	reparsed, err := adapter.Parse(output, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, reparsed, "first", 1)
	assertCorpusTextMatches(t, reparsed, "second-updated", 1)
}

func TestCorpusASCII85FlateDecodeFilterArrayRewrite(t *testing.T) {
	input := readCorpusPDF(t, "ascii85-flate-content-stream.pdf")
	adapter := NewAdapter()

	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, tree, "08-15-2024", 1)

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
	reparsed, err := adapter.Parse(output, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, reparsed, "05-04-2026", 1)
}

func TestCorpusASCIIHexFlateDecodeFilterArrayRewrite(t *testing.T) {
	input := readCorpusPDF(t, "asciihex-flate-content-stream.pdf")
	adapter := NewAdapter()

	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, tree, "08-15-2024", 1)

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 5, 2026"})
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
	reparsed, err := adapter.Parse(output, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, reparsed, "May 5, 2026", 1)
}

func TestCorpusASCIIHexDecodeFixtureRewrite(t *testing.T) {
	input := readCorpusPDF(t, "asciihex-content-stream.pdf")
	adapter := NewAdapter()

	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, tree, "08-15-2024", 1)

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
	reparsed, err := adapter.Parse(output, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, reparsed, "05-04-2026", 1)
}

func TestCorpusDecodeParmsPredictor12Columns1Rewrite(t *testing.T) {
	input := readCorpusPDF(t, "flate-decodeparms-predictor12-columns1.pdf")
	adapter := NewAdapter()

	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, tree, "08-15-2024", 1)

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 5, 2026"})
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
	reparsed, err := adapter.Parse(output, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, reparsed, "May 5, 2026", 1)
}

func TestCorpusDecodeParmsPredictor12Columns4Rewrite(t *testing.T) {
	input := readCorpusPDF(t, "flate-decodeparms-predictor12-columns4.pdf")
	adapter := NewAdapter()

	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, tree, "08-15-2024", 1)

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 5, 2026"})
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
	reparsed, err := adapter.Parse(output, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, reparsed, "May 5, 2026", 1)
}

func TestCorpusDecodeParmsPredictor12RGBRewrite(t *testing.T) {
	input := readCorpusPDF(t, "flate-decodeparms-predictor12-rgb.pdf")
	adapter := NewAdapter()

	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, tree, "ABCDEF", 1)

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "ABCDEF"}, core.Mutation{Replace: "UVWXYZ123"})
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
	reparsed, err := adapter.Parse(output, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, reparsed, "UVWXYZ123", 1)
}

func TestCorpusDecodeParmsPredictor12BitsPerComponent16Rewrite(t *testing.T) {
	input := readCorpusPDF(t, "flate-decodeparms-predictor12-bpc16.pdf")
	adapter := NewAdapter()

	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, tree, "ABCD", 1)

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "ABCD"}, core.Mutation{Replace: "WXYZ"})
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
	reparsed, err := adapter.Parse(output, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, reparsed, "WXYZ", 1)
}

func TestCorpusMalformedMissingEOFStrictError(t *testing.T) {
	input := readCorpusPDF(t, "malformed-missing-eof.pdf")
	adapter := NewAdapter()

	_, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err == nil {
		t.Fatal("expected strict parse error")
	}
	if err.Error() != "malformed PDF: missing EOF marker" {
		t.Fatalf("error = %q", err)
	}

	tree, err := adapter.Parse(input, core.ParseOptions{})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, tree, "loose", 1)
}

func TestCorpusXrefStreamFixtureFailsClosed(t *testing.T) {
	tree := assertCorpusParseError(t, "xref-stream.pdf", "unsupported PDF: xref streams are not implemented")
	assertUnsupportedParseXrefMetadata(t, tree, "xref stream corpus fixture", map[string]any{
		"has_stream":        true,
		"stream_count":      1,
		"has_object_stream": false,
	})
}

func TestCorpusObjectStreamFixtureParses(t *testing.T) {
	input := readCorpusPDF(t, "object-stream.pdf")
	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	xref := tree.Nodes[tree.Root].Value.(map[string]any)["xref"].(map[string]any)
	if xref["has_object_stream"] != true {
		t.Fatalf("xref has_object_stream = %v", xref["has_object_stream"])
	}
}

func TestCorpusQuoteOperatorsRewriteLiteralAndHexText(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n14 TL\n(LineOld) '\n3 4 <4865784F6C64> \"\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 3 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	output := assertCorpusRewrite(t, input, "LineOld", "LineNew")
	if bytes.Contains(output, []byte("(LineOld) '")) {
		t.Fatal("old single-quote text show remains")
	}
	if !bytes.Contains(output, []byte("(LineNew) '")) {
		t.Fatalf("new single-quote text show missing:\n%s", output)
	}

	output = assertCorpusRewrite(t, output, "HexOld", "HexNew")
	if bytes.Contains(output, []byte(`3 4 <4865784F6C64> "`)) {
		t.Fatal("old double-quote text show remains")
	}
	if !bytes.Contains(output, []byte(`3 4 <4865784E6577> "`)) {
		t.Fatalf("new double-quote text show missing:\n%s", output)
	}
}

func TestCorpusSimpleFontDifferencesRewritePreservesEncodingBytes(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<4142> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type1 /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /Euro /Aacute] >> >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	output := assertCorpusRewrite(t, input, "\u20ac\u00c1", "\u00c1\u20ac")
	if bytes.Contains(output, []byte("<4142> Tj")) {
		t.Fatal("old Differences-encoded operand remains")
	}
	if !bytes.Contains(output, []byte("<4241> Tj")) {
		t.Fatalf("new Differences-encoded operand missing:\n%s", output)
	}
}

func TestCorpusType0CIDCMapRewriteCarriesWidthProof(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<00010002> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /DescendantFonts [6 0 R] /ToUnicode 5 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		testTwoCIDToUnicodeCMapStream(),
		"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /CIDFixture /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /W [1 [500 610]] >>",
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "AB"}, core.Mutation{Replace: "BA"})
	if err != nil {
		t.Fatal(err)
	}
	if plan.Meta["layout_proof"] != layoutProofStatusWidthProven {
		t.Fatalf("layout_proof = %v, want %q; meta=%+v", plan.Meta["layout_proof"], layoutProofStatusWidthProven, plan.Meta)
	}
	if plan.Meta["old_width_units"] != 1110 || plan.Meta["new_width_units"] != 1110 || plan.Meta["width_delta_units"] != 0 {
		t.Fatalf("width proof meta = %+v, want equal 1110 widths", plan.Meta)
	}

	output, report, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if report.Meta["layout_proof"] != layoutProofStatusWidthProven {
		t.Fatalf("report layout_proof = %v, want %q; meta=%+v", report.Meta["layout_proof"], layoutProofStatusWidthProven, report.Meta)
	}
	if !bytes.Contains(output, []byte("<00020001> Tj")) {
		t.Fatalf("new Type0 CMap operand missing:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
}

func TestCorpusTJKerningArrayRewrite(t *testing.T) {
	content := []byte("BT\n[(08) -30 (\\05515) 25 (\\0552024)] TJ\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 3 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	output := assertCorpusRewrite(t, input, "08-15-2024", "05-05-2026")
	if bytes.Contains(output, []byte("[(08) -30 (\\05515) 25 (\\0552024)] TJ")) {
		t.Fatal("old TJ kerning array remains")
	}
	if !bytes.Contains(output, []byte("[(05\\05505\\0552026)] TJ")) {
		t.Fatalf("new TJ array replacement missing:\n%s", output)
	}
}

func TestCorpusDirectFormXObjectTextRewrite(t *testing.T) {
	pageContent := []byte("q\n/Fm1 Do\nQ\n")
	formContent := []byte("BT\n(FormOld) Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /XObject << /Fm1 5 0 R >> >> >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(pageContent), pageContent),
		fmt.Sprintf("<< /Type /XObject /Subtype /Form /Length %d >>\nstream\n%sendstream", len(formContent), formContent),
	)

	output := assertCorpusRewrite(t, input, "FormOld", "FormNew")
	if bytes.Contains(output, []byte("(FormOld) Tj")) {
		t.Fatal("old Form XObject text remains")
	}
	if !bytes.Contains(output, []byte("(FormNew) Tj")) {
		t.Fatalf("new Form XObject text missing:\n%s", output)
	}
}

func readCorpusPDF(t *testing.T, name string) []byte {
	t.Helper()
	path := filepath.Join("..", "..", "..", "testdata", "pdf", name)
	input, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return input
}

func assertCorpusParseError(t *testing.T, name, want string) *core.Tree {
	t.Helper()
	input := readCorpusPDF(t, name)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err == nil {
		t.Fatalf("expected parse error for %s", name)
	}
	if err.Error() != want {
		t.Fatalf("error = %q, want %q", err, want)
	}
	return tree
}

func assertCorpusRewrite(t *testing.T, input []byte, oldText, newText string) []byte {
	t.Helper()
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, tree, oldText, 1)

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: oldText}, core.Mutation{Replace: newText})
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
	reparsed, err := adapter.Parse(output, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, reparsed, newText, 1)
	return output
}

func assertCorpusTextMatches(t *testing.T, tree *core.Tree, text string, want int) {
	t.Helper()
	got := tree.Query(core.Match{Kind: KindTextShow, Text: text})
	if len(got) != want {
		t.Fatalf("%q matches = %d, want %d", text, len(got), want)
	}
}
