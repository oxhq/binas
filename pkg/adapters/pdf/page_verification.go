package pdf

import (
	"fmt"
	"sort"

	"github.com/oxhq/binas/pkg/core"
)

type PageOperationVerificationOptions struct {
	ExpectedPageCount  int
	ExpectedText       []string
	RequirePageContent bool
	RequireResources   bool
}

type PageOperationVerification struct {
	ReparseOK            bool                       `json:"reparse_ok"`
	PageCountOK          bool                       `json:"page_count_ok"`
	PageCount            int                        `json:"page_count"`
	ExpectedPageCount    int                        `json:"expected_page_count,omitempty"`
	ActualPageCount      int                        `json:"actual_page_count"`
	PageContentAvailable bool                       `json:"page_content_available"`
	ResourcesAvailable   bool                       `json:"resources_available"`
	TextAvailable        bool                       `json:"text_available"`
	MissingText          []string                   `json:"missing_text,omitempty"`
	NoDanglingRefs       bool                       `json:"no_dangling_refs"`
	DanglingRefs         []PageOperationDanglingRef `json:"dangling_refs,omitempty"`
}

type PageOperationDanglingRef struct {
	ObjectNumber int    `json:"object_number"`
	Generation   int    `json:"generation"`
	Path         string `json:"path"`
}

func VerifyPageOperationOutput(output []byte, opts PageOperationVerificationOptions) (PageOperationVerification, error) {
	graph, err := parsePDFGraph(output)
	if err != nil {
		return PageOperationVerification{}, err
	}

	actualPageCount := graph.pageCount()
	pageCountOK := true
	if opts.ExpectedPageCount > 0 {
		pageCountOK = actualPageCount == opts.ExpectedPageCount
	}

	missingText, err := missingPageOperationText(graph, opts.ExpectedText)
	if err != nil {
		return PageOperationVerification{}, err
	}
	danglingRefs := graph.danglingIndirectReferences()

	return PageOperationVerification{
		ReparseOK:            true,
		PageCountOK:          pageCountOK,
		PageCount:            actualPageCount,
		ExpectedPageCount:    opts.ExpectedPageCount,
		ActualPageCount:      actualPageCount,
		PageContentAvailable: !opts.RequirePageContent || graph.pageContentAvailable(),
		ResourcesAvailable:   !opts.RequireResources || graph.pageResourcesAvailable(),
		TextAvailable:        len(missingText) == 0,
		MissingText:          missingText,
		NoDanglingRefs:       len(danglingRefs) == 0,
		DanglingRefs:         danglingRefs,
	}, nil
}

func (v PageOperationVerification) CoreVerification() core.Verification {
	return core.Verification{
		ReparseOK:     v.ReparseOK,
		NewSelectable: v.TextAvailable,
		PageUnchanged: v.PageCountOK,
	}
}

func (v PageOperationVerification) Metadata() map[string]any {
	meta := map[string]any{
		"reparse_ok":             v.ReparseOK,
		"page_count_ok":          v.PageCountOK,
		"page_count":             v.PageCount,
		"actual_page_count":      v.ActualPageCount,
		"page_content_available": v.PageContentAvailable,
		"resources_available":    v.ResourcesAvailable,
		"text_available":         v.TextAvailable,
		"no_dangling_refs":       v.NoDanglingRefs,
		"dangling_ref_count":     len(v.DanglingRefs),
		"missing_text_count":     len(v.MissingText),
	}
	if v.ExpectedPageCount > 0 {
		meta["expected_page_count"] = v.ExpectedPageCount
	}
	if len(v.MissingText) > 0 {
		meta["missing_text"] = append([]string(nil), v.MissingText...)
	}
	if len(v.DanglingRefs) > 0 {
		meta["dangling_refs"] = append([]PageOperationDanglingRef(nil), v.DanglingRefs...)
	}
	return meta
}

func missingPageOperationText(graph *pdfGraph, expected []string) ([]string, error) {
	if len(expected) == 0 {
		return nil, nil
	}
	missing := make([]string, 0)
	cmapContext := graph.cmapContext()
	for _, text := range expected {
		candidates, err := graph.textShowCandidatesWithCMapContext(text, cmapContext)
		if err != nil {
			return nil, err
		}
		if len(candidates) == 0 {
			missing = append(missing, text)
		}
	}
	return missing, nil
}

func (g *pdfGraph) pageResourcesAvailable() bool {
	for _, page := range g.pageDictionaries() {
		if _, _, ok := g.pageResourcesWithInheritance(page); !ok {
			return false
		}
	}
	return true
}

func (g *pdfGraph) pageContentAvailable() bool {
	for _, page := range g.pageDictionaries() {
		if len(g.pageContentStreamObjects(page)) == 0 {
			return false
		}
	}
	return true
}

func (g *pdfGraph) pageDictionaries() []pdfDict {
	pages := make([]pdfDict, 0)
	for _, object := range sortedPDFObjects(g.Objects) {
		switch value := object.Value.(type) {
		case pdfDict:
			if dictHasType(value, "Page") {
				pages = append(pages, value)
			}
		case pdfStreamObject:
			if dictHasType(value.Dict, "Page") {
				pages = append(pages, value.Dict)
			}
		}
	}
	return pages
}

func (g *pdfGraph) danglingIndirectReferences() []PageOperationDanglingRef {
	found := make(map[pdfObjectID]PageOperationDanglingRef)
	if g.Trailer != nil {
		g.collectDanglingIndirectReferences(g.Trailer, "trailer", found, nil)
	}
	for _, object := range sortedPDFObjects(g.Objects) {
		path := fmt.Sprintf("object %d %d", object.ID.Number, object.ID.Generation)
		g.collectDanglingIndirectReferences(object.Value, path, found, nil)
	}

	out := make([]PageOperationDanglingRef, 0, len(found))
	for _, ref := range found {
		out = append(out, ref)
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i].ObjectNumber != out[j].ObjectNumber {
			return out[i].ObjectNumber < out[j].ObjectNumber
		}
		if out[i].Generation != out[j].Generation {
			return out[i].Generation < out[j].Generation
		}
		return out[i].Path < out[j].Path
	})
	return out
}

func (g *pdfGraph) collectDanglingIndirectReferences(value pdfValue, path string, found map[pdfObjectID]PageOperationDanglingRef, seen map[pdfObjectID]bool) {
	switch v := value.(type) {
	case pdfRef:
		if _, ok := g.Objects[v.ID]; !ok {
			if _, exists := found[v.ID]; !exists {
				found[v.ID] = PageOperationDanglingRef{
					ObjectNumber: v.ID.Number,
					Generation:   v.ID.Generation,
					Path:         path,
				}
			}
			return
		}
		if seen == nil {
			seen = make(map[pdfObjectID]bool)
		}
		if seen[v.ID] {
			return
		}
		nextSeen := clonePDFObjectIDSet(seen)
		nextSeen[v.ID] = true
		g.collectDanglingIndirectReferences(g.Objects[v.ID].Value, fmt.Sprintf("%s -> %d %d R", path, v.ID.Number, v.ID.Generation), found, nextSeen)
	case pdfDict:
		keys := make([]string, 0, len(v))
		for key := range v {
			keys = append(keys, key)
		}
		sort.Strings(keys)
		for _, key := range keys {
			g.collectDanglingIndirectReferences(v[key], path+"/"+key, found, seen)
		}
	case pdfArray:
		for i, item := range v {
			g.collectDanglingIndirectReferences(item, fmt.Sprintf("%s[%d]", path, i), found, seen)
		}
	case pdfStreamObject:
		g.collectDanglingIndirectReferences(v.Dict, path+"/stream_dict", found, seen)
	}
}
