package pdf

import (
	"bytes"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestListXFADatasetFieldsDiscoversSimplePathsAndValues(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(template) (<template><field name=\"ignored\"/></template>) (datasets) (<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\"><xfa:data><form1><payer><name>David</name><email>david@example.test</email></payer><amount>42</amount><empty/></form1></xfa:data></xfa:datasets>)] >> >>")

	fields, err := ListXFADatasetFields(input)
	if err != nil {
		t.Fatal(err)
	}

	want := []XFADatasetField{
		{PacketIndex: 1, Label: "datasets", Path: "form1.payer.name", Value: "David"},
		{PacketIndex: 1, Label: "datasets", Path: "form1.payer.email", Value: "david@example.test"},
		{PacketIndex: 1, Label: "datasets", Path: "form1.amount", Value: "42"},
	}
	if len(fields) != len(want) {
		t.Fatalf("fields = %+v, want %d fields", fields, len(want))
	}
	for i := range want {
		if fields[i] != want[i] {
			t.Fatalf("field %d = %+v, want %+v", i, fields[i], want[i])
		}
	}
}

func TestListXFADatasetFieldsFindsDatasetsInsideXDP(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA (<xdp:xdp xmlns:xdp=\"http://ns.adobe.com/xdp/\"><template><field name=\"ignored\"/></template><xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\"><xfa:data><invoice><number>INV-7</number></invoice></xfa:data></xfa:datasets></xdp:xdp>) >> >>")

	fields, err := ListXFADatasetFields(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(fields) != 1 {
		t.Fatalf("fields = %+v, want one field", fields)
	}
	if fields[0].PacketIndex != 0 || fields[0].Path != "invoice.number" || fields[0].Value != "INV-7" {
		t.Fatalf("field = %+v", fields[0])
	}
}

func TestListXFADatasetFieldsWithOptionsFiltersBySelector(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(left) (<datasets><data><field>one</field></data></datasets>) /right (<datasets><data><field>two</field></data></datasets>) (xdp) (<xdp:xdp xmlns:xdp=\"http://ns.adobe.com/xdp/\"><datasets><data><field>three</field></data></datasets></xdp:xdp>)] >> >>")

	fields, err := ListXFADatasetFieldsWithOptions(input, XFADatasetFieldListOptions{
		Selector: XFASelector{Label: "right"},
	})
	if err != nil {
		t.Fatal(err)
	}

	want := []XFADatasetField{{PacketIndex: 1, Label: "right", Path: "field", Value: "two"}}
	if len(fields) != len(want) {
		t.Fatalf("fields = %+v, want %d field", fields, len(want))
	}
	if fields[0] != want[0] {
		t.Fatalf("field = %+v, want %+v", fields[0], want[0])
	}

	fields, err = ListXFADatasetFieldsWithOptions(input, XFADatasetFieldListOptions{
		Selector: XFASelector{PacketKind: "xdp"},
	})
	if err != nil {
		t.Fatal(err)
	}
	if len(fields) != 1 || fields[0].PacketIndex != 2 || fields[0].Label != "xdp" || fields[0].Value != "three" {
		t.Fatalf("xdp fields = %+v", fields)
	}
}

func TestListXFADatasetFieldsRejectsUnsafeXML(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(datasets) (<!DOCTYPE datasets [<!ENTITY secret SYSTEM \"file:///etc/passwd\">]><datasets><data><field>&secret;</field></data></datasets>)] >> >>")

	_, err := ListXFADatasetFields(input)
	if err == nil {
		t.Fatal("expected unsafe XML to fail closed")
	}
	if err.Error() != "XFA datasets packet 0: unsafe XML declaration: DOCTYPE and ENTITY are not supported" {
		t.Fatalf("error = %q", err)
	}
}

func TestApplyXFADatasetFieldUpdateWithOptionsUpdatesExactlyOneLeaf(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(template) (<template><field name=\"ignored\"/></template>) (datasets) (<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\"><xfa:data><form1><payer><name>David</name><email>david@example.test</email></payer></form1></xfa:data></xfa:datasets>)] >> >>")

	output, report, verification, err := ApplyXFADatasetFieldUpdateWithOptions(input, "form1.payer.name", "Ana & Co", XFADatasetFieldUpdateOptions{})
	if err != nil {
		t.Fatal(err)
	}

	if !bytes.Contains(output, []byte("(template) (<template><field name=\"ignored\"/></template>) (datasets)")) {
		t.Fatalf("XFA packet order or labels changed:\n%s", output)
	}
	if bytes.Contains(output, []byte("<name>David</name>")) {
		t.Fatalf("old dataset value remains:\n%s", output)
	}
	if !bytes.Contains(output, []byte("<name>Ana &amp; Co</name>")) {
		t.Fatalf("escaped dataset value missing:\n%s", output)
	}
	if report.Edit != "pdf.xfa_dataset_field_update" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v", report)
	}
	if !verification.ReparseOK || !verification.NewSelectable || !verification.OldTextRemoved {
		t.Fatalf("verification = %+v", verification)
	}

	fields, err := ListXFADatasetFields(output)
	if err != nil {
		t.Fatal(err)
	}
	if len(fields) != 2 || fields[0].Path != "form1.payer.name" || fields[0].Value != "Ana & Co" {
		t.Fatalf("fields = %+v", fields)
	}
}

func TestApplyXFADatasetFieldUpdateWithOptionsSelectsPacketLabel(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(left) (<datasets><data><field>one</field></data></datasets>) (right) (<datasets><data><field>two</field></data></datasets>)] >> >>")

	output, _, _, err := ApplyXFADatasetFieldUpdateWithOptions(input, "field", "updated", XFADatasetFieldUpdateOptions{
		Selector: XFASelector{Label: "right"},
	})
	if err != nil {
		t.Fatal(err)
	}

	if !bytes.Contains(output, []byte("<field>one</field>")) {
		t.Fatalf("unselected dataset packet changed:\n%s", output)
	}
	if !bytes.Contains(output, []byte("<field>updated</field>")) || bytes.Contains(output, []byte("<field>two</field>")) {
		t.Fatalf("selected dataset packet was not updated:\n%s", output)
	}
}

func TestApplyXFADatasetFieldUpdateWithOptionsRejectsUnsafeXML(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(datasets) (<!DOCTYPE datasets [<!ENTITY secret SYSTEM \"file:///etc/passwd\">]><datasets><data><field>old</field></data></datasets>)] >> >>")

	_, _, _, err := ApplyXFADatasetFieldUpdateWithOptions(input, "field", "new", XFADatasetFieldUpdateOptions{})
	if err == nil {
		t.Fatal("expected unsafe XML to fail closed")
	}
	if err.Error() != "XFA datasets packet 0: unsafe XML declaration: DOCTYPE and ENTITY are not supported" {
		t.Fatalf("error = %q", err)
	}
}

func TestApplyXFADatasetFieldUpdateWithOptionsRejectsAmbiguousPath(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(datasets) (<datasets><data><form><field>one</field><field>two</field></form></data></datasets>)] >> >>")

	_, _, _, err := ApplyXFADatasetFieldUpdateWithOptions(input, "form.field", "new", XFADatasetFieldUpdateOptions{})
	if err == nil {
		t.Fatal("expected ambiguous dataset path to fail closed")
	}
	if err.Error() != `XFA dataset field path "form.field" is ambiguous: 2 matches` {
		t.Fatalf("error = %q", err)
	}
}

func TestApplyXFADatasetFieldUpdateWithOptionsRejectsMissingPath(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(datasets) (<datasets><data><form><field>old</field></form></data></datasets>)] >> >>")

	_, _, _, err := ApplyXFADatasetFieldUpdateWithOptions(input, "form.missing", "new", XFADatasetFieldUpdateOptions{})
	if err == nil {
		t.Fatal("expected missing dataset path to fail closed")
	}
	if err.Error() != `no XFA dataset field path "form.missing"` {
		t.Fatalf("error = %q", err)
	}
}

func TestApplyXFADatasetFieldUpdateWithOptionsRejectsContainerPath(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(datasets) (<datasets><data><form><group><field>old</field></group></form></data></datasets>)] >> >>")

	_, _, _, err := ApplyXFADatasetFieldUpdateWithOptions(input, "form.group", "new", XFADatasetFieldUpdateOptions{})
	if err == nil {
		t.Fatal("expected container dataset path to fail closed")
	}
	if err.Error() != `XFA dataset field path "form.group" is not a leaf field` {
		t.Fatalf("error = %q", err)
	}
}

func TestXFADatasetReadDoesNotOpenUnsupportedEditBoundary(t *testing.T) {
	content := []byte("BT\n(old) Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog /AcroForm << /XFA [(datasets) (<datasets><data><field>old</field></data></datasets>)] /Fields [] >> >>",
		string(bytes.Join([][]byte{
			[]byte("<< /Length 15 >>"),
			[]byte("stream"),
			content,
			[]byte("endstream"),
		}, []byte("\n"))),
	)

	fields, err := ListXFADatasetFields(input)
	if err != nil {
		t.Fatal(err)
	}
	if len(fields) != 1 || fields[0].Path != "field" || fields[0].Value != "old" {
		t.Fatalf("fields = %+v", fields)
	}

	output, report, verification, err := ApplyCanonicalEdit(input, core.Match{Kind: KindTextShow, Text: "old"}, core.Mutation{Replace: "new"}, nil)
	if err == nil {
		t.Fatal("expected canonical edit to keep refusing XFA")
	}
	if err.Error() != "unsupported PDF: XFA forms are not implemented" {
		t.Fatalf("error = %q", err)
	}
	if output != nil || report.FallbackUsed || verification.ReparseOK {
		t.Fatalf("generic XFA refusal leaked output/report/verification: output=%q report=%+v verification=%+v", output, report, verification)
	}
}
