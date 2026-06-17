package pdf

import "testing"

func TestInspectXFADynamicReportsNoXFA(t *testing.T) {
	input := testPDF("<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [] /Count 0 >>")

	report, err := InspectXFADynamic(input)
	if err != nil {
		t.Fatal(err)
	}

	if report.Present || report.Dynamic || report.Static {
		t.Fatalf("report = %+v", report)
	}
	if len(report.Markers) != 0 || len(report.UnsupportedReasons) != 0 {
		t.Fatalf("unexpected markers/reasons = %+v", report)
	}
}

func TestInspectXFADynamicClassifiesStaticTemplateDatasets(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(template) (<template><subform name=\"form\"><field name=\"name\"/></subform></template>) (datasets) (<datasets><data><form><name>Alice</name></form></data></datasets>)] >> >>")

	report, err := InspectXFADynamic(input)
	if err != nil {
		t.Fatal(err)
	}

	if !report.Present || report.Dynamic || !report.Static {
		t.Fatalf("report = %+v", report)
	}
	if len(report.Markers) != 0 || len(report.UnsupportedReasons) != 0 {
		t.Fatalf("unexpected dynamic metadata = %+v", report)
	}
}

func TestInspectXFADynamicReportsDynamicMarkersAndReasons(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(template) (<template><subform name=\"rows\" layout=\"flowed\"><occur max=\"-1\"/><breakBefore/></subform><field name=\"hidden\" presence=\"hidden\"/></template>) (config) (<config><present><pdf><dynamicRender>required</dynamicRender></pdf></present></config>) (datasets) (<datasets><data><rows><name>Alice</name></rows></data></datasets>)] >> >>")

	report, err := InspectXFADynamic(input)
	if err != nil {
		t.Fatal(err)
	}

	if !report.Present || !report.Dynamic || report.Static {
		t.Fatalf("report = %+v", report)
	}
	wantMarkers := []XFADynamicMarker{
		{PacketIndex: 0, Label: "template", PacketKind: "template", Path: "template.subform", Reason: `template layout="flowed"`},
		{PacketIndex: 0, Label: "template", PacketKind: "template", Path: "template.subform.occur", Reason: "template occur allows repeatable content"},
		{PacketIndex: 0, Label: "template", PacketKind: "template", Path: "template.subform.breakBefore", Reason: "template pagination/layout node requires XFA renderer semantics"},
		{PacketIndex: 0, Label: "template", PacketKind: "template", Path: "template.field", Reason: `template presence="hidden"`},
		{PacketIndex: 1, Label: "config", PacketKind: "config", Path: "config.present.pdf.dynamicRender", Reason: `config dynamicRender="required"`},
	}
	if len(report.Markers) != len(wantMarkers) {
		t.Fatalf("markers = %+v, want %+v", report.Markers, wantMarkers)
	}
	for i := range wantMarkers {
		if report.Markers[i] != wantMarkers[i] {
			t.Fatalf("marker %d = %+v, want %+v", i, report.Markers[i], wantMarkers[i])
		}
	}
	wantReasons := []string{
		"dynamic XFA requires renderer-grade layout/render semantics",
		"XFA edit/render support is not implemented for dynamic packets",
	}
	if len(report.UnsupportedReasons) != len(wantReasons) {
		t.Fatalf("unsupported reasons = %+v, want %+v", report.UnsupportedReasons, wantReasons)
	}
	for i := range wantReasons {
		if report.UnsupportedReasons[i] != wantReasons[i] {
			t.Fatalf("reason %d = %q, want %q", i, report.UnsupportedReasons[i], wantReasons[i])
		}
	}
}

func TestInspectXFADynamicFailsClosedForUnknownXFAFamily(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(config) (<config><present/></config>) (datasets) (<datasets><data><name>Alice</name></data></datasets>)] >> >>")

	report, err := InspectXFADynamic(input)
	if err != nil {
		t.Fatal(err)
	}

	if !report.Present || report.Dynamic || report.Static {
		t.Fatalf("report = %+v", report)
	}
	want := []string{
		"XFA packet family is not limited to static template/datasets packets",
		"XFA edit/render support is not implemented for this packet family",
	}
	if len(report.UnsupportedReasons) != len(want) {
		t.Fatalf("unsupported reasons = %+v, want %+v", report.UnsupportedReasons, want)
	}
	for i := range want {
		if report.UnsupportedReasons[i] != want[i] {
			t.Fatalf("reason %d = %q, want %q", i, report.UnsupportedReasons[i], want[i])
		}
	}
}
