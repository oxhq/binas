package pdf

import (
	"bytes"
	"fmt"
	"strings"
	"testing"
)

func TestXrefStreamEntriesUseWAndDefaultIndex(t *testing.T) {
	var stream bytes.Buffer
	stream.Write([]byte{0, 0, 0, 0})
	stream.Write([]byte{1, 0, 9, 0})
	stream.Write([]byte{2, 0, 7, 0})

	input := []byte("%PDF-1.5\n1 0 obj\n<<>>\nendobj\n7 0 obj\n<< /Type /ObjStm /N 1 /First 4 /Length 8 >>\nstream\n2 0 <<>>\nendstream\nendobj\n8 0 obj\n<< /Type /XRef /Size 3 /W [1 2 1] /Length 12 >>\nstream\n" + stream.String() + "\nendstream\nendobj\nstartxref\n116\n%%EOF\n")

	entries, err := parseXrefStreamEntries(input, findXrefObjectOffsets(input))
	if err != nil {
		t.Fatalf("parse xref stream entries: %v", err)
	}

	assertXrefGraphEntry(t, entries, 1, 0, 9)
	got := mustXrefGraphEntry(t, entries, 2, 0)
	if !got.Compressed || got.Offset != -1 || got.ObjectStreamNumber != 7 || got.ObjectStreamIndex != 0 {
		t.Fatalf("compressed xref entry = %+v, want object stream 7 index 0", got)
	}
}

func TestXrefStreamEntriesUseExplicitIndex(t *testing.T) {
	var stream bytes.Buffer
	stream.Write([]byte{1, 0, 23, 0})
	stream.Write([]byte{1, 0, 42, 0})

	input := []byte("%PDF-1.5\n9 0 obj\n<< /Type /XRef /Size 12 /Index [10 2] /W [1 2 1] /Length 8 >>\nstream\n" + stream.String() + "\nendstream\nendobj\nstartxref\n9\n%%EOF\n")

	entries, err := parseXrefStreamEntries(input, findXrefObjectOffsets(input))
	if err != nil {
		t.Fatalf("parse xref stream entries: %v", err)
	}

	assertXrefGraphEntry(t, entries, 10, 0, 23)
	assertXrefGraphEntry(t, entries, 11, 0, 42)
}

func TestObjectStreamEntriesUseNAndFirst(t *testing.T) {
	input := []byte("%PDF-1.5\n7 0 obj\n<< /Type /ObjStm /N 2 /First 11 /Length 30 >>\nstream\n10 0 11 11 << /A 1 >> (literal)\nendstream\nendobj\n%%EOF\n")

	entries, err := parseObjectStreamEntries(input, findXrefObjectOffsets(input))
	if err != nil {
		t.Fatalf("parse object stream entries: %v", err)
	}

	assertXrefGraphEntry(t, entries, 10, 0, bytes.Index(input, []byte("<< /A 1 >>")))
	got := mustXrefGraphEntry(t, entries, 11, 0)
	if got.Offset != bytes.Index(input, []byte("(literal)")) || !got.Compressed || got.ObjectStreamNumber != 7 || got.ObjectStreamIndex != 1 {
		t.Fatalf("object stream entry = %+v", got)
	}
}

func TestCanonicalWriterPreservesNonZeroGenerationObjectXref(t *testing.T) {
	var input bytes.Buffer
	input.WriteString("%PDF-1.4\n")
	catalogOffset := input.Len()
	input.WriteString("1 0 obj\n<< /Type /Catalog >>\nendobj\n")
	generatedOffset := input.Len()
	input.WriteString("5 2 obj\n<< /Answer 42 >>\nendobj\n")
	xrefOffset := input.Len()
	input.WriteString("xref\n")
	input.WriteString("0 1\n0000000000 65535 f \n")
	fmt.Fprintf(&input, "1 1\n%010d 00000 n \n", catalogOffset)
	fmt.Fprintf(&input, "5 1\n%010d 00002 n \n", generatedOffset)
	fmt.Fprintf(&input, "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n", xrefOffset)

	graph, err := parsePDFGraph(input.Bytes())
	if err != nil {
		t.Fatalf("parse input graph: %v", err)
	}

	output, err := writeCanonicalPDF(graph)
	if err != nil {
		t.Fatalf("write canonical PDF: %v", err)
	}

	reparsed, err := parsePDFGraph(output)
	if err != nil {
		t.Fatalf("reparse canonical output: %v\n%s", err, output)
	}
	if _, ok := reparsed.Objects[pdfObjectID{Number: 5, Generation: 2}]; !ok {
		t.Fatalf("canonical output lost object 5 2: %+v", reparsed.Objects)
	}
	outputOffset := bytes.Index(output, []byte("5 2 obj"))
	if outputOffset < 0 {
		t.Fatalf("canonical output missing 5 2 object header:\n%s", output)
	}
	expectedXref := []byte(fmt.Sprintf("5 1\n%010d 00002 n \n", outputOffset))
	if !bytes.Contains(output, expectedXref) {
		t.Fatalf("canonical xref missing non-zero generation entry %q:\n%s", expectedXref, output)
	}
	if bytes.Contains(output, []byte("/Type /XRef")) || bytes.Contains(output, []byte("/Type /ObjStm")) {
		t.Fatalf("canonical output should not preserve xref/object stream objects:\n%s", output)
	}
}

func TestHybridXRefStmGraphParsesAndCanonicalWriterDropsStreamTrailer(t *testing.T) {
	fixture := buildHybridXrefPDFFixture(t, validHybridXrefStreamData)

	graph, err := parsePDFGraph(fixture.input)
	if err != nil {
		t.Fatalf("parse hybrid xref graph: %v", err)
	}

	if !graph.Xref.HasTable || !graph.Xref.HasHybridStream || !graph.Xref.HasStream {
		t.Fatalf("hybrid xref summary = %+v", graph.Xref)
	}
	if graph.Xref.HybridStreamOffset != fixture.xrefStreamOffset {
		t.Fatalf("hybrid xref stream offset = %d, want %d", graph.Xref.HybridStreamOffset, fixture.xrefStreamOffset)
	}
	if len(graph.XrefStream) != 4 {
		t.Fatalf("xref stream entries = %d, want 4", len(graph.XrefStream))
	}
	if _, ok := graph.Objects[pdfObjectID{Number: 3, Generation: 0}]; !ok {
		t.Fatalf("graph missing hybrid xref object 3 0: %+v", graph.Objects)
	}
	if _, ok := graph.Trailer["XRefStm"]; !ok {
		t.Fatalf("graph trailer missing original /XRefStm: %+v", graph.Trailer)
	}

	output, err := writeCanonicalPDF(graph)
	if err != nil {
		t.Fatalf("write canonical hybrid PDF: %v", err)
	}
	if !bytes.Contains(output, []byte("\nxref\n0 4\n")) {
		t.Fatalf("canonical output did not rebuild table xref:\n%s", output)
	}
	if bytes.Contains(output, []byte("/XRefStm")) {
		t.Fatalf("canonical output preserved /XRefStm:\n%s", output)
	}
	if bytes.Contains(output, []byte("/Type /XRef")) {
		t.Fatalf("canonical output preserved xref stream object:\n%s", output)
	}
	if _, err := parsePDFGraph(output); err != nil {
		t.Fatalf("reparse canonical hybrid output: %v\n%s", err, output)
	}
}

func TestHybridXRefStmMalformedStreamFailsClosed(t *testing.T) {
	fixture := buildHybridXrefPDFFixture(t, func(catalogOffset, hybridObjectOffset int) []byte {
		return []byte{0, 0, 0, 0, 0, 0, 1}
	})

	_, err := parsePDFGraph(fixture.input)
	if err == nil {
		t.Fatal("expected malformed hybrid xref stream error")
	}
	if !strings.Contains(err.Error(), "hybrid xref /XRefStm stream: xref stream data ended before all entries") {
		t.Fatalf("error = %q, want focused hybrid xref stream failure", err)
	}
}

func assertXrefGraphEntry(t *testing.T, entries []xrefObjectOffset, number, generation, offset int) {
	t.Helper()
	got := mustXrefGraphEntry(t, entries, number, generation)
	if got.Offset != offset {
		t.Fatalf("object %d %d offset = %d, want %d in %+v", number, generation, got.Offset, offset, entries)
	}
}

func mustXrefGraphEntry(t *testing.T, entries []xrefObjectOffset, number, generation int) xrefObjectOffset {
	t.Helper()
	for _, entry := range entries {
		if entry.Number == number && entry.Generation == generation {
			return entry
		}
	}
	t.Fatalf("object %d %d not found in %+v", number, generation, entries)
	return xrefObjectOffset{}
}

type hybridXrefPDFFixture struct {
	input              []byte
	tableOffset        int
	catalogOffset      int
	hybridObjectOffset int
	xrefStreamOffset   int
}

func buildHybridXrefPDFFixture(t *testing.T, streamData func(catalogOffset, hybridObjectOffset int) []byte) hybridXrefPDFFixture {
	t.Helper()

	var input bytes.Buffer
	input.WriteString("%PDF-1.5\n")
	catalogOffset := input.Len()
	input.WriteString("1 0 obj\n<< /Type /Catalog >>\nendobj\n")
	hybridObjectOffset := input.Len()
	input.WriteString("3 0 obj\n<< /Hybrid true >>\nendobj\n")
	data := streamData(catalogOffset, hybridObjectOffset)
	xrefStreamOffset := input.Len()
	fmt.Fprintf(&input, "8 0 obj\n<< /Type /XRef /Size 9 /W [1 4 1] /Index [0 4] /Length %d >>\nstream\n", len(data))
	input.Write(data)
	input.WriteString("\nendstream\nendobj\n")
	tableOffset := input.Len()
	input.WriteString("xref\n")
	input.WriteString("0 2\n")
	input.WriteString("0000000000 65535 f \n")
	fmt.Fprintf(&input, "%010d 00000 n \n", catalogOffset)
	input.WriteString("8 1\n")
	fmt.Fprintf(&input, "%010d 00000 n \n", xrefStreamOffset)
	fmt.Fprintf(&input, "trailer\n<< /Size 9 /Root 1 0 R /XRefStm %d >>\nstartxref\n%d\n%%%%EOF\n", xrefStreamOffset, tableOffset)

	return hybridXrefPDFFixture{
		input:              input.Bytes(),
		tableOffset:        tableOffset,
		catalogOffset:      catalogOffset,
		hybridObjectOffset: hybridObjectOffset,
		xrefStreamOffset:   xrefStreamOffset,
	}
}

func validHybridXrefStreamData(catalogOffset, hybridObjectOffset int) []byte {
	var data bytes.Buffer
	writeXrefStreamEntry(&data, 0, 0, 0)
	writeXrefStreamEntry(&data, 1, catalogOffset, 0)
	writeXrefStreamEntry(&data, 0, 0, 0)
	writeXrefStreamEntry(&data, 1, hybridObjectOffset, 0)
	return data.Bytes()
}

func writeXrefStreamEntry(out *bytes.Buffer, entryType, field2, field3 int) {
	out.WriteByte(byte(entryType))
	out.WriteByte(byte(field2 >> 24))
	out.WriteByte(byte(field2 >> 16))
	out.WriteByte(byte(field2 >> 8))
	out.WriteByte(byte(field2))
	out.WriteByte(byte(field3))
}
