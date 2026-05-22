package pdf

import (
	"bytes"
	"encoding/json"
	"encoding/xml"
	"errors"
	"fmt"
	"io"
	"math"
	"strconv"
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

type ocrTextLayerJSONItem struct {
	PageIndex  *int                 `json:"page_index"`
	Text       *string              `json:"text"`
	Box        *ocrTextLayerJSONBox `json:"box"`
	Confidence *float64             `json:"confidence"`
}

type ocrTextLayerJSONBox struct {
	XMin *float64 `json:"x_min"`
	YMin *float64 `json:"y_min"`
	XMax *float64 `json:"x_max"`
	YMax *float64 `json:"y_max"`
}

func ParseOCRTextLayerALTOXML(input []byte) ([]OCRTextLayerOptions, error) {
	trimmed := bytes.TrimSpace(input)
	if len(trimmed) == 0 {
		return nil, errors.New("ocr text-layer ALTO XML input cannot be empty")
	}

	decoder := xml.NewDecoder(bytes.NewReader(trimmed))
	pageIndex := -1
	pageDepth := 0
	opts := make([]OCRTextLayerOptions, 0)
	for {
		token, err := decoder.Token()
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return nil, fmt.Errorf("invalid ocr text-layer ALTO XML: %w", err)
		}

		switch token := token.(type) {
		case xml.StartElement:
			if pageDepth > 0 {
				pageDepth++
			}
			if token.Name.Local == "Page" {
				pageIndex++
				pageDepth = 1
				continue
			}
			if token.Name.Local != "String" || pageDepth == 0 {
				continue
			}
			opt, err := altoStringToOCRTextLayerOptions(token, pageIndex)
			if err != nil {
				return nil, fmt.Errorf("ocr text-layer ALTO page %d string %d: %w", pageIndex, len(opts), err)
			}
			opts = append(opts, opt)
		case xml.EndElement:
			if pageDepth > 0 {
				pageDepth--
			}
		}
	}

	if len(opts) == 0 {
		return nil, errors.New("ocr text-layer ALTO XML must contain at least one OCR string")
	}
	return opts, nil
}

func PlanExplicitOCRTextLayerALTOXML(pdfBytes, altoBytes []byte) ([]OCRTextLayerPlan, error) {
	opts, err := ParseOCRTextLayerALTOXML(altoBytes)
	if err != nil {
		return nil, err
	}

	plans := make([]OCRTextLayerPlan, 0, len(opts))
	for i, opt := range opts {
		plan, err := PlanExplicitOCRTextLayer(pdfBytes, opt)
		if err != nil {
			return nil, fmt.Errorf("ocr text-layer ALTO item %d: %w", i, err)
		}
		plans = append(plans, plan)
	}
	return plans, nil
}

func ParseOCRTextLayerJSON(input []byte) ([]OCRTextLayerOptions, error) {
	trimmed := bytes.TrimSpace(input)
	if len(trimmed) == 0 {
		return nil, errors.New("ocr text-layer JSON input cannot be empty")
	}

	var items []ocrTextLayerJSONItem
	switch trimmed[0] {
	case '[':
		if err := decodeOCRTextLayerJSON(trimmed, &items); err != nil {
			return nil, err
		}
	case '{':
		var wrapper struct {
			Items []ocrTextLayerJSONItem `json:"items"`
		}
		if err := decodeOCRTextLayerJSON(trimmed, &wrapper); err != nil {
			return nil, err
		}
		items = wrapper.Items
	default:
		return nil, errors.New("ocr text-layer JSON must be an array or object with items")
	}

	if len(items) == 0 {
		return nil, errors.New("ocr text-layer JSON must contain at least one item")
	}

	opts := make([]OCRTextLayerOptions, 0, len(items))
	for i, item := range items {
		opt, err := item.toOptions()
		if err != nil {
			return nil, fmt.Errorf("ocr text-layer item %d: %w", i, err)
		}
		opts = append(opts, opt)
	}
	return opts, nil
}

func PlanExplicitOCRTextLayerJSON(pdfBytes, jsonBytes []byte) ([]OCRTextLayerPlan, error) {
	opts, err := ParseOCRTextLayerJSON(jsonBytes)
	if err != nil {
		return nil, err
	}

	plans := make([]OCRTextLayerPlan, 0, len(opts))
	for i, opt := range opts {
		plan, err := PlanExplicitOCRTextLayer(pdfBytes, opt)
		if err != nil {
			return nil, fmt.Errorf("ocr text-layer item %d: %w", i, err)
		}
		plans = append(plans, plan)
	}
	return plans, nil
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

func altoStringToOCRTextLayerOptions(element xml.StartElement, pageIndex int) (OCRTextLayerOptions, error) {
	attrs := make(map[string]string, len(element.Attr))
	for _, attr := range element.Attr {
		attrs[attr.Name.Local] = attr.Value
	}

	text, ok := attrs["CONTENT"]
	if !ok {
		return OCRTextLayerOptions{}, errors.New("CONTENT is required")
	}
	if strings.TrimSpace(text) == "" {
		return OCRTextLayerOptions{}, errors.New("CONTENT cannot be blank")
	}

	hpos, err := parseRequiredALTONumber(attrs, "HPOS")
	if err != nil {
		return OCRTextLayerOptions{}, err
	}
	vpos, err := parseRequiredALTONumber(attrs, "VPOS")
	if err != nil {
		return OCRTextLayerOptions{}, err
	}
	width, err := parseRequiredALTONumber(attrs, "WIDTH")
	if err != nil {
		return OCRTextLayerOptions{}, err
	}
	height, err := parseRequiredALTONumber(attrs, "HEIGHT")
	if err != nil {
		return OCRTextLayerOptions{}, err
	}

	confidence := 0.0
	if wc, ok := attrs["WC"]; ok {
		confidence, err = parseALTONumber("WC", wc)
		if err != nil {
			return OCRTextLayerOptions{}, err
		}
		if math.IsNaN(confidence) || math.IsInf(confidence, 0) || confidence < 0 || confidence > 1 {
			return OCRTextLayerOptions{}, errors.New("WC confidence must be between 0 and 1")
		}
	}

	box := OCRTextLayerBox{
		XMin: hpos,
		YMin: vpos,
		XMax: hpos + width,
		YMax: vpos + height,
	}
	if err := box.validate(); err != nil {
		return OCRTextLayerOptions{}, err
	}

	return OCRTextLayerOptions{
		PageIndex:  pageIndex,
		Text:       text,
		Box:        box,
		Confidence: confidence,
	}, nil
}

func parseRequiredALTONumber(attrs map[string]string, name string) (float64, error) {
	value, ok := attrs[name]
	if !ok {
		return 0, fmt.Errorf("%s is required", name)
	}
	return parseALTONumber(name, value)
}

func parseALTONumber(name, value string) (float64, error) {
	number, err := strconv.ParseFloat(value, 64)
	if err != nil {
		return 0, fmt.Errorf("%s must be numeric", name)
	}
	if math.IsNaN(number) || math.IsInf(number, 0) {
		return 0, fmt.Errorf("%s must be finite", name)
	}
	return number, nil
}

func decodeOCRTextLayerJSON(input []byte, v any) error {
	decoder := json.NewDecoder(bytes.NewReader(input))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(v); err != nil {
		return fmt.Errorf("invalid ocr text-layer JSON: %w", err)
	}
	if err := decoder.Decode(&struct{}{}); !errors.Is(err, io.EOF) {
		return errors.New("invalid ocr text-layer JSON: multiple top-level values")
	}
	return nil
}

func (item ocrTextLayerJSONItem) toOptions() (OCRTextLayerOptions, error) {
	if item.PageIndex == nil {
		return OCRTextLayerOptions{}, errors.New("page_index is required")
	}
	if item.Text == nil {
		return OCRTextLayerOptions{}, errors.New("text is required")
	}
	if item.Box == nil {
		return OCRTextLayerOptions{}, errors.New("box is required")
	}
	if item.Confidence == nil {
		return OCRTextLayerOptions{}, errors.New("confidence is required")
	}
	box, err := item.Box.toBox()
	if err != nil {
		return OCRTextLayerOptions{}, err
	}
	return OCRTextLayerOptions{
		PageIndex:  *item.PageIndex,
		Text:       *item.Text,
		Box:        box,
		Confidence: *item.Confidence,
	}, nil
}

func (b ocrTextLayerJSONBox) toBox() (OCRTextLayerBox, error) {
	if b.XMin == nil {
		return OCRTextLayerBox{}, errors.New("box.x_min is required")
	}
	if b.YMin == nil {
		return OCRTextLayerBox{}, errors.New("box.y_min is required")
	}
	if b.XMax == nil {
		return OCRTextLayerBox{}, errors.New("box.x_max is required")
	}
	if b.YMax == nil {
		return OCRTextLayerBox{}, errors.New("box.y_max is required")
	}
	return OCRTextLayerBox{
		XMin: *b.XMin,
		YMin: *b.YMin,
		XMax: *b.XMax,
		YMax: *b.YMax,
	}, nil
}
