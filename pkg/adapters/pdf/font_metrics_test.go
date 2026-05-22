package pdf

import (
	"fmt"
	"strings"
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
	if meta["layout_proof"] != layoutProofStatusUnknown {
		t.Fatalf("layout_proof = %v, want %q before replacement planning", meta["layout_proof"], layoutProofStatusUnknown)
	}
	if _, ok := meta["width_delta_units"]; ok {
		t.Fatalf("width_delta_units present before replacement planning: %+v", meta)
	}
	if meta["width_source"] != "/Widths+/MissingWidth" {
		t.Fatalf("width_source = %v, want /Widths+/MissingWidth", meta["width_source"])
	}
	if meta["width_proof"] != textWidthProofStatusKnown || meta["font_metrics_source"] != "simple_font_widths" || meta["text_editability_status"] != textEditabilityStatusReplaceableCandidate {
		t.Fatalf("editability width metadata = width_proof:%v source:%v status:%v", meta["width_proof"], meta["font_metrics_source"], meta["text_editability_status"])
	}
	if meta["missing_width_used"] != true {
		t.Fatalf("missing_width_used = %v, want true", meta["missing_width_used"])
	}
	if meta["font_first_char"] != 65 {
		t.Fatalf("font_first_char = %v, want 65", meta["font_first_char"])
	}
	if widths, ok := meta["font_widths"].([]int); !ok || len(widths) != 2 || widths[0] != 600 || widths[1] != 610 {
		t.Fatalf("font_widths = %#v, want [600 610]", meta["font_widths"])
	}
	if meta["font_missing_width"] != 333 {
		t.Fatalf("font_missing_width = %v, want 333", meta["font_missing_width"])
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
	if _, ok := meta["layout_proof"]; ok {
		t.Fatalf("layout_proof present despite unknown glyph width: %+v", meta)
	}
	if _, ok := meta["missing_width_used"]; ok {
		t.Fatalf("missing_width_used present despite unknown glyph width: %+v", meta)
	}
}

func TestPlanEditReportsReplacementWidthProofForSimpleFont(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n(AB) Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 65 /Widths [600 610] >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
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
	if plan.Meta["old_width_units"] != 1210 || plan.Meta["new_width_units"] != 1210 || plan.Meta["width_delta_units"] != 0 {
		t.Fatalf("plan width proof meta = %+v, want equal 1210 widths", plan.Meta)
	}
	if plan.Meta["text_editability_status"] != textEditabilityStatusReplaceable || plan.Meta["width_proof"] != textWidthProofStatusEqual || plan.Meta["font_metrics_source"] != "simple_font_widths" {
		t.Fatalf("plan editability metadata = %+v", plan.Meta)
	}
	_, report, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if report.Meta["layout_proof"] != layoutProofStatusWidthProven {
		t.Fatalf("report layout_proof = %v, want %q; meta=%+v", report.Meta["layout_proof"], layoutProofStatusWidthProven, report.Meta)
	}
}

func TestPlanEditFailsClosedWhenSimpleFontReplacementRequiresReflow(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n(AA) Tj\nET\n")
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
	_, err = NewAdapter().PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "AA"}, core.Mutation{Replace: "BB"})
	if err == nil {
		t.Fatal("expected replacement requiring reflow to fail closed")
	}
	unsupported := requireTextReplacementUnsupportedError(t, err, textReplacementUnsupportedReflowRequired)
	if unsupported.Metadata["width_proof"] != textWidthProofStatusReflowRequired || unsupported.Metadata["font_metrics_source"] != "simple_font_widths" || unsupported.Metadata["reflow_required"] != true {
		t.Fatalf("unsupported metadata = %+v, want reflow width proof and font metrics source", unsupported.Metadata)
	}
	got := err.Error()
	for _, want := range []string{"layout_proof=reflow_required", "old_width_units=1200", "new_width_units=1220", "width_delta_units=20"} {
		if !strings.Contains(got, want) {
			t.Fatalf("error = %q, want metadata %q", got, want)
		}
	}
}

func TestPlanEditAllowsProvenNarrowerSimpleFontReplacement(t *testing.T) {
	content := []byte("BT\n/F1 12 Tf\n(BB) Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog >>",
		"<< /Type /Page /Contents 4 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
		"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 65 /Widths [600 610] >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	adapter := NewAdapter()
	tree, err := adapter.Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	plan, err := adapter.PlanEdit(tree, core.Match{Kind: KindTextShow, Text: "BB"}, core.Mutation{Replace: "AA"})
	if err != nil {
		t.Fatal(err)
	}
	if plan.Meta["layout_proof"] != layoutProofStatusWidthNarrower {
		t.Fatalf("plan layout_proof = %v, want %q; meta=%+v", plan.Meta["layout_proof"], layoutProofStatusWidthNarrower, plan.Meta)
	}
	if plan.Meta["old_width_units"] != 1220 || plan.Meta["new_width_units"] != 1200 || plan.Meta["width_delta_units"] != -20 {
		t.Fatalf("plan width proof meta = %+v, want narrower replacement", plan.Meta)
	}
	output, report, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	if report.Meta["layout_proof"] != layoutProofStatusWidthNarrower {
		t.Fatalf("report layout_proof = %v, want %q; meta=%+v", report.Meta["layout_proof"], layoutProofStatusWidthNarrower, report.Meta)
	}
	verification, err := adapter.Verify(output, plan)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable {
		t.Fatalf("verification = %+v, want narrower selectable replacement", verification)
	}
}
