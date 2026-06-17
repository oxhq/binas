package pdf

import (
	"bytes"
	"errors"
	"fmt"
	"strings"
	"testing"
)

func TestCreateFormFieldAddsSimpleTextWidgetToPageAndAcroForm(t *testing.T) {
	input := formMutationTestPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources <<>> >>",
	)

	output, report, verification, err := CreateFormField(input, FormFieldCreateOptions{
		Name:         "payer.name",
		PageIndex:    0,
		Rect:         [4]float64{10, 20, 110, 40},
		DefaultValue: "Alice",
	})
	if err != nil {
		t.Fatal(err)
	}
	if report.Operation != "pdf.create_form_field" {
		t.Fatalf("operation = %q", report.Operation)
	}
	if report.FieldName != "payer.name" || report.FieldType != "Tx" || report.PageIndex != 0 {
		t.Fatalf("report = %+v", report)
	}
	if !verification.ReparseOK || !verification.FieldPresent || !verification.WidgetOnPage || !verification.NeedAppearancesSet {
		t.Fatalf("verification = %+v", verification)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	fields, err := ListFormFields(output)
	if err != nil {
		t.Fatal(err)
	}
	if len(fields) != 1 || fields[0].Name != "payer.name" || fields[0].FieldType != "Tx" {
		t.Fatalf("fields = %+v", fields)
	}
	fieldRef, ok := catalogAcroFormFieldRef(t, graph, 0)
	if !ok {
		t.Fatal("created field reference missing from catalog AcroForm")
	}
	field, ok := graph.objectDict(fieldRef.ID)
	if !ok {
		t.Fatal("created field object was not a dictionary")
	}
	if field["Subtype"] != pdfName("Widget") || field["FT"] != pdfName("Tx") {
		t.Fatalf("created field = %#v", field)
	}
	if got := field["T"]; got != pdfLiteralString("payer.name") {
		t.Fatalf("/T = %#v", got)
	}
	if got := field["V"]; got != pdfLiteralString("Alice") {
		t.Fatalf("/V = %#v", got)
	}
	pages, err := graph.orderedPages()
	if err != nil {
		t.Fatal(err)
	}
	annots, ok := pages[0].Dict["Annots"].(pdfArray)
	if !ok || len(annots) != 1 || annots[0] != fieldRef {
		t.Fatalf("page /Annots = %#v, want created field ref %#v", pages[0].Dict["Annots"], fieldRef)
	}
}

func TestRemoveFormFieldRemovesSimpleWidgetFromAcroFormAndPageAnnots(t *testing.T) {
	input := formMutationTestPDF(
		"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] /NeedAppearances true >> >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [4 0 R] /Resources <<>> >>",
		"<< /FT /Tx /Subtype /Widget /T (payer.name) /Rect [10 20 110 40] /V (Alice) /P 3 0 R >>",
	)

	output, report, verification, err := RemoveFormField(input, "payer.name")
	if err != nil {
		t.Fatal(err)
	}
	if report.Operation != "pdf.remove_form_field" || report.FieldName != "payer.name" {
		t.Fatalf("report = %+v", report)
	}
	if !verification.ReparseOK || verification.FieldPresent || verification.WidgetOnPage || !verification.NeedAppearancesSet {
		t.Fatalf("verification = %+v", verification)
	}
	fields, err := ListFormFields(output)
	if err != nil {
		t.Fatal(err)
	}
	if len(fields) != 0 {
		t.Fatalf("fields = %+v, want none", fields)
	}
	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	acroForm, ok := catalogAcroFormDict(graph)
	if !ok {
		t.Fatal("AcroForm missing")
	}
	if fieldsArray, ok := acroForm["Fields"].(pdfArray); !ok || len(fieldsArray) != 0 {
		t.Fatalf("AcroForm /Fields = %#v, want empty array", acroForm["Fields"])
	}
	pages, err := graph.orderedPages()
	if err != nil {
		t.Fatal(err)
	}
	if annots, ok := pages[0].Dict["Annots"].(pdfArray); ok && len(annots) != 0 {
		t.Fatalf("page /Annots = %#v, want empty or absent", annots)
	}
	if _, exists := graph.Objects[pdfObjectID{Number: 4, Generation: 0}]; exists {
		t.Fatal("removed field object remains in graph")
	}
}

func TestRemoveFormFieldFailsClosedForParentKidTree(t *testing.T) {
	input := formMutationTestPDF(
		"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] >> >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [5 0 R] /Resources <<>> >>",
		"<< /FT /Tx /T (payer.name) /Kids [5 0 R] >>",
		"<< /Subtype /Widget /Rect [10 20 110 40] /Parent 4 0 R /P 3 0 R >>",
	)

	_, _, _, err := RemoveFormField(input, "payer.name")
	if err == nil {
		t.Fatal("expected parent/kid removal to fail closed")
	}
	if !strings.Contains(err.Error(), "unsupported AcroForm field removal") {
		t.Fatalf("error = %q", err)
	}
}

func TestFlattenFormFieldsFailsClosedWithStructuredUnsupportedError(t *testing.T) {
	input := formMutationTestPDF(
		"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] >> >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [4 0 R] /Resources <<>> >>",
		"<< /FT /Tx /Subtype /Widget /T (payer.name) /Rect [10 20 110 40] /V (Alice) /P 3 0 R >>",
	)

	_, report, verification, err := FlattenFormFields(input)
	if err == nil {
		t.Fatal("expected flattening to fail closed")
	}
	var unsupported *UnsupportedFormFlatteningError
	if !errors.As(err, &unsupported) {
		t.Fatalf("error = %T %q, want UnsupportedFormFlatteningError", err, err)
	}
	if unsupported.Reason == "" || !strings.Contains(err.Error(), "unsupported AcroForm flattening") {
		t.Fatalf("unsupported error = %+v text=%q", unsupported, err.Error())
	}
	if report.Operation != "pdf.flatten_form_fields" || report.UnsupportedReason == "" {
		t.Fatalf("report = %+v", report)
	}
	if verification.ReparseOK || verification.Flattened {
		t.Fatalf("verification = %+v", verification)
	}
}

func catalogAcroFormFieldRef(t *testing.T, graph *pdfGraph, index int) (pdfRef, bool) {
	t.Helper()
	acroForm, ok := catalogAcroFormDict(graph)
	if !ok {
		return pdfRef{}, false
	}
	fields, ok := acroForm["Fields"].(pdfArray)
	if !ok || index < 0 || index >= len(fields) {
		return pdfRef{}, false
	}
	ref, ok := fields[index].(pdfRef)
	return ref, ok
}

func formMutationTestPDF(objects ...string) []byte {
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
