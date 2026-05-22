package pdf

import (
	"errors"
	"fmt"
	"math"
	"strings"

	"github.com/oxhq/binas/pkg/core"
)

const explicitOCRTextLayerOperation = "pdf.ocr_text_layer_explicit_plan"

type OCRTextLayerBox struct {
	XMin float64 `json:"x_min"`
	YMin float64 `json:"y_min"`
	XMax float64 `json:"x_max"`
	YMax float64 `json:"y_max"`
}

type OCRTextLayerOptions struct {
	PageIndex  int             `json:"page_index"`
	Text       string          `json:"text"`
	Box        OCRTextLayerBox `json:"box"`
	Confidence float64         `json:"confidence"`
}

type OCRTextLayerPlan struct {
	Operation  string              `json:"operation"`
	PageIndex  int                 `json:"page_index"`
	Text       string              `json:"text"`
	Box        OCRTextLayerBox     `json:"box"`
	Confidence float64             `json:"confidence"`
	Report     core.Report         `json:"report"`
	Policy     core.FallbackPolicy `json:"policy"`
}

func PlanExplicitOCRTextLayer(input []byte, opts OCRTextLayerOptions) (OCRTextLayerPlan, error) {
	if opts.PageIndex < 0 {
		return OCRTextLayerPlan{}, errors.New("ocr text-layer page index cannot be negative")
	}
	if strings.TrimSpace(opts.Text) == "" {
		return OCRTextLayerPlan{}, errors.New("ocr text-layer text cannot be empty")
	}
	if err := opts.Box.validate(); err != nil {
		return OCRTextLayerPlan{}, err
	}
	if math.IsNaN(opts.Confidence) || math.IsInf(opts.Confidence, 0) || opts.Confidence < 0 || opts.Confidence > 1 {
		return OCRTextLayerPlan{}, errors.New("ocr text-layer confidence must be between 0 and 1")
	}

	graph, err := parsePDFGraph(input)
	if err != nil {
		return OCRTextLayerPlan{}, err
	}
	pageCount := graph.pageCount()
	if pageCount == 0 {
		return OCRTextLayerPlan{}, errors.New("ocr text-layer requires at least one page")
	}
	if opts.PageIndex >= pageCount {
		return OCRTextLayerPlan{}, fmt.Errorf("ocr text-layer page index %d out of range for %d pages (zero-based)", opts.PageIndex, pageCount)
	}

	policy := OverlayPolicy{Fallback: FallbackOCRTextLayer, Mode: FallbackModeExplicit}
	report := WithFallbackPolicy(core.Report{
		Format:        "pdf",
		Edit:          explicitOCRTextLayerOperation,
		NodesModified: 0,
		MatchIndex:    &opts.PageIndex,
		Invariants: []core.Invariant{
			core.InvariantPageUnchanged,
		},
		Meta: map[string]any{
			"operation":    "ocr_text_layer",
			"planned_only": true,
			"page_index":   opts.PageIndex,
			"text":         opts.Text,
			"box":          opts.Box,
			"confidence":   opts.Confidence,
		},
	}, policy)

	return OCRTextLayerPlan{
		Operation:  explicitOCRTextLayerOperation,
		PageIndex:  opts.PageIndex,
		Text:       opts.Text,
		Box:        opts.Box,
		Confidence: opts.Confidence,
		Report:     report,
		Policy: core.FallbackPolicy{
			Fallback: string(policy.Fallback),
			Mode:     string(policy.Mode),
		},
	}, nil
}

func (b OCRTextLayerBox) validate() error {
	values := []float64{b.XMin, b.YMin, b.XMax, b.YMax}
	for _, value := range values {
		if math.IsNaN(value) || math.IsInf(value, 0) {
			return errors.New("ocr text-layer bounding box values must be finite")
		}
	}
	if b.XMax <= b.XMin || b.YMax <= b.YMin {
		return errors.New("ocr text-layer bounding box must have positive width and height")
	}
	return nil
}
