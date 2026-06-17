package pdf

import (
	"bytes"
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"testing"
)

func TestGraphPageTreeNestedFixtureTraversesCatalogOrder(t *testing.T) {
	graph := readGraphTraversalFixture(t)
	catalog := mustGraphCatalog(t, graph)

	pages := collectGraphPageTreeRefs(t, graph, catalog["Pages"])
	want := []pdfObjectID{
		{Number: 4, Generation: 0},
		{Number: 5, Generation: 0},
		{Number: 7, Generation: 0},
	}
	if !reflect.DeepEqual(pages, want) {
		t.Fatalf("page tree refs = %+v, want %+v", pages, want)
	}
	if got := graph.pageCount(); got != len(want) {
		t.Fatalf("graph page count = %d, want %d", got, len(want))
	}

	pageDict := mustGraphDictObject(t, graph, pages[1])
	resources, inherited, ok := graph.pageResourcesWithInheritance(pageDict)
	if !ok || !inherited {
		t.Fatalf("page resources inherited=%t ok=%t resources=%+v, want inherited catalog page resources", inherited, ok, resources)
	}
	procSet, ok := resources["ProcSet"].(pdfArray)
	if !ok || len(procSet) != 2 || procSet[0] != pdfName("PDF") || procSet[1] != pdfName("Text") {
		t.Fatalf("inherited ProcSet = %+v, want [/PDF /Text]", resources["ProcSet"])
	}
}

func TestGraphNameTreeCatalogEntriesIncludeDestinationsAttachmentsAndJavaScript(t *testing.T) {
	graph := readGraphTraversalFixture(t)
	catalog := mustGraphCatalog(t, graph)

	names := mustGraphDict(t, graph, catalog["Names"])
	dests := collectGraphNameTreeEntries(t, graph, names["Dests"])
	if got := sortedGraphNameTreeKeys(dests); !reflect.DeepEqual(got, []string{"chapter1", "chapter2"}) {
		t.Fatalf("destination name keys = %v, want chapter1/chapter2", got)
	}
	chapter1, ok := dests["chapter1"].(pdfArray)
	if !ok || len(chapter1) == 0 || chapter1[0] != (pdfRef{ID: pdfObjectID{Number: 4, Generation: 0}}) {
		t.Fatalf("chapter1 destination = %+v, want page 4 destination array", dests["chapter1"])
	}
	if dests["chapter2"] != (pdfRef{ID: pdfObjectID{Number: 7, Generation: 0}}) {
		t.Fatalf("chapter2 destination = %+v, want page 7 ref", dests["chapter2"])
	}

	embedded := collectGraphNameTreeEntries(t, graph, names["EmbeddedFiles"])
	fileSpec := mustGraphDict(t, graph, embedded["invoice.txt"])
	if got := graphPDFString(t, fileSpec["F"]); got != "invoice.txt" {
		t.Fatalf("embedded file name = %q, want invoice.txt", got)
	}
	ef := mustGraphDict(t, graph, fileSpec["EF"])
	attachment := mustGraphStream(t, graph, ef["F"])
	if !bytes.Contains(attachment.Data, []byte("attachment bytes")) {
		t.Fatalf("embedded file stream data = %q, want attachment bytes", attachment.Data)
	}

	scripts := collectGraphNameTreeEntries(t, graph, names["JavaScript"])
	action := mustGraphDict(t, graph, scripts["openAction"])
	if action["S"] != pdfName("JavaScript") {
		t.Fatalf("JavaScript action subtype = %+v, want /JavaScript", action["S"])
	}
	if got := graphPDFString(t, action["JS"]); got != "app.alert('ready')" {
		t.Fatalf("JavaScript action = %q, want app.alert('ready')", got)
	}
}

func TestGraphCatalogMetadataOutlineAndLabelFixture(t *testing.T) {
	graph := readGraphTraversalFixture(t)
	catalog := mustGraphCatalog(t, graph)

	if graph.Root == nil || *graph.Root != (pdfObjectID{Number: 1, Generation: 0}) {
		t.Fatalf("graph root = %+v, want catalog object 1 0", graph.Root)
	}
	for _, key := range []string{"Pages", "Names", "Outlines", "PageLabels", "Metadata", "OpenAction"} {
		if _, ok := catalog[key]; !ok {
			t.Fatalf("catalog missing /%s: %+v", key, catalog)
		}
	}

	metadata := mustGraphStream(t, graph, catalog["Metadata"])
	if metadata.Dict["Type"] != pdfName("Metadata") || metadata.Dict["Subtype"] != pdfName("XML") {
		t.Fatalf("metadata stream dict = %+v, want /Type /Metadata /Subtype /XML", metadata.Dict)
	}
	if !bytes.Contains(metadata.Data, []byte("graph fixture")) {
		t.Fatalf("metadata stream data = %q, want graph fixture marker", metadata.Data)
	}

	outlines := mustGraphDict(t, graph, catalog["Outlines"])
	firstOutline := mustGraphDict(t, graph, outlines["First"])
	if got := graphPDFString(t, firstOutline["Title"]); got != "Chapter 1" {
		t.Fatalf("outline title = %q, want Chapter 1", got)
	}
	dest, ok := firstOutline["Dest"].(pdfArray)
	if !ok || len(dest) != 2 || dest[0] != (pdfRef{ID: pdfObjectID{Number: 4, Generation: 0}}) || dest[1] != pdfName("Fit") {
		t.Fatalf("outline destination = %+v, want page 4 /Fit", firstOutline["Dest"])
	}

	labels := mustGraphDict(t, graph, catalog["PageLabels"])
	nums := collectGraphNumberTreeEntries(t, graph, labels)
	firstLabel := mustGraphDict(t, graph, nums[0])
	if firstLabel["S"] != pdfName("D") || graphPDFString(t, firstLabel["P"]) != "A-" {
		t.Fatalf("page label 0 = %+v, want decimal prefix A-", firstLabel)
	}
	secondLabel := mustGraphDict(t, graph, nums[2])
	if secondLabel["S"] != pdfName("r") {
		t.Fatalf("page label 2 = %+v, want lower-roman style", secondLabel)
	}
}

func readGraphTraversalFixture(t *testing.T) *pdfGraph {
	t.Helper()

	input, err := os.ReadFile(filepath.Join("..", "..", "..", "testdata", "pdf", "graph-traversal-catalog.pdf"))
	if err != nil {
		t.Fatal(err)
	}
	graph, err := parsePDFGraph(input)
	if err != nil {
		t.Fatalf("parse graph traversal fixture: %v", err)
	}
	return graph
}

func mustGraphCatalog(t *testing.T, graph *pdfGraph) pdfDict {
	t.Helper()

	catalog, ok := graph.catalogDict()
	if !ok {
		t.Fatal("graph catalog not found")
	}
	return catalog
}

func collectGraphPageTreeRefs(t *testing.T, graph *pdfGraph, value pdfValue) []pdfObjectID {
	t.Helper()

	ref, ok := value.(pdfRef)
	if !ok {
		t.Fatalf("page tree root = %T, want ref", value)
	}
	out := make([]pdfObjectID, 0)
	collectGraphPageTreeRefsFromID(t, graph, ref.ID, map[pdfObjectID]bool{}, &out)
	return out
}

func collectGraphPageTreeRefsFromID(t *testing.T, graph *pdfGraph, id pdfObjectID, seen map[pdfObjectID]bool, out *[]pdfObjectID) {
	t.Helper()

	if seen[id] {
		t.Fatalf("cycle in page tree at object %+v", id)
	}
	seen[id] = true
	dict := mustGraphDictObject(t, graph, id)
	if dictHasType(dict, "Page") {
		*out = append(*out, id)
		return
	}
	if !dictHasType(dict, "Pages") {
		t.Fatalf("page tree object %+v type = %+v, want /Pages or /Page", id, dict["Type"])
	}
	kids, ok := dict["Kids"].(pdfArray)
	if !ok {
		t.Fatalf("page tree object %+v has Kids = %T, want array", id, dict["Kids"])
	}
	for _, kid := range kids {
		ref, ok := kid.(pdfRef)
		if !ok {
			t.Fatalf("page tree kid = %T, want ref", kid)
		}
		collectGraphPageTreeRefsFromID(t, graph, ref.ID, seen, out)
	}
}

func collectGraphNameTreeEntries(t *testing.T, graph *pdfGraph, value pdfValue) map[string]pdfValue {
	t.Helper()

	out := make(map[string]pdfValue)
	collectGraphNameTreeEntriesInto(t, graph, value, map[pdfObjectID]bool{}, out)
	return out
}

func collectGraphNameTreeEntriesInto(t *testing.T, graph *pdfGraph, value pdfValue, seen map[pdfObjectID]bool, out map[string]pdfValue) {
	t.Helper()

	if ref, ok := value.(pdfRef); ok {
		if seen[ref.ID] {
			t.Fatalf("cycle in name tree at object %+v", ref.ID)
		}
		seen[ref.ID] = true
		value = mustGraphObject(t, graph, ref.ID).Value
	}
	dict, ok := objectDictValue(value)
	if !ok {
		t.Fatalf("name tree node = %T, want dictionary", value)
	}
	if names, ok := dict["Names"].(pdfArray); ok {
		if len(names)%2 != 0 {
			t.Fatalf("name tree /Names length = %d, want key/value pairs", len(names))
		}
		for i := 0; i < len(names); i += 2 {
			out[graphPDFString(t, names[i])] = names[i+1]
		}
	}
	if kids, ok := dict["Kids"].(pdfArray); ok {
		for _, kid := range kids {
			collectGraphNameTreeEntriesInto(t, graph, kid, seen, out)
		}
	}
}

func collectGraphNumberTreeEntries(t *testing.T, graph *pdfGraph, value pdfValue) map[int]pdfValue {
	t.Helper()

	out := make(map[int]pdfValue)
	collectGraphNumberTreeEntriesInto(t, graph, value, map[pdfObjectID]bool{}, out)
	return out
}

func collectGraphNumberTreeEntriesInto(t *testing.T, graph *pdfGraph, value pdfValue, seen map[pdfObjectID]bool, out map[int]pdfValue) {
	t.Helper()

	if ref, ok := value.(pdfRef); ok {
		if seen[ref.ID] {
			t.Fatalf("cycle in number tree at object %+v", ref.ID)
		}
		seen[ref.ID] = true
		value = mustGraphObject(t, graph, ref.ID).Value
	}
	dict, ok := objectDictValue(value)
	if !ok {
		t.Fatalf("number tree node = %T, want dictionary", value)
	}
	if nums, ok := dict["Nums"].(pdfArray); ok {
		if len(nums)%2 != 0 {
			t.Fatalf("number tree /Nums length = %d, want key/value pairs", len(nums))
		}
		for i := 0; i < len(nums); i += 2 {
			key, ok := nums[i].(int)
			if !ok {
				t.Fatalf("number tree key = %T, want int", nums[i])
			}
			out[key] = nums[i+1]
		}
	}
	if kids, ok := dict["Kids"].(pdfArray); ok {
		for _, kid := range kids {
			collectGraphNumberTreeEntriesInto(t, graph, kid, seen, out)
		}
	}
}

func sortedGraphNameTreeKeys(entries map[string]pdfValue) []string {
	keys := make([]string, 0, len(entries))
	for key := range entries {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	return keys
}

func mustGraphDict(t *testing.T, graph *pdfGraph, value pdfValue) pdfDict {
	t.Helper()

	if ref, ok := value.(pdfRef); ok {
		return mustGraphDictObject(t, graph, ref.ID)
	}
	dict, ok := objectDictValue(value)
	if !ok {
		t.Fatalf("value = %T, want dictionary", value)
	}
	return dict
}

func mustGraphDictObject(t *testing.T, graph *pdfGraph, id pdfObjectID) pdfDict {
	t.Helper()

	dict, ok := objectDictValue(mustGraphObject(t, graph, id).Value)
	if !ok {
		t.Fatalf("object %+v = %T, want dictionary", id, mustGraphObject(t, graph, id).Value)
	}
	return dict
}

func mustGraphStream(t *testing.T, graph *pdfGraph, value pdfValue) pdfStreamObject {
	t.Helper()

	if ref, ok := value.(pdfRef); ok {
		value = mustGraphObject(t, graph, ref.ID).Value
	}
	stream, ok := value.(pdfStreamObject)
	if !ok {
		t.Fatalf("value = %T, want stream", value)
	}
	return stream
}

func mustGraphObject(t *testing.T, graph *pdfGraph, id pdfObjectID) *pdfIndirectObject {
	t.Helper()

	object := graph.Objects[id]
	if object == nil {
		t.Fatalf("object %+v not found", id)
	}
	return object
}

func graphPDFString(t *testing.T, value pdfValue) string {
	t.Helper()

	out, ok, err := pdfStringBytes(value)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatalf("value = %T, want PDF string", value)
	}
	return string(out)
}
