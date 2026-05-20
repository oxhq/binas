package pdf

import (
	"bytes"
	"fmt"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestTextLayoutMetadataTracksFontPositionMatrixAndQuoteMovement(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n10 20 Td\n(First) Tj\n1 0 0 1 72 144 Tm\n(Second) Tj\n14 TL\nT*\n(Third) Tj\n(Quote) '\n3 4 (Double) \"\nET\n")
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}

	assertTextShowLayoutMeta(t, tree, "First", map[string]any{
		"font":      "F1",
		"font_size": 12.0,
		"text_x":    10.0,
		"text_y":    20.0,
	})
	assertTextShowLayoutMeta(t, tree, "Second", map[string]any{
		"text_x":      72.0,
		"text_y":      144.0,
		"text_matrix": [6]float64{1, 0, 0, 1, 72, 144},
	})
	assertTextShowLayoutMeta(t, tree, "Third", map[string]any{
		"text_x":       72.0,
		"text_y":       130.0,
		"text_leading": 14.0,
	})
	assertTextShowLayoutMeta(t, tree, "Quote", map[string]any{
		"operator": "'",
		"text_x":   72.0,
		"text_y":   116.0,
	})
	assertTextShowLayoutMeta(t, tree, "Double", map[string]any{
		"operator":     "\"",
		"text_x":       72.0,
		"text_y":       102.0,
		"word_spacing": 3.0,
		"char_spacing": 4.0,
	})
}

func TestCanonicalTextShowsCarryTextStateSnapshots(t *testing.T) {
	content := []byte("BT\n/F2 9 Tf\n5 6 Td\n(Canonical) Tj\n18 TL\n(Line) '\nET\n")

	shows := parseCanonicalTextShows(content, nil, nil)
	if len(shows) != 2 {
		t.Fatalf("shows = %d, want 2", len(shows))
	}

	first := shows[0].TextState
	if shows[0].Text != "Canonical" || first.FontName != "F2" || first.FontSize != 9 || first.X != 5 || first.Y != 6 {
		t.Fatalf("first show = %+v with state %+v, want F2 9 at 5 6", shows[0], first)
	}

	second := shows[1].TextState
	if shows[1].Text != "Line" || second.X != 5 || second.Y != -12 || second.Leading != 18 {
		t.Fatalf("second show = %+v with state %+v, want quote movement to 5 -12 with leading 18", shows[1], second)
	}
}

func TestQuoteTextShowSurgicalReplacementPreservesOperatorAndSelectableText(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n14 TL\n(LineOld) '\nET\n")
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}

	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "LineOld"}, core.Mutation{Replace: "LineNew"})
	if err != nil {
		t.Fatal(err)
	}
	output, _, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}

	if bytes.Contains(output, []byte("(LineOld) '")) {
		t.Fatal("old quote text show remains")
	}
	if !bytes.Contains(output, []byte("(LineNew) '")) {
		t.Fatalf("new quote text show missing:\n%s", output)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification failed: %+v", verification)
	}
}

func TestDoubleQuoteHexTextShowCanonicalReplacementPreservesSpacingOperator(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n14 TL\n3 4 <4865784F6C64> \"\nET\n")
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	output, report, verification, err := ApplyCanonicalEdit(
		input,
		core.Match{Kind: KindTextShow, Text: "HexOld"},
		core.Mutation{Replace: "HexNew"},
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
	if bytes.Contains(output, []byte("3 4 <4865784F6C64> \"")) {
		t.Fatal("old double-quote hex text show remains")
	}
	if !bytes.Contains(output, []byte("3 4 <4865784E6577> \"")) {
		t.Fatalf("new double-quote hex text show missing:\n%s", output)
	}
}

func TestDoubleQuoteTextShowWithMalformedSpacingFailsClosed(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n3 bad (Nope) \"\nET\n")
	input := testPDF(
		"<< /Type /Page >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "Nope"}); len(matches) != 0 {
		t.Fatalf("surgical matches = %d, want 0", len(matches))
	}
	if shows := parseCanonicalTextShows(content, nil, nil); len(shows) != 0 {
		t.Fatalf("canonical shows = %d, want 0", len(shows))
	}
}

func assertTextShowLayoutMeta(t *testing.T, tree *core.Tree, text string, want map[string]any) {
	t.Helper()
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: text})
	if len(matches) != 1 {
		t.Fatalf("%q matches = %d, want 1", text, len(matches))
	}
	for key, wantValue := range want {
		gotValue, ok := matches[0].Meta[key]
		if !ok {
			t.Fatalf("%q metadata missing %q: %+v", text, key, matches[0].Meta)
		}
		if gotValue != wantValue {
			t.Fatalf("%q metadata %s = %v, want %v", text, key, gotValue, wantValue)
		}
	}
}
