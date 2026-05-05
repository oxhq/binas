package pdf

import (
	"bytes"
	"fmt"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestTJArrayFindsConcatenatedSimpleStringElements(t *testing.T) {
	content := []byte("BT\n[(08) 25 <2D31352D32303234>] TJ\nET\n")
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
	if matches[0].Meta["operator"] != "TJ" {
		t.Fatalf("operator meta = %v, want TJ", matches[0].Meta["operator"])
	}
	if matches[0].Meta["encoding"] != "tj-array" {
		t.Fatalf("encoding meta = %v, want tj-array", matches[0].Meta["encoding"])
	}
	if matches[0].Span.Len() != int64(len("[(08) 25 <2D31352D32303234>]")) {
		t.Fatalf("span len = %d", matches[0].Span.Len())
	}
}

func TestPlanEditRewritesTJArrayAsSingleLiteralStringElement(t *testing.T) {
	content := []byte("BT\n[(08) 25 (\\05515\\0552024)] TJ\nET\n")
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "05-05-2026"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}

	if bytes.Contains(output, []byte("[(08) 25 (\\05515\\0552024)] TJ")) {
		t.Fatal("old TJ array remains")
	}
	if !bytes.Contains(output, []byte("[(05\\05505\\0552026)] TJ")) {
		t.Fatalf("new TJ array missing:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
}

func TestFlateDecodeTJArrayRewriteReencodesContainingStream(t *testing.T) {
	decoded := []byte("BT\n[(08) 25 (\\05515\\0552024)] TJ\nET\n")
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

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "05-05-2026"})
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
	updatedDecoded, err := decodeFlateDecode(output[stream.dataStart:stream.dataEnd])
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Contains(updatedDecoded, []byte("[(08) 25 (\\05515\\0552024)] TJ")) {
		t.Fatal("old decoded TJ array remains")
	}
	if !bytes.Contains(updatedDecoded, []byte("[(05\\05505\\0552026)] TJ")) {
		t.Fatalf("new decoded TJ array missing: %q", updatedDecoded)
	}
}

func TestCanonicalEditRewritesTJArrayAsSingleLiteralStringElement(t *testing.T) {
	content := []byte("BT\n[(08) 25 (\\05515\\0552024)] TJ\nET\n")
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	output, report, verification, err := ApplyCanonicalEdit(
		input,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-05-2026"},
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" {
		t.Fatalf("report edit = %q", report.Edit)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
	if bytes.Contains(output, []byte("[(08) 25 (\\05515\\0552024)] TJ")) {
		t.Fatal("old TJ array remains")
	}
	if !bytes.Contains(output, []byte("[(05\\05505\\0552026)] TJ")) {
		t.Fatalf("new TJ array missing:\n%s", output)
	}
}

func TestTJArrayWithUnsupportedElementFailsClosed(t *testing.T) {
	content := []byte("BT\n[(08) /Bad (\\05515\\0552024)] TJ\nET\n")
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "08-15-2024"}); len(matches) != 0 {
		t.Fatalf("matches = %d, want 0", len(matches))
	}
	_, _, _, err = ApplyCanonicalEdit(
		input,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-05-2026"},
		nil,
	)
	if err == nil {
		t.Fatal("expected canonical edit to fail closed")
	}
}
