package pdf

import (
	"bytes"
	"errors"
	"strings"
	"testing"
)

func TestMutateStreamReplacesDirectStreamRefAndReparses(t *testing.T) {
	input := testPDFWithObjects(
		testPDFObject{number: 1, body: "<< /Type /Catalog /Pages 2 0 R >>"},
		testPDFObject{number: 2, body: "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"},
		testPDFObject{number: 3, body: "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>"},
		testPDFObject{number: 4, body: "<< /Length 5 >>\nstream\nHello\nendstream"},
	)

	output, report, verification, err := MutateStream(input, StreamMutationOptions{
		ObjectNumber: 4,
		Generation:   0,
		Replacement:  []byte("BT\n(World) Tj\nET"),
		DictUpdates:  map[string]any{"Stage": StreamName("Mutated")},
	})
	if err != nil {
		t.Fatalf("MutateStream error: %v", err)
	}
	if report.Edit != "pdf.stream_mutation" || report.ObjectNumber != 4 || report.NodesModified != 1 {
		t.Fatalf("report = %+v", report)
	}
	if !verification.ReparseOK || !verification.NoDanglingRefs {
		t.Fatalf("verification = %+v", verification)
	}
	if !bytes.Contains(output, []byte("/Length 16")) {
		t.Fatalf("updated stream length missing:\n%s", output)
	}
	if !bytes.Contains(output, []byte("/Stage /Mutated")) {
		t.Fatalf("dict update missing:\n%s", output)
	}
	if bytes.Contains(output, []byte("Hello")) || !bytes.Contains(output, []byte("World")) {
		t.Fatalf("stream bytes not replaced:\n%s", output)
	}
	if _, err := parsePDFGraph(output); err != nil {
		t.Fatalf("reparse output: %v", err)
	}
}

func TestMutateStreamRejectsDanglingDictUpdate(t *testing.T) {
	input := testPDFWithObjects(
		testPDFObject{number: 1, body: "<< /Type /Catalog /Pages 2 0 R >>"},
		testPDFObject{number: 2, body: "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"},
		testPDFObject{number: 3, body: "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>"},
		testPDFObject{number: 4, body: "<< /Length 5 >>\nstream\nHello\nendstream"},
	)

	_, _, _, err := MutateStream(input, StreamMutationOptions{
		ObjectNumber: 4,
		Replacement:  []byte("World"),
		DictUpdates:  map[string]any{"Missing": StreamObjectRef{ObjectNumber: 99, Generation: 0}},
	})
	if err == nil {
		t.Fatal("expected dangling reference error")
	}
	if !strings.Contains(err.Error(), "dangling indirect references") {
		t.Fatalf("error = %v", err)
	}
}

func TestReplaceImageXObjectReplacesSimpleImageStreamRef(t *testing.T) {
	input := testPDFWithObjects(
		testPDFObject{number: 1, body: "<< /Type /Catalog /Pages 2 0 R >>"},
		testPDFObject{number: 2, body: "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"},
		testPDFObject{number: 3, body: "<< /Type /Page /Parent 2 0 R /Resources << /XObject << /Im1 4 0 R >> >> /Contents 5 0 R >>"},
		testPDFObject{number: 4, body: "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 1 >>\nstream\nA\nendstream"},
		testPDFObject{number: 5, body: "<< /Length 7 >>\nstream\n/Im1 Do\nendstream"},
	)

	output, report, verification, err := ReplaceImageXObject(input, ReplaceImageXObjectOptions{
		ObjectNumber: 4,
		Generation:   0,
		ImageData:    []byte{0x10, 0x20, 0x30},
		DictUpdates:  map[string]any{"Width": 3},
	})
	if err != nil {
		t.Fatalf("ReplaceImageXObject error: %v", err)
	}
	if report.Edit != "pdf.image_xobject_replace" || !report.ImageXObject {
		t.Fatalf("report = %+v", report)
	}
	if !verification.ReparseOK || !verification.NoDanglingRefs {
		t.Fatalf("verification = %+v", verification)
	}
	if !bytes.Contains(output, []byte("/Length 3")) || !bytes.Contains(output, []byte("/Width 3")) {
		t.Fatalf("image stream dict not updated:\n%s", output)
	}
	if !bytes.Contains(output, []byte{0x10, 0x20, 0x30}) {
		t.Fatalf("image bytes missing:\n%v", output)
	}
}

func TestReplaceImageXObjectRejectsNonImageStream(t *testing.T) {
	input := testPDFWithObjects(
		testPDFObject{number: 1, body: "<< /Type /Catalog >>"},
		testPDFObject{number: 4, body: "<< /Length 5 >>\nstream\nHello\nendstream"},
	)

	_, _, _, err := ReplaceImageXObject(input, ReplaceImageXObjectOptions{
		ObjectNumber: 4,
		ImageData:    []byte("World"),
	})
	if err == nil {
		t.Fatal("expected non-image rejection")
	}
	if !errors.Is(err, ErrUnsupportedImageMutation) {
		t.Fatalf("error = %v, want ErrUnsupportedImageMutation", err)
	}
}

func TestReplaceImageXObjectRejectsFilteredImageStream(t *testing.T) {
	input := testPDFWithObjects(
		testPDFObject{number: 1, body: "<< /Type /Catalog >>"},
		testPDFObject{number: 4, body: "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /DCTDecode /Length 4 >>\nstream\nJPEG\nendstream"},
	)

	_, _, _, err := ReplaceImageXObject(input, ReplaceImageXObjectOptions{
		ObjectNumber: 4,
		ImageData:    []byte("raw"),
	})
	if err == nil {
		t.Fatal("expected filtered image rejection")
	}
	if !errors.Is(err, ErrUnsupportedImageMutation) {
		t.Fatalf("error = %v, want ErrUnsupportedImageMutation", err)
	}
	if !strings.Contains(err.Error(), "filter") {
		t.Fatalf("error = %v, want filter policy", err)
	}
}

func TestReplaceInlineImageFailsClosed(t *testing.T) {
	_, _, _, err := ReplaceInlineImage([]byte("%PDF-1.3\n%%EOF\n"), ReplaceInlineImageOptions{})
	if err == nil {
		t.Fatal("expected unsupported inline image error")
	}
	if !errors.Is(err, ErrUnsupportedImageMutation) {
		t.Fatalf("error = %v, want ErrUnsupportedImageMutation", err)
	}
}
