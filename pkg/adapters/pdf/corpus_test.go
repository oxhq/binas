package pdf

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
	"strings"
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

func TestCorpusP3DecodeParmsDirectDictionaryRewrite(t *testing.T) {
	p3AssertDecodeParmsRewrite(t, p3DecodeParmsRewriteFixture{
		name:        "p3-decodeparms-direct-dictionary.pdf",
		filter:      "FlateDecode",
		decodeParms: "<< /Predictor 12 /Columns 1 >>",
		filterChain: []string{"FlateDecode"},
	})
}

func TestCorpusP3DecodeParmsFilterArrayRewrite(t *testing.T) {
	p3AssertDecodeParmsRewrite(t, p3DecodeParmsRewriteFixture{
		name:        "p3-decodeparms-filter-array.pdf",
		filter:      "[/ASCIIHexDecode /FlateDecode]",
		decodeParms: "[null << /Predictor 12 /Columns 1 >>]",
		filterChain: []string{"ASCIIHexDecode", "FlateDecode"},
	})
}

func TestCorpusP3MalformedUnsupportedDecodeParmsFailsClosed(t *testing.T) {
	input := readCorpusPDF(t, "p3-decodeparms-malformed-unsupported.pdf")
	adapter := NewAdapter()

	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	streams := tree.Query(core.Match{Kind: KindStream})
	if len(streams) != 1 {
		t.Fatalf("stream nodes = %d, want 1", len(streams))
	}
	meta := streams[0].Meta
	if meta["filter"] != "[/FlateDecode /FlateDecode]" {
		t.Fatalf("filter = %v, want filter array; meta=%+v", meta["filter"], meta)
	}
	if meta["decode_parms"] != "<< /Predictor 1 >>" {
		t.Fatalf("decode_parms = %v, want malformed direct dictionary; meta=%+v", meta["decode_parms"], meta)
	}
	assertStringSliceMeta(t, meta, "filter_chain", []string{"FlateDecode", "FlateDecode"})
	if got := meta["unsupported"]; got != "unsupported stream: direct /DecodeParms dictionary requires a single /Filter" {
		t.Fatalf("unsupported metadata = %q, want malformed DecodeParms reason; meta=%+v", got, meta)
	}
	if _, ok := meta["decoded_length"]; ok {
		t.Fatalf("decoded_length present for malformed unsupported DecodeParms stream: %+v", meta)
	}
	assertCorpusTextMatches(t, tree, "08-15-2024", 0)

	if _, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "05-04-2026"}); err == nil {
		t.Fatal("expected malformed unsupported DecodeParms target edit to fail closed")
	}
}

func TestCorpusP2ImageOnlyFiltersPassThroughWhileSupportedContentEdits(t *testing.T) {
	input := readCorpusPDF(t, "p2-image-only-filters-supported-content.pdf")
	adapter := NewAdapter()

	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	streams := tree.Query(core.Match{Kind: KindStream})
	if len(streams) != 6 {
		t.Fatalf("stream nodes = %d, want 6", len(streams))
	}
	assertCorpusTextMatches(t, tree, "P2-TEXT", 1)

	p2AssertEditableStreamMetadata(t, streams[0].Meta, false)
	p2AssertPassThroughImageStreamMetadata(t, streams[1].Meta, []string{"DCTDecode"})
	p2AssertPassThroughImageStreamMetadata(t, streams[2].Meta, []string{"JPXDecode"})
	p2AssertPassThroughImageStreamMetadata(t, streams[3].Meta, []string{"CCITTFaxDecode"})
	p2AssertPassThroughImageStreamMetadata(t, streams[4].Meta, []string{"JBIG2Decode"})
	p2AssertPassThroughImageStreamMetadata(t, streams[5].Meta, []string{"DCTDecode", "JPXDecode"})

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "P2-TEXT"}, core.Mutation{Replace: "P2-DONE"})
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
	if bytes.Count(output, []byte("p2 image bytes")) != 5 {
		t.Fatal("image stream bytes changed during supported content edit")
	}

	reparsed, err := adapter.Parse(output, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	assertCorpusTextMatches(t, reparsed, "P2-DONE", 1)
}

func TestCorpusP2UnsupportedNonImageFilterFailsClosedButSupportedContentEdits(t *testing.T) {
	input := readCorpusPDF(t, "p2-unsupported-nonimage-filter-text.pdf")
	adapter := NewAdapter()

	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	streams := tree.Query(core.Match{Kind: KindStream})
	if len(streams) != 2 {
		t.Fatalf("stream nodes = %d, want 2", len(streams))
	}
	p2AssertUnsupportedTargetStreamMetadata(t, streams[0].Meta, "FooDecode")
	p2AssertEditableStreamMetadata(t, streams[1].Meta, false)
	assertCorpusTextMatches(t, tree, "UNSUPPORTED-P2", 0)
	assertCorpusTextMatches(t, tree, "SUPPORTED-P2", 1)

	if _, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "UNSUPPORTED-P2"}, core.Mutation{Replace: "P2-BLOCKED"}); err == nil {
		t.Fatal("expected unsupported non-image stream target edit to fail closed")
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "SUPPORTED-P2"}, core.Mutation{Replace: "SUPPORTED-OK"})
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
	if !bytes.Contains(output, []byte("(UNSUPPORTED-P2) Tj")) {
		t.Fatal("unsupported stream bytes changed during supported content edit")
	}
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

func TestCorpusP4PreserveStructureObjectStreamOutputKeepsMarkerAndSelectableText(t *testing.T) {
	input := readCorpusPDF(t, "p4-object-stream-preserve-text.pdf")

	output, report, verification, err := ApplyCanonicalEditWithWriterMode(
		input,
		PDFWriterModePreserveStructure,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-20-2026"},
		nil,
	)
	if err != nil {
		t.Fatalf("ApplyCanonicalEditWithWriterMode preserve-structure object stream: %v", err)
	}
	assertCorpusP4PreserveStructureReport(t, report, verification)
	if !bytes.Contains(output, []byte("/Type /ObjStm")) {
		t.Fatalf("preserve-structure object-stream output lost /ObjStm marker:\n%s", output)
	}
	assertCorpusP4OutputSelectableText(t, output, "08-15-2024", "05-20-2026")
}

func TestCorpusP4PreserveStructureXrefStreamOutputKeepsMarkerAndSelectableText(t *testing.T) {
	input := readCorpusPDF(t, "p4-xref-stream-preserve-text.pdf")

	output, report, verification, err := ApplyCanonicalEditWithWriterMode(
		input,
		PDFWriterModePreserveStructure,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-20-2026"},
		nil,
	)
	if err != nil {
		t.Fatalf("ApplyCanonicalEditWithWriterMode preserve-structure xref stream: %v", err)
	}
	assertCorpusP4PreserveStructureReport(t, report, verification)
	if !bytes.Contains(output, []byte("/Type /XRef")) {
		t.Fatalf("preserve-structure xref-stream output lost /Type /XRef marker:\n%s", output)
	}
	if bytes.Contains(output, []byte("\nxref\n")) {
		t.Fatalf("preserve-structure xref-stream output rebuilt a table xref:\n%s", output)
	}
	assertCorpusP4OutputSelectableText(t, output, "08-15-2024", "05-20-2026")
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

func assertCorpusP4PreserveStructureReport(t *testing.T, report core.Report, verification core.Verification) {
	t.Helper()
	if report.FallbackUsed {
		t.Fatalf("report = %+v, want no fallback", report)
	}
	if report.Meta["writer_mode"] != string(PDFWriterModePreserveStructure) {
		t.Fatalf("writer_mode = %v, want %q; meta=%+v", report.Meta["writer_mode"], PDFWriterModePreserveStructure, report.Meta)
	}
	if report.Meta["used_canonical_writer_path"] != false {
		t.Fatalf("used_canonical_writer_path = %v, want false; meta=%+v", report.Meta["used_canonical_writer_path"], report.Meta)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v, want reparse/old-gone/new-selectable/page-unchanged", verification)
	}
}

func assertCorpusP4OutputSelectableText(t *testing.T, output []byte, oldText, newText string) {
	t.Helper()
	reparsed, err := NewAdapter().Parse(output, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatalf("reparse preserve-structure output: %v\n%s", err, output)
	}
	assertCorpusTextMatches(t, reparsed, oldText, 0)
	assertCorpusTextMatches(t, reparsed, newText, 1)
}

type p3DecodeParmsRewriteFixture struct {
	name        string
	filter      string
	decodeParms string
	filterChain []string
}

func p3AssertDecodeParmsRewrite(t *testing.T, fixture p3DecodeParmsRewriteFixture) {
	t.Helper()
	const (
		oldText = "08-15-2024"
		newText = "05-04-2026"
	)
	decoded := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	input := readCorpusPDF(t, fixture.name)
	adapter := NewAdapter()

	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	streams := tree.Query(core.Match{Kind: KindStream})
	if len(streams) != 1 {
		t.Fatalf("stream nodes = %d, want 1", len(streams))
	}
	p3AssertSupportedDecodeParmsStreamMetadata(t, streams[0].Meta, fixture, len(decoded))
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
	assertCorpusTextMatches(t, reparsed, oldText, 0)

	stream, ok, err := findNextStream(output, 0)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("missing rewritten stream")
	}
	if stream.lengthIndirect {
		t.Fatal("stream length unexpectedly became indirect")
	}
	if stream.lengthValue != stream.dataEnd-stream.dataStart {
		t.Fatalf("stream length = %d, want %d", stream.lengthValue, stream.dataEnd-stream.dataStart)
	}
	updatedDecoded, err := decodeStreamFilterWithDecodeParms(fixture.filter, fixture.decodeParms, output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(updatedDecoded, []byte(encodeLiteralString(oldText))) || !bytes.Contains(updatedDecoded, []byte("("+encodeLiteralString(newText)+") Tj")) {
		t.Fatalf("decoded rewritten stream = %q", updatedDecoded)
	}
}

func p3AssertSupportedDecodeParmsStreamMetadata(t *testing.T, meta map[string]any, fixture p3DecodeParmsRewriteFixture, wantDecodedLength int) {
	t.Helper()
	if meta["filter"] != fixture.filter {
		t.Fatalf("filter = %v, want %q; meta=%+v", meta["filter"], fixture.filter, meta)
	}
	if meta["decode_parms"] != fixture.decodeParms {
		t.Fatalf("decode_parms = %v, want %q; meta=%+v", meta["decode_parms"], fixture.decodeParms, meta)
	}
	if meta["filter_capability"] != string(pdfStreamFilterCapabilityEditableReversible) {
		t.Fatalf("filter_capability = %v, want %q; meta=%+v", meta["filter_capability"], pdfStreamFilterCapabilityEditableReversible, meta)
	}
	if meta["filter_editable"] != true || meta["filter_pass_through"] != false || meta["filter_target"] != true {
		t.Fatalf("filter metadata = editable:%v pass_through:%v target:%v, want true/false/true; meta=%+v", meta["filter_editable"], meta["filter_pass_through"], meta["filter_target"], meta)
	}
	if meta["decoded_length"] != wantDecodedLength {
		t.Fatalf("decoded_length = %v, want %d; meta=%+v", meta["decoded_length"], wantDecodedLength, meta)
	}
	if encodedLength, ok := meta["encoded_length"].(int); !ok || encodedLength <= 0 {
		t.Fatalf("encoded_length = %v, want positive int; meta=%+v", meta["encoded_length"], meta)
	}
	if got, ok := meta["unsupported"]; ok {
		t.Fatalf("unsupported metadata present for supported DecodeParms stream: %v; meta=%+v", got, meta)
	}
	assertStringSliceMeta(t, meta, "filter_chain", fixture.filterChain)
	if strings.TrimSpace(fixture.decodeParms) == "" {
		t.Fatal("P3 DecodeParms fixture must assert an explicit DecodeParms shape")
	}
}

func p2AssertEditableStreamMetadata(t *testing.T, meta map[string]any, wantFilterTarget bool) {
	t.Helper()
	if meta["filter_capability"] != string(pdfStreamFilterCapabilityIdentityPassThrough) {
		t.Fatalf("filter_capability = %v, want %q; meta=%+v", meta["filter_capability"], pdfStreamFilterCapabilityIdentityPassThrough, meta)
	}
	if meta["filter_editable"] != false || meta["filter_pass_through"] != true || meta["filter_target"] != wantFilterTarget {
		t.Fatalf("filter metadata = editable:%v pass_through:%v target:%v, want false/true/%v; meta=%+v", meta["filter_editable"], meta["filter_pass_through"], meta["filter_target"], wantFilterTarget, meta)
	}
	if _, ok := meta["unsupported"]; ok {
		t.Fatalf("unsupported metadata present for editable stream: %+v", meta)
	}
}

func p2AssertPassThroughImageStreamMetadata(t *testing.T, meta map[string]any, wantChain []string) {
	t.Helper()
	if meta["image_xobject"] != true {
		t.Fatalf("image_xobject = %v, want true; meta=%+v", meta["image_xobject"], meta)
	}
	if meta["filter_capability"] != string(pdfStreamFilterCapabilityPassThroughImage) {
		t.Fatalf("filter_capability = %v, want %q; meta=%+v", meta["filter_capability"], pdfStreamFilterCapabilityPassThroughImage, meta)
	}
	if meta["filter_editable"] != false || meta["filter_pass_through"] != true || meta["filter_target"] != false {
		t.Fatalf("filter metadata = editable:%v pass_through:%v target:%v, want false/true/false; meta=%+v", meta["filter_editable"], meta["filter_pass_through"], meta["filter_target"], meta)
	}
	if _, ok := meta["unsupported"]; ok {
		t.Fatalf("unsupported metadata present for image pass-through stream: %+v", meta)
	}
	if _, ok := meta["decoded_length"]; ok {
		t.Fatalf("decoded_length present for image pass-through stream: %+v", meta)
	}
	assertStringSliceMeta(t, meta, "filter_chain", wantChain)
}

func p2AssertUnsupportedTargetStreamMetadata(t *testing.T, meta map[string]any, wantFilter string) {
	t.Helper()
	if meta["unsupported"] != fmt.Sprintf("unsupported PDF stream filter %q", wantFilter) {
		t.Fatalf("unsupported metadata = %v, want unsupported filter %q; meta=%+v", meta["unsupported"], wantFilter, meta)
	}
	if meta["filter_capability"] != string(pdfStreamFilterCapabilityUnsupportedTarget) {
		t.Fatalf("filter_capability = %v, want %q; meta=%+v", meta["filter_capability"], pdfStreamFilterCapabilityUnsupportedTarget, meta)
	}
	if meta["filter_editable"] != false || meta["filter_pass_through"] != false || meta["filter_target"] != true {
		t.Fatalf("filter metadata = editable:%v pass_through:%v target:%v, want false/false/true; meta=%+v", meta["filter_editable"], meta["filter_pass_through"], meta["filter_target"], meta)
	}
	if _, ok := meta["decoded_length"]; ok {
		t.Fatalf("decoded_length present for unsupported target stream: %+v", meta)
	}
}
