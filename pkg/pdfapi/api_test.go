package pdfapi

import (
	"bytes"
	"fmt"
	"strings"
	"testing"
)

func TestInspectAndQueryTextDefaultToPDFTextShows(t *testing.T) {
	input := testPDFAPIFile("<< /Type /Page >>", streamObject("alpha"))

	tree, err := Inspect(input, Options{})
	if err != nil {
		t.Fatal(err)
	}
	if tree.Format != "pdf" {
		t.Fatalf("format = %q, want pdf", tree.Format)
	}

	matches, err := QueryText(input, TextSelector{Text: "alpha"}, Options{})
	if err != nil {
		t.Fatal(err)
	}
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].Kind != "pdf.content.text_show" {
		t.Fatalf("kind = %q, want default PDF text show kind", matches[0].Kind)
	}
}

func TestEditTextAutoUsesCanonicalRewriteAndVerifiesNoFallbackOldGoneNewSelectable(t *testing.T) {
	input := testPDFAPIFile("<< /Type /Page >>", streamObject("old"))

	output, report, verification, err := EditText(
		input,
		TextSelector{Text: "old"},
		TextReplacement{Replace: "new"},
		Options{},
	)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Equal(output, input) {
		t.Fatal("output did not change")
	}
	if report.FallbackUsed {
		t.Fatalf("fallback used: %+v", report)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" {
		t.Fatalf("edit = %q, want canonical rewrite", report.Edit)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v, want reparse/old gone/new selectable/page unchanged", verification)
	}

	oldMatches, err := QueryText(output, TextSelector{Text: "old"}, Options{})
	if err != nil {
		t.Fatal(err)
	}
	if len(oldMatches) != 0 {
		t.Fatalf("old matches = %d, want 0", len(oldMatches))
	}
	newMatches, err := QueryText(output, TextSelector{Text: "new"}, Options{})
	if err != nil {
		t.Fatal(err)
	}
	if len(newMatches) != 1 {
		t.Fatalf("new matches = %d, want 1", len(newMatches))
	}
}

func TestDSLRewritesTextWithVerificationInvariants(t *testing.T) {
	input := testPDFAPIFile("<< /Type /Page >>", streamObject("Invoice #1234"))

	output, report, verification, err := New(input).
		Rewrite(RewriteModeAuto).
		FindText("Invoice #1234").
		ReplaceWith("Invoice #5678").
		Verify("reparse", "old-gone", "new-selectable", "page-count-unchanged", "no-fallback").
		Bytes()
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Equal(output, input) {
		t.Fatal("output did not change")
	}
	if report.FallbackUsed {
		t.Fatalf("report = %+v, want no fallback", report)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v, want reparse/old gone/new selectable/page unchanged", verification)
	}

	matches, err := New(output).FindText("Invoice #5678").Query()
	if err != nil {
		t.Fatal(err)
	}
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want rewritten text query match", len(matches))
	}
}

func TestQueryTextHonorsMatchIndex(t *testing.T) {
	input := testPDFAPIFile(
		"<< /Type /Page >>",
		streamObject("repeat"),
		streamObject("repeat"),
	)
	index := 1

	matches, err := QueryText(input, TextSelector{Text: "repeat", MatchIndex: &index}, Options{})
	if err != nil {
		t.Fatal(err)
	}
	if len(matches) != 1 {
		t.Fatalf("matches = %d, want 1", len(matches))
	}
	if matches[0].ID == 0 {
		t.Fatalf("unexpected root match: %+v", matches[0])
	}
}

func TestValidateReturnsMinimalParseVerification(t *testing.T) {
	input := testPDFAPIFile("<< /Type /Page >>", streamObject("ok"))

	verification, err := Validate(input, Options{})
	if err != nil {
		t.Fatal(err)
	}
	if !verification.ReparseOK {
		t.Fatalf("verification = %+v, want ReparseOK", verification)
	}
}

func TestEditTextRejectsUnknownRewriteAndSignatureModes(t *testing.T) {
	input := testPDFAPIFile("<< /Type /Page >>", streamObject("old"))

	_, _, _, err := EditText(input, TextSelector{Text: "old"}, TextReplacement{Replace: "new"}, Options{Rewrite: RewriteMode("sideways")})
	if err == nil || !strings.Contains(err.Error(), "unsupported rewrite mode") {
		t.Fatalf("rewrite error = %v, want unsupported rewrite mode", err)
	}

	_, _, _, err = EditText(input, TextSelector{Text: "old"}, TextReplacement{Replace: "new"}, Options{SignatureMode: "sideways"})
	if err == nil || !strings.Contains(err.Error(), "unsupported signature mode") {
		t.Fatalf("signature error = %v, want unsupported signature mode", err)
	}
}

func streamObject(text string) string {
	content := fmt.Sprintf("BT\n(%s) Tj\nET\n", text)
	return fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content)
}

func testPDFAPIFile(objects ...string) []byte {
	var out bytes.Buffer
	out.WriteString("%PDF-1.3\n")
	offsets := make([]int, 0, len(objects))
	for i, object := range objects {
		offsets = append(offsets, out.Len())
		fmt.Fprintf(&out, "%d 0 obj\n%s\nendobj\n", i+1, object)
	}
	xrefOffset := out.Len()
	fmt.Fprintf(&out, "xref\n0 %d\n", len(objects)+1)
	out.WriteString("0000000000 65535 f \n")
	for _, offset := range offsets {
		fmt.Fprintf(&out, "%010d 00000 n \n", offset)
	}
	fmt.Fprintf(&out, "trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n", len(objects)+1, xrefOffset)
	return out.Bytes()
}
