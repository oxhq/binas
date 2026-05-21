package pdf

import (
	"bytes"
	"errors"
	"fmt"
	"strings"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestIncrementalUpdateAppendsObjectsXrefTrailerAndPreservesOriginalBytes(t *testing.T) {
	input, originalXrefOffset := minimalIncrementalPDF(t)

	output, err := appendIncrementalUpdate(input, []incrementalObjectUpdate{
		{
			ID: pdfObjectID{Number: 2, Generation: 0},
			Value: pdfDict{
				"Updated": true,
				"Text":    pdfLiteralString("replacement"),
			},
		},
		{
			ID: pdfObjectID{Number: 4, Generation: 0},
			Value: pdfDict{
				"New": pdfName("Object"),
			},
		},
	}, pdfDict{"Info": pdfRef{ID: pdfObjectID{Number: 4, Generation: 0}}})
	if err != nil {
		t.Fatalf("append incremental update: %v", err)
	}

	if !bytes.HasPrefix(output, input) {
		t.Fatal("incremental output did not preserve the original PDF bytes as a prefix")
	}
	appended := output[len(input):]
	xrefSections := bytes.Count(output, []byte("\nxref\n"))
	if bytes.HasPrefix(output, []byte("xref\n")) {
		xrefSections++
	}
	if xrefSections != 2 {
		t.Fatalf("output should contain original and incremental xref sections:\n%s", output)
	}
	for _, objectNumber := range []int{2, 4} {
		if !bytes.Contains(appended, []byte(fmt.Sprintf("%d 1\n", objectNumber))) {
			t.Fatalf("incremental xref missing one-entry subsection for object %d:\n%s", objectNumber, appended)
		}
		objectOffset := bytes.LastIndex(output, []byte(fmt.Sprintf("%d 0 obj\n", objectNumber)))
		expectedEntry := []byte(fmt.Sprintf("%010d 00000 n \n", objectOffset))
		if objectOffset < len(input) || !bytes.Contains(appended, expectedEntry) {
			t.Fatalf("incremental xref missing offset entry %q for object %d:\n%s", expectedEntry, objectNumber, appended)
		}
	}
	if !bytes.Contains(appended, []byte(fmt.Sprintf("/Prev %d", originalXrefOffset))) {
		t.Fatalf("incremental trailer missing /Prev %d:\n%s", originalXrefOffset, appended)
	}
	if !bytes.Contains(appended, []byte("/Size 5")) {
		t.Fatalf("incremental trailer missing updated /Size 5:\n%s", appended)
	}
	if !bytes.Contains(appended, []byte("/Info 4 0 R")) {
		t.Fatalf("incremental trailer missing override /Info 4 0 R:\n%s", appended)
	}
	incrementalXrefOffset := bytes.LastIndex(output, []byte("\nxref\n")) + 1
	if !bytes.Contains(appended, []byte(fmt.Sprintf("startxref\n%d\n%%%%EOF", incrementalXrefOffset))) {
		t.Fatalf("incremental startxref does not point at appended xref offset %d:\n%s", incrementalXrefOffset, appended)
	}
}

func TestIncrementalUpdateGraphSeesUpdatedObjectValue(t *testing.T) {
	input, _ := minimalIncrementalPDF(t)
	output, err := appendIncrementalUpdate(input, []incrementalObjectUpdate{
		{
			ID: pdfObjectID{Number: 2, Generation: 0},
			Value: pdfDict{
				"Version": int(2),
			},
		},
	}, nil)
	if err != nil {
		t.Fatalf("append incremental update: %v", err)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatalf("parse incremental output: %v\n%s", err, output)
	}
	object := graph.Objects[pdfObjectID{Number: 2, Generation: 0}]
	if object == nil {
		t.Fatalf("updated object 2 0 not found in graph: %+v", graph.Objects)
	}
	dict, ok := object.Value.(pdfDict)
	if !ok {
		t.Fatalf("updated object value = %T, want pdfDict", object.Value)
	}
	version, _ := dictInt(dict, "Version")
	if version != 2 {
		t.Fatalf("updated object /Version = %d, want 2", version)
	}
	if object.Offset < len(input) {
		t.Fatalf("graph object offset = %d, want appended offset >= %d", object.Offset, len(input))
	}
	if prev, ok := dictInt(graph.Trailer, "Prev"); !ok || prev <= 0 {
		t.Fatalf("graph trailer /Prev = %v, want previous xref offset", graph.Trailer["Prev"])
	}
}

func TestIncrementalUpdateRejectsMissingStartxref(t *testing.T) {
	_, err := appendIncrementalUpdate([]byte("%PDF-1.4\n1 0 obj\n<<>>\nendobj\n"), []incrementalObjectUpdate{
		{ID: pdfObjectID{Number: 2, Generation: 0}, Value: pdfDict{}},
	}, nil)
	if err == nil {
		t.Fatal("expected missing startxref error")
	}
}

func TestIncrementalTextReplacementAppendsUpdatedStreamAndPreservesOriginalBytes(t *testing.T) {
	content := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	originalXrefOffset, err := lastStartXrefOffset(input)
	if err != nil {
		t.Fatalf("startxref: %v", err)
	}

	output, err := appendIncrementalContentStreamTextReplacement(input, "08-15-2024", "May 5, 2026")
	if err != nil {
		t.Fatalf("incremental text replacement: %v", err)
	}

	if !bytes.HasPrefix(output, input) {
		t.Fatal("incremental text replacement did not preserve the original PDF bytes as a prefix")
	}
	appended := output[len(input):]
	if !bytes.Contains(appended, []byte("4 0 obj\n")) {
		t.Fatalf("incremental update did not append a replacement for the original content stream object:\n%s", appended)
	}
	if bytes.Contains(appended, []byte("08\\05515\\0552024")) {
		t.Fatalf("appended replacement still contains old encoded text:\n%s", appended)
	}
	if !bytes.Contains(appended, []byte("(May 5, 2026) Tj")) {
		t.Fatalf("appended replacement missing new text-show operand:\n%s", appended)
	}
	if !bytes.Contains(appended, []byte(fmt.Sprintf("/Prev %d", originalXrefOffset))) {
		t.Fatalf("incremental trailer missing /Prev %d:\n%s", originalXrefOffset, appended)
	}

	graph, err := parsePDFGraph(output)
	if err != nil {
		t.Fatalf("parse incremental output: %v", err)
	}
	oldMatches, err := graph.textShowCandidates("08-15-2024")
	if err != nil {
		t.Fatalf("old text lookup: %v", err)
	}
	newMatches, err := graph.textShowCandidates("May 5, 2026")
	if err != nil {
		t.Fatalf("new text lookup: %v", err)
	}
	if len(oldMatches) != 0 || len(newMatches) != 1 {
		t.Fatalf("text candidates after incremental update: old=%d new=%d", len(oldMatches), len(newMatches))
	}
	if newMatches[0].Object.ID != (pdfObjectID{Number: 4, Generation: 0}) || newMatches[0].Object.Offset < len(input) {
		t.Fatalf("new text resolved through appended object: object=%+v offset=%d input_len=%d", newMatches[0].Object.ID, newMatches[0].Object.Offset, len(input))
	}
}

func TestIncrementalTextReplacementRejectsAmbiguousMatches(t *testing.T) {
	content := []byte("BT\n(repeated) Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>",
		"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		"<< /Type /Page /Parent 2 0 R /Contents 6 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	output, err := appendIncrementalContentStreamTextReplacement(input, "repeated", "unique")
	if err == nil {
		t.Fatal("expected ambiguous match error")
	}
	if output != nil {
		t.Fatal("ambiguous match should not return output")
	}
	if !strings.Contains(err.Error(), "ambiguous") {
		t.Fatalf("error = %v, want ambiguous match", err)
	}
}

func TestIncrementalTextReplacementAllowsSignedPDFWithoutRewritingOriginalBytes(t *testing.T) {
	input := signedTextPDF("08-15-2024")

	output, err := appendIncrementalContentStreamTextReplacement(input, "08-15-2024", "May 5, 2026")
	if err != nil {
		t.Fatalf("incremental signed text replacement: %v", err)
	}

	if !bytes.HasPrefix(output, input) {
		t.Fatal("incremental signed text replacement did not preserve the original PDF bytes as a prefix")
	}
	graph, err := parsePDFGraphWithOptions(output, pdfGraphParseOptions{AllowSignature: true})
	if err != nil {
		t.Fatalf("parse signed incremental output: %v", err)
	}
	oldMatches, err := graph.textShowCandidates("08-15-2024")
	if err != nil {
		t.Fatalf("old text lookup: %v", err)
	}
	newMatches, err := graph.textShowCandidates("May 5, 2026")
	if err != nil {
		t.Fatalf("new text lookup: %v", err)
	}
	if len(oldMatches) != 0 || len(newMatches) != 1 {
		t.Fatalf("text candidates after signed incremental update: old=%d new=%d", len(oldMatches), len(newMatches))
	}
	if prev, ok := dictInt(graph.Trailer, "Prev"); !ok || prev <= 0 {
		t.Fatalf("signed incremental trailer /Prev = %v, want previous xref offset", graph.Trailer["Prev"])
	}
}

func TestApplyIncrementalTextEditPreservingSignaturesReportsByteRangeProof(t *testing.T) {
	input := signedTextPDF("08-15-2024")

	output, report, verification, preservation, err := ApplyIncrementalTextEditPreservingSignatures(
		input,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "May 5, 2026"},
		nil,
	)
	if err != nil {
		t.Fatalf("ApplyIncrementalTextEditPreservingSignatures: %v", err)
	}
	if report.Edit != incrementalTextRewriteOperation || report.NodesModified != 1 || report.FallbackUsed {
		t.Fatalf("report = %+v", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v", verification)
	}
	if !preservation.IncrementalUpdate || !preservation.OriginalBytesPreserved || !preservation.ByteRangeProof || preservation.ByteRangesChecked != 2 || !preservation.SignedByteRangesUnchanged || preservation.CryptographicValidation {
		t.Fatalf("signature preservation = %+v", preservation)
	}
	if !bytes.HasPrefix(output, input) {
		t.Fatal("output does not preserve original bytes as prefix")
	}
}

func TestApplyIncrementalTextEditPreservingSignaturesUnsupportedSubFilterDoesNotClaimValidation(t *testing.T) {
	input := signedTextPDFWithSignatureDictionary(
		"08-15-2024",
		"<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /ETSI.RFC3161 /ByteRange [0 10 20 30] /Contents <01020f> >>",
	)

	output, _, verification, preservation, err := ApplyIncrementalTextEditPreservingSignatures(
		input,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "May 5, 2026"},
		nil,
	)
	if err != nil {
		t.Fatalf("ApplyIncrementalTextEditPreservingSignatures: %v", err)
	}
	if output == nil || !bytes.HasPrefix(output, input) {
		t.Fatal("output should preserve original signed bytes as a prefix")
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v", verification)
	}
	if !preservation.ByteRangeProof || preservation.ByteRangesChecked != 2 || !preservation.SignedByteRangesUnchanged {
		t.Fatalf("byte range preservation = %+v", preservation)
	}
	if preservation.CryptographicValidation {
		t.Fatalf("CryptographicValidation = true, want false for unsupported subfilter")
	}
	if !strings.Contains(preservation.CryptographicValidationNote, "not performed") {
		t.Fatalf("CryptographicValidationNote = %q, want explicit not performed note", preservation.CryptographicValidationNote)
	}

	info := inspectSignatureInfo(input)
	if info.SubFilter != "ETSI.RFC3161" {
		t.Fatalf("SubFilter = %q, want ETSI.RFC3161", info.SubFilter)
	}
	if info.CryptographicValidation || info.CryptographicValidationStatus != signatureCryptographicValidationNotPerformed {
		t.Fatalf("signature info validation = %t/%q, want false/%q", info.CryptographicValidation, info.CryptographicValidationStatus, signatureCryptographicValidationNotPerformed)
	}
}

func TestApplyIncrementalTextEditPreservingSignaturesSelectsMatchIndex(t *testing.T) {
	content := []byte("BT\n(repeated) Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>",
		"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		"<< /Type /Page /Parent 2 0 R /Contents 6 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)
	matchIndex := 1

	output, report, verification, _, err := ApplyIncrementalTextEditPreservingSignatures(
		input,
		core.Match{Kind: KindTextShow, Text: "repeated", MatchIndex: &matchIndex},
		core.Mutation{Replace: "unique"},
		nil,
	)
	if err != nil {
		t.Fatalf("ApplyIncrementalTextEditPreservingSignatures: %v", err)
	}
	if report.MatchIndex == nil || *report.MatchIndex != 1 {
		t.Fatalf("match index report = %v, want 1", report.MatchIndex)
	}
	if !verification.ReparseOK || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v", verification)
	}
	if !bytes.HasPrefix(output, input) {
		t.Fatal("output does not preserve original bytes as prefix")
	}
}

func TestApplyIncrementalTextEditPreservingSignaturesRequiresByteRangeForSignedPDF(t *testing.T) {
	content := []byte("BT\n(08\\05515\\0552024) Tj\nET\n")
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R /SigFlags 3 >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
	)

	_, _, _, _, err := ApplyIncrementalTextEditPreservingSignatures(
		input,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "May 5, 2026"},
		nil,
	)
	if !errors.Is(err, ErrSignedPDFByteRangeProofRequired) {
		t.Fatalf("error = %v, want ErrSignedPDFByteRangeProofRequired", err)
	}
}

func TestApplyIncrementalTextEditPreservingSignaturesRejectsMalformedSignatureDictionaries(t *testing.T) {
	tests := []struct {
		name      string
		signature string
	}{
		{
			name:      "missing byte range",
			signature: "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /ETSI.RFC3161 /Contents <01020f> >>",
		},
		{
			name:      "non array byte range",
			signature: "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /ETSI.RFC3161 /ByteRange (0 10 20 30) /Contents <01020f> >>",
		},
		{
			name:      "odd byte range values",
			signature: "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /ETSI.RFC3161 /ByteRange [0 10 20] /Contents <01020f> >>",
		},
		{
			name:      "wrong byte range value type",
			signature: "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /ETSI.RFC3161 /ByteRange [0 /bad 20 30] /Contents <01020f> >>",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			input := signedTextPDFWithSignatureDictionary("08-15-2024", tt.signature)

			output, report, verification, preservation, err := ApplyIncrementalTextEditPreservingSignatures(
				input,
				core.Match{Kind: KindTextShow, Text: "08-15-2024"},
				core.Mutation{Replace: "May 5, 2026"},
				nil,
			)
			if !errors.Is(err, ErrSignedPDFByteRangeProofRequired) {
				t.Fatalf("error = %v, want ErrSignedPDFByteRangeProofRequired", err)
			}
			if output != nil {
				t.Fatal("malformed signature dictionary should not return edited output")
			}
			if report.Edit != "" || verification.ReparseOK || preservation.ByteRangeProof || preservation.CryptographicValidation {
				t.Fatalf("partial metadata = report %+v verification %+v preservation %+v, want no validation claim", report, verification, preservation)
			}
		})
	}
}

func TestIncrementalTextReplacementRejectsFilteredStreams(t *testing.T) {
	content := []byte("BT\n(filtered) Tj\nET\n")
	encoded, err := encodeFlateDecode(content)
	if err != nil {
		t.Fatalf("flate encode: %v", err)
	}
	input := testPDF(
		"<< /Type /Catalog /Pages 2 0 R >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
		fmt.Sprintf("<< /Length %d /Filter /FlateDecode >>\nstream\n%sendstream", len(encoded), encoded),
	)

	output, err := appendIncrementalContentStreamTextReplacement(input, "filtered", "plain")
	if err == nil {
		t.Fatal("expected filtered stream error")
	}
	if output != nil {
		t.Fatal("filtered stream should not return output")
	}
	if !strings.Contains(err.Error(), "unsupported incremental text replacement stream filter") {
		t.Fatalf("error = %v, want unsupported filter", err)
	}
}

func TestIncrementalTextReplacementRejectsObjectAndXrefStreams(t *testing.T) {
	content := []byte("BT\n(keep) Tj\nET\n")
	tests := []struct {
		name    string
		objects []string
		want    string
	}{
		{
			name: "object stream",
			objects: []string{
				"<< /Type /Catalog /Pages 2 0 R >>",
				"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
				"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
				fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
				"<< /Type /ObjStm /N 1 /First 4 /Length 8 >>\nstream\n0 0 \nendstream",
			},
			want: "object streams",
		},
		{
			name: "xref stream",
			objects: []string{
				"<< /Type /Catalog /Pages 2 0 R >>",
				"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
				"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
				fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
				"<< /Type /XRef /Length 4 /Size 6 /W [1 2 1] >>\nstream\nabcd\nendstream",
			},
			want: "xref streams",
		},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			input := testPDF(tc.objects...)

			output, err := appendIncrementalContentStreamTextReplacement(input, "keep", "changed")
			if err == nil {
				t.Fatal("expected unsupported stream error")
			}
			if output != nil {
				t.Fatal("unsupported stream PDF should not return output")
			}
			if !strings.Contains(err.Error(), tc.want) {
				t.Fatalf("error = %v, want %q", err, tc.want)
			}
		})
	}
}

func minimalIncrementalPDF(t *testing.T) ([]byte, int) {
	t.Helper()

	var input bytes.Buffer
	input.WriteString("%PDF-1.4\n")
	catalogOffset := input.Len()
	input.WriteString("1 0 obj\n<< /Type /Catalog >>\nendobj\n")
	objectOffset := input.Len()
	input.WriteString("2 0 obj\n<< /Version 1 >>\nendobj\n")
	xrefOffset := input.Len()
	input.WriteString("xref\n")
	input.WriteString("0 3\n")
	input.WriteString("0000000000 65535 f \n")
	fmt.Fprintf(&input, "%010d 00000 n \n", catalogOffset)
	fmt.Fprintf(&input, "%010d 00000 n \n", objectOffset)
	fmt.Fprintf(&input, "trailer\n<< /Size 3 /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n", xrefOffset)
	return input.Bytes(), xrefOffset
}

func signedTextPDFWithSignatureDictionary(text, signatureDictionary string) []byte {
	content := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", encodeLiteralString(text)))
	return testPDF(
		"<< /Type /Catalog /Pages 2 0 R /SigFlags 3 /AcroForm << /Fields [5 0 R] >> >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		"<< /FT /Sig /T (Approval) /V 6 0 R >>",
		signatureDictionary,
	)
}
