package pdf

import (
	"errors"
	"fmt"
	"strings"

	"github.com/oxhq/binas/pkg/core"
)

const explicitOverlayStampOperation = "pdf.overlay_explicit_stamp"

type ExplicitOverlayStampOptions struct {
	PageIndex int
	Text      string
	X         float64
	Y         float64
	FontSize  float64
}

func ApplyExplicitOverlayStamp(input []byte, opts ExplicitOverlayStampOptions) ([]byte, core.Report, core.Verification, error) {
	if opts.PageIndex < 0 {
		return nil, core.Report{}, core.Verification{}, errors.New("overlay page index cannot be negative")
	}
	if opts.Text == "" {
		return nil, core.Report{}, core.Verification{}, errors.New("overlay text cannot be empty")
	}
	if opts.FontSize == 0 {
		opts.FontSize = 12
	}
	if opts.FontSize < 0 {
		return nil, core.Report{}, core.Verification{}, errors.New("overlay font size cannot be negative")
	}

	graph, err := parsePDFGraph(input)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	pages := overlayPageObjects(graph)
	if len(pages) == 0 {
		return nil, core.Report{}, core.Verification{}, errors.New("overlay requires at least one page")
	}
	if opts.PageIndex >= len(pages) {
		return nil, core.Report{}, core.Verification{}, fmt.Errorf("overlay page index %d out of range for %d pages (zero-based)", opts.PageIndex, len(pages))
	}

	page := pages[opts.PageIndex]
	pageDict, pageStream, err := overlayPageDict(page)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}

	fontID := pdfObjectID{Number: nextPDFObjectNumber(graph), Generation: 0}
	contentID := pdfObjectID{Number: fontID.Number + 1, Generation: 0}
	fontResourceName := "BinasOverlayFont"

	graph.Objects[fontID] = &pdfIndirectObject{
		ID: fontID,
		Value: pdfDict{
			"Type":     pdfName("Font"),
			"Subtype":  pdfName("Type1"),
			"BaseFont": pdfName("Helvetica"),
		},
	}
	graph.Objects[contentID] = &pdfIndirectObject{
		ID: contentID,
		Value: pdfStreamObject{
			Dict: pdfDict{},
			Data: []byte(overlayStampContentStream(fontResourceName, opts)),
		},
	}

	if err := addOverlayFontResource(graph, pageDict, fontResourceName, pdfRef{ID: fontID}); err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	pageDict["Contents"] = appendOverlayContent(pageDict["Contents"], pdfRef{ID: contentID})
	if pageStream != nil {
		stream := *pageStream
		stream.Dict = pageDict
		page.Value = stream
	} else {
		page.Value = pageDict
	}

	output, err := writeCanonicalPDF(graph)
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	pageVerification, err := verifyExplicitOverlayStampPageOperation(output, opts.Text, len(pages))
	if err != nil {
		return nil, core.Report{}, core.Verification{}, err
	}
	verification := pageVerification.CoreVerification()
	report := WithFallbackPolicy(core.Report{
		Format:        "pdf",
		Edit:          explicitOverlayStampOperation,
		NodesModified: 1,
		MatchIndex:    &opts.PageIndex,
		Invariants: []core.Invariant{
			core.InvariantReparse,
			core.InvariantNewSelectable,
			core.InvariantPageUnchanged,
		},
		Verification: &verification,
		Meta: map[string]any{
			"page_index":        opts.PageIndex,
			"operation":         "overlay_stamp",
			"page_verification": pageVerification.Metadata(),
		},
	}, OverlayPolicy{Fallback: FallbackOverlay, Mode: FallbackModeExplicit})
	return output, report, verification, nil
}

func overlayPageObjects(graph *pdfGraph) []*pdfIndirectObject {
	pages := make([]*pdfIndirectObject, 0)
	for _, object := range sortedPDFObjects(graph.Objects) {
		switch value := object.Value.(type) {
		case pdfDict:
			if dictHasType(value, "Page") {
				pages = append(pages, object)
			}
		case pdfStreamObject:
			if dictHasType(value.Dict, "Page") {
				pages = append(pages, object)
			}
		}
	}
	return pages
}

func overlayPageDict(object *pdfIndirectObject) (pdfDict, *pdfStreamObject, error) {
	switch value := object.Value.(type) {
	case pdfDict:
		return clonePDFDict(value), nil, nil
	case pdfStreamObject:
		stream := value
		return clonePDFDict(value.Dict), &stream, nil
	default:
		return nil, nil, fmt.Errorf("overlay page object %d %d is not a dictionary", object.ID.Number, object.ID.Generation)
	}
}

func addOverlayFontResource(graph *pdfGraph, page pdfDict, fontName string, fontRef pdfRef) error {
	resources, err := overlayResourceDict(graph, page)
	if err != nil {
		return err
	}
	fonts, err := overlayFontDict(graph, resources)
	if err != nil {
		return err
	}
	fonts[fontName] = fontRef
	return nil
}

func overlayResourceDict(graph *pdfGraph, page pdfDict) (pdfDict, error) {
	switch resources := page["Resources"].(type) {
	case nil:
		created := pdfDict{}
		page["Resources"] = created
		return created, nil
	case pdfDict:
		created := clonePDFDict(resources)
		page["Resources"] = created
		return created, nil
	case pdfRef:
		object := graph.Objects[resources.ID]
		if object == nil {
			return nil, fmt.Errorf("overlay page /Resources reference %d %d R was not found", resources.ID.Number, resources.ID.Generation)
		}
		dict, ok := object.Value.(pdfDict)
		if !ok {
			return nil, fmt.Errorf("overlay page /Resources reference %d %d R is not a dictionary", resources.ID.Number, resources.ID.Generation)
		}
		created := clonePDFDict(dict)
		object.Value = created
		return created, nil
	default:
		return nil, fmt.Errorf("overlay page /Resources has unsupported type %T", resources)
	}
}

func overlayFontDict(graph *pdfGraph, resources pdfDict) (pdfDict, error) {
	switch fonts := resources["Font"].(type) {
	case nil:
		created := pdfDict{}
		resources["Font"] = created
		return created, nil
	case pdfDict:
		created := clonePDFDict(fonts)
		resources["Font"] = created
		return created, nil
	case pdfRef:
		object := graph.Objects[fonts.ID]
		if object == nil {
			return nil, fmt.Errorf("overlay /Font reference %d %d R was not found", fonts.ID.Number, fonts.ID.Generation)
		}
		dict, ok := object.Value.(pdfDict)
		if !ok {
			return nil, fmt.Errorf("overlay /Font reference %d %d R is not a dictionary", fonts.ID.Number, fonts.ID.Generation)
		}
		created := clonePDFDict(dict)
		object.Value = created
		return created, nil
	default:
		return nil, fmt.Errorf("overlay /Font resource has unsupported type %T", fonts)
	}
}

func appendOverlayContent(current pdfValue, overlay pdfRef) pdfValue {
	switch contents := current.(type) {
	case nil:
		return overlay
	case pdfArray:
		out := append(pdfArray{}, contents...)
		return append(out, overlay)
	default:
		return pdfArray{contents, overlay}
	}
}

func overlayStampContentStream(fontName string, opts ExplicitOverlayStampOptions) string {
	var out strings.Builder
	out.WriteString("q\nBT\n/")
	out.WriteString(fontName)
	out.WriteByte(' ')
	out.WriteString(pdfNumberToken(opts.FontSize))
	out.WriteString(" Tf\n")
	out.WriteString(pdfNumberToken(opts.X))
	out.WriteByte(' ')
	out.WriteString(pdfNumberToken(opts.Y))
	out.WriteString(" Td\n(")
	out.WriteString(encodeLiteralString(opts.Text))
	out.WriteString(") Tj\nET\nQ")
	return out.String()
}

func verifyExplicitOverlayStamp(output []byte, text string, pageCount int) (core.Verification, error) {
	verification, err := verifyExplicitOverlayStampPageOperation(output, text, pageCount)
	if err != nil {
		return core.Verification{}, err
	}
	return verification.CoreVerification(), nil
}

func verifyExplicitOverlayStampPageOperation(output []byte, text string, pageCount int) (PageOperationVerification, error) {
	return VerifyPageOperationOutput(output, PageOperationVerificationOptions{
		ExpectedPageCount:  pageCount,
		ExpectedText:       []string{text},
		RequirePageContent: true,
		RequireResources:   true,
	})
}
