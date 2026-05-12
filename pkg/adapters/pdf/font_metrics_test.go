package pdf

import (
	"fmt"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestFontWidthsAttachDirectWidthMetadataToTextShow(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n(ABZ) Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 65 /Widths [600 610] /MissingWidth 333 >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "ABZ"})
	if len(matches) != 1 {
		t.Fatalf("ABZ matches = %d, want 1", len(matches))
	}
	meta := matches[0].Meta
	if meta["font"] != "F1" {
		t.Fatalf("font = %v, want F1", meta["font"])
	}
	if meta["width_units"] != 1543 {
		t.Fatalf("width_units = %v, want 1543", meta["width_units"])
	}
	if meta["width_source"] != "/Widths+/MissingWidth" {
		t.Fatalf("width_source = %v, want /Widths+/MissingWidth", meta["width_source"])
	}
	if meta["missing_width_used"] != true {
		t.Fatalf("missing_width_used = %v, want true", meta["missing_width_used"])
	}
}

func TestFontWidthsOmitWidthMetadataWhenGlyphWidthIsUnknown(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n(ABZ) Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 65 /Widths [600 610] >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "ABZ"})
	if len(matches) != 1 {
		t.Fatalf("ABZ matches = %d, want 1", len(matches))
	}
	meta := matches[0].Meta
	if meta["font"] != "F1" {
		t.Fatalf("font = %v, want F1", meta["font"])
	}
	if _, ok := meta["width_units"]; ok {
		t.Fatalf("width_units present despite unknown glyph width: %+v", meta)
	}
	if _, ok := meta["width_source"]; ok {
		t.Fatalf("width_source present despite unknown glyph width: %+v", meta)
	}
	if _, ok := meta["missing_width_used"]; ok {
		t.Fatalf("missing_width_used present despite unknown glyph width: %+v", meta)
	}
}
