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
