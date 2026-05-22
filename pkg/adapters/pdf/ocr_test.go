package pdf

import (
	"encoding/json"
	"errors"
	"math"
	"strings"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestOCRTextLayerExplicitInputReportsFallbackPolicy(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Resources << >> >>",
	)

	plan, err := PlanExplicitOCRTextLayer(input, OCRTextLayerOptions{
		PageIndex:  0,
		Text:       "External OCR text",
		Box:        OCRTextLayerBox{XMin: 10, YMin: 20, XMax: 110, YMax: 45},
		Confidence: 0.82,
	})
	if err != nil {
		t.Fatal(err)
	}

	if plan.Operation != explicitOCRTextLayerOperation {
		t.Fatalf("operation = %q, want %q", plan.Operation, explicitOCRTextLayerOperation)
	}
	if plan.Policy.Fallback != string(FallbackOCRTextLayer) || plan.Policy.Mode != string(FallbackModeExplicit) {
		t.Fatalf("plan policy = %+v, want ocr_text_layer/explicit", plan.Policy)
	}
	if plan.Report.Edit != explicitOCRTextLayerOperation {
		t.Fatalf("report edit = %q, want %q", plan.Report.Edit, explicitOCRTextLayerOperation)
	}
	if !plan.Report.FallbackUsed {
		t.Fatal("fallback_used = false, want true")
	}
	if plan.Report.FallbackKind != string(FallbackOCRTextLayer) {
		t.Fatalf("fallback_kind = %q, want %q", plan.Report.FallbackKind, FallbackOCRTextLayer)
	}
	if plan.Report.FallbackPolicy == nil || plan.Report.FallbackPolicy.Fallback != string(FallbackOCRTextLayer) || plan.Report.FallbackPolicy.Mode != string(FallbackModeExplicit) {
		t.Fatalf("report fallback policy = %+v, want ocr_text_layer/explicit", plan.Report.FallbackPolicy)
	}
	if hasCoreInvariant(plan.Report.Invariants, core.InvariantNoFallbackUsed) {
		t.Fatalf("ocr text-layer fallback must not claim %s: %+v", core.InvariantNoFallbackUsed, plan.Report.Invariants)
	}
	if plan.Report.NodesModified != 0 {
		t.Fatalf("nodes modified = %d, want 0 for planning-only OCR text-layer metadata", plan.Report.NodesModified)
	}

	encoded, err := json.Marshal(plan.Report)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(encoded), `"fallback_used":true`) ||
		!strings.Contains(string(encoded), `"fallback_kind":"ocr_text_layer"`) ||
		!strings.Contains(string(encoded), `"fallback_policy":{"fallback":"ocr_text_layer","mode":"explicit"}`) {
		t.Fatalf("report JSON missing explicit OCR fallback contract: %s", encoded)
	}
}

func TestOCRTextLayerExplicitInputFailsClosedForInvalidInputs(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Resources << >> >>",
	)
	valid := OCRTextLayerOptions{
		PageIndex:  0,
		Text:       "OCR",
		Box:        OCRTextLayerBox{XMin: 1, YMin: 2, XMax: 3, YMax: 4},
		Confidence: 1,
	}

	tests := []struct {
		name string
		opts OCRTextLayerOptions
	}{
		{name: "negative page index", opts: withOCROption(valid, func(o *OCRTextLayerOptions) { o.PageIndex = -1 })},
		{name: "out of range page index", opts: withOCROption(valid, func(o *OCRTextLayerOptions) { o.PageIndex = 1 })},
		{name: "empty text", opts: withOCROption(valid, func(o *OCRTextLayerOptions) { o.Text = "" })},
		{name: "blank text", opts: withOCROption(valid, func(o *OCRTextLayerOptions) { o.Text = " \t\n" })},
		{name: "zero width box", opts: withOCROption(valid, func(o *OCRTextLayerOptions) { o.Box.XMax = o.Box.XMin })},
		{name: "zero height box", opts: withOCROption(valid, func(o *OCRTextLayerOptions) { o.Box.YMax = o.Box.YMin })},
		{name: "nan box", opts: withOCROption(valid, func(o *OCRTextLayerOptions) { o.Box.XMin = math.NaN() })},
		{name: "negative confidence", opts: withOCROption(valid, func(o *OCRTextLayerOptions) { o.Confidence = -0.01 })},
		{name: "confidence over one", opts: withOCROption(valid, func(o *OCRTextLayerOptions) { o.Confidence = 1.01 })},
		{name: "nan confidence", opts: withOCROption(valid, func(o *OCRTextLayerOptions) { o.Confidence = math.NaN() })},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if _, err := PlanExplicitOCRTextLayer(input, tt.opts); err == nil {
				t.Fatal("expected invalid OCR text-layer input to fail")
			}
		})
	}
}

func TestOCRTextLayerFallbackRemainsSeparateFromTrueEdit(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Resources << >> >>",
	)

	plan, err := PlanExplicitOCRTextLayer(input, OCRTextLayerOptions{
		PageIndex:  0,
		Text:       "External OCR text",
		Box:        OCRTextLayerBox{XMin: 10, YMin: 20, XMax: 110, YMax: 45},
		Confidence: 0,
	})
	if err != nil {
		t.Fatal(err)
	}

	if err := ValidateTrueTextEditReportFallbackPolicy(plan.Report); !errors.Is(err, ErrTrueTextEditRejectsFallbackPolicy) {
		t.Fatalf("ValidateTrueTextEditReportFallbackPolicy() error = %v, want ErrTrueTextEditRejectsFallbackPolicy", err)
	}
	if hasCoreInvariant(plan.Report.Invariants, core.InvariantNoFallbackUsed) {
		t.Fatalf("ocr text-layer fallback must remain separate from true edit invariant %s", core.InvariantNoFallbackUsed)
	}
}

func withOCROption(opts OCRTextLayerOptions, mutate func(*OCRTextLayerOptions)) OCRTextLayerOptions {
	mutate(&opts)
	return opts
}
