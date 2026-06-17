package pdf

import (
	"bytes"
	"fmt"
	"testing"
)

func TestPageOpsPreserveInheritedPageBoxesAndResources(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 612 792] /CropBox [18 18 594 774] /Rotate 90 /Resources << /Font << /F1 5 0 R >> >> >>",
		"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
		pageOpsContentStream("INHERITED"),
		"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
	)

	output, _, verification, err := ExtractPages(input, []int{0})
	if err != nil {
		t.Fatalf("ExtractPages: %v", err)
	}
	if !verification.ResourcesAvailable || !verification.PageContentAvailable || !verification.NoDanglingRefs {
		t.Fatalf("verification = %+v", verification)
	}

	page := firstPageDict(t, output)
	assertPDFValueContains(t, page["MediaBox"], "[0 0 612 792]")
	assertPDFValueContains(t, page["CropBox"], "[18 18 594 774]")
	if got, ok := page["Rotate"].(int); !ok || got != 90 {
		t.Fatalf("Rotate = %#v, want 90", page["Rotate"])
	}
	resources, ok := page["Resources"].(pdfDict)
	if !ok {
		t.Fatalf("Resources = %#v, want direct materialized dict", page["Resources"])
	}
	fonts, ok := resources["Font"].(pdfDict)
	if !ok {
		t.Fatalf("Resources.Font = %#v, want dict", resources["Font"])
	}
	fontRef, ok := fonts["F1"].(pdfRef)
	if !ok {
		t.Fatalf("Resources.Font.F1 = %#v, want cloned font ref", fonts["F1"])
	}
	graph := parseGraphForPreservationTest(t, output)
	font, ok := graph.objectDict(fontRef.ID)
	if !ok || !dictHasName(font, "BaseFont", "Helvetica") {
		t.Fatalf("cloned font = %#v, want /BaseFont /Helvetica", font)
	}
}

func TestPageOpsPreserveAnnotationsReachableFromPageDictionaries(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources <<>> /Contents 4 0 R /Annots [5 0 R] >>",
		pageOpsContentStream("ANNOTATED"),
		"<< /Type /Annot /Subtype /Text /Rect [10 10 30 30] /Contents (sticky note) /AP << /N 6 0 R >> /Popup 7 0 R >>",
		"<< /Subtype /Form /BBox [0 0 20 20] /Resources <<>> /Length 5 >>\nstream\nq\nQ\nendstream",
		"<< /Type /Annot /Subtype /Popup /Rect [30 30 80 60] /Parent 5 0 R >>",
	)

	output, _, verification, err := CopyPages(input, []int{0})
	if err != nil {
		t.Fatalf("CopyPages: %v", err)
	}
	if !verification.NoDanglingRefs {
		t.Fatalf("verification = %+v", verification)
	}

	graph := parseGraphForPreservationTest(t, output)
	page := firstPageDictFromGraph(t, graph)
	annots, ok := page["Annots"].(pdfArray)
	if !ok || len(annots) != 1 {
		t.Fatalf("Annots = %#v, want one annotation ref", page["Annots"])
	}
	annotRef, ok := annots[0].(pdfRef)
	if !ok {
		t.Fatalf("Annots[0] = %#v, want ref", annots[0])
	}
	annot, ok := graph.objectDict(annotRef.ID)
	if !ok {
		t.Fatalf("annotation ref %v was not cloned", annotRef.ID)
	}
	if subtype, ok := annot["Subtype"].(pdfName); !ok || subtype != "Text" {
		t.Fatalf("annotation subtype = %#v, want /Text", annot["Subtype"])
	}
	if contents, ok := annot["Contents"].(pdfLiteralString); !ok || string(contents) != "sticky note" {
		t.Fatalf("annotation contents = %#v, want sticky note", annot["Contents"])
	}
	ap, ok := annot["AP"].(pdfDict)
	if !ok {
		t.Fatalf("annotation AP = %#v, want dict", annot["AP"])
	}
	normalAppearance, ok := ap["N"].(pdfRef)
	if !ok {
		t.Fatalf("annotation AP.N = %#v, want cloned appearance stream ref", ap["N"])
	}
	if stream, ok := graph.Objects[normalAppearance.ID].Value.(pdfStreamObject); !ok || !bytes.Contains(stream.Data, []byte("q\nQ")) {
		t.Fatalf("appearance stream = %#v, want cloned stream data", graph.Objects[normalAppearance.ID])
	}
	popupRef, ok := annot["Popup"].(pdfRef)
	if !ok {
		t.Fatalf("annotation Popup = %#v, want cloned popup ref", annot["Popup"])
	}
	popup, ok := graph.objectDict(popupRef.ID)
	if !ok || !dictHasName(popup, "Subtype", "Popup") {
		t.Fatalf("popup annotation = %#v, want /Subtype /Popup", popup)
	}
}

func TestPageOpsPreserveDocumentCatalogStructures(t *testing.T) {
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R /Names << /Dests << /Names [(chapter-one) 6 0 R] >> >> /PageLabels << /Nums [0 << /S /r /P (intro-) /St 3 >>] >> /Outlines 7 0 R /Metadata 8 0 R /AcroForm << /Fields [9 0 R] /NeedAppearances true >> >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources <<>> /Contents 4 0 R /Annots [9 0 R] >>",
		pageOpsContentStream("LABELED"),
		"<< /Title (not page reachable) >>",
		"<< /D [3 0 R /Fit] >>",
		"<< /First 10 0 R /Last 10 0 R /Count 1 >>",
		"<< /Type /Metadata /Subtype /XML /Length 17 >>\nstream\n<xml>meta</xml>\nendstream",
		"<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /Rect [10 10 80 30] /P 3 0 R >>",
		"<< /Title (Chapter One) /Dest [3 0 R /Fit] >>",
	)

	output, _, verification, err := ExtractPages(input, []int{0})
	if err != nil {
		t.Fatalf("ExtractPages: %v", err)
	}
	if !verification.NoDanglingRefs {
		t.Fatalf("verification = %+v", verification)
	}

	graph := parseGraphForPreservationTest(t, output)
	catalog, ok := graph.catalogDict()
	if !ok {
		t.Fatal("output catalog missing")
	}
	names := mustPreservedCatalogDict(t, graph, catalog, "Names")
	dests := mustPreservedCatalogDict(t, graph, names, "Dests")
	nameArray, ok := dests["Names"].(pdfArray)
	if !ok || len(nameArray) != 2 {
		t.Fatalf("Dests.Names = %#v, want one name/value pair", dests["Names"])
	}
	if got, ok := nameArray[0].(pdfLiteralString); !ok || string(got) != "chapter-one" {
		t.Fatalf("destination name = %#v, want chapter-one", nameArray[0])
	}
	destDict := mustPreservedRefDict(t, graph, nameArray[1])
	destination, ok := destDict["D"].(pdfArray)
	if !ok || len(destination) != 2 {
		t.Fatalf("destination value = %#v, want destination array", destDict["D"])
	}
	destPageRef, ok := destination[0].(pdfRef)
	if !ok || destPageRef.ID.Number != 3 {
		t.Fatalf("destination page ref = %#v, want remapped output page 3 0 R", destination[0])
	}

	pageLabels := mustPreservedCatalogDict(t, graph, catalog, "PageLabels")
	nums, ok := pageLabels["Nums"].(pdfArray)
	if !ok || len(nums) != 2 {
		t.Fatalf("PageLabels.Nums = %#v, want one number-tree entry", pageLabels["Nums"])
	}
	labelDict, ok := nums[1].(pdfDict)
	if !ok || labelDict["S"] != pdfName("r") {
		t.Fatalf("page label dict = %#v, want lower-roman style", nums[1])
	}
	if got, ok := labelDict["P"].(pdfLiteralString); !ok || string(got) != "intro-" {
		t.Fatalf("page label prefix = %#v, want intro-", labelDict["P"])
	}

	outlines := mustPreservedCatalogDict(t, graph, catalog, "Outlines")
	outlineRef, ok := outlines["First"].(pdfRef)
	if !ok {
		t.Fatalf("Outlines.First = %#v, want cloned outline ref", outlines["First"])
	}
	outline := mustPreservedRefDict(t, graph, outlineRef)
	if got, ok := outline["Title"].(pdfLiteralString); !ok || string(got) != "Chapter One" {
		t.Fatalf("outline title = %#v, want Chapter One", outline["Title"])
	}

	metadataRef, ok := catalog["Metadata"].(pdfRef)
	if !ok {
		t.Fatalf("catalog Metadata = %#v, want cloned metadata ref", catalog["Metadata"])
	}
	metadata, ok := graph.Objects[metadataRef.ID].Value.(pdfStreamObject)
	if !ok || !bytes.Contains(metadata.Data, []byte("<xml>meta</xml>")) {
		t.Fatalf("metadata stream = %#v, want cloned XML stream", graph.Objects[metadataRef.ID])
	}

	acroForm := mustPreservedCatalogDict(t, graph, catalog, "AcroForm")
	fields, ok := acroForm["Fields"].(pdfArray)
	if !ok || len(fields) != 1 {
		t.Fatalf("AcroForm.Fields = %#v, want one field ref", acroForm["Fields"])
	}
	fieldRef, ok := fields[0].(pdfRef)
	if !ok {
		t.Fatalf("AcroForm.Fields[0] = %#v, want widget ref", fields[0])
	}
	page := firstPageDictFromGraph(t, graph)
	annots, ok := page["Annots"].(pdfArray)
	if !ok || len(annots) != 1 || annots[0] != fieldRef {
		t.Fatalf("page Annots = %#v, AcroForm field ref = %#v, want shared remapped widget", page["Annots"], fieldRef)
	}
}

func TestMergeReconcilesCatalogStructuresFromAllSources(t *testing.T) {
	left := pageOpsCatalogFixture("LEFT", "left-dest", "L-", "Left Chapter", "left_field")
	right := pageOpsCatalogFixture("RIGHT", "right-dest", "R-", "Right Chapter", "right_field")

	output, _, verification, err := Merge([][]byte{left, right})
	if err != nil {
		t.Fatalf("Merge: %v", err)
	}
	if !verification.NoDanglingRefs || verification.ActualPageCount != 2 {
		t.Fatalf("verification = %+v, want two-page graph with no dangling refs", verification)
	}

	graph := parseGraphForPreservationTest(t, output)
	pages, err := graph.orderedPages()
	if err != nil {
		t.Fatalf("ordered pages: %v", err)
	}
	if len(pages) != 2 {
		t.Fatalf("page count = %d, want 2", len(pages))
	}
	catalog, ok := graph.catalogDict()
	if !ok {
		t.Fatal("output catalog missing")
	}

	names := mustPreservedCatalogDict(t, graph, catalog, "Names")
	dests := mustPreservedCatalogDict(t, graph, names, "Dests")
	destinationNames, ok := dests["Names"].(pdfArray)
	if !ok || len(destinationNames) != 4 {
		t.Fatalf("merged destination names = %#v, want two name/value pairs", dests["Names"])
	}
	if string(destinationNames[0].(pdfLiteralString)) != "left-dest" || string(destinationNames[2].(pdfLiteralString)) != "right-dest" {
		t.Fatalf("merged destination keys = %#v, want left-dest/right-dest", destinationNames)
	}
	leftDest := mustPreservedRefDict(t, graph, destinationNames[1])
	rightDest := mustPreservedRefDict(t, graph, destinationNames[3])
	assertDestinationPageRef(t, leftDest["D"], pages[0].ID.Number)
	assertDestinationPageRef(t, rightDest["D"], pages[1].ID.Number)

	pageLabels := mustPreservedCatalogDict(t, graph, catalog, "PageLabels")
	nums, ok := pageLabels["Nums"].(pdfArray)
	if !ok || len(nums) != 4 {
		t.Fatalf("merged page labels = %#v, want two number-tree entries", pageLabels["Nums"])
	}
	if nums[0] != 0 || nums[2] != 1 {
		t.Fatalf("merged page label indexes = %#v, want 0 and 1", nums)
	}
	if labelPrefix(t, nums[1]) != "L-" || labelPrefix(t, nums[3]) != "R-" {
		t.Fatalf("merged page label prefixes = %#v, want L-/R-", nums)
	}

	outlines := mustPreservedCatalogDict(t, graph, catalog, "Outlines")
	if count, ok := outlines["Count"].(int); !ok || count != 2 {
		t.Fatalf("merged outline count = %#v, want 2", outlines["Count"])
	}
	first := mustPreservedRefDict(t, graph, outlines["First"])
	second := mustPreservedRefDict(t, graph, first["Next"])
	if titleString(t, first["Title"]) != "Left Chapter" || titleString(t, second["Title"]) != "Right Chapter" {
		t.Fatalf("merged outline titles = %#v / %#v, want left then right", first["Title"], second["Title"])
	}
	assertDestinationPageRef(t, first["Dest"], pages[0].ID.Number)
	assertDestinationPageRef(t, second["Dest"], pages[1].ID.Number)

	acroForm := mustPreservedCatalogDict(t, graph, catalog, "AcroForm")
	fields, ok := acroForm["Fields"].(pdfArray)
	if !ok || len(fields) != 2 {
		t.Fatalf("merged AcroForm fields = %#v, want two field refs", acroForm["Fields"])
	}
	if fieldName(t, graph, fields[0]) != "left_field" || fieldName(t, graph, fields[1]) != "right_field" {
		t.Fatalf("merged AcroForm field names = %#v, want left/right fields", fields)
	}
}

func parseGraphForPreservationTest(t *testing.T, input []byte) *pdfGraph {
	t.Helper()
	graph, err := parsePDFGraph(input)
	if err != nil {
		t.Fatalf("parse output graph: %v\n%s", err, input)
	}
	return graph
}

func firstPageDict(t *testing.T, input []byte) pdfDict {
	t.Helper()
	return firstPageDictFromGraph(t, parseGraphForPreservationTest(t, input))
}

func firstPageDictFromGraph(t *testing.T, graph *pdfGraph) pdfDict {
	t.Helper()
	pages, err := graph.orderedPages()
	if err != nil {
		t.Fatalf("ordered pages: %v", err)
	}
	if len(pages) != 1 {
		t.Fatalf("page count = %d, want 1", len(pages))
	}
	return pages[0].Dict
}

func assertPDFValueContains(t *testing.T, value pdfValue, want string) {
	t.Helper()
	var out bytes.Buffer
	if err := writePDFValue(&out, value); err != nil {
		t.Fatalf("write PDF value %#v: %v", value, err)
	}
	if got := out.String(); got != want {
		t.Fatalf("PDF value = %s, want %s", got, want)
	}
}

func pageOpsCatalogFixture(text, destName, labelPrefix, outlineTitle, fieldName string) []byte {
	return testPDF(
		fmt.Sprintf("<< /Type /Catalog /Pages 2 0 R /Names << /Dests << /Names [(%s) 5 0 R] >> >> /PageLabels << /Nums [0 << /S /D /P (%s) >>] >> /Outlines 6 0 R /AcroForm << /Fields [8 0 R] /NeedAppearances true >> >>", destName, labelPrefix),
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources <<>> /Contents 4 0 R /Annots [8 0 R] >>",
		pageOpsContentStream(text),
		"<< /D [3 0 R /Fit] >>",
		"<< /First 7 0 R /Last 7 0 R /Count 1 >>",
		fmt.Sprintf("<< /Title (%s) /Dest [3 0 R /Fit] >>", outlineTitle),
		fmt.Sprintf("<< /Type /Annot /Subtype /Widget /FT /Tx /T (%s) /Rect [10 10 80 30] /P 3 0 R >>", fieldName),
	)
}

func assertDestinationPageRef(t *testing.T, value pdfValue, wantObjectNumber int) {
	t.Helper()
	destination, ok := value.(pdfArray)
	if !ok || len(destination) == 0 {
		t.Fatalf("destination = %#v, want array", value)
	}
	pageRef, ok := destination[0].(pdfRef)
	if !ok || pageRef.ID.Number != wantObjectNumber {
		t.Fatalf("destination page ref = %#v, want object %d", destination[0], wantObjectNumber)
	}
}

func labelPrefix(t *testing.T, value pdfValue) string {
	t.Helper()
	dict, ok := value.(pdfDict)
	if !ok {
		t.Fatalf("page label value = %#v, want dict", value)
	}
	return titleString(t, dict["P"])
}

func titleString(t *testing.T, value pdfValue) string {
	t.Helper()
	text, ok := value.(pdfLiteralString)
	if !ok {
		t.Fatalf("value = %#v, want literal string", value)
	}
	return string(text)
}

func fieldName(t *testing.T, graph *pdfGraph, value pdfValue) string {
	t.Helper()
	field := mustPreservedRefDict(t, graph, value)
	return titleString(t, field["T"])
}

func mustPreservedCatalogDict(t *testing.T, graph *pdfGraph, catalog pdfDict, key string) pdfDict {
	t.Helper()
	value, ok := catalog[key]
	if !ok {
		t.Fatalf("catalog missing /%s: %#v", key, catalog)
	}
	return mustPreservedRefDict(t, graph, value)
}

func mustPreservedRefDict(t *testing.T, graph *pdfGraph, value pdfValue) pdfDict {
	t.Helper()
	if ref, ok := value.(pdfRef); ok {
		dict, ok := graph.objectDict(ref.ID)
		if !ok {
			t.Fatalf("ref %v did not resolve to dict", ref.ID)
		}
		return dict
	}
	dict, ok := value.(pdfDict)
	if !ok {
		t.Fatalf("value = %#v, want dict or ref to dict", value)
	}
	return dict
}
