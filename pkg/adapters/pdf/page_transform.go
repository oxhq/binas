package pdf

import (
	"errors"
	"fmt"
)

const pageOperationTransform = "pdf.transform_pages"

type PageWriteOptions = PageOperationOptions

type PageSelector struct {
	Indexes []int `json:"indexes,omitempty"`
}

type PageBox struct {
	Left   float64 `json:"left"`
	Bottom float64 `json:"bottom"`
	Right  float64 `json:"right"`
	Top    float64 `json:"top"`
}

type PageTransform struct {
	Rotate  *int       `json:"rotate,omitempty"`
	CropBox *PageBox   `json:"crop_box,omitempty"`
	Scale   *PageScale `json:"scale,omitempty"`
}

type PageScale struct {
	X float64 `json:"x"`
	Y float64 `json:"y"`
}

func TransformPages(input []byte, selector PageSelector, transform PageTransform, opts PageWriteOptions) ([]byte, PageOperationReport, PageOperationVerification, error) {
	if transform.Rotate == nil && transform.CropBox == nil && transform.Scale == nil {
		return nil, PageOperationReport{}, PageOperationVerification{}, errors.New("at least one page transform is required")
	}
	if transform.Rotate != nil {
		if _, err := normalizedPageRotation(*transform.Rotate); err != nil {
			return nil, PageOperationReport{}, PageOperationVerification{}, err
		}
	}
	if transform.CropBox != nil {
		if err := validatePageBox(*transform.CropBox); err != nil {
			return nil, PageOperationReport{}, PageOperationVerification{}, err
		}
	}
	if transform.Scale != nil {
		if err := validatePageScale(*transform.Scale); err != nil {
			return nil, PageOperationReport{}, PageOperationVerification{}, err
		}
	}

	graph, err := parsePDFGraph(input)
	if err != nil {
		return nil, PageOperationReport{}, PageOperationVerification{}, err
	}
	pages, err := graph.orderedPages()
	if err != nil {
		return nil, PageOperationReport{}, PageOperationVerification{}, err
	}

	selected, err := selectedPageIndexes(selector, len(pages))
	if err != nil {
		return nil, PageOperationReport{}, PageOperationVerification{}, err
	}

	builder := newPageOperationBuilder()
	if err := builder.copyPagesFromGraph(graph, pages, allPageIndexes(len(pages)), 0); err != nil {
		return nil, PageOperationReport{}, PageOperationVerification{}, err
	}
	for index := range selected {
		ref, ok := builder.kids[index].(pdfRef)
		if !ok {
			return nil, PageOperationReport{}, PageOperationVerification{}, fmt.Errorf("copied page %d is not an indirect reference", index)
		}
		dict, ok := builder.graph.objectDict(ref.ID)
		if !ok {
			return nil, PageOperationReport{}, PageOperationVerification{}, fmt.Errorf("copied page %d dictionary is missing", index)
		}
		if err := applyPageTransform(builder.graph, dict, transform); err != nil {
			return nil, PageOperationReport{}, PageOperationVerification{}, fmt.Errorf("transform page %d: %w", index, err)
		}
	}
	if err := builder.preserveCatalogFromGraph(graph, 0); err != nil {
		return nil, PageOperationReport{}, PageOperationVerification{}, err
	}

	output, verification, err := builder.write(PageOperationOptions(opts), len(pages))
	if err != nil {
		return nil, PageOperationReport{}, PageOperationVerification{}, err
	}
	return output, PageOperationReport{
		Operation:      pageOperationTransform,
		InputDocuments: 1,
		InputPages:     len(pages),
		OutputPages:    len(pages),
		CopiedPages:    builder.copiedPages,
		Verification:   &verification,
	}, verification, nil
}

func selectedPageIndexes(selector PageSelector, pageCount int) (map[int]bool, error) {
	indexes := selector.Indexes
	if len(indexes) == 0 {
		indexes = allPageIndexes(pageCount)
	}
	selected := make(map[int]bool, len(indexes))
	for _, index := range indexes {
		if index < 0 || index >= pageCount {
			return nil, fmt.Errorf("page index %d out of range for %d pages (zero-based)", index, pageCount)
		}
		selected[index] = true
	}
	return selected, nil
}

func applyPageTransform(graph *pdfGraph, dict pdfDict, transform PageTransform) error {
	if transform.Rotate != nil {
		rotate, err := normalizedPageRotation(*transform.Rotate)
		if err != nil {
			return err
		}
		dict["Rotate"] = rotate
	}
	if transform.CropBox != nil {
		if err := validatePageBox(*transform.CropBox); err != nil {
			return err
		}
		dict["CropBox"] = pdfArray{
			transform.CropBox.Left,
			transform.CropBox.Bottom,
			transform.CropBox.Right,
			transform.CropBox.Top,
		}
	}
	if transform.Scale != nil {
		if err := applyPageScaleTransform(graph, dict, *transform.Scale); err != nil {
			return err
		}
	}
	return nil
}

func normalizedPageRotation(degrees int) (int, error) {
	if degrees%90 != 0 {
		return 0, fmt.Errorf("page rotation %d is invalid: expected a multiple of 90 degrees", degrees)
	}
	normalized := degrees % 360
	if normalized < 0 {
		normalized += 360
	}
	return normalized, nil
}

func validatePageBox(box PageBox) error {
	if box.Right <= box.Left {
		return fmt.Errorf("invalid page box: right %.4g must be greater than left %.4g", box.Right, box.Left)
	}
	if box.Top <= box.Bottom {
		return fmt.Errorf("invalid page box: top %.4g must be greater than bottom %.4g", box.Top, box.Bottom)
	}
	return nil
}

func validatePageScale(scale PageScale) error {
	if scale.X <= 0 {
		return fmt.Errorf("invalid page scale: x %.4g must be greater than zero", scale.X)
	}
	if scale.Y <= 0 {
		return fmt.Errorf("invalid page scale: y %.4g must be greater than zero", scale.Y)
	}
	return nil
}

func applyPageScaleTransform(graph *pdfGraph, dict pdfDict, scale PageScale) error {
	for _, key := range []string{"MediaBox", "CropBox", "ArtBox", "BleedBox", "TrimBox"} {
		box, ok := pageBoxFromPDFValue(dict[key])
		if !ok {
			continue
		}
		dict[key] = pdfArray{
			box.Left * scale.X,
			box.Bottom * scale.Y,
			box.Right * scale.X,
			box.Top * scale.Y,
		}
	}
	contents, ok := dict["Contents"]
	if !ok {
		return nil
	}
	wrapper := []byte(fmt.Sprintf("q\n%.8g 0 0 %.8g 0 0 cm\n", scale.X, scale.Y))
	suffix := []byte("\nQ\n")
	switch v := contents.(type) {
	case pdfRef:
		return scaleContentStream(graph, v.ID, wrapper, suffix)
	case pdfArray:
		for _, item := range v {
			ref, ok := item.(pdfRef)
			if !ok {
				return errors.New("unsupported page scale: /Contents array contains a non-reference item")
			}
			if err := scaleContentStream(graph, ref.ID, wrapper, suffix); err != nil {
				return err
			}
		}
		return nil
	default:
		return errors.New("unsupported page scale: /Contents is not a reference or reference array")
	}
}

func scaleContentStream(graph *pdfGraph, id pdfObjectID, prefix, suffix []byte) error {
	object := graph.Objects[id]
	if object == nil {
		return fmt.Errorf("content stream object %d %d is missing", id.Number, id.Generation)
	}
	stream, ok := object.Value.(pdfStreamObject)
	if !ok {
		return fmt.Errorf("content stream object %d %d is not a stream", id.Number, id.Generation)
	}
	if filter := pdfGraphStreamFilterString(stream.Dict); filter != "" {
		return fmt.Errorf("unsupported page scale: content stream %d %d has filter %s", id.Number, id.Generation, filter)
	}
	data := make([]byte, 0, len(prefix)+len(stream.Data)+len(suffix))
	data = append(data, prefix...)
	data = append(data, stream.Data...)
	data = append(data, suffix...)
	delete(stream.Dict, "Length")
	stream.Data = data
	object.Value = stream
	return nil
}

func pageBoxFromPDFValue(value pdfValue) (PageBox, bool) {
	array, ok := value.(pdfArray)
	if !ok || len(array) != 4 {
		return PageBox{}, false
	}
	values := [4]float64{}
	for i, item := range array {
		number, ok := pdfValueFloat(item)
		if !ok {
			return PageBox{}, false
		}
		values[i] = number
	}
	return PageBox{Left: values[0], Bottom: values[1], Right: values[2], Top: values[3]}, true
}

func pdfValueFloat(value pdfValue) (float64, bool) {
	switch v := value.(type) {
	case int:
		return float64(v), true
	case int64:
		return float64(v), true
	case float64:
		return v, true
	default:
		return 0, false
	}
}
