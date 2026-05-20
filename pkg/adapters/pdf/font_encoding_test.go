package pdf

import (
	"bytes"
	"fmt"
	"strings"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestFontEncodingWinAnsiDecodesAndEditsHighByteHexText(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<8041> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type1 /Encoding /WinAnsiEncoding >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "€A"})
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].Meta["encoding"] != "hex-font-encoding" {
		t.Fatalf("encoding meta = %v, want hex-font-encoding", matches[0].Meta["encoding"])
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "€A"}, core.Mutation{Replace: "A€"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(output, []byte("<4180> Tj")) {
		t.Fatalf("expected replacement to preserve WinAnsi bytes:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}

	_, err = adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "€A"}, core.Mutation{Replace: "A☃"})
	if err == nil {
		t.Fatal("expected unrepresentable WinAnsi replacement to fail closed")
	}
	if !strings.Contains(err.Error(), "not representable by the font encoding") {
		t.Fatalf("error = %q, want font encoding representability refusal", err)
	}
}

func TestFontEncodingWinAnsiDecodesLiteralText(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n(\\200A) Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type1 /Encoding /WinAnsiEncoding >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "€A"})
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].Meta["encoding"] != "literal-font-encoding" {
		t.Fatalf("encoding meta = %v, want literal-font-encoding", matches[0].Meta["encoding"])
	}

	_, err = adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: fmt.Sprint(matches[0].Value)}, core.Mutation{Replace: "A☃"})
	if err == nil {
		t.Fatal("expected unrepresentable WinAnsi literal replacement to fail closed")
	}
	if !strings.Contains(err.Error(), "not representable by the font encoding") {
		t.Fatalf("error = %q, want font encoding representability refusal", err)
	}
}

func TestFontEncodingDifferencesOverrideDecodesAndEditsHexText(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<4142> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type1 /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /Euro /Aacute] >> >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "€Á"})
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].Meta["encoding"] != "hex-font-encoding" {
		t.Fatalf("encoding meta = %v, want hex-font-encoding", matches[0].Meta["encoding"])
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "€Á"}, core.Mutation{Replace: "Á€"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(output, []byte("<4241> Tj")) {
		t.Fatalf("expected replacement to preserve Differences bytes:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
}
