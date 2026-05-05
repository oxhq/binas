package pdf

import (
	"bytes"
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

func assertCorpusTextMatches(t *testing.T, tree *core.Tree, text string, want int) {
	t.Helper()
	got := tree.Query(core.Match{Kind: KindTextShow, Text: text})
	if len(got) != want {
		t.Fatalf("%q matches = %d, want %d", text, len(got), want)
	}
}
