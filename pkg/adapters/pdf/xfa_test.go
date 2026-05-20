package pdf

import (
	"bytes"
	"encoding/json"
	"testing"
	"unicode/utf8"

	"github.com/oxhq/binas/pkg/core"
)

func TestApplyXFAReplaceDirectLiteralString(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA (<template>old</template>) >> >>")

	output, report, verification, err := ApplyXFAReplace(input, "<template>old</template>", "<template>new</template>")
	if err != nil {
		t.Fatal(err)
	}

	if bytes.Contains(output, []byte("<template>old</template>")) {
		t.Fatalf("old XFA XML remains:\n%s", output)
	}
	if !bytes.Contains(output, []byte("<template>new</template>")) {
		t.Fatalf("new XFA XML missing:\n%s", output)
	}
	if report.Edit != "pdf.xfa_replace" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v", verification)
	}
	if _, err := NewAdapter().Parse(output, core.ParseOptions{Strict: true}); err == nil {
		t.Fatal("public parser unexpectedly accepted XFA output")
	}
}

func TestApplyXFAReplaceIndirectStream(t *testing.T) {
	xml := []byte("<template>old</template>")
	input := testPDF(
		"<< /Type /Catalog /AcroForm << /XFA 2 0 R >> >>",
		string(bytes.Join([][]byte{
			[]byte("<< /Length 24 >>"),
			[]byte("stream"),
			xml,
			[]byte("endstream"),
		}, []byte("\n"))),
	)

	output, report, verification, err := ApplyXFAReplace(input, "old", "newer")
	if err != nil {
		t.Fatal(err)
	}

	if bytes.Contains(output, []byte("<template>old</template>")) {
		t.Fatalf("old XFA stream XML remains:\n%s", output)
	}
	if !bytes.Contains(output, []byte("<template>newer</template>")) {
		t.Fatalf("new XFA stream XML missing:\n%s", output)
	}
	if !bytes.Contains(output, []byte("/Length 26")) {
		t.Fatalf("stream length was not updated:\n%s", output)
	}
	if report.NodesModified != 1 {
		t.Fatalf("report = %+v", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable {
		t.Fatalf("verification = %+v", verification)
	}
}

func TestApplyXFAReplaceArrayStreamEntry(t *testing.T) {
	xml := []byte("<datasets>old</datasets>")
	input := testPDF(
		"<< /Type /Catalog /AcroForm << /XFA [(template) 2 0 R (datasets) 3 0 R] >> >>",
		"<< /Length 25 >>\nstream\n<template>keep</template>\nendstream",
		string(bytes.Join([][]byte{
			[]byte("<< /Length 24 >>"),
			[]byte("stream"),
			xml,
			[]byte("endstream"),
		}, []byte("\n"))),
	)

	output, _, _, err := ApplyXFAReplace(input, "<datasets>old</datasets>", "<datasets>new</datasets>")
	if err != nil {
		t.Fatal(err)
	}

	if bytes.Contains(output, []byte("<datasets>old</datasets>")) {
		t.Fatalf("old array XFA XML remains:\n%s", output)
	}
	if !bytes.Contains(output, []byte("<datasets>new</datasets>")) {
		t.Fatalf("new array XFA XML missing:\n%s", output)
	}
	if !bytes.Contains(output, []byte("<template>keep</template>")) {
		t.Fatalf("unmatched XFA packet changed:\n%s", output)
	}
}

func TestApplyXFAReplaceArrayLiteralEntryInPlace(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(template) (<template>old</template>) (datasets) (<datasets>keep</datasets>)] >> >>")

	output, _, _, err := ApplyXFAReplace(input, "<template>old</template>", "<template>new</template>")
	if err != nil {
		t.Fatal(err)
	}

	if !bytes.Contains(output, []byte("[(template) (<template>new</template>) (datasets) (<datasets>keep</datasets>)]")) {
		t.Fatalf("XFA array entry was not replaced in place:\n%s", output)
	}
}

func TestApplyXFAReplaceArrayHexLabelEntryInPlace(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [<74656d706c617465> (<template>old</template>) /datasets (<datasets>keep</datasets>)] >> >>")

	output, _, _, err := ApplyXFAReplace(input, "<template>old</template>", "<template>new</template>")
	if err != nil {
		t.Fatal(err)
	}

	if !bytes.Contains(output, []byte("<template>new</template>")) {
		t.Fatalf("XFA array entry after hex label was not replaced:\n%s", output)
	}
	if bytes.Contains(output, []byte("<template>old</template>")) {
		t.Fatalf("old XFA array entry after hex label remains:\n%s", output)
	}
	if !bytes.Contains(output, []byte("<datasets>keep</datasets>")) {
		t.Fatalf("unmatched XFA packet changed:\n%s", output)
	}
}

func TestApplyXFAReplaceArrayNameLabelEntryInPlace(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [/template (<template>old</template>) /datasets (<datasets>keep</datasets>)] >> >>")

	output, _, _, err := ApplyXFAReplace(input, "<template>old</template>", "<template>new</template>")
	if err != nil {
		t.Fatal(err)
	}

	if !bytes.Contains(output, []byte("<template>new</template>")) {
		t.Fatalf("XFA array entry after name label was not replaced:\n%s", output)
	}
	if bytes.Contains(output, []byte("<template>old</template>")) {
		t.Fatalf("old XFA array entry after name label remains:\n%s", output)
	}
	if !bytes.Contains(output, []byte("<datasets>keep</datasets>")) {
		t.Fatalf("unmatched XFA packet changed:\n%s", output)
	}
}

func TestApplyXFAReplaceDoesNotMatchHexArrayLabelText(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [<74656d706c617465> (<packet>keep</packet>)] >> >>")

	_, _, _, err := ApplyXFAReplace(input, "template", "renamed")
	if err == nil {
		t.Fatal("expected XFA replacement to ignore hex array label text")
	}
	if err.Error() != `no XFA packet contains "template"` {
		t.Fatalf("error = %q", err)
	}
}

func TestApplyXFAReplaceFailsClosedWhenAmbiguous(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(template) (old) (datasets) (old)] >> >>")

	_, _, _, err := ApplyXFAReplace(input, "old", "new")
	if err == nil {
		t.Fatal("expected ambiguous XFA replacement to fail closed")
	}
	if err.Error() != `XFA replacement is ambiguous: 2 matches for "old"` {
		t.Fatalf("error = %q", err)
	}
}

func TestApplyXFAReplaceSelectsIndexedXFAPacket(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(template) (<template>old</template>) (datasets) (<datasets>old</datasets>)] >> >>")
	matchIndex := 1

	output, _, _, err := ApplyXFAReplace(input, "old", "new", &matchIndex)
	if err != nil {
		t.Fatal(err)
	}

	if !bytes.Contains(output, []byte("<template>old</template>")) {
		t.Fatalf("unselected XFA packet changed:\n%s", output)
	}
	if !bytes.Contains(output, []byte("<datasets>new</datasets>")) {
		t.Fatalf("selected XFA packet was not changed:\n%s", output)
	}
	if bytes.Contains(output, []byte("<datasets>old</datasets>")) {
		t.Fatalf("selected XFA packet still contains old text:\n%s", output)
	}
}

func TestApplyXFAReplaceWithOptionsSelectsPacketKind(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(template) (<template>old</template>) (datasets) (<datasets>old</datasets>)] >> >>")

	output, _, _, err := ApplyXFAReplaceWithOptions(input, "old", "new", XFAReplaceOptions{
		Selector: XFASelector{PacketKind: "datasets"},
	})
	if err != nil {
		t.Fatal(err)
	}

	if !bytes.Contains(output, []byte("<template>old</template>")) {
		t.Fatalf("unselected packet kind changed:\n%s", output)
	}
	if !bytes.Contains(output, []byte("<datasets>new</datasets>")) {
		t.Fatalf("selected packet kind was not changed:\n%s", output)
	}
	if bytes.Contains(output, []byte("<datasets>old</datasets>")) {
		t.Fatalf("selected packet kind still contains old text:\n%s", output)
	}
}

func TestApplyXFAReplaceWithOptionsSelectsArrayLabel(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(keepTemplate) (<template>old</template>) (targetTemplate) (<template>old</template>)] >> >>")

	output, _, _, err := ApplyXFAReplaceWithOptions(input, "old", "new", XFAReplaceOptions{
		Selector: XFASelector{Label: "targetTemplate"},
	})
	if err != nil {
		t.Fatal(err)
	}

	if !bytes.Contains(output, []byte("(keepTemplate) (<template>old</template>)")) {
		t.Fatalf("unselected label changed:\n%s", output)
	}
	if !bytes.Contains(output, []byte("(targetTemplate) (<template>new</template>)")) {
		t.Fatalf("selected label was not changed:\n%s", output)
	}
	if bytes.Contains(output, []byte("(targetTemplate) (<template>old</template>)")) {
		t.Fatalf("selected label still contains old text:\n%s", output)
	}
}

func TestApplyXFAReplaceSelectsIndexedOccurrenceWithinSameXFAPacket(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(datasets) (<datasets>old old</datasets>)] >> >>")
	matchIndex := 1

	output, _, _, err := ApplyXFAReplace(input, "old", "new", &matchIndex)
	if err != nil {
		t.Fatal(err)
	}

	if !bytes.Contains(output, []byte("<datasets>old new</datasets>")) {
		t.Fatalf("second occurrence was not replaced in place:\n%s", output)
	}
	if bytes.Contains(output, []byte("<datasets>new old</datasets>")) {
		t.Fatalf("first occurrence was replaced instead of selected occurrence:\n%s", output)
	}
}

func TestApplyXFAReplaceRejectsInvalidMatchIndex(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(template) (old)] >> >>")

	for _, tc := range []struct {
		name       string
		matchIndex int
		wantErr    string
	}{
		{name: "negative", matchIndex: -1, wantErr: "XFA replacement match index -1 is out of range: 1 matches for \"old\""},
		{name: "out of range", matchIndex: 1, wantErr: "XFA replacement match index 1 is out of range: 1 matches for \"old\""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			_, _, _, err := ApplyXFAReplace(input, "old", "new", &tc.matchIndex)
			if err == nil {
				t.Fatal("expected invalid match index to fail")
			}
			if err.Error() != tc.wantErr {
				t.Fatalf("error = %q", err)
			}
		})
	}
}

func TestApplyXFAReplaceWithOptionsFailsClosedWhenSelectorMatchesZero(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(template) (<template>old</template>)] >> >>")

	_, _, _, err := ApplyXFAReplaceWithOptions(input, "old", "new", XFAReplaceOptions{
		Selector: XFASelector{Label: "datasets"},
	})
	if err == nil {
		t.Fatal("expected unmatched XFA selector to fail closed")
	}
	if err.Error() != `no XFA packet matches selector label="datasets"` {
		t.Fatalf("error = %q", err)
	}
}

func TestApplyXFAReplaceWithOptionsFailsClosedWhenSelectorRemainsAmbiguous(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(datasets) (<datasets>old</datasets>) (datasets) (<datasets>old</datasets>)] >> >>")

	_, _, _, err := ApplyXFAReplaceWithOptions(input, "old", "new", XFAReplaceOptions{
		Selector: XFASelector{Label: "datasets"},
	})
	if err == nil {
		t.Fatal("expected ambiguous XFA selector replacement to fail closed")
	}
	if err.Error() != `XFA replacement is ambiguous: 2 matches for "old"` {
		t.Fatalf("error = %q", err)
	}
}

func TestApplyXFAReplaceFailsClosedWithoutDirectXFA(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /Fields [] >> >>")

	_, _, _, err := ApplyXFAReplace(input, "old", "new")
	if err == nil {
		t.Fatal("expected missing XFA to fail closed")
	}
	if err.Error() != "unsupported PDF: XFA packet is not directly represented" {
		t.Fatalf("error = %q", err)
	}
}

func TestListXFAPacketsDirectLiteral(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA (<template>alpha</template>) >> >>")

	packets, err := ListXFAPackets(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(packets) != 1 {
		t.Fatalf("packets = %+v, want one packet", packets)
	}
	packet := packets[0]
	if packet.Index != 0 || packet.Label != "" || packet.ObjectNumber != nil || packet.ObjectGeneration != nil || packet.IsStream {
		t.Fatalf("packet metadata = %+v", packet)
	}
	if packet.PacketKind != "template" {
		t.Fatalf("packet kind = %q, want template", packet.PacketKind)
	}
	if packet.HasXMLProlog || packet.RootElement != "template" {
		t.Fatalf("packet XML metadata = %+v", packet)
	}
	if packet.TextLength != len("<template>alpha</template>") || packet.Preview != "<template>alpha</template>" {
		t.Fatalf("packet text metadata = %+v", packet)
	}
}

func TestListXFAPacketsReportsByteLengthSeparatelyFromTextLength(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA (<template>caf\xc3\xa9</template>) >> >>")

	packets, err := ListXFAPackets(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(packets) != 1 {
		t.Fatalf("packets = %+v, want one packet", packets)
	}
	packet := packets[0]
	if packet.TextLength != utf8.RuneCountInString("<template>café</template>") {
		t.Fatalf("text length = %d, want rune count", packet.TextLength)
	}
	if packet.ByteLength != len([]byte("<template>café</template>")) {
		t.Fatalf("byte length = %d, want UTF-8 byte count", packet.ByteLength)
	}
	if packet.ByteLength == packet.TextLength {
		t.Fatalf("byte length should differ from text length for non-ASCII text: %+v", packet)
	}
	encoded, err := json.Marshal(packet)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(encoded, []byte(`"byte_length":26`)) {
		t.Fatalf("packet JSON %s missing byte_length", encoded)
	}
}

func TestListXFAPacketsWithOptionsFiltersBySelector(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(template) (<template>one</template>) /datasets (<datasets>two</datasets>) (target) (<template>three</template>)] >> >>")

	packets, err := ListXFAPacketsWithOptions(input, XFAPacketListOptions{
		Selector: XFASelector{PacketKind: "datasets", Label: "datasets"},
	})
	if err != nil {
		t.Fatal(err)
	}

	if len(packets) != 1 {
		t.Fatalf("packets = %+v, want one packet", packets)
	}
	if packets[0].Index != 1 || packets[0].Label != "datasets" || packets[0].PacketKind != "datasets" || packets[0].Preview != "<datasets>two</datasets>" {
		t.Fatalf("filtered packet = %+v", packets[0])
	}
}

func TestListXFAPacketsReferencedStream(t *testing.T) {
	xml := []byte("<template>stream text</template>")
	input := testPDF(
		"<< /Type /Catalog /AcroForm << /XFA 2 0 R >> >>",
		string(bytes.Join([][]byte{
			[]byte("<< /Length 32 >>"),
			[]byte("stream"),
			xml,
			[]byte("endstream"),
		}, []byte("\n"))),
	)

	packets, err := ListXFAPackets(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(packets) != 1 {
		t.Fatalf("packets = %+v, want one packet", packets)
	}
	packet := packets[0]
	if packet.Index != 0 || packet.ObjectNumber == nil || *packet.ObjectNumber != 2 || packet.ObjectGeneration == nil || *packet.ObjectGeneration != 0 || !packet.IsStream {
		t.Fatalf("packet object metadata = %+v", packet)
	}
	if packet.TextLength != len("<template>stream text</template>") || packet.Preview != "<template>stream text</template>" {
		t.Fatalf("packet text metadata = %+v", packet)
	}
	if packet.PacketKind != "template" {
		t.Fatalf("packet kind = %q, want template", packet.PacketKind)
	}
	if packet.HasXMLProlog || packet.RootElement != "template" {
		t.Fatalf("packet XML metadata = %+v", packet)
	}
	if packet.HasDecodeError || packet.DecodeError != "" {
		t.Fatalf("packet decode metadata = %+v", packet)
	}
}

func TestListXFAPacketsReferencedStreamDecodeError(t *testing.T) {
	for _, tc := range []struct {
		name            string
		acroForm        string
		streamDict      string
		wantLabel       string
		wantFilter      string
		wantDecodeParms string
		wantDecodeError string
	}{
		{
			name:            "invalid lzw stream",
			acroForm:        "<< /Type /Catalog /AcroForm << /XFA 2 0 R >> >>",
			streamDict:      "<< /Length 6 /Filter /LZWDecode >>",
			wantFilter:      "/LZWDecode",
			wantDecodeError: "decode LZWDecode stream: invalid code 393",
		},
		{
			name:            "invalid DecodeParms",
			acroForm:        "<< /Type /Catalog /AcroForm << /XFA [(template) 2 0 R] >> >>",
			streamDict:      "<< /Length 6 /Filter /FlateDecode /DecodeParms 12 >>",
			wantLabel:       "template",
			wantFilter:      "/FlateDecode",
			wantDecodeParms: "12",
			wantDecodeError: "unsupported stream: /DecodeParms must be a dictionary, array, or null",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			input := testPDF(
				tc.acroForm,
				string(bytes.Join([][]byte{
					[]byte(tc.streamDict),
					[]byte("stream"),
					[]byte("abcdef"),
					[]byte("endstream"),
				}, []byte("\n"))),
			)

			packets, err := ListXFAPackets(input)
			if err != nil {
				t.Fatal(err)
			}

			if len(packets) != 1 {
				t.Fatalf("packets = %+v, want one packet", packets)
			}
			packet := packets[0]
			if packet.Index != 0 || packet.Label != tc.wantLabel || packet.ObjectNumber == nil || *packet.ObjectNumber != 2 || packet.ObjectGeneration == nil || *packet.ObjectGeneration != 0 || !packet.IsStream {
				t.Fatalf("packet object metadata = %+v", packet)
			}
			if packet.Filter != tc.wantFilter || packet.DecodeParms != tc.wantDecodeParms {
				t.Fatalf("packet stream metadata = %+v", packet)
			}
			if tc.wantLabel != "" && packet.PacketKind != tc.wantLabel {
				t.Fatalf("packet kind = %q, want %q", packet.PacketKind, tc.wantLabel)
			}
			if !packet.HasDecodeError || packet.DecodeError != tc.wantDecodeError {
				t.Fatalf("packet decode metadata = %+v", packet)
			}
			if packet.TextLength != 0 || packet.Preview != "" {
				t.Fatalf("packet text metadata = %+v", packet)
			}
		})
	}
}

func TestListXFAPacketsArrayLabeledPackets(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(template) (<template>one</template>) /datasets (<datasets>two</datasets>)] >> >>")

	packets, err := ListXFAPackets(input)
	if err != nil {
		t.Fatal(err)
	}

	if len(packets) != 2 {
		t.Fatalf("packets = %+v, want two packets", packets)
	}
	if packets[0].Index != 0 || packets[0].Label != "template" || packets[0].PacketKind != "template" || packets[0].Preview != "<template>one</template>" {
		t.Fatalf("first packet = %+v", packets[0])
	}
	if packets[1].Index != 1 || packets[1].Label != "datasets" || packets[1].PacketKind != "datasets" || packets[1].Preview != "<datasets>two</datasets>" {
		t.Fatalf("second packet = %+v", packets[1])
	}
}

func TestListXFAPacketsDirectXMLPacketKinds(t *testing.T) {
	for _, tc := range []struct {
		name string
		xfa  string
		want string
	}{
		{name: "xml declaration before xdp root", xfa: "(<?xml version=\"1.0\"?><xdp:xdp xmlns:xdp=\"http://ns.adobe.com/xdp/\">text</xdp:xdp>)", want: "xdp"},
		{name: "namespaced datasets root", xfa: "(<!-- packet comment --><?xfa generator=\"binas\"?><xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\">text</xfa:datasets>)", want: "datasets"},
		{name: "generic xml root", xfa: "(<something>text</something>)", want: "xml"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			input := testPDF("<< /Type /Catalog /AcroForm << /XFA " + tc.xfa + " >> >>")

			packets, err := ListXFAPackets(input)
			if err != nil {
				t.Fatal(err)
			}

			if len(packets) != 1 {
				t.Fatalf("packets = %+v, want one packet", packets)
			}
			if packets[0].PacketKind != tc.want {
				t.Fatalf("packet kind = %q, want %q; packet = %+v", packets[0].PacketKind, tc.want, packets[0])
			}
		})
	}
}

func TestListXFAPacketsReportsExplicitPacketFamilyLabels(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA [(template) (<template/>) (datasets) (<datasets/>) (config) (<config/>) (localeSet) (<localeSet/>) (sourceSet) (<sourceSet/>) (xdc) (<xdc/>) (xfdf) (<xfdf/>) (notXFA) (<template/>)] >> >>")

	packets, err := ListXFAPackets(input)
	if err != nil {
		t.Fatal(err)
	}

	want := []string{"template", "datasets", "config", "localeSet", "sourceSet", "xdc", "xfdf", "unknown"}
	if len(packets) != len(want) {
		t.Fatalf("packets = %+v, want %d packets", packets, len(want))
	}
	for i, wantKind := range want {
		if packets[i].PacketKind != wantKind {
			t.Fatalf("packet %d kind = %q, want %q; packet = %+v", i, packets[i].PacketKind, wantKind, packets[i])
		}
		encoded, err := json.Marshal(packets[i])
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Contains(encoded, []byte(`"packet_kind":"`+wantKind+`"`)) {
			t.Fatalf("packet JSON %s missing explicit packet_kind %q", encoded, wantKind)
		}
	}
}

func TestListXFAPacketsXMLDiagnostics(t *testing.T) {
	for _, tc := range []struct {
		name          string
		xfa           string
		wantProlog    bool
		wantRoot      string
		wantJSONField []byte
	}{
		{
			name:          "xml declaration before root",
			xfa:           "(<?xml version=\"1.0\"?><xdp:xdp xmlns:xdp=\"http://ns.adobe.com/xdp/\">text</xdp:xdp>)",
			wantProlog:    true,
			wantRoot:      "xdp:xdp",
			wantJSONField: []byte(`"has_xml_prolog":true`),
		},
		{
			name:          "comments and processing instructions before root",
			xfa:           "(<!-- packet comment --><?xfa generator=\"binas\"?><xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\">text</xfa:datasets>)",
			wantRoot:      "xfa:datasets",
			wantJSONField: []byte(`"root_element":"xfa:datasets"`),
		},
		{
			name:          "plain text leaves root empty",
			xfa:           "(plain text)",
			wantJSONField: []byte(`"has_xml_prolog":false`),
		},
		{
			name:       "unterminated comment leaves root empty",
			xfa:        "(<!-- ambiguous <template>text</template>)",
			wantProlog: false,
			wantRoot:   "",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			input := testPDF("<< /Type /Catalog /AcroForm << /XFA " + tc.xfa + " >> >>")

			packets, err := ListXFAPackets(input)
			if err != nil {
				t.Fatal(err)
			}

			if len(packets) != 1 {
				t.Fatalf("packets = %+v, want one packet", packets)
			}
			packet := packets[0]
			if packet.HasXMLProlog != tc.wantProlog || packet.RootElement != tc.wantRoot {
				t.Fatalf("packet XML metadata = %+v, want prolog=%v root=%q", packet, tc.wantProlog, tc.wantRoot)
			}
			if tc.wantJSONField != nil {
				encoded, err := json.Marshal(packet)
				if err != nil {
					t.Fatal(err)
				}
				if !bytes.Contains(encoded, tc.wantJSONField) {
					t.Fatalf("packet JSON %s missing %s", encoded, tc.wantJSONField)
				}
			}
		})
	}
}

func TestListXFAPacketsUnsafeXMLDeclarationsReportDiagnosticsSafely(t *testing.T) {
	input := testPDF("<< /Type /Catalog /AcroForm << /XFA (<?xml version=\"1.0\"?><!DOCTYPE xdp [<!ENTITY secret SYSTEM \"file:///etc/passwd\">]><template>&secret;</template>) >> >>")

	packets, err := ListXFAPackets(input)
	if err != nil {
		t.Fatal(err)
	}
	if len(packets) != 1 {
		t.Fatalf("packets = %+v, want one packet", packets)
	}
	packet := packets[0]
	if !packet.HasXMLProlog || !packet.UnsafeXML || packet.XMLParseError != "unsafe XML declaration: DOCTYPE is not supported" || packet.RootElement != "" {
		t.Fatalf("packet XML diagnostics = %+v", packet)
	}
	encoded, err := json.Marshal(packet)
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range [][]byte{
		[]byte(`"has_xml_prolog":true`),
		[]byte(`"unsafe_xml":true`),
		[]byte(`"xml_parse_error":"unsafe XML declaration: DOCTYPE is not supported"`),
		[]byte(`"packet_kind":"unknown"`),
	} {
		if !bytes.Contains(encoded, want) {
			t.Fatalf("packet JSON %s missing %s", encoded, want)
		}
	}
}

func TestListXFAPacketsXMLSafetyDiagnostics(t *testing.T) {
	for _, tc := range []struct {
		name           string
		xfa            string
		wantUnsafe     bool
		wantParseError string
		wantRoot       string
	}{
		{
			name:           "doctype is unsafe",
			xfa:            "(<!DOCTYPE xdp [<!ENTITY secret SYSTEM \"file:///etc/passwd\">]><template>&secret;</template>)",
			wantUnsafe:     true,
			wantParseError: "unsafe XML declaration: DOCTYPE is not supported",
		},
		{
			name:           "entity declaration is unsafe",
			xfa:            "(<!ENTITY secret \"value\"><template>text</template>)",
			wantUnsafe:     true,
			wantParseError: "unsafe XML declaration: ENTITY is not supported",
		},
		{
			name:           "unterminated processing instruction reports parse error",
			xfa:            "(<?xfa generator=\"binas\"<template>text</template>)",
			wantParseError: "unterminated XML processing instruction",
		},
		{
			name:           "malformed namespaced datasets fails closed",
			xfa:            "(<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\"><value></xfa:datasets>)",
			wantParseError: "malformed XML root element",
		},
		{
			name:     "safe template root has no safety error",
			xfa:      "(<template>text</template>)",
			wantRoot: "template",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			input := testPDF("<< /Type /Catalog /AcroForm << /XFA " + tc.xfa + " >> >>")

			packets, err := ListXFAPackets(input)
			if err != nil {
				t.Fatal(err)
			}
			if len(packets) != 1 {
				t.Fatalf("packets = %+v, want one packet", packets)
			}
			packet := packets[0]
			if packet.UnsafeXML != tc.wantUnsafe || packet.XMLParseError != tc.wantParseError || packet.RootElement != tc.wantRoot {
				t.Fatalf("packet XML safety metadata = %+v, want unsafe=%v parse_error=%q root=%q", packet, tc.wantUnsafe, tc.wantParseError, tc.wantRoot)
			}
			if tc.wantParseError != "" && packet.PacketKind != "unknown" {
				t.Fatalf("packet kind = %q, want unknown for failed XML diagnostics; packet = %+v", packet.PacketKind, packet)
			}
		})
	}
}

func TestListXFAPacketsReportsUnknownPacketKind(t *testing.T) {
	for _, tc := range []struct {
		name string
		xfa  string
	}{
		{name: "plain direct packet", xfa: "(plain text)"},
		{name: "unknown label does not fall back to body", xfa: "[(notXFA) (<template>text</template>)]"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			input := testPDF("<< /Type /Catalog /AcroForm << /XFA " + tc.xfa + " >> >>")

			packets, err := ListXFAPackets(input)
			if err != nil {
				t.Fatal(err)
			}

			if len(packets) != 1 {
				t.Fatalf("packets = %+v, want one packet", packets)
			}
			if packets[0].PacketKind != "unknown" {
				t.Fatalf("packet kind = %q, want unknown", packets[0].PacketKind)
			}
			encoded, err := json.Marshal(packets[0])
			if err != nil {
				t.Fatal(err)
			}
			if !bytes.Contains(encoded, []byte(`"packet_kind":"unknown"`)) {
				t.Fatalf("packet_kind should be explicit for unknown packets: %s", encoded)
			}
		})
	}
}

func TestApplyCanonicalEditRefusesXFAWithoutFallback(t *testing.T) {
	content := []byte("BT\n(old) Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog /AcroForm << /XFA (<template>old</template>) >> >>",
		string(bytes.Join([][]byte{
			[]byte("<< /Length 15 >>"),
			[]byte("stream"),
			content,
			[]byte("endstream"),
		}, []byte("\n"))),
	)

	output, report, verification, err := ApplyCanonicalEdit(input, core.Match{Kind: KindTextShow, Text: "old"}, core.Mutation{Replace: "new"}, nil)
	if err == nil {
		t.Fatal("expected generic canonical edit to refuse XFA")
	}
	if err.Error() != "unsupported PDF: XFA forms are not implemented" {
		t.Fatalf("error = %q", err)
	}
	if output != nil || report.FallbackUsed || verification.ReparseOK {
		t.Fatalf("generic XFA refusal leaked output/report/verification: output=%q report=%+v verification=%+v", output, report, verification)
	}
}
