package pdf

import (
	"fmt"
	"strings"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestCIDFontWidthsAttachMetadataForType0CMapHexText(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<000100020003> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /DescendantFonts [6 0 R] /ToUnicode 5 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		testThreeCIDToUnicodeCMapStream(),
		"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /CIDFixture /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /W [1 [500 610] 3 4 700] >>",
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "ABC"})
	if len(matches) != 1 {
		t.Fatalf("ABC matches = %d, want 1", len(matches))
	}
	meta := matches[0].Meta
	if meta["encoding"] != "hex-cmap" {
		t.Fatalf("encoding = %v, want hex-cmap", meta["encoding"])
	}
	if meta["font"] != "F1" {
		t.Fatalf("font = %v, want F1", meta["font"])
	}
	if meta["width_units"] != 1810 {
		t.Fatalf("width_units = %v, want 1810", meta["width_units"])
	}
	if meta["width_source"] != "/DescendantFonts/W" {
		t.Fatalf("width_source = %v, want /DescendantFonts/W", meta["width_source"])
	}
	if meta["cid_encoding"] != "Identity-H" {
		t.Fatalf("cid_encoding = %v, want Identity-H", meta["cid_encoding"])
	}
	if meta["cid_system_registry"] != "Adobe" || meta["cid_system_ordering"] != "Identity" || meta["cid_system_supplement"] != 0 {
		t.Fatalf("cid system metadata = registry:%v ordering:%v supplement:%v", meta["cid_system_registry"], meta["cid_system_ordering"], meta["cid_system_supplement"])
	}
}

func TestCIDFontWidthsOmitMetadataWhenAnyCIDWidthIsUnknown(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<00010002> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /DescendantFonts [6 0 R] /ToUnicode 5 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		testTwoCIDToUnicodeCMapStream(),
		"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /CIDFixture /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /W [1 [500]] >>",
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "AB"})
	if len(matches) != 1 {
		t.Fatalf("AB matches = %d, want 1", len(matches))
	}
	meta := matches[0].Meta
	if _, ok := meta["width_units"]; ok {
		t.Fatalf("width_units present despite unknown CID width: %+v", meta)
	}
	if _, ok := meta["width_source"]; ok {
		t.Fatalf("width_source present despite unknown CID width: %+v", meta)
	}
	if _, ok := meta["layout_proof"]; ok {
		t.Fatalf("layout_proof present despite unknown CID width: %+v", meta)
	}
}

func TestCIDFontWidthsUseDWForCIDsAbsentFromW(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<000100020003> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /DescendantFonts [6 0 R] /ToUnicode 5 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		testThreeCIDToUnicodeCMapStream(),
		"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /CIDFixture /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 880 /W [1 [500] 3 [700]] >>",
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "ABC"})
	if len(matches) != 1 {
		t.Fatalf("ABC matches = %d, want 1", len(matches))
	}
	meta := matches[0].Meta
	if meta["width_units"] != 2080 {
		t.Fatalf("width_units = %v, want 2080", meta["width_units"])
	}
	if meta["width_source"] != "/DescendantFonts/W+/DW" {
		t.Fatalf("width_source = %v, want /DescendantFonts/W+/DW", meta["width_source"])
	}
	if meta["cid_default_width_used"] != true {
		t.Fatalf("cid_default_width_used = %v, want true", meta["cid_default_width_used"])
	}
	if meta["cid_default_width"] != 880 {
		t.Fatalf("cid_default_width = %v, want 880", meta["cid_default_width"])
	}
}

func TestCIDFontWidthsUseDirectDWWhenWIsAbsent(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n<00010002> Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /DescendantFonts [6 0 R] /ToUnicode 5 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		testTwoCIDToUnicodeCMapStream(),
		"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /CIDFixture /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 900 >>",
	)

	tree, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	matches := tree.Query(core.Match{Kind: KindTextShow, Text: "AB"})
	if len(matches) != 1 {
		t.Fatalf("AB matches = %d, want 1", len(matches))
	}
	meta := matches[0].Meta
	if meta["width_units"] != 1800 {
		t.Fatalf("width_units = %v, want 1800", meta["width_units"])
	}
	if meta["width_source"] != "/DescendantFonts/DW" {
		t.Fatalf("width_source = %v, want /DescendantFonts/DW", meta["width_source"])
	}
	if meta["cid_default_width_used"] != true {
		t.Fatalf("cid_default_width_used = %v, want true", meta["cid_default_width_used"])
	}
	if meta["cid_default_width"] != 900 {
		t.Fatalf("cid_default_width = %v, want 900", meta["cid_default_width"])
	}
}

func TestPlanEditReportsReplacementWidthProofForType0CMap(t *testing.T) {
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
		t.Fatalf("plan layout_proof = %v, want %q; meta=%+v", plan.Meta["layout_proof"], layoutProofStatusWidthProven, plan.Meta)
	}
	if plan.Meta["old_width_units"] != 1110 || plan.Meta["new_width_units"] != 1110 || plan.Meta["width_delta_units"] != 0 {
		t.Fatalf("plan width proof meta = %+v, want equal 1110 widths", plan.Meta)
	}
	_, report, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if report.Meta["layout_proof"] != layoutProofStatusWidthProven {
		t.Fatalf("report layout_proof = %v, want %q; meta=%+v", report.Meta["layout_proof"], layoutProofStatusWidthProven, report.Meta)
	}
}

func TestPlanEditFailsClosedWhenType0CMapTJArrayReplacementRequiresReflow(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n[<0001> -20 <0001>] TJ\nET\n")
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
	_, err = adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "AA"}, core.Mutation{Replace: "BB"})
	if err == nil {
		t.Fatal("expected replacement requiring reflow to fail closed")
	}
	got := err.Error()
	for _, want := range []string{"layout_proof=reflow_required", "old_width_units=1000", "new_width_units=1220", "width_delta_units=220"} {
		if !strings.Contains(got, want) {
			t.Fatalf("error = %q, want metadata %q", got, want)
		}
	}
}

func testTwoCIDToUnicodeCMapStream() string {
	cmap := []byte(`/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
2 beginbfchar
<0001> <0041>
<0002> <0042>
endbfchar
endcmap
CMapName currentdict /CMap defineresource pop
end
end`)
	return fmt.Sprintf("<< /Length %d >>\nstream\n%s\nendstream", len(cmap)+1, cmap)
}

func testThreeCIDToUnicodeCMapStream() string {
	cmap := []byte(`/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
3 beginbfchar
<0001> <0041>
<0002> <0042>
<0003> <0043>
endbfchar
endcmap
CMapName currentdict /CMap defineresource pop
end
end`)
	return fmt.Sprintf("<< /Length %d >>\nstream\n%s\nendstream", len(cmap)+1, cmap)
}
