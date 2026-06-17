package pdfapi

import (
	"bytes"
	"fmt"
	"strings"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestInspectAndQueryTextDefaultToPDFTextShows(t *testing.T) {
	input := testPDFAPIFile("<< /Type /Page >>", streamObject("alpha"))

	parsed, err := Parse(input, core.ParseOptions{Strict: true})
	if err != nil {
		t.Fatal(err)
	}
	if parsed.Format != "pdf" {
		t.Fatalf("format = %q, want pdf", parsed.Format)
	}

	tree, err := Inspect(input, Options{})
	if err != nil {
		t.Fatal(err)
	}
	if tree.Format != "pdf" {
		t.Fatalf("format = %q, want pdf", tree.Format)
	}

	matches, err := QueryText(input, TextSelector{Text: "alpha"}, Options{})
	if err != nil {
		t.Fatal(err)
	}
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].Kind != "pdf.content.text_show" {
		t.Fatalf("kind = %q, want default PDF text show kind", matches[0].Kind)
	}
}

func TestEditTextAutoUsesCanonicalRewriteAndVerifiesNoFallbackOldGoneNewSelectable(t *testing.T) {
	input := testPDFAPIFile("<< /Type /Page >>", streamObject("old"))

	output, report, verification, err := EditText(
		input,
		TextSelector{Text: "old"},
		TextReplacement{Replace: "new"},
		Options{},
	)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Equal(output, input) {
		t.Fatal("output did not change")
	}
	if report.FallbackUsed {
		t.Fatalf("fallback used: %+v", report)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" {
		t.Fatalf("edit = %q, want canonical rewrite", report.Edit)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v, want reparse/old gone/new selectable/page unchanged", verification)
	}

	oldMatches, err := QueryText(output, TextSelector{Text: "old"}, Options{})
	if err != nil {
		t.Fatal(err)
	}
	if len(oldMatches) != 0 {
		t.Fatalf("old matches = %d, want 0", len(oldMatches))
	}
	newMatches, err := QueryText(output, TextSelector{Text: "new"}, Options{})
	if err != nil {
		t.Fatal(err)
	}
	if len(newMatches) != 1 {
		t.Fatalf("new matches = %d, want 1", len(newMatches))
	}
}

func TestEditTextHonorsVerificationInvariants(t *testing.T) {
	input := testPDFAPIFile("<< /Type /Page >>", streamObject("Invoice #1234"))

	output, report, verification, err := EditText(
		input,
		TextSelector{Text: "Invoice #1234"},
		TextReplacement{Replace: "Invoice #5678"},
		Options{
			Rewrite: RewriteModeAuto,
			Verify:  []string{"reparse", "old-gone", "new-selectable", "page-count-unchanged", "no-fallback"},
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Equal(output, input) {
		t.Fatal("output did not change")
	}
	if report.FallbackUsed {
		t.Fatalf("report = %+v, want no fallback", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v, want reparse/old gone/new selectable/page unchanged", verification)
	}

	matches, err := QueryText(output, TextSelector{Text: "Invoice #5678"}, Options{})
	if err != nil {
		t.Fatal(err)
	}
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want rewritten text query match", len(matches))
	}
}

func TestQueryTextHonorsMatchIndex(t *testing.T) {
	input := testPDFAPIFile(
		"<< /Type /Page >>",
		streamObject("repeat"),
		streamObject("repeat"),
	)
	index := 1

	matches, err := QueryText(input, TextSelector{Text: "repeat", MatchIndex: &index}, Options{})
	if err != nil {
		t.Fatal(err)
	}
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].ID == 0 {
		t.Fatalf("unexpected root match: %+v", matches[0])
	}
}

func TestValidateReturnsMinimalParseVerification(t *testing.T) {
	input := testPDFAPIFile("<< /Type /Page >>", streamObject("ok"))

	verification, err := Validate(input, Options{})
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK {
		t.Fatalf("verification = %+v, want ReparseOK", verification)
	}
}

func TestParseGraphExposesCatalogPagesAndNameTrees(t *testing.T) {
	input := testPDFAPIGraphFile(
		"<< /Type /Catalog /Pages 2 0 R /Names << /Dests 5 0 R >> >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Resources <<>> /Contents 4 0 R >>",
		streamObject("graph text"),
		"<< /Names [(home) 3 0 R] >>",
	)

	graph, err := ParseGraph(input, GraphOptions{})
	if err != nil {
		t.Fatal(err)
	}
	catalogRef, catalog, ok := graph.Catalog()
	if !ok {
		t.Fatal("missing catalog")
	}
	if catalogRef.Number != 1 || catalog["Pages"] != (Ref{Number: 2, Generation: 0}) {
		t.Fatalf("catalog = ref %+v dict %+v, want root 1 and pages 2", catalogRef, catalog)
	}
	pages, err := graph.PageTree()
	if err != nil {
		t.Fatal(err)
	}
	if len(pages) != 1 || pages[0].Ref.Number != 3 || pages[0].Index != 0 {
		t.Fatalf("pages = %+v, want one page node for object 3", pages)
	}
	names, err := graph.NameTree(NameTreeDests)
	if err != nil {
		t.Fatal(err)
	}
	if len(names) != 1 || names[0].Name != "home" {
		t.Fatalf("names = %+v, want home destination", names)
	}
	stream, ok := graph.Stream(Ref{Number: 4, Generation: 0})
	if !ok || !bytes.Contains(stream.Data, []byte("graph text")) {
		t.Fatalf("stream ok=%t data=%q, want graph text", ok, stream.Data)
	}
}

func TestPDFAPIExtractAndMergePageOperations(t *testing.T) {
	first := testPDFAPIPagedFile("first", "second")
	second := testPDFAPIPagedFile("third")

	extracted, report, verification, err := ExtractPages(first, []int{1})
	if err != nil {
		t.Fatal(err)
	}
	if report.OutputPages != 1 || !verification.ReparseOK || !verification.PageCountOK || !verification.NoDanglingRefs {
		t.Fatalf("extract report=%+v verification=%+v", report, verification)
	}
	matches, err := QueryText(extracted, TextSelector{Text: "second"}, Options{})
	if err != nil || len(matches) != 1 {
		t.Fatalf("extracted text matches=%d err=%v, want one second match", len(matches), err)
	}

	merged, report, verification, err := Merge([][]byte{first, second})
	if err != nil {
		t.Fatal(err)
	}
	if report.OutputPages != 3 || !verification.ReparseOK || !verification.PageCountOK || !verification.NoDanglingRefs {
		t.Fatalf("merge report=%+v verification=%+v", report, verification)
	}
	for _, text := range []string{"first", "second", "third"} {
		matches, err := QueryText(merged, TextSelector{Text: text}, Options{})
		if err != nil || len(matches) != 1 {
			t.Fatalf("merged text %q matches=%d err=%v, want one match", text, len(matches), err)
		}
	}
}

func TestPDFAPIInsertAndTransformPageOperations(t *testing.T) {
	base := testPDFAPIPagedFile("base")
	source := testPDFAPIPagedFile("inserted")

	inserted, report, verification, err := InsertPages(base, 1, []PageSource{{Input: source}})
	if err != nil {
		t.Fatal(err)
	}
	if report.OutputPages != 2 || !verification.ReparseOK || !verification.PageCountOK || !verification.NoDanglingRefs {
		t.Fatalf("insert report=%+v verification=%+v", report, verification)
	}

	rotate := 180
	transformed, report, verification, err := TransformPages(inserted, PageSelector{Indexes: []int{1}}, PageTransform{Rotate: &rotate}, PageWriteOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if report.OutputPages != 2 || !verification.ReparseOK || !verification.PageCountOK || !verification.NoDanglingRefs {
		t.Fatalf("transform report=%+v verification=%+v", report, verification)
	}
	graph, err := ParseGraph(transformed, GraphOptions{})
	if err != nil {
		t.Fatal(err)
	}
	pages, err := graph.PageTree()
	if err != nil {
		t.Fatal(err)
	}
	if len(pages) != 2 || pages[1].Dict["Rotate"] != (Number{Value: 180, Integer: true}) {
		t.Fatalf("pages = %+v, want second page rotated 180", pages)
	}

	scale := PageScale{X: 2, Y: 2}
	scaled, _, verification, err := TransformPages(transformed, PageSelector{Indexes: []int{0}}, PageTransform{Scale: &scale}, PageWriteOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK || !verification.NoDanglingRefs {
		t.Fatalf("scale verification=%+v", verification)
	}
	scaledGraph, err := ParseGraph(scaled, GraphOptions{})
	if err != nil {
		t.Fatal(err)
	}
	scaledPages, err := scaledGraph.PageTree()
	if err != nil {
		t.Fatal(err)
	}
	if scaledPages[0].Dict["MediaBox"].(Array)[2] != (Number{Value: 400, Integer: true}) {
		t.Fatalf("scaled page = %+v, want doubled MediaBox width", scaledPages[0])
	}
}

func TestPDFAPIMutationWrappersExposeFormsAndStreams(t *testing.T) {
	input := testPDFAPIPagedFile("stream")

	withField, formReport, formVerification, err := CreateFormField(input, FormFieldCreateOptions{
		Name:         "customer",
		PageIndex:    0,
		Rect:         [4]float64{10, 10, 120, 30},
		DefaultValue: "Ada",
	})
	if err != nil {
		t.Fatal(err)
	}
	if formReport.Operation == "" || !formVerification.ReparseOK || !formVerification.FieldPresent || !formVerification.WidgetOnPage {
		t.Fatalf("form report=%+v verification=%+v", formReport, formVerification)
	}

	fields, err := ListFormFields(withField)
	if err != nil {
		t.Fatal(err)
	}
	if len(fields) != 1 || fields[0].Name != "customer" {
		t.Fatalf("fields = %+v, want one customer field", fields)
	}
	fieldEdited, editReport, editVerification, err := ApplyFormFieldEdit(withField, "customer", "Grace")
	if err != nil {
		t.Fatal(err)
	}
	if editReport.Edit == "" || !editVerification.ReparseOK || !editVerification.FieldValueSet {
		t.Fatalf("form edit report=%+v verification=%+v", editReport, editVerification)
	}
	fields, err = ListFormFields(fieldEdited)
	if err != nil {
		t.Fatal(err)
	}
	if len(fields) != 1 || fields[0].Value == nil || *fields[0].Value != "Grace" {
		t.Fatalf("edited fields = %+v, want customer value Grace", fields)
	}

	mutated, streamReport, streamVerification, err := MutateStream(withField, StreamMutationOptions{
		ObjectNumber: 4,
		Replacement:  []byte("BT\n(changed) Tj\nET\n"),
	})
	if err != nil {
		t.Fatal(err)
	}
	if streamReport.Edit == "" || !streamVerification.ReparseOK || !streamVerification.NoDanglingRefs {
		t.Fatalf("stream report=%+v verification=%+v", streamReport, streamVerification)
	}
	matches, err := QueryText(mutated, TextSelector{Text: "changed"}, Options{})
	if err != nil || len(matches) != 1 {
		t.Fatalf("mutated text matches=%d err=%v, want one changed match", len(matches), err)
	}
}

func TestPDFAPIFacadeExposesAnnotationSecurityAndXFABoundaries(t *testing.T) {
	input := testPDFAPIGraphFile(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [4 0 R] >>",
		"<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (old note) >>",
	)

	annotations, err := ListAnnotationCandidates(input)
	if err != nil {
		t.Fatal(err)
	}
	if len(annotations) != 1 || annotations[0].Contents != "old note" {
		t.Fatalf("annotations = %+v, want old note", annotations)
	}
	updated, report, verification, err := ApplyAnnotationContentsEdit(input, 0, "new note")
	if err != nil {
		t.Fatal(err)
	}
	if report.Edit == "" || !verification.ReparseOK || !verification.ContentsUpdated || !verification.PageUnchanged {
		t.Fatalf("annotation report=%+v verification=%+v", report, verification)
	}
	annotations, err = ListAnnotationCandidates(updated)
	if err != nil {
		t.Fatal(err)
	}
	if len(annotations) != 1 || annotations[0].Contents != "new note" {
		t.Fatalf("updated annotations = %+v, want new note", annotations)
	}

	security := SecurityMetadataForInput(input)
	if security.HasSecurityBoundary() {
		t.Fatalf("security = %+v, want no boundary", security)
	}
	if err := CheckSecurity(input, SecurityOptions{SignatureInvalidation: SignatureInvalidationRefuse}); err != nil {
		t.Fatal(err)
	}
	semantics, err := InspectXFASemantics(input)
	if err != nil {
		t.Fatal(err)
	}
	if semantics.Classification != "none" {
		t.Fatalf("semantics = %+v, want no XFA", semantics)
	}
	if DefaultOverlayPolicy().UsesFallback() {
		t.Fatal("default overlay policy should not use fallback")
	}
}

func TestEditTextRejectsUnknownRewriteAndSignatureModes(t *testing.T) {
	input := testPDFAPIFile("<< /Type /Page >>", streamObject("old"))

	_, _, _, err := EditText(input, TextSelector{Text: "old"}, TextReplacement{Replace: "new"}, Options{Rewrite: RewriteMode("sideways")})
	if err == nil || !strings.Contains(err.Error(), "unsupported rewrite mode") {
		t.Fatalf("rewrite error = %v, want unsupported rewrite mode", err)
	}

	_, _, _, err = EditText(input, TextSelector{Text: "old"}, TextReplacement{Replace: "new"}, Options{SignatureMode: "sideways"})
	if err == nil || !strings.Contains(err.Error(), "unsupported signature mode") {
		t.Fatalf("signature error = %v, want unsupported signature mode", err)
	}
}

func testPDFAPIGraphFile(objects ...string) []byte {
	return testPDFAPIFile(objects...)
}

func testPDFAPIPagedFile(labels ...string) []byte {
	objects := make([]string, 0, 2+len(labels)*2)
	kids := make([]string, 0, len(labels))
	objects = append(objects, "<< /Type /Catalog /Pages 2 0 R >>")
	for i, label := range labels {
		pageObject := 3 + i*2
		contentObject := pageObject + 1
		kids = append(kids, fmt.Sprintf("%d 0 R", pageObject))
		objects = append(objects,
			fmt.Sprintf("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources <<>> /Contents %d 0 R >>", contentObject),
			streamObject(label),
		)
	}
	pages := fmt.Sprintf("<< /Type /Pages /Kids [%s] /Count %d >>", strings.Join(kids, " "), len(labels))
	objects = append(objects[:1], append([]string{pages}, objects[1:]...)...)
	return testPDFAPIFile(objects...)
}

func streamObject(text string) string {
	content := fmt.Sprintf("BT\n(%s) Tj\nET\n", text)
	return fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content)
}

func testPDFAPIFile(objects ...string) []byte {
	var out bytes.Buffer
	out.WriteString("%PDF-1.3\n")
	offsets := make([]int, 0, len(objects))
	for i, object := range objects {
		offsets = append(offsets, out.Len())
		fmt.Fprintf(&out, "%d 0 obj\n%s\nendobj\n", i+1, object)
	}
	xrefOffset := out.Len()
	fmt.Fprintf(&out, "xref\n0 %d\n", len(objects)+1)
	out.WriteString("0000000000 65535 f \n")
	for _, offset := range offsets {
		fmt.Fprintf(&out, "%010d 00000 n \n", offset)
	}
	fmt.Fprintf(&out, "trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n", len(objects)+1, xrefOffset)
	return out.Bytes()
}
