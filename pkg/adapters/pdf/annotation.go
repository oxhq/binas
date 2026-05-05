package pdf

import (
	"errors"
	"fmt"

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
	RemoveAppearance bool
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
	HasAppearance        bool      `json:"has_appearance"`
	Rect                 []float64 `json:"rect,omitempty"`
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
	verification, err := verifyAnnotationContentsEdit(output, index, contents, graph.pageCount(), appearanceRemoved)
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
		AppearanceRegenerated: false,
		AppearanceInvalidated: appearanceRemoved,
		AppearanceRemoved:     appearanceRemoved,
		AppearanceNote:        annotationContentsEditAppearanceNote(appearanceRemoved),
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

func annotationContentsEditAppearanceNote(appearanceRemoved bool) string {
	if appearanceRemoved {
		return "appearance regeneration is not implemented; stale annotation /AP was removed after updating /Contents"
	}
	return "appearance regeneration is not implemented; only the annotation dictionary /Contents value was updated"
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
		Index:         c.Index,
		Subtype:       annotationSubtype(c.Dict),
		Contents:      c.OldContent,
		HasAppearance: annotationHasAppearance(c.Dict),
		Rect:          annotationRect(c.Dict),
		Flags:         flags,
		FlagNames:     annotationFlagNames(flags),
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

func verifyAnnotationContentsEdit(output []byte, index int, contents string, pageCount int, expectAppearanceRemoved bool) (AnnotationContentsEditVerification, error) {
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
	return AnnotationContentsEditVerification{
		ReparseOK:             true,
		ContentsUpdated:       updated,
		PageUnchanged:         pageCount == 0 || graph.pageCount() == pageCount,
		AppearanceRegenerated: false,
		AppearanceInvalidated: appearanceRemoved,
		AppearanceRemoved:     appearanceRemoved,
	}, nil
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
