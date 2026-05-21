package pdf

import (
	"bytes"
	"errors"
	"fmt"
	"strings"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestPackedWriterModeNormalizesPreserveStructure(t *testing.T) {
	mode, err := NormalizePDFWriterMode("preserve-structure")
	if err != nil {
		t.Fatalf("NormalizePDFWriterMode: %v", err)
	}
	if mode != PDFWriterModePreserveStructure {
		t.Fatalf("mode = %q, want %q", mode, PDFWriterModePreserveStructure)
	}
}

func TestPackedWriterPreserveStructureObjectStreamRepacksWhileCanonicalStillInflates(t *testing.T) {
	input := testObjectStreamStructurePDF()

	output, report, verification, err := ApplyCanonicalEdit(
		input,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-04-2026"},
		nil,
	)
	if err != nil {
		t.Fatalf("ApplyCanonicalEdit: %v", err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want canonical edit without fallback", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable {
		t.Fatalf("verification = %+v, want canonical proof", verification)
	}
	if bytes.Contains(output, []byte("/ObjStm")) {
		t.Fatalf("canonical output preserved object stream container:\n%s", output)
	}

	output, report, verification, err = ApplyCanonicalEditWithWriterMode(
		input,
		PDFWriterModePreserveStructure,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-04-2026"},
		nil,
	)
	if err != nil {
		t.Fatalf("ApplyCanonicalEditWithWriterMode preserve-structure: %v", err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want preserve-structure edit without fallback", report)
	}
	assertPreserveStructureReportMeta(t, report.Meta, "preserve-packed", false, map[string]any{
		"has_table_xref":         true,
		"has_xref_stream":        false,
		"has_hybrid_xref":        false,
		"object_stream_objects":  1,
		"xref_stream_objects":    0,
		"requires_packed_writer": true,
	})
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable {
		t.Fatalf("verification = %+v, want preserve-structure proof", verification)
	}
	if !bytes.Contains(output, []byte("/Type /ObjStm")) {
		t.Fatalf("preserve-structure output did not keep object stream container:\n%s", output)
	}
	reparsed, err := parsePDFGraph(output)
	if err != nil {
		t.Fatalf("reparse preserve-structure output: %v\n%s", err, output)
	}
	packedObject := reparsed.Objects[pdfObjectID{Number: 5, Generation: 0}]
	if packedObject == nil || !packedObject.InObjectStream {
		t.Fatalf("object 5 = %+v, want reparsed from object stream", packedObject)
	}
	if !xrefSummaryContainsCompressedObject(reparsed.Xref, 5, 4, 0) {
		t.Fatalf("reparsed xref = %+v, want type-2 metadata for object 5 in object stream 4 index 0", reparsed.Xref)
	}
}

func TestPackedWriterPreserveStructureNormalTableXrefUsesCanonicalWriterPath(t *testing.T) {
	content := "BT\n(08\\05515\\0552024) Tj\nET\n"
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Contents 3 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	output, report, verification, err := ApplyCanonicalEditWithWriterMode(
		input,
		PDFWriterModePreserveStructure,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-04-2026"},
		nil,
	)
	if err != nil {
		t.Fatalf("ApplyCanonicalEditWithWriterMode: %v", err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want canonical writer path without fallback", report)
	}
	assertPreserveStructureReportMeta(t, report.Meta, "canonical", true, map[string]any{
		"has_table_xref":         true,
		"has_xref_stream":        false,
		"has_hybrid_xref":        false,
		"object_stream_objects":  0,
		"xref_stream_objects":    0,
		"requires_packed_writer": false,
	})
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable {
		t.Fatalf("verification = %+v, want canonical proof", verification)
	}
	if !bytes.Contains(output, []byte("xref\n0 4\n")) {
		t.Fatalf("preserve-structure normal table xref output did not rebuild table xref:\n%s", output)
	}
	if bytes.Contains(output, []byte("/ObjStm")) || bytes.Contains(output, []byte("/Type /XRef")) || bytes.Contains(output, []byte("/XRefStm")) {
		t.Fatalf("normal table xref preserve-structure output unexpectedly contains packed structure:\n%s", output)
	}
}

func TestPackedWriterPreserveStructureXrefStreamWritesXrefStream(t *testing.T) {
	input := testXrefStreamPDF(t)

	output, report, verification, err := ApplyCanonicalEdit(
		input,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-04-2026"},
		nil,
	)
	if err != nil {
		t.Fatalf("ApplyCanonicalEdit: %v", err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want canonical edit without fallback", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable {
		t.Fatalf("verification = %+v, want canonical proof", verification)
	}
	if bytes.Contains(output, []byte("/Type /XRef")) {
		t.Fatalf("canonical output preserved xref stream container:\n%s", output)
	}

	output, report, verification, err = ApplyCanonicalEditWithWriterMode(
		input,
		PDFWriterModePreserveStructure,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-04-2026"},
		nil,
	)
	if err != nil {
		t.Fatalf("ApplyCanonicalEditWithWriterMode preserve-structure: %v", err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want preserve-structure edit without fallback", report)
	}
	if report.Meta["writer_path"] != "xref_stream" || report.Meta["used_canonical_writer_path"] != false {
		t.Fatalf("writer path metadata = %+v, want xref_stream path proof", report.Meta)
	}
	assertPreserveStructureReportMeta(t, report.Meta, "xref_stream", false, map[string]any{
		"has_table_xref":         false,
		"has_xref_stream":        true,
		"has_hybrid_xref":        false,
		"object_stream_objects":  0,
		"xref_stream_objects":    1,
		"requires_packed_writer": true,
	})
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable {
		t.Fatalf("verification = %+v, want preserve-structure xref-stream proof", verification)
	}
	if bytes.Contains(output, []byte("\nxref\n")) {
		t.Fatalf("preserve-structure xref-stream output rebuilt a table xref:\n%s", output)
	}
	if !bytes.Contains(output, []byte("/Type /XRef")) {
		t.Fatalf("preserve-structure output did not write an xref stream:\n%s", output)
	}
	reparsed, err := parsePDFGraph(output)
	if err != nil {
		t.Fatalf("reparse preserve-structure xref-stream output: %v\n%s", err, output)
	}
	if !reparsed.Xref.HasStream || reparsed.Xref.HasTable {
		t.Fatalf("reparsed xref summary = %+v, want stream without table", reparsed.Xref)
	}
	startxref, err := lastStartXrefOffset(output)
	if err != nil {
		t.Fatalf("startxref: %v", err)
	}
	if !bytes.HasPrefix(output[startxref:], []byte("4 0 obj\n<<")) {
		t.Fatalf("startxref = %d, want offset of xref stream object 4 0:\n%s", startxref, output[startxref:])
	}
}

func TestPackedWriterHybridXrefCanonicalAndPreserveStructureEditBehaviors(t *testing.T) {
	input := testHybridXrefTextPDF(t, validHybridTextXrefStreamData)

	output, report, verification, err := ApplyCanonicalEditWithWriterMode(
		input,
		PDFWriterModeCanonical,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-04-2026"},
		nil,
	)
	if err != nil {
		t.Fatalf("canonical hybrid xref edit: %v", err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want canonical hybrid edit without fallback", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable {
		t.Fatalf("verification = %+v, want canonical hybrid proof", verification)
	}
	if bytes.Contains(output, []byte("/XRefStm")) || bytes.Contains(output, []byte("/Type /XRef")) {
		t.Fatalf("canonical hybrid output preserved xref stream structure:\n%s", output)
	}
	if !bytes.Contains(output, []byte("xref\n")) {
		t.Fatalf("canonical hybrid output missing rebuilt table xref:\n%s", output)
	}

	output, report, verification, err = ApplyCanonicalEditWithWriterMode(
		input,
		PDFWriterModePreserveStructure,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "05-04-2026"},
		nil,
	)
	if err != nil {
		t.Fatalf("preserve-structure hybrid xref edit: %v", err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v, want preserve-structure hybrid edit without fallback", report)
	}
	assertPreserveStructureReportMeta(t, report.Meta, "hybrid_xref_stream", false, map[string]any{
		"has_table_xref":         true,
		"has_xref_stream":        true,
		"has_hybrid_xref":        true,
		"object_stream_objects":  0,
		"xref_stream_objects":    1,
		"requires_packed_writer": true,
	})
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable {
		t.Fatalf("verification = %+v, want preserve-structure hybrid proof", verification)
	}
	if !bytes.Contains(output, []byte("/Type /XRef")) || !bytes.Contains(output, []byte("/XRefStm")) || !bytes.Contains(output, []byte("\nxref\n")) {
		t.Fatalf("preserve-structure hybrid output did not keep table + xref stream:\n%s", output)
	}
	reparsed, err := parsePDFGraph(output)
	if err != nil {
		t.Fatalf("reparse preserve-structure hybrid output: %v\n%s", err, output)
	}
	if !reparsed.Xref.HasTable || !reparsed.Xref.HasHybridStream || !reparsed.Xref.HasStream {
		t.Fatalf("reparsed xref summary = %+v, want hybrid table+xref stream", reparsed.Xref)
	}
}

func TestPackedWriterPreserveStructureMalformedPackedPDFsFailClosedWithStructureDetails(t *testing.T) {
	cases := []struct {
		name        string
		input       []byte
		wantDetails map[string]any
		wantParse   string
	}{
		{
			name:  "malformed xref stream",
			input: testMalformedXrefStreamPDF(t),
			wantDetails: map[string]any{
				"has_table_xref":          false,
				"has_xref_stream":         true,
				"has_hybrid_xref":         false,
				"object_stream_objects":   0,
				"xref_stream_objects":     1,
				"unsupported_xref_stream": true,
			},
			wantParse: "xref stream data ended before all entries",
		},
		{
			name:  "malformed hybrid xref stream",
			input: testHybridXrefTextPDF(t, malformedHybridTextXrefStreamData),
			wantDetails: map[string]any{
				"has_table_xref":          true,
				"has_xref_stream":         true,
				"has_hybrid_xref":         true,
				"object_stream_objects":   0,
				"xref_stream_objects":     1,
				"unsupported_xref_stream": true,
			},
			wantParse: "hybrid xref /XRefStm stream: xref stream data ended before all entries",
		},
		{
			name:  "malformed object stream",
			input: testMalformedObjectStreamPDF(),
			wantDetails: map[string]any{
				"has_table_xref":            true,
				"has_xref_stream":           false,
				"has_hybrid_xref":           false,
				"object_stream_objects":     1,
				"xref_stream_objects":       0,
				"unsupported_object_stream": true,
			},
			wantParse: "object stream 4 0",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			_, _, _, err := ApplyCanonicalEditWithWriterMode(
				tc.input,
				PDFWriterModePreserveStructure,
				core.Match{Kind: KindTextShow, Text: "08-15-2024"},
				core.Mutation{Replace: "05-04-2026"},
				nil,
			)
			assertPreserveStructureUnsupported(t, err)

			var structureErr *PreserveStructureUnsupportedError
			if !errors.As(err, &structureErr) {
				t.Fatalf("error = %T, want PreserveStructureUnsupportedError", err)
			}
			details := structureErr.StructureDetails()
			if details["requires_packed_writer"] != true {
				t.Fatalf("structure details = %+v, want requires_packed_writer=true", details)
			}
			for key, want := range tc.wantDetails {
				if got := details[key]; got != want {
					t.Fatalf("structure details[%q] = %v, want %v; details=%+v", key, got, want, details)
				}
			}
			parseError, ok := details["parse_error"].(string)
			if !ok || !strings.Contains(parseError, tc.wantParse) {
				t.Fatalf("parse_error = %#v, want substring %q; details=%+v", details["parse_error"], tc.wantParse, details)
			}
		})
	}
}

func TestPackedWriterPreserveStructureMalformedRepackMetadataFailsClosed(t *testing.T) {
	graph := &pdfGraph{
		Header: "%PDF-1.5",
		Objects: map[pdfObjectID]*pdfIndirectObject{
			{Number: 1, Generation: 0}: {
				ID:    pdfObjectID{Number: 1, Generation: 0},
				Value: pdfDict{"Type": pdfName("Catalog")},
			},
			{Number: 5, Generation: 0}: {
				ID:             pdfObjectID{Number: 5, Generation: 0},
				Value:          pdfDict{"Fixture": true},
				InObjectStream: true,
			},
		},
		Trailer: pdfDict{"Size": 6, "Root": pdfRef{ID: pdfObjectID{Number: 1, Generation: 0}}},
	}

	_, err := writePreserveStructurePDFWithOptions(graph, pdfCanonicalWriteOptions{})
	if err == nil {
		t.Fatal("expected malformed preserve-structure repack to fail closed")
	}
	assertPreserveStructureUnsupported(t, err, "object 5 0 is marked in an object stream but has no object stream xref metadata")
}

func testObjectStreamStructurePDF() []byte {
	content := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	objectStreamData := "5 0 << /Fixture true >>"
	return testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Contents 3 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		fmt.Sprintf("<< /Type /ObjStm /N 1 /First 4 /Length %d >>\nstream\n%sendstream", len(objectStreamData), objectStreamData),
	)
}

func testHybridXrefTextPDF(t *testing.T, streamData func([]int) []byte) []byte {
	t.Helper()

	var input bytes.Buffer
	offsets := make([]int, 9)
	input.WriteString("%PDF-1.5\n")
	offsets[1] = input.Len()
	input.WriteString("1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n")
	offsets[2] = input.Len()
	input.WriteString("2 0 obj\n<< /Type /Page /Contents 3 0 R >>\nendobj\n")
	content := "BT\n(08\\05515\\0552024) Tj\nET\n"
	offsets[3] = input.Len()
	fmt.Fprintf(&input, "3 0 obj\n<< /Length %d >>\nstream\n%sendstream\nendobj\n", len(content), content)
	offsets[8] = input.Len()
	data := streamData(offsets)
	fmt.Fprintf(&input, "8 0 obj\n<< /Type /XRef /Size 9 /W [1 4 1] /Index [0 4] /Length %d >>\nstream\n", len(data))
	input.Write(data)
	input.WriteString("\nendstream\nendobj\n")
	xrefOffset := input.Len()
	input.WriteString("xref\n")
	input.WriteString("0 4\n")
	input.WriteString("0000000000 65535 f \n")
	fmt.Fprintf(&input, "%010d 00000 n \n", offsets[1])
	fmt.Fprintf(&input, "%010d 00000 n \n", offsets[2])
	fmt.Fprintf(&input, "%010d 00000 n \n", offsets[3])
	input.WriteString("8 1\n")
	fmt.Fprintf(&input, "%010d 00000 n \n", offsets[8])
	fmt.Fprintf(&input, "trailer\n<< /Size 9 /Root 1 0 R /XRefStm %d >>\nstartxref\n%d\n%%%%EOF\n", offsets[8], xrefOffset)
	return input.Bytes()
}

func validHybridTextXrefStreamData(offsets []int) []byte {
	var data bytes.Buffer
	writeXrefStreamEntry(&data, 0, 0, 0)
	writeXrefStreamEntry(&data, 1, offsets[1], 0)
	writeXrefStreamEntry(&data, 1, offsets[2], 0)
	writeXrefStreamEntry(&data, 1, offsets[3], 0)
	return data.Bytes()
}

func malformedHybridTextXrefStreamData(offsets []int) []byte {
	return []byte{0, 0, 0, 0, 0, 0, 1}
}

func testMalformedXrefStreamPDF(t *testing.T) []byte {
	t.Helper()

	var input bytes.Buffer
	offsets := make([]int, 9)
	input.WriteString("%PDF-1.5\n")
	offsets[1] = input.Len()
	input.WriteString("1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n")
	offsets[2] = input.Len()
	input.WriteString("2 0 obj\n<< /Type /Page /Contents 3 0 R >>\nendobj\n")
	content := "BT\n(08\\05515\\0552024) Tj\nET\n"
	offsets[3] = input.Len()
	fmt.Fprintf(&input, "3 0 obj\n<< /Length %d >>\nstream\n%sendstream\nendobj\n", len(content), content)
	offsets[8] = input.Len()
	input.WriteString("8 0 obj\n<< /Type /XRef /Size 9 /W [1 4 1] /Index [0 4] /Length 7 >>\nstream\n")
	input.Write([]byte{0, 0, 0, 0, 0, 0, 1})
	fmt.Fprintf(&input, "\nendstream\nendobj\nstartxref\n%d\n%%%%EOF\n", offsets[8])
	return input.Bytes()
}

func testMalformedObjectStreamPDF() []byte {
	content := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	return testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Page /Contents 3 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		"<< /Type /ObjStm /N 1 /First 4 /Length 4 >>\nstream\n5 \nendstream",
	)
}

func assertPreserveStructureUnsupported(t *testing.T, err error, details ...string) {
	t.Helper()
	if !errors.Is(err, ErrPreserveStructureRepackUnsupported) {
		t.Fatalf("error = %v, want ErrPreserveStructureRepackUnsupported", err)
	}
	var structureErr *PreserveStructureUnsupportedError
	if !errors.As(err, &structureErr) {
		t.Fatalf("error = %T, want PreserveStructureUnsupportedError", err)
	}
	structureDetails := structureErr.StructureDetails()
	for _, detail := range details {
		if !strings.Contains(err.Error(), detail) {
			t.Fatalf("error = %q, want detail %q", err, detail)
		}
	}
	if structureDetails["requires_packed_writer"] != true {
		t.Fatalf("structure details = %+v, want requires_packed_writer=true", structureDetails)
	}
}

func assertPreserveStructureReportMeta(t *testing.T, meta map[string]any, wantPath string, wantCanonicalPath bool, wantStructure map[string]any) {
	t.Helper()
	if meta["writer_mode"] != string(PDFWriterModePreserveStructure) {
		t.Fatalf("writer_mode = %v, want %q; meta=%+v", meta["writer_mode"], PDFWriterModePreserveStructure, meta)
	}
	if meta["writer_path"] != wantPath || meta["used_canonical_writer_path"] != wantCanonicalPath {
		t.Fatalf("writer path metadata = %+v, want writer_path=%q used_canonical_writer_path=%t", meta, wantPath, wantCanonicalPath)
	}
	structure, ok := meta["structure_plan"].(map[string]any)
	if !ok {
		t.Fatalf("structure_plan = %#v, want map", meta["structure_plan"])
	}
	for key, want := range wantStructure {
		if got := structure[key]; got != want {
			t.Fatalf("structure_plan[%q] = %v, want %v; structure=%+v", key, got, want, structure)
		}
	}
}

func xrefSummaryContainsCompressedObject(xref xrefSummary, objectNumber, streamNumber, streamIndex int) bool {
	for _, object := range xref.Objects {
		if object.Number == objectNumber &&
			object.Compressed &&
			object.ObjectStreamNumber == streamNumber &&
			object.ObjectStreamIndex == streamIndex {
			return true
		}
	}
	return false
}
