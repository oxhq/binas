package pdf

import (
	"fmt"
	"strings"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestLayoutMetadataAnnotatesKnownEqualWidthAsProven(t *testing.T) {
	meta := map[string]any{}
	oldWidth := 1200
	newWidth := 1200

	annotateTextShowLayoutProofMetadata(meta, &oldWidth, &newWidth)

	if meta["layout_proof"] != layoutProofStatusWidthProven {
		t.Fatalf("layout_proof = %v, want %q", meta["layout_proof"], layoutProofStatusWidthProven)
	}
	if meta["width_delta_units"] != 0 {
		t.Fatalf("width_delta_units = %v, want 0", meta["width_delta_units"])
	}
}

func TestLayoutMetadataAnnotatesKnownChangedWidthAsReflowRequired(t *testing.T) {
	meta := map[string]any{}
	oldWidth := 1200
	newWidth := 1333

	annotateTextShowLayoutProofMetadata(meta, &oldWidth, &newWidth)

	if meta["layout_proof"] != layoutProofStatusReflowRequired {
		t.Fatalf("layout_proof = %v, want %q", meta["layout_proof"], layoutProofStatusReflowRequired)
	}
	if meta["width_delta_units"] != 133 {
		t.Fatalf("width_delta_units = %v, want 133", meta["width_delta_units"])
	}
}

func TestLayoutMetadataAnnotatesKnownNarrowerWidthAsSupported(t *testing.T) {
	meta := map[string]any{}
	oldWidth := 1333
	newWidth := 1200

	annotateTextShowLayoutProofMetadata(meta, &oldWidth, &newWidth)

	if meta["layout_proof"] != layoutProofStatusWidthNarrower {
		t.Fatalf("layout_proof = %v, want %q", meta["layout_proof"], layoutProofStatusWidthNarrower)
	}
	if meta["width_delta_units"] != -133 {
		t.Fatalf("width_delta_units = %v, want -133", meta["width_delta_units"])
	}
	if err := rejectUnsupportedTextReplacementLayout(meta, textReplacementEncodingProof{}); err != nil {
		t.Fatalf("narrower proven replacement failed layout guard: %v", err)
	}
}

func TestLayoutMetadataAnnotatesUnknownWidthWithoutDelta(t *testing.T) {
	meta := map[string]any{}
	oldWidth := 1200

	annotateTextShowLayoutProofMetadata(meta, &oldWidth, nil)

	if meta["layout_proof"] != layoutProofStatusUnknown {
		t.Fatalf("layout_proof = %v, want %q", meta["layout_proof"], layoutProofStatusUnknown)
	}
	if _, ok := meta["width_delta_units"]; ok {
		t.Fatalf("width_delta_units present for unknown width metadata: %+v", meta)
	}
}

func TestLayoutMetadataReportIncludesDecodeFontEncodingAndWidthProof(t *testing.T) {
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
	assertLayoutReportMetadata(t, plan.Meta, map[string]any{
		"encoding":           "literal",
		"encoding_path":      "text_show/literal",
		"text_decode_source": "pdf_literal_string",
		"font_id":            "F1",
		"layout_proof":       layoutProofStatusWidthProven,
		"old_width_units":    1210,
		"new_width_units":    1210,
		"width_delta_units":  0,
	})

	_, report, err := adapter.Apply(input, plan)
	if err != nil {
		t.Fatal(err)
	}
	assertLayoutReportMetadata(t, report.Meta, map[string]any{
		"encoding":           "literal",
		"encoding_path":      "text_show/literal",
		"text_decode_source": "pdf_literal_string",
		"font_id":            "F1",
		"layout_proof":       layoutProofStatusWidthProven,
		"old_width_units":    1210,
		"new_width_units":    1210,
		"width_delta_units":  0,
	})
}

func TestLayoutMetadataFailClosedErrorIncludesDecodeFontEncodingAndWidthProof(t *testing.T) {
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
	got := err.Error()
	for _, want := range []string{
		"layout_proof=reflow_required",
		"old_width_units=1200",
		"new_width_units=1220",
		"width_delta_units=20",
		"text_decode_source=pdf_literal_string",
		"font_id=F1",
		"encoding_path=text_show/literal",
	} {
		if !strings.Contains(got, want) {
			t.Fatalf("error = %q, want metadata %q", got, want)
		}
	}
}

func assertLayoutReportMetadata(t *testing.T, got map[string]any, want map[string]any) {
	t.Helper()
	for key, wantValue := range want {
		if got[key] != wantValue {
			t.Fatalf("meta[%q] = %v, want %v; meta=%+v", key, got[key], wantValue, got)
		}
	}
}
