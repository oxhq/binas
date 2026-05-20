package pdf

import (
	"fmt"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestFormXObjectUsesOwnFontResourcesForCMapTextShow(t *testing.T) {
	pageContent := []byte("q\n/Fm1 Do\nQ\n")
	formContent := []byte("BT\n/F1 12 Tf\n<01> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Contents 6 0 R /Resources << /Font << /F1 4 0 R >> /XObject << /Fm1 7 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /ToUnicode 5 0 R >>",
		testTwoCharToUnicodeCMapStream("0050", "0051"),
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(pageContent), pageContent),
		fmt.Sprintf("<< /Type /XObject /Subtype /Form /Resources << /Font << /F1 8 0 R >> >> /Length %d >>\nstream\n%sendstream", len(formContent), formContent),
		"<< /Type /Font /Subtype /Type0 /ToUnicode 9 0 R >>",
		testTwoCharToUnicodeCMapStream("0046", "0047"),
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if matches := tree.Query(core.Match{Kind: KindTextShow, Text: "P"}); len(matches) != 0 {
		t.Fatalf("page font CMap matches = %d, want 0", len(matches))
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "F"})
	if len(matches) != 1 {
		t.Fatalf("form font CMap matches = %d, want 1", len(matches))
	}
	if matches[0].Meta["encoding"] != "hex-cmap" {
		t.Fatalf("encoding = %v, want hex-cmap", matches[0].Meta["encoding"])
	}
	if matches[0].Meta["font"] != "F1" {
		t.Fatalf("font = %v, want F1", matches[0].Meta["font"])
	}
}
