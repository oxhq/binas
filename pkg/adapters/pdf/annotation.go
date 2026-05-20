package pdf

import (
	"errors"
	"fmt"
	"strings"

	"github.com/oxhq/binas/pkg/core"
)

const annotationContentsEditOperation = "pdf.annotation_contents_update"

type AnnotationContentsEditReport struct {
	core.Report
	AnnotationIndex       int    `json:"annotation_index"`
	ObjectNumber          int    `json:"object_number,omitempty"`
	ObjectGeneration      int    `json:"object_generation,omitempty"`
	OldContents           string `json:"old_contents,omitempty"`
	NewContents           string `json:"new_contents"`
	AppearanceRegenerated bool   `json:"appearance_regenerated"`
	AppearanceInvalidated bool   `json:"appearance_invalidated,omitempty"`
	AppearanceRemoved     bool   `json:"appearance_removed,omitempty"`
	AppearanceNote        string `json:"appearance_note"`
}

type AnnotationContentsEditVerification struct {
	ReparseOK             bool `json:"reparse_ok"`
	ContentsUpdated       bool `json:"contents_updated"`
	PageUnchanged         bool `json:"page_count_unchanged"`
	AppearanceRegenerated bool `json:"appearance_regenerated"`
	AppearanceInvalidated bool `json:"appearance_invalidated,omitempty"`
	AppearanceRemoved     bool `json:"appearance_removed,omitempty"`
}

type AnnotationContentsEditOptions struct {
	RemoveAppearance     bool
	RegenerateAppearance bool
}

type AnnotationCandidateMetadata struct {
	Index                int       `json:"index"`
	ObjectNumber         *int      `json:"object_number,omitempty"`
	ObjectGeneration     *int      `json:"object_generation,omitempty"`
	PageIndex            *int      `json:"page_index,omitempty"`
	PageObjectNumber     *int      `json:"page_object_number,omitempty"`
	PageObjectGeneration *int      `json:"page_object_generation,omitempty"`
	Subtype              string    `json:"subtype,omitempty"`
	Contents             string    `json:"contents"`
	Name                 string    `json:"name,omitempty"`
	Modified             string    `json:"modified,omitempty"`
	Title                string    `json:"title,omitempty"`
	HasAppearance        bool      `json:"has_appearance"`
	Rect                 []float64 `json:"rect,omitempty"`
	Color                []float64 `json:"color,omitempty"`
	Border               []float64 `json:"border,omitempty"`
	QuadPointsCount      int       `json:"quad_points_count,omitempty"`
	Flags                int       `json:"flags"`
	FlagNames            []string  `json:"flag_names,omitempty"`
	Invisible            bool      `json:"invisible"`
	Hidden               bool      `json:"hidden"`
	Print                bool      `json:"print"`
	NoZoom               bool      `json:"no_zoom"`
	NoRotate             bool      `json:"no_rotate"`
	NoView               bool      `json:"no_view"`
	ReadOnly             bool      `json:"read_only"`
	Locked               bool      `json:"locked"`
	ToggleNoView         bool      `json:"toggle_no_view"`
	LockedContents       bool      `json:"locked_contents"`
}

type annotationFlagDefinition struct {
	bit  int
	name string
}

var annotationFlagDefinitions = []annotationFlagDefinition{
	{bit: 1 << 0, name: "invisible"},
	{bit: 1 << 1, name: "hidden"},
	{bit: 1 << 2, name: "print"},
	{bit: 1 << 3, name: "no_zoom"},
	{bit: 1 << 4, name: "no_rotate"},
	{bit: 1 << 5, name: "no_view"},
	{bit: 1 << 6, name: "read_only"},
	{bit: 1 << 7, name: "locked"},
	{bit: 1 << 8, name: "toggle_no_view"},
	{bit: 1 << 9, name: "locked_contents"},
}

type annotationCandidate struct {
	Dict       pdfDict
	Object     *pdfIndirectObject
	Page       *annotationPageReference
	Index      int
	OldContent string
}

type annotationPageReference struct {
	Index  int
	Object *pdfIndirectObject
}

func ListAnnotationCandidates(input []byte) ([]AnnotationCandidateMetadata, error) {
	graph, err := parsePDFGraph(input)
	if err != nil {
		return nil, err
	}
	candidates := graph.annotationCandidates()
	out := make([]AnnotationCandidateMetadata, 0, len(candidates))
	for _, candidate := range candidates {
		out = append(out, candidate.metadata())
	}
	return out, nil
}

func ApplyAnnotationContentsEdit(input []byte, index int, contents string, options ...AnnotationContentsEditOptions) ([]byte, AnnotationContentsEditReport, AnnotationContentsEditVerification, error) {
	if index < 0 {
		return nil, AnnotationContentsEditReport{}, AnnotationContentsEditVerification{}, errors.New("annotation index cannot be negative")
	}
	opts := annotationContentsEditOptions(options)
	if opts.RemoveAppearance && opts.RegenerateAppearance {
		return nil, AnnotationContentsEditReport{}, AnnotationContentsEditVerification{}, errors.New("use only one of --remove-appearance or --regenerate-appearance")
	}
	graph, err := parsePDFGraph(input)
	if err != nil {
		return nil, AnnotationContentsEditReport{}, AnnotationContentsEditVerification{}, err
	}
	candidates := graph.annotationCandidates()
	if len(candidates) == 0 {
		return nil, AnnotationContentsEditReport{}, AnnotationContentsEditVerification{}, errors.New("no annotation dictionaries found")
	}
	if index >= len(candidates) {
		return nil, AnnotationContentsEditReport{}, AnnotationContentsEditVerification{}, fmt.Errorf("annotation index %d out of range for %d annotations (zero-based)", index, len(candidates))
	}

	candidate := candidates[index]
	updated := clonePDFDict(candidate.Dict)
	updated["Contents"] = pdfLiteralString(encodeLiteralString(contents))
	_, hadAppearance := updated["AP"]
	if opts.RemoveAppearance {
		delete(updated, "AP")
	}
	appearanceRegenerated := false
	if opts.RegenerateAppearance {
		appearance, err := buildBasicAnnotationAppearance(candidate, contents)
		if err != nil {
			return nil, AnnotationContentsEditReport{}, AnnotationContentsEditVerification{}, err
		}
		appearanceID := nextAnnotationAppearanceObjectID(graph)
		graph.Objects[appearanceID] = &pdfIndirectObject{
			ID:    appearanceID,
			Value: appearance,
		}
		updated["AP"] = pdfDict{"N": pdfRef{ID: appearanceID}}
		appearanceRegenerated = true
	}
	if candidate.Object != nil {
		if stream, ok := candidate.Object.Value.(pdfStreamObject); ok {
			stream.Dict = updated
			candidate.Object.Value = stream
		} else {
			candidate.Object.Value = updated
		}
	} else {
		for key := range candidate.Dict {
			delete(candidate.Dict, key)
		}
		for key, value := range updated {
			candidate.Dict[key] = value
		}
	}

	output, err := writeCanonicalPDF(graph)
	if err != nil {
		return nil, AnnotationContentsEditReport{}, AnnotationContentsEditVerification{}, err
	}
	appearanceRemoved := opts.RemoveAppearance && hadAppearance
	verification, err := verifyAnnotationContentsEdit(output, index, contents, graph.pageCount(), appearanceRemoved, appearanceRegenerated)
	if err != nil {
		return nil, AnnotationContentsEditReport{}, AnnotationContentsEditVerification{}, err
	}

	matchIndex := index
	report := AnnotationContentsEditReport{
		Report: core.Report{
			Format:        "pdf",
			Edit:          annotationContentsEditOperation,
			FallbackUsed:  false,
			NodesModified: 1,
			MatchIndex:    &matchIndex,
			Invariants: []core.Invariant{
				core.InvariantReparse,
				core.InvariantPageUnchanged,
				core.InvariantNoFallbackUsed,
			},
		},
		AnnotationIndex:       index,
		OldContents:           candidate.OldContent,
		NewContents:           contents,
		AppearanceRegenerated: verification.AppearanceRegenerated,
		AppearanceInvalidated: appearanceRemoved,
		AppearanceRemoved:     appearanceRemoved,
		AppearanceNote:        annotationContentsEditAppearanceNote(verification.AppearanceRegenerated, appearanceRemoved),
	}
	if candidate.Object != nil {
		report.ObjectNumber = candidate.Object.ID.Number
		report.ObjectGeneration = candidate.Object.ID.Generation
	}
	return output, report, verification, nil
}

func annotationContentsEditOptions(options []AnnotationContentsEditOptions) AnnotationContentsEditOptions {
	var out AnnotationContentsEditOptions
	if len(options) > 0 {
		out = options[0]
	}
	return out
}

func annotationContentsEditAppearanceNote(appearanceRegenerated, appearanceRemoved bool) string {
	if appearanceRegenerated {
		return "basic annotation /AP /N appearance stream was regenerated from /Subtype, /Rect, and supported style entries"
	}
	if appearanceRemoved {
		return "stale annotation /AP was removed after updating /Contents"
	}
	return "annotation /Contents was updated; appearance stream was left unchanged"
}

func buildBasicAnnotationAppearance(candidate annotationCandidate, contents string) (pdfStreamObject, error) {
	subtype := annotationSubtype(candidate.Dict)
	rect := annotationRect(candidate.Dict)
	if len(rect) != 4 || rect[2] <= rect[0] || rect[3] <= rect[1] {
		return pdfStreamObject{}, fmt.Errorf("cannot regenerate annotation appearance: annotation %d has no usable /Rect", candidate.Index)
	}
	width := rect[2] - rect[0]
	height := rect[3] - rect[1]
	switch subtype {
	case "Text", "FreeText":
		return basicAnnotationAppearanceStream(width, height, contents)
	case "Link":
		return basicLinkAnnotationAppearanceStream(candidate.Dict, candidate.Index, width, height)
	case "Square", "Circle":
		return basicShapeAnnotationAppearanceStream(subtype, width, height, annotationNumericArray(candidate.Dict, "C")), nil
	case "Highlight", "Underline", "StrikeOut":
		quadPoints, err := annotationQuadPoints(candidate.Dict, candidate.Index)
		if err != nil {
			return pdfStreamObject{}, err
		}
		return basicTextMarkupAnnotationAppearanceStream(subtype, rect, width, height, annotationNumericArray(candidate.Dict, "C"), quadPoints), nil
	default:
		return pdfStreamObject{}, fmt.Errorf("cannot regenerate annotation appearance: unsupported annotation subtype %q", subtype)
	}
}

func basicAnnotationAppearanceStream(width, height float64, contents string) (pdfStreamObject, error) {
	lines, err := simpleAppearanceTextLines(width, height, contents)
	if err != nil {
		return pdfStreamObject{}, fmt.Errorf("cannot regenerate annotation appearance: %w", err)
	}
	var data strings.Builder
	writeSimpleAppearanceText(&data, width, height, lines, true)

	return pdfStreamObject{
		Dict: pdfDict{
			"Type":     pdfName("XObject"),
			"Subtype":  pdfName("Form"),
			"FormType": 1,
			"BBox":     pdfArray{0, 0, width, height},
			"Resources": pdfDict{
				"Font": pdfDict{
					"Helv": pdfDict{
						"Type":     pdfName("Font"),
						"Subtype":  pdfName("Type1"),
						"BaseFont": pdfName("Helvetica"),
					},
				},
			},
		},
		Data: []byte(data.String()),
	}, nil
}

func basicLinkAnnotationAppearanceStream(dict pdfDict, annotationIndex int, width, height float64) (pdfStreamObject, error) {
	if _, ok := dict["BS"]; ok {
		return pdfStreamObject{}, fmt.Errorf("cannot regenerate annotation appearance: annotation %d has unsupported /BS border style", annotationIndex)
	}
	if annotationHasJavaScriptAction(dict) {
		return pdfStreamObject{}, fmt.Errorf("cannot regenerate annotation appearance: annotation %d has unsupported JavaScript action", annotationIndex)
	}
	borderWidth, err := annotationLinkBorderWidth(dict, annotationIndex)
	if err != nil {
		return pdfStreamObject{}, err
	}
	color, err := annotationLinkColor(dict, annotationIndex)
	if err != nil {
		return pdfStreamObject{}, err
	}

	var data strings.Builder
	data.WriteString("q\n")
	if borderWidth > 0 {
		fmt.Fprintf(&data, "%s w\n", pdfNumberToken(borderWidth))
		writeBasicAnnotationStrokeColor(&data, color)
		inset := borderWidth / 2
		rectWidth := width - borderWidth
		rectHeight := height - borderWidth
		if rectWidth <= 0 || rectHeight <= 0 {
			return pdfStreamObject{}, fmt.Errorf("cannot regenerate annotation appearance: annotation %d has /Border width larger than /Rect", annotationIndex)
		}
		fmt.Fprintf(&data, "%s %s %s %s re S\n", pdfNumberToken(inset), pdfNumberToken(inset), pdfNumberToken(rectWidth), pdfNumberToken(rectHeight))
	}
	data.WriteString("Q")

	return pdfStreamObject{
		Dict: pdfDict{
			"Type":     pdfName("XObject"),
			"Subtype":  pdfName("Form"),
			"FormType": 1,
			"BBox":     pdfArray{0, 0, width, height},
		},
		Data: []byte(data.String()),
	}, nil
}

func basicShapeAnnotationAppearanceStream(subtype string, width, height float64, color []float64) pdfStreamObject {
	var data strings.Builder
	data.WriteString("q\n")
	data.WriteString("1 w\n")
	writeBasicAnnotationStrokeColor(&data, color)
	switch subtype {
	case "Circle":
		writeBasicAnnotationEllipse(&data, width, height)
	default:
		fmt.Fprintf(&data, "0.5 0.5 %s %s re S\n", pdfNumberToken(width-1), pdfNumberToken(height-1))
	}
	data.WriteString("Q")

	return pdfStreamObject{
		Dict: pdfDict{
			"Type":     pdfName("XObject"),
			"Subtype":  pdfName("Form"),
			"FormType": 1,
			"BBox":     pdfArray{0, 0, width, height},
		},
		Data: []byte(data.String()),
	}
}

func basicTextMarkupAnnotationAppearanceStream(subtype string, rect []float64, width, height float64, color []float64, quadPoints []float64) pdfStreamObject {
	var data strings.Builder
	data.WriteString("q\n")
	switch subtype {
	case "Highlight":
		writeBasicAnnotationFillColor(&data, color, []float64{1, 1, 0})
		writeBasicHighlightQuads(&data, rect, quadPoints)
	case "Underline", "StrikeOut":
		data.WriteString("1 w\n")
		writeBasicAnnotationStrokeColor(&data, color)
		writeBasicTextMarkupLines(&data, subtype, rect, quadPoints)
	}
	data.WriteString("Q")

	return pdfStreamObject{
		Dict: pdfDict{
			"Type":     pdfName("XObject"),
			"Subtype":  pdfName("Form"),
			"FormType": 1,
			"BBox":     pdfArray{0, 0, width, height},
		},
		Data: []byte(data.String()),
	}
}

func writeBasicHighlightQuads(data *strings.Builder, rect []float64, quadPoints []float64) {
	for i := 0; i < len(quadPoints); i += 8 {
		x1, y1 := annotationAppearancePoint(rect, quadPoints[i], quadPoints[i+1])
		x2, y2 := annotationAppearancePoint(rect, quadPoints[i+2], quadPoints[i+3])
		x3, y3 := annotationAppearancePoint(rect, quadPoints[i+4], quadPoints[i+5])
		x4, y4 := annotationAppearancePoint(rect, quadPoints[i+6], quadPoints[i+7])
		fmt.Fprintf(data, "%s %s m\n", pdfNumberToken(x1), pdfNumberToken(y1))
		fmt.Fprintf(data, "%s %s l\n", pdfNumberToken(x2), pdfNumberToken(y2))
		fmt.Fprintf(data, "%s %s l\n", pdfNumberToken(x4), pdfNumberToken(y4))
		fmt.Fprintf(data, "%s %s l f\n", pdfNumberToken(x3), pdfNumberToken(y3))
	}
}

func writeBasicTextMarkupLines(data *strings.Builder, subtype string, rect []float64, quadPoints []float64) {
	for i := 0; i < len(quadPoints); i += 8 {
		minX, minY, maxX, maxY := annotationQuadBounds(rect, quadPoints[i:i+8])
		y := minY + (maxY-minY)*0.1
		if subtype == "StrikeOut" {
			y = minY + (maxY-minY)*0.5
		}
		fmt.Fprintf(data, "%s %s m\n", pdfNumberToken(minX), pdfNumberToken(y))
		fmt.Fprintf(data, "%s %s l S\n", pdfNumberToken(maxX), pdfNumberToken(y))
	}
}

func annotationAppearancePoint(rect []float64, x, y float64) (float64, float64) {
	return x - rect[0], y - rect[1]
}

func annotationQuadBounds(rect []float64, quad []float64) (float64, float64, float64, float64) {
	x, y := annotationAppearancePoint(rect, quad[0], quad[1])
	minX, minY, maxX, maxY := x, y, x, y
	for i := 2; i < len(quad); i += 2 {
		x, y = annotationAppearancePoint(rect, quad[i], quad[i+1])
		if x < minX {
			minX = x
		}
		if y < minY {
			minY = y
		}
		if x > maxX {
			maxX = x
		}
		if y > maxY {
			maxY = y
		}
	}
	return minX, minY, maxX, maxY
}

func writeBasicAnnotationStrokeColor(data *strings.Builder, color []float64) {
	switch len(color) {
	case 1:
		fmt.Fprintf(data, "%s G\n", pdfNumberToken(color[0]))
	case 3:
		fmt.Fprintf(data, "%s %s %s RG\n", pdfNumberToken(color[0]), pdfNumberToken(color[1]), pdfNumberToken(color[2]))
	default:
		data.WriteString("0 0 0 RG\n")
	}
}

func writeBasicAnnotationFillColor(data *strings.Builder, color []float64, defaultRGB []float64) {
	switch len(color) {
	case 1:
		fmt.Fprintf(data, "%s g\n", pdfNumberToken(color[0]))
	case 3:
		fmt.Fprintf(data, "%s %s %s rg\n", pdfNumberToken(color[0]), pdfNumberToken(color[1]), pdfNumberToken(color[2]))
	default:
		fmt.Fprintf(data, "%s %s %s rg\n", pdfNumberToken(defaultRGB[0]), pdfNumberToken(defaultRGB[1]), pdfNumberToken(defaultRGB[2]))
	}
}

func writeBasicAnnotationEllipse(data *strings.Builder, width, height float64) {
	const bezierCircleKappa = 0.5522847498
	cx := width / 2
	cy := height / 2
	rx := (width - 1) / 2
	ry := (height - 1) / 2
	ox := rx * bezierCircleKappa
	oy := ry * bezierCircleKappa
	fmt.Fprintf(data, "%s %s m\n", pdfNumberToken(cx+rx), pdfNumberToken(cy))
	fmt.Fprintf(data, "%s %s %s %s %s %s c\n", pdfNumberToken(cx+rx), pdfNumberToken(cy+oy), pdfNumberToken(cx+ox), pdfNumberToken(cy+ry), pdfNumberToken(cx), pdfNumberToken(cy+ry))
	fmt.Fprintf(data, "%s %s %s %s %s %s c\n", pdfNumberToken(cx-ox), pdfNumberToken(cy+ry), pdfNumberToken(cx-rx), pdfNumberToken(cy+oy), pdfNumberToken(cx-rx), pdfNumberToken(cy))
	fmt.Fprintf(data, "%s %s %s %s %s %s c\n", pdfNumberToken(cx-rx), pdfNumberToken(cy-oy), pdfNumberToken(cx-ox), pdfNumberToken(cy-ry), pdfNumberToken(cx), pdfNumberToken(cy-ry))
	fmt.Fprintf(data, "%s %s %s %s %s %s c S\n", pdfNumberToken(cx+ox), pdfNumberToken(cy-ry), pdfNumberToken(cx+rx), pdfNumberToken(cy-oy), pdfNumberToken(cx+rx), pdfNumberToken(cy))
}

func nextAnnotationAppearanceObjectID(graph *pdfGraph) pdfObjectID {
	maxObjectNumber := 0
	for id := range graph.Objects {
		if id.Number > maxObjectNumber {
			maxObjectNumber = id.Number
		}
	}
	return pdfObjectID{Number: maxObjectNumber + 1, Generation: 0}
}

func (g *pdfGraph) annotationCandidates() []annotationCandidate {
	candidates := make([]annotationCandidate, 0)
	seen := make(map[*pdfIndirectObject]bool)
	pageRefs, pageObjects := g.annotationPageReferences()
	for _, object := range sortedPDFObjects(g.Objects) {
		if dict, ok := object.Value.(pdfDict); ok && isAnnotationDict(dict) {
			candidates = append(candidates, annotationCandidate{
				Dict:       dict,
				Object:     object,
				Page:       pageRefs[object.ID],
				Index:      len(candidates),
				OldContent: annotationContents(dict),
			})
			seen[object] = true
			continue
		}
		if stream, ok := object.Value.(pdfStreamObject); ok && isAnnotationDict(stream.Dict) {
			candidates = append(candidates, annotationCandidate{
				Dict:       stream.Dict,
				Object:     object,
				Page:       pageRefs[object.ID],
				Index:      len(candidates),
				OldContent: annotationContents(stream.Dict),
			})
			seen[object] = true
		}
	}
	for _, object := range sortedPDFObjects(g.Objects) {
		dict, ok := object.Value.(pdfDict)
		if !ok {
			continue
		}
		annots, ok := dict["Annots"].(pdfArray)
		if !ok {
			continue
		}
		for _, item := range annots {
			switch v := item.(type) {
			case pdfRef:
				object := g.Objects[v.ID]
				if object == nil || seen[object] {
					continue
				}
				if dict, ok := object.Value.(pdfDict); ok && isAnnotationDict(dict) {
					candidates = append(candidates, annotationCandidate{
						Dict:       dict,
						Object:     object,
						Page:       pageRefs[v.ID],
						Index:      len(candidates),
						OldContent: annotationContents(dict),
					})
					seen[object] = true
				}
			case pdfDict:
				if isAnnotationDict(v) {
					candidates = append(candidates, annotationCandidate{
						Dict:       v,
						Page:       pageObjects[object],
						Index:      len(candidates),
						OldContent: annotationContents(v),
					})
				}
			}
		}
	}
	return candidates
}

func (g *pdfGraph) annotationPageReferences() (map[pdfObjectID]*annotationPageReference, map[*pdfIndirectObject]*annotationPageReference) {
	refs := make(map[pdfObjectID]*annotationPageReference)
	pageObjects := make(map[*pdfIndirectObject]*annotationPageReference)
	for _, page := range g.annotationPages() {
		pageCopy := page
		pageObjects[page.Object] = &pageCopy
		annots, ok := page.dict()["Annots"].(pdfArray)
		if !ok {
			continue
		}
		for _, item := range annots {
			ref, ok := item.(pdfRef)
			if !ok {
				continue
			}
			if _, exists := refs[ref.ID]; !exists {
				refs[ref.ID] = &pageCopy
			}
		}
	}
	return refs, pageObjects
}

func (g *pdfGraph) annotationPages() []annotationPageReference {
	catalog, ok := g.catalogDict()
	if !ok {
		return nil
	}
	pagesRef, ok := dictRef(catalog, "Pages")
	if !ok {
		return nil
	}
	out := make([]annotationPageReference, 0)
	seen := make(map[pdfObjectID]bool)
	g.collectAnnotationPagesFromRef(pagesRef.ID, seen, &out)
	return out
}

func (g *pdfGraph) catalogDict() (pdfDict, bool) {
	if g.Root != nil {
		if dict, ok := g.objectDict(*g.Root); ok && dictHasType(dict, "Catalog") {
			return dict, true
		}
	}
	root, ok := g.findCatalogRoot()
	if !ok {
		return nil, false
	}
	return g.objectDict(root)
}

func (g *pdfGraph) collectAnnotationPagesFromRef(id pdfObjectID, seen map[pdfObjectID]bool, out *[]annotationPageReference) {
	if seen[id] {
		return
	}
	seen[id] = true
	object := g.Objects[id]
	if object == nil {
		return
	}
	dict, ok := objectDictValue(object.Value)
	if !ok {
		return
	}
	if dictHasType(dict, "Page") {
		*out = append(*out, annotationPageReference{Index: len(*out), Object: object})
		return
	}
	if !dictHasType(dict, "Pages") {
		return
	}
	kids, ok := dict["Kids"].(pdfArray)
	if !ok {
		return
	}
	for _, kid := range kids {
		ref, ok := kid.(pdfRef)
		if !ok {
			continue
		}
		g.collectAnnotationPagesFromRef(ref.ID, seen, out)
	}
}

func (g *pdfGraph) objectDict(id pdfObjectID) (pdfDict, bool) {
	object := g.Objects[id]
	if object == nil {
		return nil, false
	}
	return objectDictValue(object.Value)
}

func objectDictValue(value pdfValue) (pdfDict, bool) {
	if dict, ok := value.(pdfDict); ok {
		return dict, true
	}
	if stream, ok := value.(pdfStreamObject); ok {
		return stream.Dict, true
	}
	return nil, false
}

func (p annotationPageReference) dict() pdfDict {
	if p.Object == nil {
		return nil
	}
	dict, _ := objectDictValue(p.Object.Value)
	return dict
}

func (c annotationCandidate) metadata() AnnotationCandidateMetadata {
	flags := annotationFlags(c.Dict)
	metadata := AnnotationCandidateMetadata{
		Index:           c.Index,
		Subtype:         annotationSubtype(c.Dict),
		Contents:        c.OldContent,
		Name:            annotationTextField(c.Dict, "NM"),
		Modified:        annotationTextField(c.Dict, "M"),
		Title:           annotationTextField(c.Dict, "T"),
		HasAppearance:   annotationHasAppearance(c.Dict),
		Rect:            annotationRect(c.Dict),
		Color:           annotationNumericArray(c.Dict, "C"),
		Border:          annotationNumericArray(c.Dict, "Border"),
		QuadPointsCount: annotationQuadPointsCount(c.Dict),
		Flags:           flags,
		FlagNames:       annotationFlagNames(flags),
	}
	metadata.Invisible = annotationFlagSet(flags, "invisible")
	metadata.Hidden = annotationFlagSet(flags, "hidden")
	metadata.Print = annotationFlagSet(flags, "print")
	metadata.NoZoom = annotationFlagSet(flags, "no_zoom")
	metadata.NoRotate = annotationFlagSet(flags, "no_rotate")
	metadata.NoView = annotationFlagSet(flags, "no_view")
	metadata.ReadOnly = annotationFlagSet(flags, "read_only")
	metadata.Locked = annotationFlagSet(flags, "locked")
	metadata.ToggleNoView = annotationFlagSet(flags, "toggle_no_view")
	metadata.LockedContents = annotationFlagSet(flags, "locked_contents")
	if c.Object != nil {
		number := c.Object.ID.Number
		generation := c.Object.ID.Generation
		metadata.ObjectNumber = &number
		metadata.ObjectGeneration = &generation
	}
	if c.Page != nil {
		pageIndex := c.Page.Index
		metadata.PageIndex = &pageIndex
		if c.Page.Object != nil {
			pageNumber := c.Page.Object.ID.Number
			pageGeneration := c.Page.Object.ID.Generation
			metadata.PageObjectNumber = &pageNumber
			metadata.PageObjectGeneration = &pageGeneration
		}
	}
	return metadata
}

func verifyAnnotationContentsEdit(output []byte, index int, contents string, pageCount int, expectAppearanceRemoved bool, expectAppearanceRegenerated bool) (AnnotationContentsEditVerification, error) {
	graph, err := parsePDFGraph(output)
	if err != nil {
		return AnnotationContentsEditVerification{}, err
	}
	candidates := graph.annotationCandidates()
	updated := index >= 0 && index < len(candidates) && candidates[index].OldContent == contents
	appearanceRemoved := false
	if expectAppearanceRemoved && index >= 0 && index < len(candidates) {
		_, hasAppearance := candidates[index].Dict["AP"]
		appearanceRemoved = !hasAppearance
	}
	appearanceRegenerated := false
	if expectAppearanceRegenerated && index >= 0 && index < len(candidates) {
		appearanceRegenerated = annotationHasNormalAppearanceStream(graph, candidates[index].Dict)
	}
	if expectAppearanceRegenerated && !appearanceRegenerated {
		return AnnotationContentsEditVerification{
			ReparseOK:             true,
			ContentsUpdated:       updated,
			PageUnchanged:         pageCount == 0 || graph.pageCount() == pageCount,
			AppearanceRegenerated: false,
			AppearanceInvalidated: appearanceRemoved,
			AppearanceRemoved:     appearanceRemoved,
		}, errors.New("verification failed: annotation /AP /N appearance stream was not regenerated")
	}
	return AnnotationContentsEditVerification{
		ReparseOK:             true,
		ContentsUpdated:       updated,
		PageUnchanged:         pageCount == 0 || graph.pageCount() == pageCount,
		AppearanceRegenerated: appearanceRegenerated,
		AppearanceInvalidated: appearanceRemoved,
		AppearanceRemoved:     appearanceRemoved,
	}, nil
}

func annotationHasNormalAppearanceStream(graph *pdfGraph, dict pdfDict) bool {
	ap, ok := dict["AP"].(pdfDict)
	if !ok {
		return false
	}
	switch normal := ap["N"].(type) {
	case pdfStreamObject:
		return annotationAppearanceStreamIsFormXObject(normal)
	case pdfRef:
		object := graph.Objects[normal.ID]
		if object == nil {
			return false
		}
		stream, ok := object.Value.(pdfStreamObject)
		return ok && annotationAppearanceStreamIsFormXObject(stream)
	default:
		return false
	}
}

func annotationAppearanceStreamIsFormXObject(stream pdfStreamObject) bool {
	return dictHasType(stream.Dict, "XObject") && annotationSubtype(stream.Dict) == "Form"
}

func isAnnotationDict(dict pdfDict) bool {
	if dictHasType(dict, "Annot") {
		return true
	}
	_, hasSubtype := dict["Subtype"].(pdfName)
	_, hasRect := dict["Rect"].(pdfArray)
	return hasSubtype && hasRect
}

func annotationSubtype(dict pdfDict) string {
	subtype, ok := dict["Subtype"].(pdfName)
	if !ok {
		return ""
	}
	return string(subtype)
}

func annotationHasAppearance(dict pdfDict) bool {
	_, ok := dict["AP"]
	return ok
}

func annotationFlags(dict pdfDict) int {
	value, ok := dict["F"]
	if !ok {
		return 0
	}
	number, ok := pdfNumericValue(value)
	if !ok {
		return 0
	}
	return int(number)
}

func annotationFlagNames(flags int) []string {
	names := make([]string, 0, len(annotationFlagDefinitions))
	for _, definition := range annotationFlagDefinitions {
		if flags&definition.bit != 0 {
			names = append(names, definition.name)
		}
	}
	return names
}

func annotationFlagSet(flags int, name string) bool {
	for _, definition := range annotationFlagDefinitions {
		if definition.name == name {
			return flags&definition.bit != 0
		}
	}
	return false
}

func annotationRect(dict pdfDict) []float64 {
	value, ok := dict["Rect"].(pdfArray)
	if !ok || len(value) != 4 {
		return nil
	}
	rect := make([]float64, 0, 4)
	for _, item := range value {
		number, ok := pdfNumericValue(item)
		if !ok {
			return nil
		}
		rect = append(rect, number)
	}
	return rect
}

func annotationLinkBorderWidth(dict pdfDict, annotationIndex int) (float64, error) {
	value, ok := dict["Border"]
	if !ok {
		return 1, nil
	}
	border, ok := annotationDirectNumericArrayValue(value)
	if !ok || len(border) != 3 {
		return 0, fmt.Errorf("cannot regenerate annotation appearance: annotation %d has unsupported /Border", annotationIndex)
	}
	if border[2] < 0 {
		return 0, fmt.Errorf("cannot regenerate annotation appearance: annotation %d has negative /Border width", annotationIndex)
	}
	return border[2], nil
}

func annotationLinkColor(dict pdfDict, annotationIndex int) ([]float64, error) {
	value, ok := dict["C"]
	if !ok {
		return nil, nil
	}
	color, ok := annotationDirectNumericArrayValue(value)
	if !ok || (len(color) != 1 && len(color) != 3) {
		return nil, fmt.Errorf("cannot regenerate annotation appearance: annotation %d has unsupported /C color", annotationIndex)
	}
	return color, nil
}

func annotationHasJavaScriptAction(dict pdfDict) bool {
	if _, ok := dict["AA"]; ok {
		return true
	}
	action, ok := dict["A"].(pdfDict)
	if !ok {
		return false
	}
	subtype, ok := action["S"].(pdfName)
	return ok && subtype == "JavaScript"
}

func annotationNumericArray(dict pdfDict, key string) []float64 {
	value, ok := dict[key].(pdfArray)
	if !ok {
		return nil
	}
	numbers, ok := annotationDirectNumericArrayValue(value)
	if !ok || len(numbers) == 0 {
		return nil
	}
	return numbers
}

func annotationDirectNumericArrayValue(value pdfValue) ([]float64, bool) {
	array, ok := value.(pdfArray)
	if !ok {
		return nil, false
	}
	numbers := make([]float64, 0, len(array))
	for _, item := range array {
		number, ok := pdfNumericValue(item)
		if !ok {
			return nil, false
		}
		numbers = append(numbers, number)
	}
	return numbers, true
}

func annotationQuadPointsCount(dict pdfDict) int {
	points := annotationNumericArray(dict, "QuadPoints")
	if len(points) == 0 || len(points)%8 != 0 {
		return 0
	}
	return len(points) / 8
}

func annotationQuadPoints(dict pdfDict, annotationIndex int) ([]float64, error) {
	value, ok := dict["QuadPoints"]
	if !ok {
		return nil, fmt.Errorf("cannot regenerate annotation appearance: annotation %d has no /QuadPoints", annotationIndex)
	}
	array, ok := value.(pdfArray)
	if !ok || len(array) == 0 || len(array)%8 != 0 {
		return nil, fmt.Errorf("cannot regenerate annotation appearance: annotation %d has malformed /QuadPoints", annotationIndex)
	}
	points := make([]float64, 0, len(array))
	for _, item := range array {
		number, ok := pdfNumericValue(item)
		if !ok {
			return nil, fmt.Errorf("cannot regenerate annotation appearance: annotation %d has malformed /QuadPoints", annotationIndex)
		}
		points = append(points, number)
	}
	return points, nil
}

func pdfNumericValue(value pdfValue) (float64, bool) {
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

func annotationContents(dict pdfDict) string {
	value, ok := dict["Contents"]
	if !ok {
		return ""
	}
	switch v := value.(type) {
	case pdfLiteralString:
		return decodeLiteralString(string(v))
	case pdfHexString:
		decoded, ok := decodeAnnotationHexTextString([]byte(v))
		if ok {
			return decoded
		}
		return string(v)
	default:
		return fmt.Sprint(v)
	}
}

func annotationTextField(dict pdfDict, key string) string {
	value, ok := dict[key]
	if !ok {
		return ""
	}
	switch v := value.(type) {
	case pdfLiteralString:
		return decodeLiteralString(string(v))
	case pdfHexString:
		decoded, ok := decodeAnnotationHexTextString([]byte(v))
		if ok {
			return decoded
		}
		return ""
	default:
		return ""
	}
}

func decodeAnnotationHexTextString(encoded []byte) (string, bool) {
	decoded, ok := decodeHexBytes(encoded)
	if !ok {
		return "", false
	}
	if len(decoded) >= 2 && decoded[0] == 0xfe && decoded[1] == 0xff {
		compact := make([]byte, 0, len(encoded))
		for _, b := range encoded {
			if !isPDFSpace(b) {
				compact = append(compact, b)
			}
		}
		if len(compact) >= 4 {
			return decodeUTF16BEHex(string(compact[4:])), true
		}
	}
	return string(decoded), true
}
