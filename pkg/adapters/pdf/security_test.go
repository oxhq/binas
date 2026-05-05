package pdf

import (
	"errors"
	"fmt"
	"testing"

	"github.com/oxhq/binas/pkg/core"
)

func TestSecurityEncryptUsesPasswordPathErrorContract(t *testing.T) {
	input := testPDF("<< /Type /Catalog /Encrypt 2 0 R >>", "<<>>")

	_, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if !errors.Is(err, ErrEncryptedPDFPasswordRequired) {
		t.Fatalf("Parse() error = %v, want ErrEncryptedPDFPasswordRequired", err)
	}

	_, err = parsePDFGraphWithOptions(input, pdfGraphParseOptions{AllowEncryption: true})
	if !errors.Is(err, ErrEncryptedPDFPasswordRequired) {
		t.Fatalf("parsePDFGraphWithOptions(AllowEncryption) error = %v, want ErrEncryptedPDFPasswordRequired", err)
	}
}

func TestSecuritySignatureDefaultsFailClosed(t *testing.T) {
	input := signedTextPDF("08-15-2024")

	_, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if !errors.Is(err, ErrSignedPDFRequiresInvalidation) {
		t.Fatalf("Parse() error = %v, want ErrSignedPDFRequiresInvalidation", err)
	}

	_, _, _, err = ApplyCanonicalEdit(input, core.Match{Kind: KindTextShow, Text: "08-15-2024"}, core.Mutation{Replace: "May 5, 2026"}, nil)
	if !errors.Is(err, ErrSignedPDFRequiresInvalidation) {
		t.Fatalf("ApplyCanonicalEdit() error = %v, want ErrSignedPDFRequiresInvalidation", err)
	}
}

func TestSecuritySignatureByteRangeOnlyDefaultsFailClosed(t *testing.T) {
	input := testPDF("<< /Type /Catalog /ByteRange [0 10 20 30] >>", "<<>>")

	_, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if !errors.Is(err, ErrSignedPDFRequiresInvalidation) {
		t.Fatalf("Parse() error = %v, want ErrSignedPDFRequiresInvalidation", err)
	}
}

func TestSecuritySignatureSigFlagsOnlyDefaultsFailClosed(t *testing.T) {
	input := testPDF("<< /Type /Catalog /SigFlags 3 >>", "<<>>")

	_, err := NewAdapter().Parse(input, core.ParseOptions{Strict: true})
	if !errors.Is(err, ErrSignedPDFRequiresInvalidation) {
		t.Fatalf("Parse() error = %v, want ErrSignedPDFRequiresInvalidation", err)
	}
}

func TestSecuritySignatureInvalidationHelperCanonicalWritesWhenExplicit(t *testing.T) {
	input := signedTextPDF("08-15-2024")

	output, report, verification, err := ApplyCanonicalEditInvalidatingSignatures(
		input,
		core.Match{Kind: KindTextShow, Text: "08-15-2024"},
		core.Mutation{Replace: "May 5, 2026"},
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	if report.Edit != "pdf.canonical_content_stream_text_rewrite" {
		t.Fatalf("report edit = %q", report.Edit)
	}
	if !verification.ReparseOK || !verification.OldTextRemoved || !verification.NewSelectable || !verification.PageUnchanged {
		t.Fatalf("verification = %+v", verification)
	}

	_, err = NewAdapter().Parse(output, core.ParseOptions{Strict: true})
	if !errors.Is(err, ErrSignedPDFRequiresInvalidation) {
		t.Fatalf("default Parse(output) error = %v, want signed-PDF refusal", err)
	}
}

func signedTextPDF(text string) []byte {
	content := []byte(fmt.Sprintf("BT\n(%s) Tj\nET\n", encodeLiteralString(text)))
	return testPDF(
		"<< /Type /Catalog /Pages 2 0 R /SigFlags 3 /AcroForm << /Fields [5 0 R] >> >>",
		"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
		"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
		fmt.Sprintf("<< /Length %d >>\nstream\n%sendstream", len(content), content),
		"<< /FT /Sig /T (Approval) /V 6 0 R >>",
		"<< /Type /Sig /ByteRange [0 10 20 30] /Contents <00> >>",
	)
}
