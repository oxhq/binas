package pdf

import (
	"bytes"
	"fmt"
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
