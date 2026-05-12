package pdf

import (
	"bytes"
	"encoding/json"
	"fmt"
	"reflect"
	"strings"
	"testing"
)

func TestApplyFormFieldEditUpdatesFieldValueAndNeedAppearances(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Tx /T (payer.name) /V (Old Name) >>",
	)

	output, report, verification, err := ApplyFormFieldEdit(input, "payer.name", "New (Name)")
	if err != nil {
		t.Fatal(err)
	}
	if report.Edit != "pdf.acroform_field_value_update" {
		t.Fatalf("edit = %q", report.Edit)
	}
	if report.FallbackUsed {
		t.Fatal("form field edit must not report fallback use")
	}
	if report.NodesModified != 1 {
		t.Fatalf("nodes modified = %d, want 1", report.NodesModified)
	}
	if !verification.ReparseOK {
		t.Fatalf("verification = %+v", verification)
	}
	if bytes.Contains(output, []byte("(Old Name)")) {
		t.Fatalf("old field value remains:\n%s", output)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	field, ok := graph.Objects[pdfObjectID{Number: 3, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("field object was not a dictionary")
	}
	if got := field["V"]; got != pdfLiteralString(`New \(Name\)`) {
		t.Fatalf("/V = %#v, want escaped literal", got)
	}
	acroForm, ok := graph.Objects[pdfObjectID{Number: 2, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("AcroForm object was not a dictionary")
	}
	if got := acroForm["NeedAppearances"]; got != true {
		t.Fatalf("/NeedAppearances = %#v, want true", got)
	}
}

func TestApplyFormFieldEditUpdatesDirectFieldDictionary(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm << /Fields [<< /FT /Tx /T (payer.name) /V (Old Name) >>] >> >>",
	)

	output, _, verification, err := ApplyFormFieldEdit(input, "payer.name", "New Name")
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK {
		t.Fatalf("verification = %+v", verification)
	}
	if bytes.Contains(output, []byte("(Old Name)")) {
		t.Fatalf("old direct field value remains:\n%s", output)
	}
	if !bytes.Contains(output, []byte("/V (New Name)")) {
		t.Fatalf("new direct field value missing:\n%s", output)
	}
	if !bytes.Contains(output, []byte("/NeedAppearances true")) {
		t.Fatalf("NeedAppearances was not set on direct AcroForm:\n%s", output)
	}
}

func TestApplyFormFieldEditMatchesHierarchicalFieldName(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /T (payer) /Kids [4 0 R] >>",
		"<< /FT /Tx /T (choice) /V (Old Choice) /Parent 3 0 R >>",
	)

	output, _, verification, err := ApplyFormFieldEdit(input, "payer.choice", "New Choice")
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK {
		t.Fatalf("verification = %+v", verification)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	parent, ok := graph.Objects[pdfObjectID{Number: 3, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("parent field object was not a dictionary")
	}
	child, ok := graph.Objects[pdfObjectID{Number: 4, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("child field object was not a dictionary")
	}
	if _, ok := parent["V"]; ok {
		t.Fatalf("parent /V was modified: %#v", parent["V"])
	}
	if got := child["V"]; got != pdfLiteralString("New Choice") {
		t.Fatalf("child /V = %#v, want new value", got)
	}
}

func TestApplyFormFieldEditAcceptsListedChoiceValue(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Ch /T (payer.plan) /V (Basic) /Opt [(Basic) (Pro)] >>",
	)

	output, _, verification, err := ApplyFormFieldEdit(input, "payer.plan", "Pro")
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK {
		t.Fatalf("verification = %+v", verification)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	field, ok := graph.Objects[pdfObjectID{Number: 3, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("field object was not a dictionary")
	}
	if got := field["V"]; got != pdfLiteralString("Pro") {
		t.Fatalf("/V = %#v, want listed literal value", got)
	}
	acroForm, ok := graph.Objects[pdfObjectID{Number: 2, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("AcroForm object was not a dictionary")
	}
	if got := acroForm["NeedAppearances"]; got != true {
		t.Fatalf("/NeedAppearances = %#v, want true", got)
	}
}

func TestApplyFormFieldEditRegeneratesTextWidgetAppearance(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Tx /T (payer.name) /V (Old Name) /Rect [0 0 120 20] >>",
	)

	output, report, verification, err := ApplyFormFieldEditWithOptions(input, "payer.name", "New Name", FormFieldEditOptions{RegenerateAppearance: true})
	if err != nil {
		t.Fatal(err)
	}
	if !report.AppearanceRegenerated {
		t.Fatalf("report did not record appearance regeneration: %+v", report)
	}
	if !verification.ReparseOK || !verification.AppearanceRegenerated {
		t.Fatalf("verification = %+v", verification)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	field, ok := graph.Objects[pdfObjectID{Number: 3, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("field object was not a dictionary")
	}
	stream := formTestNormalAppearanceStream(t, graph, field)
	if got := stream.Dict["Subtype"]; got != pdfName("Form") {
		t.Fatalf("appearance subtype = %#v, want /Form", got)
	}
	if !bytes.Contains(stream.Data, []byte("/Helv 10 Tf")) || !bytes.Contains(stream.Data, []byte("(New Name) Tj")) {
		t.Fatalf("appearance stream did not draw new text:\n%s", stream.Data)
	}
}

func TestApplyFormFieldEditRegeneratesMultilineWrappedTextWidgetAppearance(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Tx /T (payer.notes) /V (Old Notes) /Rect [0 0 64 40] >>",
	)

	output, _, verification, err := ApplyFormFieldEditWithOptions(input, "payer.notes", "first line\nsecond wraps here", FormFieldEditOptions{RegenerateAppearance: true})
	if err != nil {
		t.Fatal(err)
	}
	if !verification.AppearanceRegenerated {
		t.Fatalf("verification = %+v", verification)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	field, ok := graph.Objects[pdfObjectID{Number: 3, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("field object was not a dictionary")
	}
	stream := formTestNormalAppearanceStream(t, graph, field)
	for _, want := range [][]byte{
		[]byte("0 0 64 40 re W n"),
		[]byte("(first line) Tj"),
		[]byte("(second wraps) Tj"),
		[]byte("(here) Tj"),
		[]byte("0 -12 Td"),
	} {
		if !bytes.Contains(stream.Data, want) {
			t.Fatalf("appearance stream missing %q:\n%s", want, stream.Data)
		}
	}
}

func TestApplyFormFieldEditRegeneratesAppearanceTruncatesToRectHeight(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Tx /T (payer.notes) /V (Old Notes) /Rect [0 0 100 16] >>",
	)

	output, _, verification, err := ApplyFormFieldEditWithOptions(input, "payer.notes", "visible\ntruncated", FormFieldEditOptions{RegenerateAppearance: true})
	if err != nil {
		t.Fatal(err)
	}
	if !verification.AppearanceRegenerated {
		t.Fatalf("verification = %+v", verification)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	field, ok := graph.Objects[pdfObjectID{Number: 3, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("field object was not a dictionary")
	}
	stream := formTestNormalAppearanceStream(t, graph, field)
	if !bytes.Contains(stream.Data, []byte("(visible) Tj")) {
		t.Fatalf("visible line missing:\n%s", stream.Data)
	}
	if bytes.Contains(stream.Data, []byte("truncated")) {
		t.Fatalf("line outside rectangle was not truncated:\n%s", stream.Data)
	}
}

func TestApplyFormFieldEditRegeneratesChoiceWidgetAppearance(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Ch /T (payer.plan) /V (Basic) /Opt [(Basic) (Pro)] /Rect [0 0 100 18] >>",
	)

	output, report, verification, err := ApplyFormFieldEditWithOptions(input, "payer.plan", "Pro", FormFieldEditOptions{RegenerateAppearance: true})
	if err != nil {
		t.Fatal(err)
	}
	if !report.AppearanceRegenerated || !verification.AppearanceRegenerated {
		t.Fatalf("appearance regeneration not reported: report=%+v verification=%+v", report, verification)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	field, ok := graph.Objects[pdfObjectID{Number: 3, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("field object was not a dictionary")
	}
	stream := formTestNormalAppearanceStream(t, graph, field)
	if !bytes.Contains(stream.Data, []byte("(Pro) Tj")) {
		t.Fatalf("choice appearance stream did not draw selected value:\n%s", stream.Data)
	}
}

func TestApplyFormFieldEditRegenerateAppearanceFailsClosedWithoutRect(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Tx /T (payer.name) /V (Old Name) >>",
	)

	_, _, _, err := ApplyFormFieldEditWithOptions(input, "payer.name", "New Name", FormFieldEditOptions{RegenerateAppearance: true})
	if err == nil {
		t.Fatal("expected missing Rect error")
	}
	if !strings.Contains(err.Error(), "unsupported AcroForm appearance regeneration: widget /Rect is missing or not a direct numeric rectangle") {
		t.Fatalf("error = %q", err)
	}
}

func TestApplyFormFieldEditRegenerateAppearanceFailsClosedForButton(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /AP << /N << /Off <<>> /Yes <<>> >> >> /AS /Off /FT /Btn /T (payer.opt_in) /V /Off >>",
	)

	_, _, _, err := ApplyFormFieldEditWithOptions(input, "payer.opt_in", "true", FormFieldEditOptions{RegenerateAppearance: true})
	if err == nil {
		t.Fatal("expected unsupported button regeneration error")
	}
	if !strings.Contains(err.Error(), "unsupported AcroForm appearance regeneration for button fields") {
		t.Fatalf("error = %q", err)
	}
}

func TestApplyFormFieldEditRejectsUnlistedNonEditableChoiceValue(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Ch /T (payer.plan) /V (Basic) /Opt [(Basic) (Pro)] >>",
	)

	_, _, _, err := ApplyFormFieldEdit(input, "payer.plan", "Enterprise")
	if err == nil {
		t.Fatal("expected unlisted choice value error")
	}
	if !strings.Contains(err.Error(), `unsupported AcroForm choice field value "Enterprise"`) {
		t.Fatalf("error = %q", err)
	}
}

func TestApplyFormFieldEditAllowsUnlistedEditableChoiceValue(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		fmt.Sprintf("<< /FT /Ch /T (payer.plan) /V (Basic) /Ff %d /Opt [(Basic) (Pro)] >>", formFieldFlagChEdit),
	)

	output, _, verification, err := ApplyFormFieldEdit(input, "payer.plan", "Enterprise")
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK {
		t.Fatalf("verification = %+v", verification)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	field, ok := graph.Objects[pdfObjectID{Number: 3, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("field object was not a dictionary")
	}
	if got := field["V"]; got != pdfLiteralString("Enterprise") {
		t.Fatalf("/V = %#v, want editable literal value", got)
	}
}

func TestApplyFormFieldEditInheritedButtonFieldTypeUpdatesChildCheckbox(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Btn /T (payer) /Kids [4 0 R] >>",
		"<< /AP << /N << /Off <<>> /Yes <<>> >> >> /AS /Off /T (choice) /V /Off /Parent 3 0 R >>",
	)

	output, _, verification, err := ApplyFormFieldEdit(input, "payer.choice", "true")
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK {
		t.Fatalf("verification = %+v", verification)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	child, ok := graph.Objects[pdfObjectID{Number: 4, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("child field object was not a dictionary")
	}
	if _, ok := child["FT"]; ok {
		t.Fatalf("child unexpectedly gained /FT: %#v", child["FT"])
	}
	if got := child["V"]; got != pdfName("Yes") {
		t.Fatalf("child /V = %#v, want /Yes", got)
	}
	if got := child["AS"]; got != pdfName("Yes") {
		t.Fatalf("child /AS = %#v, want /Yes", got)
	}
}

func TestApplyFormFieldEditRequiresMatchIndexForDuplicateHierarchicalFieldNames(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R 5 0 R] >>",
		"<< /T (payer) /Kids [4 0 R] >>",
		"<< /FT /Tx /T (choice) /V (First Choice) /Parent 3 0 R >>",
		"<< /T (payer) /Kids [6 0 R] >>",
		"<< /FT /Tx /T (choice) /V (Second Choice) /Parent 5 0 R >>",
	)

	_, _, _, err := ApplyFormFieldEdit(input, "payer.choice", "New Choice")
	if err == nil {
		t.Fatal("expected duplicate hierarchical field error")
	}
	if !strings.Contains(err.Error(), `matched 2 dictionaries`) {
		t.Fatalf("error = %q", err)
	}

	matchIndex := 1
	output, _, verification, err := ApplyFormFieldEdit(input, "payer.choice", "Indexed Choice", &matchIndex)
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK {
		t.Fatalf("verification = %+v", verification)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	first, ok := graph.Objects[pdfObjectID{Number: 4, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("first child field object was not a dictionary")
	}
	second, ok := graph.Objects[pdfObjectID{Number: 6, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("second child field object was not a dictionary")
	}
	if got := first["V"]; got != pdfLiteralString("First Choice") {
		t.Fatalf("first child /V = %#v, want original value", got)
	}
	if got := second["V"]; got != pdfLiteralString("Indexed Choice") {
		t.Fatalf("second child /V = %#v, want indexed update", got)
	}
}

func TestApplyFormFieldEditUpdatesDirectCheckboxField(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /AP << /N << /Off <<>> /Yes <<>> >> >> /AS /Off /FT /Btn /T (payer.opt_in) /V /Off >>",
	)

	output, _, verification, err := ApplyFormFieldEdit(input, "payer.opt_in", "true")
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK {
		t.Fatalf("verification = %+v", verification)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	field, ok := graph.Objects[pdfObjectID{Number: 3, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("field object was not a dictionary")
	}
	if got := field["V"]; got != pdfName("Yes") {
		t.Fatalf("/V = %#v, want /Yes", got)
	}
	if got := field["AS"]; got != pdfName("Yes") {
		t.Fatalf("/AS = %#v, want /Yes", got)
	}
}

func TestApplyFormFieldEditUnchecksDirectCheckboxField(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /AP << /N << /Off <<>> /Yes <<>> >> >> /AS /Yes /FT /Btn /T (payer.opt_in) /V /Yes >>",
	)

	output, _, verification, err := ApplyFormFieldEdit(input, "payer.opt_in", "false")
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK {
		t.Fatalf("verification = %+v", verification)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	field, ok := graph.Objects[pdfObjectID{Number: 3, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("field object was not a dictionary")
	}
	if got := field["V"]; got != pdfName("Off") {
		t.Fatalf("/V = %#v, want /Off", got)
	}
	if got := field["AS"]; got != pdfName("Off") {
		t.Fatalf("/AS = %#v, want /Off", got)
	}
}

func TestApplyFormFieldEditUpdatesCheckboxFieldWithWidgetKid(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Btn /Kids [4 0 R] /T (payer.opt_in) /V /Off >>",
		"<< /AP << /N << /Off <<>> /Checked <<>> >> >> /AS /Off /Parent 3 0 R /Subtype /Widget >>",
	)

	output, _, verification, err := ApplyFormFieldEdit(input, "payer.opt_in", "Checked")
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK {
		t.Fatalf("verification = %+v", verification)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	field, ok := graph.Objects[pdfObjectID{Number: 3, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("field object was not a dictionary")
	}
	widget, ok := graph.Objects[pdfObjectID{Number: 4, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("widget object was not a dictionary")
	}
	if got := field["V"]; got != pdfName("Checked") {
		t.Fatalf("field /V = %#v, want /Checked", got)
	}
	if got := widget["AS"]; got != pdfName("Checked") {
		t.Fatalf("widget /AS = %#v, want /Checked", got)
	}
}

func TestApplyFormFieldEditSelectsRadioWidgetKid(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Btn /Kids [4 0 R 5 0 R] /T (payer.plan) /V /Off >>",
		"<< /AP << /N << /Off <<>> /Basic <<>> >> >> /AS /Off /Parent 3 0 R /Subtype /Widget >>",
		"<< /AP << /N << /Off <<>> /Pro <<>> >> >> /AS /Off /Parent 3 0 R /Subtype /Widget >>",
	)

	output, _, verification, err := ApplyFormFieldEdit(input, "payer.plan", "Pro")
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK {
		t.Fatalf("verification = %+v", verification)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	field, ok := graph.Objects[pdfObjectID{Number: 3, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("field object was not a dictionary")
	}
	firstWidget, ok := graph.Objects[pdfObjectID{Number: 4, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("first widget object was not a dictionary")
	}
	secondWidget, ok := graph.Objects[pdfObjectID{Number: 5, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("second widget object was not a dictionary")
	}
	if got := field["V"]; got != pdfName("Pro") {
		t.Fatalf("field /V = %#v, want /Pro", got)
	}
	if got := firstWidget["AS"]; got != pdfName("Off") {
		t.Fatalf("first widget /AS = %#v, want /Off", got)
	}
	if got := secondWidget["AS"]; got != pdfName("Pro") {
		t.Fatalf("second widget /AS = %#v, want /Pro", got)
	}
}

func TestApplyFormFieldEditClearsRadioWidgetKidsWithOff(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Btn /Kids [4 0 R 5 0 R] /T (payer.plan) /V /Pro >>",
		"<< /AP << /N << /Off <<>> /Basic <<>> >> >> /AS /Off /Parent 3 0 R /Subtype /Widget >>",
		"<< /AP << /N << /Off <<>> /Pro <<>> >> >> /AS /Pro /Parent 3 0 R /Subtype /Widget >>",
	)

	output, _, verification, err := ApplyFormFieldEdit(input, "payer.plan", "Off")
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK {
		t.Fatalf("verification = %+v", verification)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatal(err)
	}
	field, ok := graph.Objects[pdfObjectID{Number: 3, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("field object was not a dictionary")
	}
	firstWidget, ok := graph.Objects[pdfObjectID{Number: 4, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("first widget object was not a dictionary")
	}
	secondWidget, ok := graph.Objects[pdfObjectID{Number: 5, Generation: 0}].Value.(pdfDict)
	if !ok {
		t.Fatal("second widget object was not a dictionary")
	}
	if got := field["V"]; got != pdfName("Off") {
		t.Fatalf("field /V = %#v, want /Off", got)
	}
	if got := firstWidget["AS"]; got != pdfName("Off") {
		t.Fatalf("first widget /AS = %#v, want /Off", got)
	}
	if got := secondWidget["AS"]; got != pdfName("Off") {
		t.Fatalf("second widget /AS = %#v, want /Off", got)
	}
}

func TestApplyFormFieldEditFailsClosedForDuplicateRadioWidgetState(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Btn /Kids [4 0 R 5 0 R] /T (payer.plan) /V /Off >>",
		"<< /AP << /N << /Off <<>> /Pro <<>> >> >> /AS /Off /Parent 3 0 R /Subtype /Widget >>",
		"<< /AP << /N << /Off <<>> /Pro <<>> >> >> /AS /Off /Parent 3 0 R /Subtype /Widget >>",
	)

	_, _, _, err := ApplyFormFieldEdit(input, "payer.plan", "Pro")
	if err == nil {
		t.Fatal("expected duplicate radio state error")
	}
	if !strings.Contains(err.Error(), `duplicate button appearance state "Pro"`) {
		t.Fatalf("error = %q", err)
	}
}

func TestApplyFormFieldEditFailsClosedForCheckboxWithoutAppearanceProof(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /AS /Off /FT /Btn /T (payer.opt_in) /V /Off >>",
	)

	_, _, _, err := ApplyFormFieldEdit(input, "payer.opt_in", "true")
	if err == nil {
		t.Fatal("expected missing appearance proof error")
	}
	if !strings.Contains(err.Error(), "missing /AP proof") {
		t.Fatalf("error = %q", err)
	}
}

func TestApplyFormFieldEditFailsClosedForAmbiguousCheckboxOnValue(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /AP << /N << /Off <<>> /One <<>> /Two <<>> >> >> /AS /Off /FT /Btn /T (payer.choice) /V /Off >>",
	)

	_, _, _, err := ApplyFormFieldEdit(input, "payer.choice", "On")
	if err == nil {
		t.Fatal("expected ambiguous on-state error")
	}
	if !strings.Contains(err.Error(), "ambiguous button appearance states") {
		t.Fatalf("error = %q", err)
	}
}

func TestApplyFormFieldEditFailsClosedWhenNoFieldMatches(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Tx /T (payer.name) /V (Old Name) >>",
	)

	_, _, _, err := ApplyFormFieldEdit(input, "missing", "New Name")
	if err == nil {
		t.Fatal("expected no-match error")
	}
	if !strings.Contains(err.Error(), `no AcroForm field matches "missing"`) {
		t.Fatalf("error = %q", err)
	}
}

func TestListFormFieldsIncludesFlatFieldMetadata(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R << /FT /Ch /T (payer.choice) /V <FEFF0041> >>] >>",
		"<< /FT /Tx /T (payer.name) /V (Old Name) >>",
	)

	fields, err := ListFormFields(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(fields) != 2 {
		t.Fatalf("field count = %d, want 2: %+v", len(fields), fields)
	}
	first := fields[0]
	if first.Index != 0 || first.Name != "payer.name" || first.ObjectNumber == nil || *first.ObjectNumber != 3 || first.ObjectGeneration == nil || *first.ObjectGeneration != 0 {
		t.Fatalf("first field object metadata = %+v, want index 0 object 3 0", first)
	}
	if first.FieldType != "Tx" || first.Value == nil || *first.Value != "Old Name" || first.KidCount != 0 || first.ButtonWidgetAppearanceProof {
		t.Fatalf("first field metadata = %+v", first)
	}
	second := fields[1]
	if second.Index != 1 || second.Name != "payer.choice" || second.ObjectNumber != nil || second.ObjectGeneration != nil {
		t.Fatalf("second field object metadata = %+v, want direct index 1", second)
	}
	if second.FieldType != "Ch" || second.Value == nil || *second.Value != "A" || second.KidCount != 0 || second.ButtonWidgetAppearanceProof {
		t.Fatalf("second field metadata = %+v", second)
	}
}

func TestListFormFieldsIncludesLiteralFieldStringMetadata(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Tx /T (payer.name) /TU (Payer Name) /TM (payer_name_export) /V (Old Name) /DV (Default Name) >>",
	)

	fields, err := ListFormFields(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(fields) != 1 {
		t.Fatalf("field count = %d, want 1: %+v", len(fields), fields)
	}
	field := fields[0]
	if field.AlternateName == nil || *field.AlternateName != "Payer Name" {
		t.Fatalf("alternate name = %+v, want Payer Name", field.AlternateName)
	}
	if field.MappingName == nil || *field.MappingName != "payer_name_export" {
		t.Fatalf("mapping name = %+v, want payer_name_export", field.MappingName)
	}
	if field.DefaultValue == nil || *field.DefaultValue != "Default Name" {
		t.Fatalf("default value = %+v, want Default Name", field.DefaultValue)
	}

	encoded, err := json.Marshal(field)
	if err != nil {
		t.Fatal(err)
	}
	for _, key := range []string{`"alternate_name":"Payer Name"`, `"mapping_name":"payer_name_export"`, `"default_value":"Default Name"`} {
		if !bytes.Contains(encoded, []byte(key)) {
			t.Fatalf("JSON metadata %s missing from %s", key, encoded)
		}
	}
}

func TestListFormFieldsIncludesHexFieldStringMetadata(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Tx /T (payer.name) /TU <FEFF0041006C0074> /TM <6D61705F6E616D65> /DV <FEFF004400650066> >>",
	)

	fields, err := ListFormFields(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(fields) != 1 {
		t.Fatalf("field count = %d, want 1: %+v", len(fields), fields)
	}
	field := fields[0]
	if field.AlternateName == nil || *field.AlternateName != "Alt" {
		t.Fatalf("alternate name = %+v, want Alt", field.AlternateName)
	}
	if field.MappingName == nil || *field.MappingName != "map_name" {
		t.Fatalf("mapping name = %+v, want map_name", field.MappingName)
	}
	if field.DefaultValue == nil || *field.DefaultValue != "Def" {
		t.Fatalf("default value = %+v, want Def", field.DefaultValue)
	}
}

func TestListFormFieldsOmitsUnsupportedFieldStringMetadata(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Tx /T (payer.name) /TU /AltName /TM [(map)] /DV << /Unsupported true >> >>",
	)

	fields, err := ListFormFields(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(fields) != 1 {
		t.Fatalf("field count = %d, want 1: %+v", len(fields), fields)
	}
	field := fields[0]
	if field.AlternateName != nil || field.MappingName != nil || field.DefaultValue != nil {
		t.Fatalf("unsupported metadata should be omitted: %+v", field)
	}

	encoded, err := json.Marshal(field)
	if err != nil {
		t.Fatal(err)
	}
	for _, key := range []string{`alternate_name`, `mapping_name`, `default_value`} {
		if bytes.Contains(encoded, []byte(key)) {
			t.Fatalf("unsupported JSON metadata %s present in %s", key, encoded)
		}
	}
}

func TestListFormFieldsIncludesDirectFieldFlags(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Tx /T (payer.name) /V (Old Name) /Ff 7 >>",
	)

	fields, err := ListFormFields(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(fields) != 1 {
		t.Fatalf("field count = %d, want 1: %+v", len(fields), fields)
	}
	field := fields[0]
	if field.Flags == nil || *field.Flags != 7 {
		t.Fatalf("flags = %+v, want 7", field.Flags)
	}
	if got, want := field.FlagNames, []string{"read_only", "required", "no_export"}; !reflect.DeepEqual(got, want) {
		t.Fatalf("flag names = %+v, want %+v", got, want)
	}
	if !field.ReadOnly || !field.Required || !field.NoExport {
		t.Fatalf("decoded field flags = %+v, want read_only required no_export", field)
	}

	encoded, err := json.Marshal(field)
	if err != nil {
		t.Fatal(err)
	}
	for _, key := range []string{`"flags":7`, `"flag_names":["read_only","required","no_export"]`, `"read_only":true`, `"required":true`, `"no_export":true`} {
		if !bytes.Contains(encoded, []byte(key)) {
			t.Fatalf("JSON metadata %s missing from %s", key, encoded)
		}
	}
}

func TestListFormFieldsIncludesInheritedFieldFlags(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Tx /T (payer) /Ff 3 /Kids [4 0 R] >>",
		"<< /T (name) /V (Old Name) /Parent 3 0 R >>",
	)

	fields, err := ListFormFields(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(fields) != 2 {
		t.Fatalf("field count = %d, want 2: %+v", len(fields), fields)
	}
	parent := fields[0]
	if parent.Flags == nil || *parent.Flags != 3 || !parent.ReadOnly || !parent.Required || parent.NoExport {
		t.Fatalf("parent flags = %+v, want read_only required only", parent)
	}
	child := fields[1]
	if child.Flags == nil || *child.Flags != 3 {
		t.Fatalf("child flags = %+v, want inherited 3", child.Flags)
	}
	if got, want := child.FlagNames, []string{"read_only", "required"}; !reflect.DeepEqual(got, want) {
		t.Fatalf("child flag names = %+v, want %+v", got, want)
	}
	if !child.ReadOnly || !child.Required || child.NoExport {
		t.Fatalf("child decoded flags = %+v, want inherited read_only required only", child)
	}
}

func TestListFormFieldsIncludesTextTypeFlagNames(t *testing.T) {
	flags := formFieldFlagReadOnly |
		formFieldFlagTxMultiline |
		formFieldFlagTxPassword |
		formFieldFlagTxFileSelect |
		formFieldFlagDoNotSpellCheck |
		formFieldFlagTxDoNotScroll |
		formFieldFlagTxComb |
		formFieldFlagTxRichText
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		fmt.Sprintf("<< /FT /Tx /T (payer.notes) /Ff %d >>", flags),
	)

	fields, err := ListFormFields(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(fields) != 1 {
		t.Fatalf("field count = %d, want 1: %+v", len(fields), fields)
	}
	field := fields[0]
	if got, want := field.FlagNames, []string{"read_only"}; !reflect.DeepEqual(got, want) {
		t.Fatalf("shared flag names = %+v, want %+v", got, want)
	}
	wantTypeFlags := []string{"multiline", "password", "file_select", "do_not_spell_check", "do_not_scroll", "comb", "rich_text"}
	if got := field.TypeFlagNames; !reflect.DeepEqual(got, wantTypeFlags) {
		t.Fatalf("text type flag names = %+v, want %+v", got, wantTypeFlags)
	}

	encoded, err := json.Marshal(field)
	if err != nil {
		t.Fatal(err)
	}
	wantJSON := `"type_flag_names":["multiline","password","file_select","do_not_spell_check","do_not_scroll","comb","rich_text"]`
	if !bytes.Contains(encoded, []byte(wantJSON)) {
		t.Fatalf("JSON type flag names missing from %s", encoded)
	}
}

func TestListFormFieldsIncludesButtonTypeFlagNames(t *testing.T) {
	flags := formFieldFlagRequired |
		formFieldFlagBtnNoToggleToOff |
		formFieldFlagBtnRadio |
		formFieldFlagBtnPushbutton |
		formFieldFlagBtnRadiosInUnison
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		fmt.Sprintf("<< /FT /Btn /T (payer.choice) /Ff %d >>", flags),
	)

	fields, err := ListFormFields(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(fields) != 1 {
		t.Fatalf("field count = %d, want 1: %+v", len(fields), fields)
	}
	field := fields[0]
	if got, want := field.FlagNames, []string{"required"}; !reflect.DeepEqual(got, want) {
		t.Fatalf("shared flag names = %+v, want %+v", got, want)
	}
	wantTypeFlags := []string{"no_toggle_to_off", "radio", "pushbutton", "radios_in_unison"}
	if got := field.TypeFlagNames; !reflect.DeepEqual(got, wantTypeFlags) {
		t.Fatalf("button type flag names = %+v, want %+v", got, wantTypeFlags)
	}
}

func TestListFormFieldsIncludesChoiceTypeFlagNames(t *testing.T) {
	flags := formFieldFlagNoExport |
		formFieldFlagChCombo |
		formFieldFlagChEdit |
		formFieldFlagChSort |
		formFieldFlagChMultiSelect |
		formFieldFlagDoNotSpellCheck |
		formFieldFlagChCommitOnSelChange
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		fmt.Sprintf("<< /FT /Ch /T (payer.plan) /Ff %d >>", flags),
	)

	fields, err := ListFormFields(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(fields) != 1 {
		t.Fatalf("field count = %d, want 1: %+v", len(fields), fields)
	}
	field := fields[0]
	if got, want := field.FlagNames, []string{"no_export"}; !reflect.DeepEqual(got, want) {
		t.Fatalf("shared flag names = %+v, want %+v", got, want)
	}
	wantTypeFlags := []string{"combo", "edit", "sort", "multi_select", "do_not_spell_check", "commit_on_sel_change"}
	if got := field.TypeFlagNames; !reflect.DeepEqual(got, wantTypeFlags) {
		t.Fatalf("choice type flag names = %+v, want %+v", got, wantTypeFlags)
	}
}

func TestListFormFieldsIncludesInheritedTypeFlagNames(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R 5 0 R] >>",
		fmt.Sprintf("<< /FT /Tx /T (payer) /Ff %d /Kids [4 0 R] >>", formFieldFlagTxMultiline|formFieldFlagTxDoNotScroll),
		"<< /T (notes) /Parent 3 0 R >>",
		"<< /FT /Ch /T (plan) /Kids [6 0 R] >>",
		fmt.Sprintf("<< /T (primary) /Ff %d /Parent 5 0 R >>", formFieldFlagChCombo|formFieldFlagChCommitOnSelChange),
	)

	fields, err := ListFormFields(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(fields) != 4 {
		t.Fatalf("field count = %d, want 4: %+v", len(fields), fields)
	}
	if got, want := fields[1].TypeFlagNames, []string{"multiline", "do_not_scroll"}; !reflect.DeepEqual(got, want) {
		t.Fatalf("child inherited text type flags = %+v, want %+v", got, want)
	}
	if got, want := fields[3].TypeFlagNames, []string{"combo", "commit_on_sel_change"}; !reflect.DeepEqual(got, want) {
		t.Fatalf("child direct flags with inherited choice type = %+v, want %+v", got, want)
	}
}

func TestListFormFieldsIncludesChoiceOptions(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R 4 0 R 5 0 R 6 0 R] >>",
		"<< /FT /Ch /T (payer.plan) /V (basic) /Opt [(Basic) [(pro) (Pro Plan)] [(fallback) 42] 42] >>",
		"<< /FT /Tx /T (payer.name) /V (Old Name) /Opt [(Ignored)] >>",
		"<< /FT /Ch /T (payer.unsupported) /Opt 42 >>",
		"<< /FT /Ch /T (payer.single) /Opt (Only Choice) >>",
	)

	fields, err := ListFormFields(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(fields) != 4 {
		t.Fatalf("field count = %d, want 4: %+v", len(fields), fields)
	}
	if got, want := fields[0].Options, []string{"Basic", "Pro Plan", "fallback"}; !reflect.DeepEqual(got, want) {
		t.Fatalf("choice options = %+v, want %+v", got, want)
	}
	if fields[1].Options != nil {
		t.Fatalf("text field options = %+v, want nil", fields[1].Options)
	}
	if fields[2].Options != nil {
		t.Fatalf("unsupported choice options = %+v, want nil", fields[2].Options)
	}
	if got, want := fields[3].Options, []string{"Only Choice"}; !reflect.DeepEqual(got, want) {
		t.Fatalf("single choice option = %+v, want %+v", got, want)
	}
}

func TestListFormFieldsIncludesInheritedChoiceOptions(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Ch /T (payer) /Kids [4 0 R] >>",
		"<< /T (plan) /V (basic) /Opt [(Basic) [(pro) (Pro Plan)]] /Parent 3 0 R >>",
	)

	fields, err := ListFormFields(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(fields) != 2 {
		t.Fatalf("field count = %d, want 2: %+v", len(fields), fields)
	}
	if fields[0].Options != nil {
		t.Fatalf("parent options = %+v, want nil", fields[0].Options)
	}
	if got, want := fields[1].Options, []string{"Basic", "Pro Plan"}; !reflect.DeepEqual(got, want) {
		t.Fatalf("inherited choice options = %+v, want %+v", got, want)
	}
}

func TestListFormFieldsIncludesParentChildInheritedButtonMetadata(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Btn /T (payer) /Kids [4 0 R] >>",
		"<< /AP << /N << /Off <<>> /Yes <<>> >> >> /AS /Off /T (choice) /V /Off /Parent 3 0 R >>",
	)

	fields, err := ListFormFields(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(fields) != 2 {
		t.Fatalf("field count = %d, want 2: %+v", len(fields), fields)
	}
	parent := fields[0]
	if parent.Index != 0 || parent.Name != "payer" || parent.FieldType != "Btn" || parent.KidCount != 1 {
		t.Fatalf("parent metadata = %+v", parent)
	}
	if !parent.ButtonWidgetAppearanceProof {
		t.Fatalf("parent should report widget appearance proof from its kid: %+v", parent)
	}
	child := fields[1]
	if child.Index != 1 || child.Name != "payer.choice" || child.ObjectNumber == nil || *child.ObjectNumber != 4 {
		t.Fatalf("child object metadata = %+v, want index 1 object 4", child)
	}
	if child.FieldType != "Btn" || child.Value == nil || *child.Value != "Off" || child.KidCount != 0 || !child.ButtonWidgetAppearanceProof {
		t.Fatalf("child inherited button metadata = %+v", child)
	}
}

func TestListFormFieldsIncludesSortedButtonAppearanceStates(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R] >>",
		"<< /FT /Btn /T (payer) /Kids [4 0 R 5 0 R] >>",
		"<< /AP << /N << /Off <<>> /Pro <<>> >> >> /AS /Off /T (choice) /V /Off /Parent 3 0 R >>",
		"<< /AP << /N << /Off <<>> /Basic <<>> >> >> /AS /Off /T (backup) /V /Off /Parent 3 0 R >>",
	)

	fields, err := ListFormFields(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(fields) != 3 {
		t.Fatalf("field count = %d, want 3: %+v", len(fields), fields)
	}
	if got, want := fields[0].ButtonStates, []string{"Basic", "Off", "Pro"}; !reflect.DeepEqual(got, want) {
		t.Fatalf("parent button states = %+v, want %+v", got, want)
	}
	if got, want := fields[1].ButtonStates, []string{"Off", "Pro"}; !reflect.DeepEqual(got, want) {
		t.Fatalf("child button states = %+v, want %+v", got, want)
	}
	if got, want := fields[2].ButtonStates, []string{"Basic", "Off"}; !reflect.DeepEqual(got, want) {
		t.Fatalf("second child button states = %+v, want %+v", got, want)
	}
	if !fields[0].ButtonWidgetAppearanceProof || !fields[1].ButtonWidgetAppearanceProof || !fields[2].ButtonWidgetAppearanceProof {
		t.Fatalf("appearance proof changed unexpectedly: %+v", fields)
	}
}

func TestApplyFormFieldEditFailsClosedWhenMultipleFieldsMatch(t *testing.T) {
	input := formTestPDF(
		"<< /Type /Catalog /AcroForm 2 0 R >>",
		"<< /Fields [3 0 R 4 0 R] >>",
		"<< /FT /Tx /T (payer.name) /V (Old Name) >>",
		"<< /FT /Tx /T (payer.name) /V (Other Name) >>",
	)

	_, _, _, err := ApplyFormFieldEdit(input, "payer.name", "New Name")
	if err == nil {
		t.Fatal("expected multiple-match error")
	}
	if !strings.Contains(err.Error(), `matched 2 dictionaries`) {
		t.Fatalf("error = %q", err)
	}
}

func TestApplyFormFieldEditFailsClosedForXFAForms(t *testing.T) {
	input := formTestPDF("<< /Type /Catalog /AcroForm << /XFA (<template>old</template>) /Fields [] >> >>")

	_, _, _, err := ApplyFormFieldEdit(input, "payer.name", "New Name")
	if err == nil {
		t.Fatal("expected XFA boundary error")
	}
	if err.Error() != "unsupported PDF: XFA forms are not implemented" {
		t.Fatalf("error = %q", err)
	}
}

func formTestNormalAppearanceStream(t *testing.T, graph *pdfGraph, widget pdfDict) pdfStreamObject {
	t.Helper()
	ap, ok := widget["AP"].(pdfDict)
	if !ok {
		t.Fatalf("widget /AP = %#v, want dictionary", widget["AP"])
	}
	normalRef, ok := ap["N"].(pdfRef)
	if !ok {
		t.Fatalf("widget /AP /N = %#v, want indirect stream ref", ap["N"])
	}
	object, ok := graph.Objects[normalRef.ID]
	if !ok {
		t.Fatalf("appearance object %v missing", normalRef.ID)
	}
	stream, ok := object.Value.(pdfStreamObject)
	if !ok {
		t.Fatalf("appearance object = %#v, want stream", object.Value)
	}
	return stream
}

func formTestPDF(objects ...string) []byte {
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
