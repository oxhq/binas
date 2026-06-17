package pdf

import (
	"bytes"
	"errors"
	"fmt"
	"sort"
)

const (
	pageOperationCopy    = "pdf.copy_pages"
	pageOperationExtract = "pdf.extract_pages"
	pageOperationInsert  = "pdf.insert_pages"
	pageOperationMerge   = "pdf.merge"
)

type PageOperationOptions struct {
	WriterMode PDFWriterMode `json:"writer_mode,omitempty"`
}

type PageOperationReport struct {
	Operation      string                     `json:"operation"`
	InputDocuments int                        `json:"input_documents,omitempty"`
	InputPages     int                        `json:"input_pages"`
	OutputPages    int                        `json:"output_pages"`
	CopiedPages    []PageCopyInfo             `json:"copied_pages,omitempty"`
	Verification   *PageOperationVerification `json:"verification,omitempty"`
}

type PageCopyInfo struct {
	SourceDocument  int `json:"source_document"`
	SourcePageIndex int `json:"source_page_index"`
	OutputPageIndex int `json:"output_page_index"`
}

type PageSource struct {
	Input []byte `json:"-"`
	Pages []int  `json:"pages,omitempty"`
}

type pageOperationCatalogSource struct {
	Graph          *pdfGraph
	SourceDocument int
}

func CopyPages(input []byte, pages []int, options ...PageOperationOptions) ([]byte, PageOperationReport, PageOperationVerification, error) {
	return copyPagesOperation(input, pages, pageOperationCopy, pageOperationOptions(options))
}

func ExtractPages(input []byte, pages []int, options ...PageOperationOptions) ([]byte, PageOperationReport, PageOperationVerification, error) {
	return copyPagesOperation(input, pages, pageOperationExtract, pageOperationOptions(options))
}

func InsertPages(input []byte, index int, sources []PageSource, options ...PageOperationOptions) ([]byte, PageOperationReport, PageOperationVerification, error) {
	opts := pageOperationOptions(options)
	if len(sources) == 0 {
		return nil, PageOperationReport{}, PageOperationVerification{}, errors.New("insert requires at least one page source")
	}
	graph, err := parsePDFGraph(input)
	if err != nil {
		return nil, PageOperationReport{}, PageOperationVerification{}, err
	}
	basePages, err := graph.orderedPages()
	if err != nil {
		return nil, PageOperationReport{}, PageOperationVerification{}, err
	}
	if index < 0 || index > len(basePages) {
		return nil, PageOperationReport{}, PageOperationVerification{}, fmt.Errorf("insert index %d out of range for %d pages", index, len(basePages))
	}

	builder := newPageOperationBuilder()
	catalogSources := []pageOperationCatalogSource{{Graph: graph, SourceDocument: 0}}
	if index > 0 {
		if err := builder.copyPagesFromGraph(graph, basePages, pageIndexRange(0, index), 0); err != nil {
			return nil, PageOperationReport{}, PageOperationVerification{}, err
		}
	}

	totalInputPages := len(basePages)
	for sourceIndex, source := range sources {
		sourceGraph, err := parsePDFGraph(source.Input)
		if err != nil {
			return nil, PageOperationReport{}, PageOperationVerification{}, fmt.Errorf("parse source %d: %w", sourceIndex, err)
		}
		sourcePages, err := sourceGraph.orderedPages()
		if err != nil {
			return nil, PageOperationReport{}, PageOperationVerification{}, fmt.Errorf("source %d page tree: %w", sourceIndex, err)
		}
		catalogSources = append(catalogSources, pageOperationCatalogSource{Graph: sourceGraph, SourceDocument: sourceIndex + 1})
		totalInputPages += len(sourcePages)
		indexes := source.Pages
		if len(indexes) == 0 {
			indexes = allPageIndexes(len(sourcePages))
		}
		if err := builder.copyPagesFromGraph(sourceGraph, sourcePages, indexes, sourceIndex+1); err != nil {
			return nil, PageOperationReport{}, PageOperationVerification{}, err
		}
	}

	if index < len(basePages) {
		if err := builder.copyPagesFromGraph(graph, basePages, pageIndexRange(index, len(basePages)), 0); err != nil {
			return nil, PageOperationReport{}, PageOperationVerification{}, err
		}
	}
	if err := builder.reconcileCatalogFromGraphs(catalogSources); err != nil {
		return nil, PageOperationReport{}, PageOperationVerification{}, err
	}

	output, verification, err := builder.write(opts, len(builder.copiedPages))
	if err != nil {
		return nil, PageOperationReport{}, PageOperationVerification{}, err
	}
	return output, PageOperationReport{
		Operation:      pageOperationInsert,
		InputDocuments: 1 + len(sources),
		InputPages:     totalInputPages,
		OutputPages:    len(builder.copiedPages),
		CopiedPages:    builder.copiedPages,
		Verification:   &verification,
	}, verification, nil
}

func Merge(inputs [][]byte, options ...PageOperationOptions) ([]byte, PageOperationReport, PageOperationVerification, error) {
	opts := pageOperationOptions(options)
	if len(inputs) == 0 {
		return nil, PageOperationReport{}, PageOperationVerification{}, errors.New("merge requires at least one input PDF")
	}
	builder := newPageOperationBuilder()
	catalogSources := make([]pageOperationCatalogSource, 0, len(inputs))
	totalInputPages := 0
	for documentIndex, input := range inputs {
		graph, err := parsePDFGraph(input)
		if err != nil {
			return nil, PageOperationReport{}, PageOperationVerification{}, fmt.Errorf("parse input %d: %w", documentIndex, err)
		}
		pages, err := graph.orderedPages()
		if err != nil {
			return nil, PageOperationReport{}, PageOperationVerification{}, fmt.Errorf("input %d page tree: %w", documentIndex, err)
		}
		catalogSources = append(catalogSources, pageOperationCatalogSource{Graph: graph, SourceDocument: documentIndex})
		totalInputPages += len(pages)
		if err := builder.copyPagesFromGraph(graph, pages, allPageIndexes(len(pages)), documentIndex); err != nil {
			return nil, PageOperationReport{}, PageOperationVerification{}, err
		}
	}
	if err := builder.reconcileCatalogFromGraphs(catalogSources); err != nil {
		return nil, PageOperationReport{}, PageOperationVerification{}, err
	}
	output, verification, err := builder.write(opts, len(builder.copiedPages))
	if err != nil {
		return nil, PageOperationReport{}, PageOperationVerification{}, err
	}
	return output, PageOperationReport{
		Operation:      pageOperationMerge,
		InputDocuments: len(inputs),
		InputPages:     totalInputPages,
		OutputPages:    len(builder.copiedPages),
		CopiedPages:    builder.copiedPages,
		Verification:   &verification,
	}, verification, nil
}

func copyPagesOperation(input []byte, indexes []int, operation string, opts PageOperationOptions) ([]byte, PageOperationReport, PageOperationVerification, error) {
	graph, err := parsePDFGraph(input)
	if err != nil {
		return nil, PageOperationReport{}, PageOperationVerification{}, err
	}
	pages, err := graph.orderedPages()
	if err != nil {
		return nil, PageOperationReport{}, PageOperationVerification{}, err
	}
	builder := newPageOperationBuilder()
	if err := builder.copyPagesFromGraph(graph, pages, indexes, 0); err != nil {
		return nil, PageOperationReport{}, PageOperationVerification{}, err
	}
	if err := builder.preserveCatalogFromGraph(graph, 0); err != nil {
		return nil, PageOperationReport{}, PageOperationVerification{}, err
	}
	output, verification, err := builder.write(opts, len(indexes))
	if err != nil {
		return nil, PageOperationReport{}, PageOperationVerification{}, err
	}
	return output, PageOperationReport{
		Operation:      operation,
		InputDocuments: 1,
		InputPages:     len(pages),
		OutputPages:    len(indexes),
		CopiedPages:    builder.copiedPages,
		Verification:   &verification,
	}, verification, nil
}

func pageOperationOptions(options []PageOperationOptions) PageOperationOptions {
	var out PageOperationOptions
	if len(options) > 0 {
		out = options[0]
	}
	return out
}

func allPageIndexes(count int) []int {
	return pageIndexRange(0, count)
}

func pageIndexRange(start, end int) []int {
	out := make([]int, end-start)
	for i := range out {
		out[i] = start + i
	}
	return out
}

type pageOperationBuilder struct {
	graph       *pdfGraph
	pagesID     pdfObjectID
	nextObject  int
	kids        pdfArray
	copiedPages []PageCopyInfo
	cloners     map[int]*pageObjectCloner
}

func newPageOperationBuilder() *pageOperationBuilder {
	catalogID := pdfObjectID{Number: 1, Generation: 0}
	pagesID := pdfObjectID{Number: 2, Generation: 0}
	graph := &pdfGraph{
		Header:  "%PDF-1.7",
		Objects: make(map[pdfObjectID]*pdfIndirectObject),
		Trailer: pdfDict{"Root": pdfRef{ID: catalogID}},
		Root:    &catalogID,
	}
	graph.Objects[catalogID] = &pdfIndirectObject{
		ID: catalogID,
		Value: pdfDict{
			"Type":  pdfName("Catalog"),
			"Pages": pdfRef{ID: pagesID},
		},
	}
	graph.Objects[pagesID] = &pdfIndirectObject{
		ID: pagesID,
		Value: pdfDict{
			"Type":  pdfName("Pages"),
			"Kids":  pdfArray{},
			"Count": 0,
		},
	}
	return &pageOperationBuilder{
		graph:      graph,
		pagesID:    pagesID,
		nextObject: 3,
		cloners:    make(map[int]*pageObjectCloner),
	}
}

func (b *pageOperationBuilder) copyPagesFromGraph(source *pdfGraph, pages []pdfPageNode, indexes []int, sourceDocument int) error {
	if len(indexes) == 0 {
		return errors.New("at least one page index is required")
	}
	cloner := b.clonerForSource(source, sourceDocument)
	for _, index := range indexes {
		if index < 0 || index >= len(pages) {
			return fmt.Errorf("page index %d out of range for %d pages (zero-based)", index, len(pages))
		}
		pageRef, err := cloner.clonePage(pages[index], b.pagesID)
		if err != nil {
			return fmt.Errorf("copy source document %d page %d: %w", sourceDocument, index, err)
		}
		b.kids = append(b.kids, pageRef)
		b.copiedPages = append(b.copiedPages, PageCopyInfo{
			SourceDocument:  sourceDocument,
			SourcePageIndex: index,
			OutputPageIndex: len(b.copiedPages),
		})
	}
	return nil
}

func (b *pageOperationBuilder) clonerForSource(source *pdfGraph, sourceDocument int) *pageObjectCloner {
	if cloner, ok := b.cloners[sourceDocument]; ok && cloner.source == source {
		return cloner
	}
	cloner := &pageObjectCloner{
		source: source,
		target: b.graph,
		remap:  make(map[pdfObjectID]pdfObjectID),
		next:   &b.nextObject,
	}
	b.cloners[sourceDocument] = cloner
	return cloner
}

func (b *pageOperationBuilder) preserveCatalogFromGraph(source *pdfGraph, sourceDocument int) error {
	return b.reconcileCatalogFromGraphs([]pageOperationCatalogSource{{Graph: source, SourceDocument: sourceDocument}})
}

func (b *pageOperationBuilder) reconcileCatalogFromGraphs(sources []pageOperationCatalogSource) error {
	if len(sources) == 0 {
		return nil
	}
	targetCatalog, ok := b.graph.objectDict(pdfObjectID{Number: 1, Generation: 0})
	if !ok {
		return errors.New("page operation target catalog is missing")
	}
	if err := b.preservePrimaryCatalogEntries(targetCatalog, sources[0]); err != nil {
		return err
	}
	if err := b.reconcileNameTrees(targetCatalog, sources); err != nil {
		return err
	}
	if err := b.reconcilePageLabels(targetCatalog, sources); err != nil {
		return err
	}
	if err := b.reconcileOutlines(targetCatalog, sources); err != nil {
		return err
	}
	if err := b.reconcileAcroForm(targetCatalog, sources); err != nil {
		return err
	}
	return nil
}

func (b *pageOperationBuilder) preservePrimaryCatalogEntries(targetCatalog pdfDict, source pageOperationCatalogSource) error {
	sourceCatalog, ok := source.Graph.catalogDict()
	if !ok {
		return nil
	}
	cloner := b.clonerForSource(source.Graph, source.SourceDocument)
	for _, key := range pageOperationPrimaryCatalogKeys() {
		value, ok := sourceCatalog[key]
		if !ok {
			continue
		}
		cloned, err := cloner.cloneValue(value)
		if err != nil {
			return fmt.Errorf("preserve catalog /%s: %w", key, err)
		}
		targetCatalog[key] = cloned
	}
	return nil
}

func (b *pageOperationBuilder) reconcileNameTrees(targetCatalog pdfDict, sources []pageOperationCatalogSource) error {
	trees := make(map[string]pdfArray)
	seen := make(map[string]map[string]bool)
	addEntry := func(kind string, key pdfValue, value pdfValue, sourceDocument int) {
		if seen[kind] == nil {
			seen[kind] = make(map[string]bool)
		}
		keyString := pdfNameTreeKeyForMerge(key)
		if seen[kind][keyString] {
			key = pdfLiteralString(fmt.Sprintf("%s#doc%d", keyString, sourceDocument))
			keyString = pdfNameTreeKeyForMerge(key)
		}
		seen[kind][keyString] = true
		trees[kind] = append(trees[kind], key, value)
	}

	for _, source := range sources {
		sourceCatalog, ok := source.Graph.catalogDict()
		if !ok {
			continue
		}
		cloner := b.clonerForSource(source.Graph, source.SourceDocument)
		if namesDict, ok := resolvePDFDictFromGraph(source.Graph, sourceCatalog["Names"]); ok {
			keys := sortedPDFDictKeys(namesDict)
			for _, kind := range keys {
				pairs, err := collectPDFNameTreePairs(source.Graph, namesDict[kind], nil)
				if err != nil {
					return fmt.Errorf("merge catalog /Names /%s: %w", kind, err)
				}
				for _, pair := range pairs {
					key, err := cloner.cloneValue(pair.Key)
					if err != nil {
						return fmt.Errorf("merge catalog /Names /%s key: %w", kind, err)
					}
					value, err := cloner.cloneValue(pair.Value)
					if err != nil {
						return fmt.Errorf("merge catalog /Names /%s value: %w", kind, err)
					}
					addEntry(kind, key, value, source.SourceDocument)
				}
			}
		}
		if destsValue, ok := sourceCatalog["Dests"]; ok {
			pairs, err := collectPDFNameTreePairs(source.Graph, destsValue, nil)
			if err != nil {
				return fmt.Errorf("merge catalog /Dests: %w", err)
			}
			for _, pair := range pairs {
				key, err := cloner.cloneValue(pair.Key)
				if err != nil {
					return fmt.Errorf("merge catalog /Dests key: %w", err)
				}
				value, err := cloner.cloneValue(pair.Value)
				if err != nil {
					return fmt.Errorf("merge catalog /Dests value: %w", err)
				}
				addEntry("Dests", key, value, source.SourceDocument)
			}
		}
	}
	if len(trees) == 0 {
		return nil
	}
	names := make(pdfDict, len(trees))
	for _, kind := range sortedStringKeys(trees) {
		names[kind] = pdfDict{"Names": trees[kind]}
	}
	targetCatalog["Names"] = names
	return nil
}

func (b *pageOperationBuilder) reconcilePageLabels(targetCatalog pdfDict, sources []pageOperationCatalogSource) error {
	labels := make(map[int]pdfValue)
	for _, source := range sources {
		sourceCatalog, ok := source.Graph.catalogDict()
		if !ok {
			continue
		}
		pairs, err := collectPDFNumberTreePairs(source.Graph, sourceCatalog["PageLabels"], nil)
		if err != nil {
			return fmt.Errorf("merge catalog /PageLabels: %w", err)
		}
		if len(pairs) == 0 {
			continue
		}
		cloner := b.clonerForSource(source.Graph, source.SourceDocument)
		for _, pair := range pairs {
			outputIndex, ok := b.outputIndexForSourcePage(source.SourceDocument, pair.Key)
			if !ok {
				continue
			}
			cloned, err := cloner.cloneValue(pair.Value)
			if err != nil {
				return fmt.Errorf("merge catalog /PageLabels entry %d: %w", pair.Key, err)
			}
			labels[outputIndex] = cloned
		}
	}
	if len(labels) == 0 {
		return nil
	}
	keys := make([]int, 0, len(labels))
	for key := range labels {
		keys = append(keys, key)
	}
	sort.Ints(keys)
	nums := make(pdfArray, 0, len(keys)*2)
	for _, key := range keys {
		nums = append(nums, key, labels[key])
	}
	targetCatalog["PageLabels"] = pdfDict{"Nums": nums}
	return nil
}

func (b *pageOperationBuilder) reconcileOutlines(targetCatalog pdfDict, sources []pageOperationCatalogSource) error {
	outlineItems := make([]pdfRef, 0)
	for _, source := range sources {
		sourceCatalog, ok := source.Graph.catalogDict()
		if !ok {
			continue
		}
		outlineRoot, ok := resolvePDFDictFromGraph(source.Graph, sourceCatalog["Outlines"])
		if !ok {
			continue
		}
		items, err := collectPDFOutlineItems(source.Graph, outlineRoot["First"], nil)
		if err != nil {
			return fmt.Errorf("merge catalog /Outlines: %w", err)
		}
		cloner := b.clonerForSource(source.Graph, source.SourceDocument)
		for _, item := range items {
			cloned, err := cloner.cloneOutlineItem(item)
			if err != nil {
				return fmt.Errorf("merge catalog /Outlines item: %w", err)
			}
			outlineItems = append(outlineItems, cloned)
		}
	}
	if len(outlineItems) == 0 {
		return nil
	}
	rootID := b.allocateAnonymousObject()
	root := pdfDict{"Type": pdfName("Outlines"), "Count": len(outlineItems)}
	root["First"] = outlineItems[0]
	root["Last"] = outlineItems[len(outlineItems)-1]
	for i, item := range outlineItems {
		dict, ok := b.graph.objectDict(item.ID)
		if !ok {
			return fmt.Errorf("merged outline item %d %d is missing", item.ID.Number, item.ID.Generation)
		}
		dict["Parent"] = pdfRef{ID: rootID}
		if i > 0 {
			dict["Prev"] = outlineItems[i-1]
		}
		if i+1 < len(outlineItems) {
			dict["Next"] = outlineItems[i+1]
		}
	}
	b.graph.Objects[rootID] = &pdfIndirectObject{ID: rootID, Value: root}
	targetCatalog["Outlines"] = pdfRef{ID: rootID}
	return nil
}

func (b *pageOperationBuilder) reconcileAcroForm(targetCatalog pdfDict, sources []pageOperationCatalogSource) error {
	var acroForm pdfDict
	fields := make(pdfArray, 0)
	for _, source := range sources {
		sourceCatalog, ok := source.Graph.catalogDict()
		if !ok {
			continue
		}
		sourceAcroForm, ok := resolvePDFDictFromGraph(source.Graph, sourceCatalog["AcroForm"])
		if !ok {
			continue
		}
		cloner := b.clonerForSource(source.Graph, source.SourceDocument)
		if acroForm == nil {
			acroForm = make(pdfDict, len(sourceAcroForm))
		}
		for _, key := range sortedPDFDictKeys(sourceAcroForm) {
			if key == "Fields" {
				continue
			}
			if _, exists := acroForm[key]; exists && key != "NeedAppearances" {
				continue
			}
			cloned, err := cloner.cloneValue(sourceAcroForm[key])
			if err != nil {
				return fmt.Errorf("merge catalog /AcroForm /%s: %w", key, err)
			}
			if key == "NeedAppearances" {
				if boolValue, ok := cloned.(bool); ok && boolValue {
					acroForm[key] = true
				} else if _, exists := acroForm[key]; !exists {
					acroForm[key] = cloned
				}
				continue
			}
			acroForm[key] = cloned
		}
		if sourceFields, ok := sourceAcroForm["Fields"].(pdfArray); ok {
			for _, field := range sourceFields {
				cloned, err := cloner.cloneValue(field)
				if err != nil {
					return fmt.Errorf("merge catalog /AcroForm /Fields: %w", err)
				}
				fields = append(fields, cloned)
			}
		}
	}
	if acroForm == nil {
		return nil
	}
	acroForm["Fields"] = fields
	targetCatalog["AcroForm"] = acroForm
	return nil
}

func (b *pageOperationBuilder) outputIndexForSourcePage(sourceDocument, sourcePageIndex int) (int, bool) {
	for _, page := range b.copiedPages {
		if page.SourceDocument == sourceDocument && page.SourcePageIndex == sourcePageIndex {
			return page.OutputPageIndex, true
		}
	}
	return 0, false
}

func (b *pageOperationBuilder) allocateAnonymousObject() pdfObjectID {
	id := pdfObjectID{Number: b.nextObject, Generation: 0}
	b.nextObject++
	return id
}

func pageOperationPrimaryCatalogKeys() []string {
	return []string{
		"Metadata",
		"OpenAction",
		"ViewerPreferences",
		"PageMode",
		"PageLayout",
		"Lang",
		"MarkInfo",
	}
}

type pdfNameTreePair struct {
	Key   pdfValue
	Value pdfValue
}

type pdfNumberTreePair struct {
	Key   int
	Value pdfValue
}

func resolvePDFDictFromGraph(graph *pdfGraph, value pdfValue) (pdfDict, bool) {
	if ref, ok := value.(pdfRef); ok {
		return graph.objectDict(ref.ID)
	}
	return objectDictValue(value)
}

func collectPDFNameTreePairs(graph *pdfGraph, value pdfValue, seen map[pdfObjectID]bool) ([]pdfNameTreePair, error) {
	if value == nil {
		return nil, nil
	}
	if ref, ok := value.(pdfRef); ok {
		if seen == nil {
			seen = make(map[pdfObjectID]bool)
		}
		if seen[ref.ID] {
			return nil, fmt.Errorf("name tree cycle at object %d %d", ref.ID.Number, ref.ID.Generation)
		}
		seen[ref.ID] = true
		object := graph.Objects[ref.ID]
		if object == nil {
			return nil, fmt.Errorf("name tree object %d %d is missing", ref.ID.Number, ref.ID.Generation)
		}
		value = object.Value
	}
	dict, ok := objectDictValue(value)
	if !ok {
		return nil, fmt.Errorf("name tree node is %T, want dictionary", value)
	}
	out := make([]pdfNameTreePair, 0)
	if names, ok := dict["Names"].(pdfArray); ok {
		if len(names)%2 != 0 {
			return nil, errors.New("name tree /Names array has odd length")
		}
		for i := 0; i < len(names); i += 2 {
			out = append(out, pdfNameTreePair{Key: names[i], Value: names[i+1]})
		}
	}
	if kids, ok := dict["Kids"].(pdfArray); ok {
		for _, kid := range kids {
			pairs, err := collectPDFNameTreePairs(graph, kid, clonePDFObjectIDSet(seen))
			if err != nil {
				return nil, err
			}
			out = append(out, pairs...)
		}
	}
	if len(out) == 0 {
		for _, key := range sortedPDFDictKeys(dict) {
			if key == "Limits" || key == "Kids" || key == "Names" {
				continue
			}
			out = append(out, pdfNameTreePair{Key: pdfLiteralString(key), Value: dict[key]})
		}
	}
	return out, nil
}

func collectPDFNumberTreePairs(graph *pdfGraph, value pdfValue, seen map[pdfObjectID]bool) ([]pdfNumberTreePair, error) {
	if value == nil {
		return nil, nil
	}
	if ref, ok := value.(pdfRef); ok {
		if seen == nil {
			seen = make(map[pdfObjectID]bool)
		}
		if seen[ref.ID] {
			return nil, fmt.Errorf("number tree cycle at object %d %d", ref.ID.Number, ref.ID.Generation)
		}
		seen[ref.ID] = true
		object := graph.Objects[ref.ID]
		if object == nil {
			return nil, fmt.Errorf("number tree object %d %d is missing", ref.ID.Number, ref.ID.Generation)
		}
		value = object.Value
	}
	dict, ok := objectDictValue(value)
	if !ok {
		return nil, fmt.Errorf("number tree node is %T, want dictionary", value)
	}
	out := make([]pdfNumberTreePair, 0)
	if nums, ok := dict["Nums"].(pdfArray); ok {
		if len(nums)%2 != 0 {
			return nil, errors.New("number tree /Nums array has odd length")
		}
		for i := 0; i < len(nums); i += 2 {
			key, ok := pdfNumberTreeKey(nums[i])
			if !ok {
				return nil, fmt.Errorf("number tree key %T is not an integer", nums[i])
			}
			out = append(out, pdfNumberTreePair{Key: key, Value: nums[i+1]})
		}
	}
	if kids, ok := dict["Kids"].(pdfArray); ok {
		for _, kid := range kids {
			pairs, err := collectPDFNumberTreePairs(graph, kid, clonePDFObjectIDSet(seen))
			if err != nil {
				return nil, err
			}
			out = append(out, pairs...)
		}
	}
	return out, nil
}

func pdfNumberTreeKey(value pdfValue) (int, bool) {
	switch v := value.(type) {
	case int:
		return v, true
	case int64:
		return int(v), true
	case float64:
		if v == float64(int(v)) {
			return int(v), true
		}
	}
	return 0, false
}

func collectPDFOutlineItems(graph *pdfGraph, first pdfValue, seen map[pdfObjectID]bool) ([]pdfValue, error) {
	items := make([]pdfValue, 0)
	current := first
	for current != nil {
		if ref, ok := current.(pdfRef); ok {
			if seen == nil {
				seen = make(map[pdfObjectID]bool)
			}
			if seen[ref.ID] {
				return nil, fmt.Errorf("outline cycle at object %d %d", ref.ID.Number, ref.ID.Generation)
			}
			seen[ref.ID] = true
			object := graph.Objects[ref.ID]
			if object == nil {
				return nil, fmt.Errorf("outline object %d %d is missing", ref.ID.Number, ref.ID.Generation)
			}
			items = append(items, current)
			dict, ok := objectDictValue(object.Value)
			if !ok {
				return nil, fmt.Errorf("outline object %d %d is not a dictionary", ref.ID.Number, ref.ID.Generation)
			}
			next, ok := dict["Next"]
			if !ok {
				break
			}
			current = next
			continue
		}
		dict, ok := objectDictValue(current)
		if !ok {
			return nil, fmt.Errorf("outline item is %T, want dictionary", current)
		}
		items = append(items, current)
		next, ok := dict["Next"]
		if !ok {
			break
		}
		current = next
	}
	return items, nil
}

func (c *pageObjectCloner) cloneOutlineItem(value pdfValue) (pdfRef, error) {
	var newID pdfObjectID
	if ref, ok := value.(pdfRef); ok {
		object := c.source.Objects[ref.ID]
		if object == nil {
			return pdfRef{}, fmt.Errorf("outline object %d %d is missing", ref.ID.Number, ref.ID.Generation)
		}
		value = object.Value
		newID = c.allocate(ref.ID)
	} else {
		newID = pdfObjectID{Number: *c.next, Generation: 0}
		*c.next = *c.next + 1
	}
	dict, ok := objectDictValue(value)
	if !ok {
		return pdfRef{}, fmt.Errorf("outline item is %T, want dictionary", value)
	}
	out := make(pdfDict, len(dict))
	for _, key := range sortedPDFDictKeys(dict) {
		switch key {
		case "Parent", "Prev", "Next", "First", "Last", "Count":
			continue
		}
		cloned, err := c.cloneValue(dict[key])
		if err != nil {
			return pdfRef{}, fmt.Errorf("/%s: %w", key, err)
		}
		out[key] = cloned
	}
	c.target.Objects[newID] = &pdfIndirectObject{ID: newID, Value: out}
	return pdfRef{ID: newID}, nil
}

func pdfNameTreeKeyForMerge(value pdfValue) string {
	switch v := value.(type) {
	case pdfLiteralString:
		return string(v)
	case pdfHexString:
		return string(v)
	case pdfName:
		return string(v)
	default:
		return fmt.Sprint(v)
	}
}

func sortedPDFDictKeys(dict pdfDict) []string {
	keys := make([]string, 0, len(dict))
	for key := range dict {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	return keys
}

func sortedStringKeys(values map[string]pdfArray) []string {
	keys := make([]string, 0, len(values))
	for key := range values {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	return keys
}

func (b *pageOperationBuilder) write(opts PageOperationOptions, expectedPages int) ([]byte, PageOperationVerification, error) {
	pagesObject := b.graph.Objects[b.pagesID]
	pagesObject.Value = pdfDict{
		"Type":  pdfName("Pages"),
		"Kids":  b.kids,
		"Count": len(b.kids),
	}
	output, err := writePDFGraphWithOptions(b.graph, pdfCanonicalWriteOptions{WriterMode: opts.WriterMode})
	if err != nil {
		return nil, PageOperationVerification{}, err
	}
	verification, err := verifyPageOperationOutput(output, expectedPages)
	if err != nil {
		return nil, verification, err
	}
	return output, verification, nil
}

type pdfPageNode struct {
	ID        pdfObjectID
	Dict      pdfDict
	Inherited map[string]pdfValue
}

func (g *pdfGraph) orderedPages() ([]pdfPageNode, error) {
	catalog, ok := g.catalogDict()
	if !ok {
		return nil, errors.New("missing PDF catalog")
	}
	pagesRef, ok := dictRef(catalog, "Pages")
	if !ok {
		return nil, errors.New("catalog missing /Pages reference")
	}
	var out []pdfPageNode
	if err := g.collectOrderedPages(pagesRef.ID, nil, nil, &out); err != nil {
		return nil, err
	}
	if len(out) == 0 {
		return nil, errors.New("page tree contains no pages")
	}
	return out, nil
}

func (g *pdfGraph) collectOrderedPages(id pdfObjectID, inherited map[string]pdfValue, seen map[pdfObjectID]bool, out *[]pdfPageNode) error {
	if seen == nil {
		seen = make(map[pdfObjectID]bool)
	}
	if seen[id] {
		return fmt.Errorf("page tree cycle at object %d %d", id.Number, id.Generation)
	}
	seen[id] = true
	object := g.Objects[id]
	if object == nil {
		return fmt.Errorf("page tree object %d %d is missing", id.Number, id.Generation)
	}
	dict, ok := objectDictValue(object.Value)
	if !ok {
		return fmt.Errorf("page tree object %d %d is not a dictionary", id.Number, id.Generation)
	}
	nextInherited := mergePageInheritedValues(inherited, dict)
	if dictHasType(dict, "Page") {
		*out = append(*out, pdfPageNode{ID: id, Dict: dict, Inherited: inherited})
		return nil
	}
	if !dictHasType(dict, "Pages") {
		return fmt.Errorf("page tree object %d %d is neither /Page nor /Pages", id.Number, id.Generation)
	}
	kids, ok := dict["Kids"].(pdfArray)
	if !ok {
		return fmt.Errorf("page tree object %d %d missing /Kids array", id.Number, id.Generation)
	}
	for _, kid := range kids {
		ref, ok := kid.(pdfRef)
		if !ok {
			return fmt.Errorf("page tree object %d %d has non-reference kid", id.Number, id.Generation)
		}
		if err := g.collectOrderedPages(ref.ID, nextInherited, clonePDFObjectIDSet(seen), out); err != nil {
			return err
		}
	}
	return nil
}

func mergePageInheritedValues(parent map[string]pdfValue, dict pdfDict) map[string]pdfValue {
	out := make(map[string]pdfValue, len(parent)+6)
	for key, value := range parent {
		out[key] = value
	}
	for _, key := range []string{"Resources", "MediaBox", "CropBox", "Rotate", "ArtBox", "BleedBox", "TrimBox"} {
		if value, ok := dict[key]; ok {
			out[key] = value
		}
	}
	return out
}

type pageObjectCloner struct {
	source *pdfGraph
	target *pdfGraph
	remap  map[pdfObjectID]pdfObjectID
	next   *int
}

func (c *pageObjectCloner) clonePage(page pdfPageNode, parentID pdfObjectID) (pdfRef, error) {
	newID := c.allocate(page.ID)
	sourceObject := c.source.Objects[page.ID]
	if sourceObject == nil {
		return pdfRef{}, fmt.Errorf("source page object %d %d is missing", page.ID.Number, page.ID.Generation)
	}
	dict, ok := objectDictValue(sourceObject.Value)
	if !ok {
		return pdfRef{}, fmt.Errorf("source page object %d %d is not a dictionary", page.ID.Number, page.ID.Generation)
	}
	pageDict := clonePDFDict(dict)
	for key, value := range page.Inherited {
		if _, exists := pageDict[key]; !exists {
			pageDict[key] = value
		}
	}
	delete(pageDict, "Parent")
	cloned, err := c.cloneDict(pageDict)
	if err != nil {
		return pdfRef{}, err
	}
	cloned["Parent"] = pdfRef{ID: parentID}
	cloned["Type"] = pdfName("Page")
	c.target.Objects[newID] = &pdfIndirectObject{ID: newID, Value: cloned}
	return pdfRef{ID: newID}, nil
}

func (c *pageObjectCloner) cloneObject(id pdfObjectID) (pdfRef, error) {
	if mapped, ok := c.remap[id]; ok {
		return pdfRef{ID: mapped}, nil
	}
	sourceObject := c.source.Objects[id]
	if sourceObject == nil {
		return pdfRef{}, fmt.Errorf("referenced object %d %d is missing", id.Number, id.Generation)
	}
	newID := c.allocate(id)
	c.target.Objects[newID] = &pdfIndirectObject{ID: newID}
	value, err := c.cloneValue(sourceObject.Value)
	if err != nil {
		return pdfRef{}, fmt.Errorf("clone object %d %d: %w", id.Number, id.Generation, err)
	}
	c.target.Objects[newID].Value = value
	return pdfRef{ID: newID}, nil
}

func (c *pageObjectCloner) allocate(id pdfObjectID) pdfObjectID {
	if mapped, ok := c.remap[id]; ok {
		return mapped
	}
	newID := pdfObjectID{Number: *c.next, Generation: 0}
	*c.next = *c.next + 1
	c.remap[id] = newID
	return newID
}

func (c *pageObjectCloner) cloneValue(value pdfValue) (pdfValue, error) {
	switch v := value.(type) {
	case nil, bool, int, int64, float64, pdfName, pdfLiteralString, pdfHexString, pdfRawObject:
		return v, nil
	case pdfDecryptedString:
		return pdfDecryptedString(bytes.Clone(v)), nil
	case pdfRef:
		return c.cloneObject(v.ID)
	case pdfArray:
		out := make(pdfArray, len(v))
		for i, item := range v {
			cloned, err := c.cloneValue(item)
			if err != nil {
				return nil, err
			}
			out[i] = cloned
		}
		return out, nil
	case pdfDict:
		return c.cloneDict(v)
	case pdfStreamObject:
		dict, err := c.cloneDict(v.Dict)
		if err != nil {
			return nil, err
		}
		delete(dict, "Length")
		return pdfStreamObject{Dict: dict, Data: bytes.Clone(v.Data)}, nil
	default:
		return nil, fmt.Errorf("unsupported PDF value type %T", value)
	}
}

func (c *pageObjectCloner) cloneDict(dict pdfDict) (pdfDict, error) {
	out := make(pdfDict, len(dict))
	for key, value := range dict {
		cloned, err := c.cloneValue(value)
		if err != nil {
			return nil, fmt.Errorf("/%s: %w", key, err)
		}
		out[key] = cloned
	}
	return out, nil
}

func verifyPageOperationOutput(output []byte, expectedPages int) (PageOperationVerification, error) {
	verification, err := VerifyPageOperationOutput(output, PageOperationVerificationOptions{
		ExpectedPageCount:  expectedPages,
		RequirePageContent: expectedPages > 0,
		RequireResources:   expectedPages > 0,
	})
	if err != nil {
		return PageOperationVerification{}, err
	}
	if !verification.PageCountOK {
		return verification, fmt.Errorf("page operation wrote %d pages, want %d", verification.ActualPageCount, expectedPages)
	}
	if !verification.NoDanglingRefs {
		return verification, errors.New("page operation wrote dangling indirect references")
	}
	return verification, nil
}
