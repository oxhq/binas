package pdf

import (
	"bytes"
	"testing"
)

func TestXrefSummaryFindsIndirectObjectOffsetsAndTableMarker(t *testing.T) {
	input := []byte("%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\n12 2 obj\n<< /Length 0 >>\nstream\n\nendstream\nendobj\nxref\n0 13\n0000000000 65535 f \ntrailer\n<< /Size 13 >>\nstartxref\n96\n%%EOF\n")

	summary := summarizeXref(input)

	if !summary.HasTable {
		t.Fatal("expected table xref marker")
	}
	expectedTableOffset := bytes.Index(input, []byte("\nxref\n")) + 1
	if summary.TableOffset != expectedTableOffset {
		t.Fatalf("table offset = %d, want %d", summary.TableOffset, expectedTableOffset)
	}
	if summary.HasStream {
		t.Fatal("unexpected xref stream marker")
	}
	if summary.UnsupportedXrefStream {
		t.Fatal("table xref should not be marked unsupported")
	}
	if summary.HasObjectStream {
		t.Fatal("unexpected object stream marker")
	}
	if summary.UnsupportedObjectStream {
		t.Fatal("normal stream should not be marked as unsupported object stream")
	}
	assertObjectOffset(t, summary.Objects, 1, 0, 9)
	assertObjectOffset(t, summary.Objects, 12, 2, 45)
}

func TestXrefSummaryDetectsUnsupportedXrefStreamObject(t *testing.T) {
	input := []byte("%PDF-1.5\n1 0 obj\n<< /Type /Catalog >>\nendobj\n8 0 obj\n<< /Type /XRef /Length 4 /Size 9 /W [1 2 1] >>\nstream\nabcd\nendstream\nendobj\nstartxref\n45\n%%EOF\n")

	summary := summarizeXref(input)

	if summary.HasTable {
		t.Fatal("unexpected table xref marker")
	}
	if !summary.HasStream {
		t.Fatal("expected xref stream marker")
	}
	if !summary.UnsupportedXrefStream {
		t.Fatal("xref stream should be marked unsupported")
	}
	if summary.HasObjectStream {
		t.Fatal("xref stream should not be marked as object stream")
	}
	if summary.UnsupportedObjectStream {
		t.Fatal("xref stream should not set object stream unsupported flag")
	}
	if len(summary.StreamObjects) != 1 {
		t.Fatalf("stream objects = %d, want 1", len(summary.StreamObjects))
	}
	got := summary.StreamObjects[0]
	if got.Number != 8 || got.Generation != 0 || got.Offset != 45 {
		t.Fatalf("xref stream object = %+v, want object 8 0 at offset 45", got)
	}
	assertObjectOffset(t, summary.Objects, 1, 0, 9)
	assertObjectOffset(t, summary.Objects, 8, 0, 45)
}

func TestXrefSummaryDetectsSupportedObjectStreamObject(t *testing.T) {
	input := []byte("%PDF-1.5\n1 0 obj\n<< /Type /Catalog >>\nendobj\n3 0 obj\n<< /Length 4 >>\nstream\nwxyz\nendstream\nendobj\n7 0 obj\n<< /Type /ObjStm /N 1 /First 4 /Length 8 >>\nstream\n0 0 \nendstream\nendobj\nxref\n0 8\n0000000000 65535 f \ntrailer\n<< /Size 8 >>\nstartxref\n145\n%%EOF\n")

	summary := summarizeXref(input)

	if summary.HasStream {
		t.Fatal("object stream should not be marked as xref stream")
	}
	if summary.UnsupportedXrefStream {
		t.Fatal("object stream should not set xref stream unsupported flag")
	}
	if !summary.HasObjectStream {
		t.Fatal("expected object stream marker")
	}
	if summary.UnsupportedObjectStream {
		t.Fatal("object stream should be supported")
	}
	if len(summary.ObjectStreamObjects) != 1 {
		t.Fatalf("object stream objects = %d, want 1", len(summary.ObjectStreamObjects))
	}
	normalStreamOffset := bytes.Index(input, []byte("3 0 obj"))
	objectStreamOffset := bytes.Index(input, []byte("7 0 obj"))
	got := summary.ObjectStreamObjects[0]
	if got.Number != 7 || got.Generation != 0 || got.Offset != objectStreamOffset {
		t.Fatalf("object stream object = %+v, want object 7 0 at offset %d", got, objectStreamOffset)
	}
	if containsObjectOffset(summary.ObjectStreamObjects, 3, 0, normalStreamOffset) {
		t.Fatal("normal stream object was marked as object stream")
	}
}

func TestObjectStreamSummaryAvoidsFalsePositiveDictionaryMentions(t *testing.T) {
	tests := []struct {
		name       string
		dictionary string
	}{
		{
			name:       "bare ObjStm key",
			dictionary: "<< /ObjStm true /N 1 /First 4 /Length 8 >>",
		},
		{
			name:       "subtype ObjStm",
			dictionary: "<< /Subtype /ObjStm /Length 8 >>",
		},
		{
			name:       "literal value mentions ObjStm type",
			dictionary: "<< /Note (/Type /ObjStm) /Length 8 >>",
		},
		{
			name:       "nested dictionary mentions ObjStm type",
			dictionary: "<< /Metadata << /Type /ObjStm >> /Length 8 >>",
		},
	}

	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			input := []byte("%PDF-1.5\n1 0 obj\n<< /Type /Catalog >>\nendobj\n7 0 obj\n" + test.dictionary + "\nstream\n0 0 \nendstream\nendobj\nxref\n0 8\n0000000000 65535 f \ntrailer\n<< /Size 8 >>\nstartxref\n88\n%%EOF\n")

			summary := summarizeXref(input)

			if summary.HasObjectStream {
				t.Fatalf("dictionary %q should not be marked as an object stream", test.dictionary)
			}
			if summary.UnsupportedObjectStream {
				t.Fatalf("dictionary %q should not set unsupported object stream", test.dictionary)
			}
			if len(summary.ObjectStreamObjects) != 0 {
				t.Fatalf("object stream objects = %+v, want none", summary.ObjectStreamObjects)
			}
		})
	}
}

func TestXrefSummaryDoesNotClassifyContentStreamLiteralAsObjectStream(t *testing.T) {
	input := []byte("%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\n3 0 obj\n<< /Length 21 >>\nstream\nBT\n(/Type /ObjStm) Tj\nET\nendstream\nendobj\nxref\n0 4\n0000000000 65535 f \ntrailer\n<< /Size 4 >>\nstartxref\n122\n%%EOF\n")

	summary := summarizeXref(input)

	if summary.HasObjectStream {
		t.Fatal("content stream literal should not be marked as object stream")
	}
	if summary.UnsupportedObjectStream {
		t.Fatal("content stream literal should not set object stream unsupported flag")
	}
	if len(summary.ObjectStreamObjects) != 0 {
		t.Fatalf("object stream objects = %+v, want none", summary.ObjectStreamObjects)
	}
}

func TestXrefSummaryDoesNotClassifyContentStreamLiteralAsXrefStream(t *testing.T) {
	input := []byte("%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\n3 0 obj\n<< /Length 19 >>\nstream\nBT\n(/Type /XRef) Tj\nET\nendstream\nendobj\nxref\n0 4\n0000000000 65535 f \ntrailer\n<< /Size 4 >>\nstartxref\n120\n%%EOF\n")

	summary := summarizeXref(input)

	if summary.HasStream {
		t.Fatal("content stream literal should not be marked as xref stream")
	}
	if summary.UnsupportedXrefStream {
		t.Fatal("content stream literal should not set xref stream unsupported flag")
	}
	if len(summary.StreamObjects) != 0 {
		t.Fatalf("xref stream objects = %+v, want none", summary.StreamObjects)
	}
}

func TestXrefSummaryDoesNotTreatStartxrefAsTableMarker(t *testing.T) {
	input := []byte("%PDF-1.4\n1 0 obj\n<<>>\nendobj\nstartxref\n9\n%%EOF\n")

	summary := summarizeXref(input)

	if summary.HasTable {
		t.Fatalf("unexpected table xref at offset %d", summary.TableOffset)
	}
	if summary.HasStream {
		t.Fatal("unexpected xref stream")
	}
	if summary.HasObjectStream {
		t.Fatal("unexpected object stream")
	}
	assertObjectOffset(t, summary.Objects, 1, 0, 9)
}

func assertObjectOffset(t *testing.T, objects []xrefObjectOffset, number, generation, offset int) {
	t.Helper()
	for _, object := range objects {
		if object.Number == number && object.Generation == generation && object.Offset == offset {
			return
		}
	}
	t.Fatalf("object %d %d at offset %d not found in %+v", number, generation, offset, objects)
}

func containsObjectOffset(objects []xrefObjectOffset, number, generation, offset int) bool {
	for _, object := range objects {
		if object.Number == number && object.Generation == generation && object.Offset == offset {
			return true
		}
	}
	return false
}
