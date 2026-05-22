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

func TestApplyExplicitOCRTextLayerEmbedsSelectableInvisibleText(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Resources << >> >>",
	)

	output, report, verification, err := ApplyExplicitOCRTextLayer(input, OCRTextLayerOptions{
		PageIndex:  0,
		Text:       "External OCR text",
		Box:        OCRTextLayerBox{XMin: 10, YMin: 20, XMax: 110, YMax: 45},
		Confidence: 0.82,
	})
	if err != nil {
		t.Fatal(err)
	}
	if report.Edit != explicitOCRTextLayerEmbedOperation {
		t.Fatalf("report edit = %q, want %q", report.Edit, explicitOCRTextLayerEmbedOperation)
	}
	if report.NodesModified != 1 || !report.FallbackUsed || report.FallbackKind != string(FallbackOCRTextLayer) {
		t.Fatalf("report = %+v, want explicit OCR text-layer fallback write", report)
	}
	if report.FallbackPolicy == nil || report.FallbackPolicy.Fallback != string(FallbackOCRTextLayer) || report.FallbackPolicy.Mode != string(FallbackModeExplicit) {
		t.Fatalf("report fallback policy = %+v, want ocr_text_layer/explicit", report.FallbackPolicy)
	}
	if !verification.ReparseOK || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v, want selectable OCR text layer", verification)
	}
	if !strings.Contains(string(output), "3 Tr") {
		t.Fatalf("output missing invisible text rendering mode:\n%s", output)
	}
	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	candidates, err := graph.textShowCandidates("External OCR text")
	if err != nil {
		t.Fatal(err)
	}
	if len(candidates) != 1 {
		t.Fatalf("OCR text candidates = %d, want 1", len(candidates))
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

func TestPlanExplicitOCRTextLayerJSONPlansArrayItems(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
		"<< /Type /Page /Parent 2 0 R /Resources << >> >>",
		"<< /Type /Page /Parent 2 0 R /Resources << >> >>",
	)
	jsonInput := []byte(`[
		{"page_index":0,"text":"First page OCR","box":{"x_min":10,"y_min":20,"x_max":110,"y_max":45},"confidence":0.82},
		{"page_index":1,"text":"Second page OCR","box":{"x_min":5,"y_min":6,"x_max":50,"y_max":60},"confidence":1}
	]`)

	plans, err := PlanExplicitOCRTextLayerJSON(input, jsonInput)
	if err != nil {
		t.Fatal(err)
	}

	if len(plans) != 2 {
		t.Fatalf("plans len = %d, want 2", len(plans))
	}
	if plans[0].PageIndex != 0 || plans[0].Text != "First page OCR" || plans[1].PageIndex != 1 || plans[1].Text != "Second page OCR" {
		t.Fatalf("plans = %+v, want OCR plans in input order", plans)
	}
	for _, plan := range plans {
		if plan.Operation != explicitOCRTextLayerOperation {
			t.Fatalf("operation = %q, want %q", plan.Operation, explicitOCRTextLayerOperation)
		}
		if plan.Policy.Fallback != string(FallbackOCRTextLayer) || plan.Policy.Mode != string(FallbackModeExplicit) {
			t.Fatalf("plan policy = %+v, want ocr_text_layer/explicit", plan.Policy)
		}
		if plan.Report.NodesModified != 0 || !plan.Report.FallbackUsed || plan.Report.FallbackKind != string(FallbackOCRTextLayer) {
			t.Fatalf("report = %+v, want planning-only explicit OCR fallback metadata", plan.Report)
		}
	}
}

func TestPlanExplicitOCRTextLayerJSONPlansWrapperItems(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Resources << >> >>",
	)
	jsonInput := []byte(`{"items":[
		{"page_index":0,"text":"Wrapped OCR","box":{"x_min":1,"y_min":2,"x_max":3,"y_max":4},"confidence":0}
	]}`)

	plans, err := PlanExplicitOCRTextLayerJSON(input, jsonInput)
	if err != nil {
		t.Fatal(err)
	}

	if len(plans) != 1 {
		t.Fatalf("plans len = %d, want 1", len(plans))
	}
	if plans[0].Text != "Wrapped OCR" || plans[0].Policy.Fallback != string(FallbackOCRTextLayer) {
		t.Fatalf("plan = %+v, want wrapped explicit OCR text-layer plan", plans[0])
	}
}

func TestPlanExplicitOCRTextLayerJSONRejectsBadPayloads(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Resources << >> >>",
	)

	tests := []struct {
		name      string
		jsonInput string
	}{
		{name: "empty input", jsonInput: ""},
		{name: "malformed json", jsonInput: `{"items":[`},
		{name: "unknown top-level shape", jsonInput: `{"rows":[]}`},
		{name: "empty array", jsonInput: `[]`},
		{name: "empty wrapper items", jsonInput: `{"items":[]}`},
		{name: "missing confidence", jsonInput: `[{"page_index":0,"text":"OCR","box":{"x_min":1,"y_min":2,"x_max":3,"y_max":4}}]`},
		{name: "missing box coordinate", jsonInput: `[{"page_index":0,"text":"OCR","box":{"x_min":1,"y_min":2,"x_max":3},"confidence":1}]`},
		{name: "negative page index", jsonInput: `[{"page_index":-1,"text":"OCR","box":{"x_min":1,"y_min":2,"x_max":3,"y_max":4},"confidence":1}]`},
		{name: "out of range page index", jsonInput: `[{"page_index":1,"text":"OCR","box":{"x_min":1,"y_min":2,"x_max":3,"y_max":4},"confidence":1}]`},
		{name: "blank text", jsonInput: `[{"page_index":0,"text":" \t\n","box":{"x_min":1,"y_min":2,"x_max":3,"y_max":4},"confidence":1}]`},
		{name: "invalid box", jsonInput: `[{"page_index":0,"text":"OCR","box":{"x_min":1,"y_min":2,"x_max":1,"y_max":4},"confidence":1}]`},
		{name: "bad confidence", jsonInput: `[{"page_index":0,"text":"OCR","box":{"x_min":1,"y_min":2,"x_max":3,"y_max":4},"confidence":1.01}]`},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if _, err := PlanExplicitOCRTextLayerJSON(input, []byte(tt.jsonInput)); err == nil {
				t.Fatal("expected OCR JSON ingestion to fail closed")
			}
		})
	}
}

func TestParseOCRTextLayerALTOXMLMapsStringsByPageOrder(t *testing.T) {
	altoInput := []byte(`<?xml version="1.0" encoding="UTF-8"?>
<alto>
	<Layout>
		<Page ID="page-10">
			<PrintSpace>
				<TextBlock>
					<TextLine>
						<String CONTENT="First" HPOS="10" VPOS="20" WIDTH="30" HEIGHT="40" WC="0.82"/>
						<String CONTENT="Second" HPOS="1.5" VPOS="2.5" WIDTH="3.5" HEIGHT="4.5"/>
					</TextLine>
				</TextBlock>
			</PrintSpace>
		</Page>
		<Page ID="page-20">
			<PrintSpace>
				<String CONTENT="Third" HPOS="5" VPOS="6" WIDTH="7" HEIGHT="8" WC="1"/>
			</PrintSpace>
		</Page>
	</Layout>
</alto>`)

	opts, err := ParseOCRTextLayerALTOXML(altoInput)
	if err != nil {
		t.Fatal(err)
	}

	if len(opts) != 3 {
		t.Fatalf("opts len = %d, want 3", len(opts))
	}
	if opts[0] != (OCRTextLayerOptions{
		PageIndex:  0,
		Text:       "First",
		Box:        OCRTextLayerBox{XMin: 10, YMin: 20, XMax: 40, YMax: 60},
		Confidence: 0.82,
	}) {
		t.Fatalf("opts[0] = %+v", opts[0])
	}
	if opts[1] != (OCRTextLayerOptions{
		PageIndex:  0,
		Text:       "Second",
		Box:        OCRTextLayerBox{XMin: 1.5, YMin: 2.5, XMax: 5, YMax: 7},
		Confidence: 0,
	}) {
		t.Fatalf("opts[1] = %+v", opts[1])
	}
	if opts[2] != (OCRTextLayerOptions{
		PageIndex:  1,
		Text:       "Third",
		Box:        OCRTextLayerBox{XMin: 5, YMin: 6, XMax: 12, YMax: 14},
		Confidence: 1,
	}) {
		t.Fatalf("opts[2] = %+v", opts[2])
	}
}

func TestPlanExplicitOCRTextLayerALTOXMLReusesPlanningValidation(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Resources << >> >>",
	)
	altoInput := []byte(`<alto><Layout><Page><String CONTENT="External OCR" HPOS="10" VPOS="20" WIDTH="30" HEIGHT="40" WC="0.5"/></Page></Layout></alto>`)

	plans, err := PlanExplicitOCRTextLayerALTOXML(input, altoInput)
	if err != nil {
		t.Fatal(err)
	}

	if len(plans) != 1 {
		t.Fatalf("plans len = %d, want 1", len(plans))
	}
	if plans[0].Text != "External OCR" || plans[0].PageIndex != 0 || plans[0].Policy.Fallback != string(FallbackOCRTextLayer) {
		t.Fatalf("plan = %+v, want explicit OCR fallback plan", plans[0])
	}
	if plans[0].Report.NodesModified != 0 || !plans[0].Report.FallbackUsed || plans[0].Report.FallbackKind != string(FallbackOCRTextLayer) {
		t.Fatalf("report = %+v, want planning-only explicit OCR fallback metadata", plans[0].Report)
	}
}

func TestParseOCRTextLayerALTOXMLRejectsBadPayloads(t *testing.T) {
	tests := []struct {
		name      string
		altoInput string
	}{
		{name: "empty input", altoInput: ""},
		{name: "malformed xml", altoInput: `<alto><Layout>`},
		{name: "no OCR strings", altoInput: `<alto><Layout><Page/></Layout></alto>`},
		{name: "blank content", altoInput: `<alto><Layout><Page><String CONTENT="  " HPOS="1" VPOS="2" WIDTH="3" HEIGHT="4"/></Page></Layout></alto>`},
		{name: "missing geometry", altoInput: `<alto><Layout><Page><String CONTENT="OCR" HPOS="1" VPOS="2" WIDTH="3"/></Page></Layout></alto>`},
		{name: "non numeric geometry", altoInput: `<alto><Layout><Page><String CONTENT="OCR" HPOS="x" VPOS="2" WIDTH="3" HEIGHT="4"/></Page></Layout></alto>`},
		{name: "zero width", altoInput: `<alto><Layout><Page><String CONTENT="OCR" HPOS="1" VPOS="2" WIDTH="0" HEIGHT="4"/></Page></Layout></alto>`},
		{name: "negative height", altoInput: `<alto><Layout><Page><String CONTENT="OCR" HPOS="1" VPOS="2" WIDTH="3" HEIGHT="-4"/></Page></Layout></alto>`},
		{name: "non numeric confidence", altoInput: `<alto><Layout><Page><String CONTENT="OCR" HPOS="1" VPOS="2" WIDTH="3" HEIGHT="4" WC="high"/></Page></Layout></alto>`},
		{name: "negative confidence", altoInput: `<alto><Layout><Page><String CONTENT="OCR" HPOS="1" VPOS="2" WIDTH="3" HEIGHT="4" WC="-0.1"/></Page></Layout></alto>`},
		{name: "percentage confidence rejected", altoInput: `<alto><Layout><Page><String CONTENT="OCR" HPOS="1" VPOS="2" WIDTH="3" HEIGHT="4" WC="82"/></Page></Layout></alto>`},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if _, err := ParseOCRTextLayerALTOXML([]byte(tt.altoInput)); err == nil {
				t.Fatal("expected ALTO OCR ingestion to fail closed")
			}
		})
	}
}

func withOCROption(opts OCRTextLayerOptions, mutate func(*OCRTextLayerOptions)) OCRTextLayerOptions {
	mutate(&opts)
	return opts
}
